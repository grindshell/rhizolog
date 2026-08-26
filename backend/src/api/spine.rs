//! `POST /api/split` and `POST /api/merge`: cutting a page in two, and folding
//! one back into another.
//!
//! Both live outside the slug namespace for the reason `/api/move` does: a slug
//! is a catch-all route, so `/api/pages/{slug}/split` cannot be routed and a
//! literal `/api/pages/split` would shadow any page actually slugged `split`.
//!
//! Both do the same three things in the same order. They write the pages, they
//! repair every `contents:` list that named the page they changed, and they
//! record a marker in the word log rather than a churn. The arithmetic is in
//! [`crate::spine`], which knows nothing about HTTP; this module is the seam.
//!
//! **The lists are repaired, not rebuilt.** An entry is matched as the string
//! somebody typed, so a gap, a repeat and a typo all keep their positions across
//! one, which is the same promise the Reorder view makes and for the same
//! reason: those are the entries a compile can say least about and the ones
//! most easily lost.

use axum::Json;
use axum::extract::State;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AppState;
use crate::api::extract::{Actor, Json as JsonBody};
use crate::api::pages::{PageView, by, readable};
use crate::auth::Viewer;
use crate::error::{AppError, AppResult};
use crate::page::Frontmatter;
use crate::slug::Slug;
use crate::spine;
use crate::store::StoreError;
use crate::words;

// ----------------------------------------------------------------- requests

/// Where to cut a page, and where the second half goes.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SplitPage {
    /// The page to cut. `404` if there is nothing there.
    #[schema(example = "book/one/the-ferry")]
    pub from: Slug,
    /// Where to cut it: a **byte** offset into the body, not a character index
    /// and not a line.
    ///
    /// The same unit a `prose/v1` finding's span uses, so a client that can
    /// reveal a finding in an editor can already produce one of these. It has to
    /// fall between characters and leave text on both sides, or the request is
    /// refused with the body's length so the offset can be worked out again.
    #[schema(example = 1840)]
    pub at: usize,
    /// Where the second half goes. `409` if a page is already there.
    #[schema(example = "book/one/the-crossing")]
    pub to: Slug,
    /// A title for the new page. Without one it falls back to the first heading
    /// of the half that was cut off, and then to the slug, exactly as any other
    /// page does.
    #[serde(default)]
    #[schema(example = "The Crossing")]
    pub title: Option<String>,
}

/// Which page is folded into which.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MergePages {
    /// The page to fold in. It is deleted, and every `contents:` entry naming it
    /// goes with it.
    #[schema(example = "book/one/the-crossing")]
    pub from: Slug,
    /// The page it joins. Its body comes first and its frontmatter is untouched.
    #[schema(example = "book/one/the-ferry")]
    pub into: Slug,
}

// ---------------------------------------------------------------- responses

/// A `contents:` list that was rewritten to keep a document in one piece.
#[derive(Debug, Serialize, ToSchema)]
pub struct Repair {
    /// The page whose list changed.
    #[schema(example = "book/one")]
    pub slug: Slug,
    /// What its `contents:` list says now, in order, as written.
    #[schema(example = json!(["book/one/opening", "book/one/the-ferry", "book/one/the-crossing"]))]
    pub contents: Vec<String>,
}

/// Both halves of a page that was cut in two.
#[derive(Debug, Serialize, ToSchema)]
pub struct SplitResult {
    /// The page that was split. Its body is everything before the offset.
    pub head: PageView,
    /// The page that was made. Its body is everything after.
    ///
    /// It inherits the tags, the visibility, the owner, the readers, the due
    /// date, the stage and the compile flag, and it inherits **no** synopsis and
    /// **no** target. A synopsis is a claim about what a chapter does and the
    /// half cut off it is not that chapter; a target is a quantity, and halving
    /// one would be arithmetic nobody did while copying one would double what the
    /// book is aiming at.
    pub tail: PageView,
    /// Every list that gained the new page, and what each says now.
    ///
    /// Empty when nothing assembled the page that was split, which is the
    /// ordinary case for a page that is not part of a manuscript.
    pub repaired: Vec<Repair>,
}

/// A page that grew by a merge, and what it cost.
#[derive(Debug, Serialize, ToSchema)]
pub struct MergeResult {
    /// The page that grew, with both bodies in it and its own frontmatter
    /// unchanged.
    pub page: PageView,
    /// The page that is gone.
    #[schema(example = "book/one/the-crossing")]
    pub removed: Slug,
    /// Every list that lost an entry, and what each says now.
    pub repaired: Vec<Repair>,
}

// ----------------------------------------------------------------- handlers

/// Cut a page in two at an offset.
///
/// The second half becomes a page of its own and is inserted into every
/// `contents:` list that named the first, immediately after it. A list that
/// already names the destination is left alone: splitting into a chapter
/// somebody outlined and never wrote is filling their gap, and a second entry
/// for it would be a `duplicate` for them to clean up.
///
/// Nothing is written and nothing is unwritten, so the word log records two
/// markers rather than a chapter losing two thousand words and another gaining
/// them.
#[utoipa::path(
    post,
    path = "/api/split",
    tag = "pages",
    request_body = SplitPage,
    responses(
        (status = 200, description = "Both halves, and every list that was repaired", body = SplitResult),
        (status = 400, description = "The offset does not divide the body", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at the source slug", body = crate::error::ErrorResponse),
        (status = 409, description = "The destination is taken, or the source assembles other pages", body = crate::error::ErrorResponse),
    ),
)]
pub async fn split_page(
    State(state): State<AppState>,
    viewer: Viewer,
    actor: Actor,
    JsonBody(request): JsonBody<SplitPage>,
) -> AppResult<Json<SplitResult>> {
    let source = state.store.read(&request.from).await?;
    if !readable(&source, &viewer) {
        return Err(AppError::Store(StoreError::NotFound { slug: request.from }));
    }
    if source.contents().is_some() {
        return Err(AppError::PageAssemblesOthers {
            slug: request.from,
            consequence: "the half cut off it would compile after every one of them",
        });
    }

    // Checked before anything is written, so a destination that is taken costs a
    // refusal rather than a half-finished split. `Store::create` refuses it too;
    // this is what makes the refusal arrive before the source has been touched.
    if state.store.exists(&request.to).await? {
        return Err(AppError::Store(StoreError::AlreadyExists {
            slug: request.to,
        }));
    }

    let (head, tail) =
        spine::divide(&source.body, request.at).map_err(|error| AppError::SplitOffsetInvalid {
            slug: request.from.clone(),
            at: request.at,
            length: source.body.len(),
            reason: error.reason(),
        })?;

    let frontmatter = Frontmatter {
        title: request.title,
        // Never inherited. A synopsis is a claim about what a chapter does, and
        // the half cut off one is not that chapter. An empty card is a chapter
        // nobody has decided about yet, which is exactly what this is.
        synopsis: None,
        tags: source.frontmatter.tags.clone(),
        created: Some(Utc::now()),
        // Inherited, and this is the one that would be a defect rather than a
        // surprise: splitting a private page must never leave half of it
        // readable by somebody the whole of it was not.
        visibility: source.frontmatter.visibility.clone(),
        owner: source.frontmatter.owner.clone(),
        readers: source.frontmatter.readers.clone(),
        // Never inherited, unlike `due` below, and the difference is what each
        // one is. A date is not divisible and both halves are due the same day.
        // A target is a quantity: copying it would double what the book aims at,
        // and halving it would be arithmetic nobody did.
        target: None,
        due: source.frontmatter.due.clone(),
        stage: source.frontmatter.stage.clone(),
        compile: source.frontmatter.compile,
        // Refused above, so this is always empty. Spelled out rather than cloned
        // so that relaxing the refusal cannot silently hand one page's chapters
        // to another.
        contents: None,
    };

    // The new page first. A create that fails leaves the source exactly as it
    // was; truncating the source first and then failing to write the tail would
    // lose the half nothing else has a copy of.
    let tail_page = state.store.create(&request.to, frontmatter, &tail).await?;
    let head_page = state
        .store
        .write(&request.from, source.frontmatter.clone(), &head)
        .await?;

    state.index.upsert(&tail_page).await?;
    state.index.upsert(&head_page).await?;

    // Before the repair, and that order is the whole of what keeps the log
    // honest. These two lines describe the two pages above, which are already on
    // disk; the repair is about other files and can fail. Written afterwards,
    // one unwritable parent would take both markers with it and leave the log
    // saying a page is a length it no longer is, silently and for good: the
    // index already holds the new size, so the next startup scan reads the file
    // as unchanged and never looks. See `knowledge-base/split-and-merge.md`.
    words::split(
        &state.words,
        &state.index,
        words::Half {
            slug: &request.from,
            total: head_page.words(),
        },
        words::Half {
            slug: &request.to,
            total: tail_page.words(),
        },
        &by(&actor, &viewer),
        Utc::now(),
    )
    .await;

    let entry = request.to.to_string();
    let named = request.from.to_string();
    let repaired = repair(&state, &viewer, &request.from, |list| {
        spine::insert_after(list, &named, &entry)
    })
    .await?;

    Ok(Json(SplitResult {
        head: PageView::new(&head_page, false),
        tail: PageView::new(&tail_page, false),
        repaired,
    }))
}

/// Fold one page into another and delete it.
///
/// The destination's body comes first, then a blank line, then the source's. The
/// destination's frontmatter is untouched: its target still says what it said,
/// now over more words, which is a thing for its author to decide about rather
/// than for two numbers to be added together behind them.
///
/// Every `contents:` entry naming the source is removed, in every list, because
/// a page listed twice is gone twice once it is gone.
///
/// **A merge moves text to where the caller said**, which is the difference
/// between it and a split: a split derives the second half's position, and a
/// derived position that would silently restructure a document is refused.
#[utoipa::path(
    post,
    path = "/api/merge",
    tag = "pages",
    request_body = MergePages,
    responses(
        (status = 200, description = "The page that grew, and every list that was repaired", body = MergeResult),
        (status = 400, description = "A page cannot be merged into itself", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at one of the slugs", body = crate::error::ErrorResponse),
        (status = 409, description = "The page being merged away assembles other pages", body = crate::error::ErrorResponse),
    ),
)]
pub async fn merge_pages(
    State(state): State<AppState>,
    viewer: Viewer,
    actor: Actor,
    JsonBody(request): JsonBody<MergePages>,
) -> AppResult<Json<MergeResult>> {
    // First, because everything below assumes two pages. Left to run, a merge
    // into itself would append a page to itself and then delete it.
    if request.from == request.into {
        return Err(AppError::MergeIntoItself { slug: request.from });
    }

    let source = state.store.read(&request.from).await?;
    if !readable(&source, &viewer) {
        return Err(AppError::Store(StoreError::NotFound { slug: request.from }));
    }
    if source.contents().is_some() {
        return Err(AppError::PageAssemblesOthers {
            slug: request.from,
            consequence: "merging it away would leave every one of them named by nothing",
        });
    }

    let destination = state.store.read(&request.into).await?;
    if !readable(&destination, &viewer) {
        return Err(AppError::Store(StoreError::NotFound { slug: request.into }));
    }

    let body = spine::join(&destination.body, &source.body);
    let page = state
        .store
        .write(&request.into, destination.frontmatter.clone(), &body)
        .await?;
    state.index.upsert(&page).await?;

    // Only once the words are safely in two places. The other order risks the
    // one outcome a merge must never have, which is neither.
    state.store.delete(&request.from).await?;
    state.index.remove(&request.from).await?;
    // The same rule a delete through the API follows: a deliberate act on a page
    // takes its pin with it.
    state.index.unpin(&request.from).await?;

    // Before the repair, for the reason a split's markers are: these describe the
    // two pages already written, the repair is about other files and can fail,
    // and a marker skipped here is a lie the next scan cannot find. The delete
    // marker is the half that would hurt most, because nothing else closes the
    // series at a slug that no longer holds a page.
    let recorded_by = by(&actor, &viewer);
    let at = Utc::now();
    words::merged(
        &state.words,
        &state.index,
        &request.from,
        &request.into,
        &recorded_by,
        at,
        page.words(),
    )
    .await;
    // And the ordinary delete marker, which is what closes the series at the
    // slug that is now empty.
    words::deleted(&state.words, &state.index, &request.from, &recorded_by, at).await;

    let named = request.from.to_string();
    let repaired = repair(&state, &viewer, &request.from, |list| {
        spine::without(list, &named)
    })
    .await?;

    Ok(Json(MergeResult {
        page: PageView::new(&page, false),
        removed: request.from,
        repaired,
    }))
}

// ------------------------------------------------------------------ helpers

/// Rewrite every `contents:` list that names `slug`, and say which ones changed.
///
/// A parent this caller cannot read is passed over without a word. That is the
/// rule a compile already follows for a chapter it may not fetch, and reporting
/// the skip would answer "does a page you cannot see list this one" for the price
/// of one request.
///
/// **Not observed in the word log.** The body is not touched, so nothing was
/// written, and the total the log checks a page against does not move. It is
/// still indexed, because `page_parts` is what the manifest and the orphan count
/// are read from.
async fn repair(
    state: &AppState,
    viewer: &Viewer,
    slug: &Slug,
    rewrite: impl Fn(&[String]) -> Option<Vec<String>>,
) -> AppResult<Vec<Repair>> {
    let mut repaired = Vec::new();

    for parent in state.index.parents_naming(slug).await? {
        // A parent that will not parse, or is gone since the last index, is
        // nothing to repair rather than a reason to fail a write that has
        // already happened.
        let Ok(page) = state.store.read(&parent).await else {
            continue;
        };
        if !readable(&page, viewer) {
            continue;
        }
        let Some(list) = page.contents() else {
            continue;
        };
        let Some(contents) = rewrite(list) else {
            continue;
        };

        let mut frontmatter = page.frontmatter.clone();
        frontmatter.contents = Some(contents.clone());

        let written = state.store.write(&parent, frontmatter, &page.body).await?;
        state.index.upsert(&written).await?;

        repaired.push(Repair {
            slug: parent,
            contents,
        });
    }

    Ok(repaired)
}
