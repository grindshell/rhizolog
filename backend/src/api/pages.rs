//! Page CRUD.
//!
//! Every write goes to the filesystem first and then updates the index, in that
//! order and before the response is sent. The index is derived, so a crash
//! between the two loses nothing that the next startup scan will not repair —
//! but a read immediately after a write must see the write, and waiting for the
//! file watcher would not guarantee that.
//!
//! ## Why slugs are a wildcard route
//!
//! Slugs contain `/`, so the page routes capture `{*slug}`. `matchit` requires
//! a catch-all to be the final segment, which rules out the
//! `/api/pages/{slug}/move` shape — and a literal `/api/pages/move` would
//! shadow any page actually slugged `move`, making it unreachable. Moving a
//! page therefore lives at `/api/move`, outside the namespace slugs occupy.
//!
//! The `{*slug}` spelling is an axum routing detail, not something callers
//! should see, so [`crate::api`] rewrites it back to `{slug}` in the published
//! OpenAPI document.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
// `#[schema(example = json!(...))]` needs no import: utoipa re-emits the macro
// fully qualified.
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::api::extract::Json as JsonBody;
use crate::error::{AppError, AppResult};
use crate::index::{ListOptions, PageRecord, SortBy, SortOrder};
use crate::markdown;
use crate::page::{Frontmatter, Page};
use crate::slug::Slug;

/// Fields a listing can be narrowed to with `?fields=`.
pub const SUMMARY_FIELDS: [&str; 6] = ["slug", "title", "tags", "created", "updated", "size"];

const SORT_KEYS: [&str; 4] = ["slug", "title", "created", "updated"];
const ORDER_KEYS: [&str; 2] = ["asc", "desc"];

const DEFAULT_LIMIT: usize = 50;
/// Caps how much one listing can return. A caller that wants the whole wiki
/// pages through it rather than asking for it in one breath.
const MAX_LIMIT: usize = 500;

// ---------------------------------------------------------------- responses

/// Every example in this document describes the same page.
///
/// A spec whose schemas each invent a different page is harder to read than one
/// that tells a single story, and for an agent the examples are most of what the
/// document teaches. This page exists in `example-wiki/`, so the document can be
/// followed against a running server rather than only imagined.
///
/// These are functions rather than constants because `#[schema(example = path)]`
/// means "call this", not "use this value" — a `const` there is a compile error
/// that reads as though the type is wrong.
fn example_title() -> &'static str {
    "Async in Rust"
}

fn example_body() -> &'static str {
    "# Async in Rust\n\nFutures are lazy. See [[notes/rust/pinning]].\n"
}

/// What [`example_body`] renders to. A unit test below keeps the two in step, so
/// the document cannot promise output the renderer does not produce.
///
/// The `data-wikilink` attribute is comrak's, not ours. It is worth leaving in
/// the example: it is the only thing in the output that distinguishes a
/// `[[wikilink]]` from an ordinary markdown link to the same page.
fn example_html() -> &'static str {
    "<h1>Async in Rust</h1>\n<p>Futures are lazy. See \
     <a href=\"/pages/notes/rust/pinning\" data-wikilink=\"true\">notes/rust/pinning</a>.</p>\n"
}

/// A page and its content.
#[derive(Debug, Serialize, ToSchema)]
pub struct PageView {
    pub slug: Slug,
    /// The page's effective title: its frontmatter `title`, or failing that the
    /// body's first heading, or failing that the slug.
    #[schema(example = example_title)]
    pub title: String,
    /// Whether `title` was derived rather than stored — from the body's first
    /// heading, or failing that the slug.
    ///
    /// This is what makes read-modify-write safe. Send a derived title back and
    /// it stops being derived: it is written into the frontmatter, and the
    /// heading it came from can never update it again. A client editing a page
    /// should send `null` for the title while this is true, and the editor in
    /// the dashboard leaves its title field empty for exactly that reason.
    pub title_derived: bool,
    /// The page's tags, in the order the frontmatter lists them.
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Vec<String>,
    /// When the page was first written. Falls back to the file's mtime for
    /// pages written by hand, which never carried the field.
    pub created: DateTime<Utc>,
    /// The file's modification time.
    pub updated: DateTime<Utc>,
    /// Size of the page's file on disk, in bytes.
    #[schema(example = 312)]
    pub size: u64,
    /// The page body as markdown, without its frontmatter. Title and tags are
    /// returned as fields above rather than left in the text, so an editor
    /// never has to reserialise YAML to change one of them.
    #[schema(example = example_body)]
    pub content: String,
    /// The body rendered to HTML. Present only when `render=true` was asked
    /// for.
    ///
    /// Raw HTML in the source is dropped rather than passed through — the
    /// markup does not appear in the output at all, escaped or otherwise. Links
    /// to pages come back as browsable `/pages/...` URLs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = example_html)]
    pub html: Option<String>,
}

impl PageView {
    fn new(page: &Page, render: bool) -> Self {
        Self {
            slug: page.slug.clone(),
            title: page.title(),
            title_derived: !page.has_stored_title(),
            tags: page.tags().to_vec(),
            created: page.created(),
            updated: page.updated,
            size: page.size,
            content: page.body.clone(),
            html: render.then(|| markdown::render(Some(&page.slug), &page.body)),
        }
    }
}

/// A page without its content.
#[derive(Debug, Serialize, ToSchema)]
pub struct PageSummary {
    pub slug: Slug,
    /// The page's effective title: its frontmatter `title`, or failing that the
    /// body's first heading, or failing that the slug.
    #[schema(example = example_title)]
    pub title: String,
    /// The page's tags, in the order the frontmatter lists them.
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Vec<String>,
    /// When the page was first written, falling back to the file's mtime.
    pub created: DateTime<Utc>,
    /// The file's modification time.
    pub updated: DateTime<Utc>,
    /// Size of the page's file on disk, in bytes.
    #[schema(example = 312)]
    pub size: u64,
}

impl From<PageRecord> for PageSummary {
    fn from(record: PageRecord) -> Self {
        Self {
            slug: record.slug,
            title: record.title,
            tags: record.tags,
            created: record.created,
            updated: record.updated,
            size: record.size,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PageListResponse {
    /// Bodies are never included here. When `fields` is given, each entry
    /// carries only the requested subset of these keys.
    pub pages: Vec<PageSummary>,
    /// Total matching pages, not the number returned.
    #[schema(example = 128)]
    pub total: usize,
    /// The limit that was applied, after clamping.
    #[schema(example = 50)]
    pub limit: usize,
    /// The offset that was applied.
    #[schema(example = 0)]
    pub offset: usize,
}

/// Markdown rendered to HTML.
#[derive(Debug, Serialize, ToSchema)]
pub struct RenderedHtml {
    /// The rendered body. Raw HTML in the source is dropped rather than passed
    /// through, and links to pages come back as browsable `/pages/...` URLs.
    #[schema(example = example_html)]
    pub html: String,
}

// ----------------------------------------------------------------- requests

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePage {
    /// Where the page will live. `409` if something is already there.
    pub slug: Slug,
    /// Optional. Without it the title falls back to the body's first heading,
    /// then to the slug.
    #[serde(default)]
    #[schema(example = example_title)]
    pub title: Option<String>,
    /// Free-form; tags are whatever you have used elsewhere. `GET /api/tags`
    /// lists the ones already in play.
    #[serde(default)]
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Vec<String>,
    /// Markdown body, without frontmatter.
    #[serde(default)]
    #[schema(example = example_body)]
    pub content: String,
}

/// A whole page. Every field is replaced, including the ones left out.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReplacePage {
    /// Send `null` to let the title follow the body's first heading. Sending a
    /// title the server derived is what freezes it — see `title_derived`.
    #[serde(default)]
    #[schema(example = example_title)]
    pub title: Option<String>,
    /// Omitting this clears the page's tags. Use `PATCH` to leave them alone.
    #[serde(default)]
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Vec<String>,
    /// Markdown body, without frontmatter. Omitting this empties the page.
    #[serde(default)]
    #[schema(example = example_body)]
    pub content: String,
}

/// A partial update. Omitted fields are left alone.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PatchPage {
    /// Omit to leave the title unchanged; send `null` to clear it and fall
    /// back to the heading or slug.
    #[serde(default, deserialize_with = "present_or_absent")]
    #[schema(value_type = Option<String>, example = example_title)]
    pub title: Option<Option<String>>,
    /// Replaces the whole tag list when present.
    #[serde(default)]
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Option<Vec<String>>,
    /// Replaces the whole body when present.
    #[serde(default)]
    #[schema(example = example_body)]
    pub content: Option<String>,
}

/// Distinguishes "field absent" from "field set to null".
///
/// Plain `Option<T>` collapses the two, which for PATCH would mean there is no
/// way to clear a title — omitting it and nulling it would both read as
/// `None`. The outer `Option` is absence; the inner is the value.
pub(crate) fn present_or_absent<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MovePage {
    /// The page to move. `404` if there is nothing there.
    #[schema(example = "notes/rust/async")]
    pub from: Slug,
    /// Where it goes. `409` if that slug is taken.
    #[schema(example = "notes/rust/futures")]
    pub to: Slug,
}

/// Markdown to render, with the context its links need.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RenderRequest {
    /// Markdown body, without frontmatter.
    #[schema(example = example_body)]
    pub content: String,
    /// The slug this content is, or would be, saved at.
    ///
    /// It matters only for relative markdown links: `[traits](traits.md)` names
    /// a different page depending on where it is written. Omit it and relative
    /// links resolve from the wiki root. Wikilinks are unaffected — they are
    /// always absolute.
    #[serde(default)]
    pub slug: Option<Slug>,
}

// -------------------------------------------------------------- query types

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReadQuery {
    /// Also return the body rendered to HTML, in an `html` field. The markdown
    /// is still returned either way.
    #[serde(default)]
    #[param(example = true)]
    pub render: bool,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Return only pages carrying this tag.
    #[param(example = "rust")]
    pub tag: Option<String>,
    /// Return only pages at or under this slug path.
    ///
    /// `notes/rust` matches the page `notes/rust` and everything beneath it.
    /// It stops at the separator, so `notes/rustlings` is a different
    /// directory and does not match. Matching is case-sensitive.
    #[param(example = "notes/rust")]
    pub prefix: Option<String>,
    /// Return only pages sitting in a directory of this name, wherever in the
    /// wiki that directory is.
    ///
    /// `rust` matches `notes/rust/async` and `code/rust/traits` alike — the
    /// flat reading of a slug, which is why it behaves like `tag` rather than
    /// like `prefix`. A page's own name is not a directory it sits in, so
    /// `async` does not match `notes/rust/async`.
    #[param(example = "rust")]
    pub segment: Option<String>,
    /// One of `slug`, `title`, `created`, `updated`. Defaults to `slug`.
    #[param(example = "updated")]
    pub sort: Option<String>,
    /// `asc` or `desc`. Defaults to `asc`.
    #[param(example = "desc")]
    pub order: Option<String>,
    /// Comma-separated subset of the summary fields to return, for cheap
    /// listings. Naming an unknown field is refused, and the error lists the
    /// ones that would have worked.
    #[param(example = "slug,title,tags")]
    pub fields: Option<String>,
    /// Defaults to 50, capped at 500.
    #[param(example = 50)]
    pub limit: Option<usize>,
    /// How many matches to skip. Pair it with `total` in the response.
    #[param(example = 0)]
    pub offset: Option<usize>,
}

// ----------------------------------------------------------------- handlers

/// List pages, without their bodies.
#[utoipa::path(
    get,
    path = "/api/pages",
    tag = "pages",
    params(ListQuery),
    responses(
        (status = 200, description = "Matching pages", body = PageListResponse),
        (status = 400, description = "Unknown field, sort key, or order", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Value>> {
    let fields = parse_fields(query.fields.as_deref())?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);

    let list = state
        .index
        .list(ListOptions {
            tag: query.tag,
            prefix: query.prefix,
            segment: query.segment,
            sort: parse_sort(query.sort.as_deref())?,
            order: parse_order(query.order.as_deref())?,
            limit,
            offset,
        })
        .await?;

    let response = PageListResponse {
        pages: list.pages.into_iter().map(PageSummary::from).collect(),
        total: list.total,
        limit,
        offset,
    };

    // Serialise the full shape and then narrow it, so the typed response above
    // stays the single description of what a summary contains.
    let mut value = serde_json::to_value(response)
        .map_err(|error| AppError::internal(format!("could not encode listing: {error}")))?;

    if let Some(fields) = fields {
        project(&mut value, &fields);
    }

    Ok(Json(value))
}

/// Create a page. Fails if one already exists at that slug.
#[utoipa::path(
    post,
    path = "/api/pages",
    tag = "pages",
    request_body = CreatePage,
    responses(
        (status = 201, description = "The page as written", body = PageView),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 409, description = "A page already exists at that slug", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<CreatePage>,
) -> AppResult<Response> {
    let frontmatter = Frontmatter {
        title: request.title,
        tags: request.tags,
        created: Some(Utc::now()),
    };

    let page = state
        .store
        .create(&request.slug, frontmatter, &request.content)
        .await?;
    state.index.upsert(&page).await?;

    Ok(created(&page))
}

/// Fetch a page, as markdown.
#[utoipa::path(
    get,
    path = "/api/pages/{*slug}",
    tag = "pages",
    params(
        ("slug" = String, Path, description = "Page slug, e.g. `notes/rust/async`", example = "notes/rust/async"),
        ReadQuery,
    ),
    responses(
        (status = 200, description = "The page", body = PageView),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at that slug", body = crate::error::ErrorResponse),
        (status = 422, description = "The page exists but could not be parsed", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Query(query): Query<ReadQuery>,
) -> AppResult<Json<PageView>> {
    let slug = parse_slug(&raw)?;
    let page = state.store.read(&slug).await?;
    Ok(Json(PageView::new(&page, query.render)))
}

/// Create or wholly replace a page.
#[utoipa::path(
    put,
    path = "/api/pages/{*slug}",
    tag = "pages",
    params(("slug" = String, Path, description = "Page slug", example = "notes/rust/async")),
    request_body = ReplacePage,
    responses(
        (status = 200, description = "The page was replaced", body = PageView),
        (status = 201, description = "The page was created", body = PageView),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
    ),
)]
pub async fn replace(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    JsonBody(request): JsonBody<ReplacePage>,
) -> AppResult<Response> {
    let slug = parse_slug(&raw)?;

    // A replace is not a re-creation: whatever the page was first written, it
    // was written then.
    let existing = state.store.read(&slug).await.ok();
    let created_at = existing.as_ref().map(Page::created);

    let frontmatter = Frontmatter {
        title: request.title,
        tags: request.tags,
        created: Some(created_at.unwrap_or_else(Utc::now)),
    };

    let page = state
        .store
        .write(&slug, frontmatter, &request.content)
        .await?;
    state.index.upsert(&page).await?;

    Ok(if existing.is_some() {
        Json(PageView::new(&page, false)).into_response()
    } else {
        created(&page)
    })
}

/// Update some of a page, leaving the rest alone.
#[utoipa::path(
    patch,
    path = "/api/pages/{*slug}",
    tag = "pages",
    params(("slug" = String, Path, description = "Page slug", example = "notes/rust/async")),
    request_body = PatchPage,
    responses(
        (status = 200, description = "The page as updated", body = PageView),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at that slug", body = crate::error::ErrorResponse),
    ),
)]
pub async fn patch(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    JsonBody(request): JsonBody<PatchPage>,
) -> AppResult<Json<PageView>> {
    let slug = parse_slug(&raw)?;
    let existing = state.store.read(&slug).await?;

    let mut frontmatter = existing.frontmatter.clone();
    if let Some(title) = request.title {
        frontmatter.title = title;
    }
    if let Some(tags) = request.tags {
        frontmatter.tags = tags;
    }
    let body = request.content.unwrap_or(existing.body);

    let page = state.store.write(&slug, frontmatter, &body).await?;
    state.index.upsert(&page).await?;

    Ok(Json(PageView::new(&page, false)))
}

/// Delete a page.
#[utoipa::path(
    delete,
    path = "/api/pages/{*slug}",
    tag = "pages",
    params(("slug" = String, Path, description = "Page slug", example = "notes/rust/async")),
    responses(
        (status = 204, description = "The page was deleted"),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at that slug", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> AppResult<StatusCode> {
    let slug = parse_slug(&raw)?;
    state.store.delete(&slug).await?;
    state.index.remove(&slug).await?;
    // Deleting a page through the API is a deliberate act on that page, so its
    // pin goes with it. A file that merely disappeared from disk is a different
    // case and keeps its pin — see `index::pins`.
    state.index.unpin(&slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Move a page to a new slug.
///
/// Inbound links are left as they are. They become wanted pages, so a move
/// shows up in the wiki's stats rather than rotting quietly.
#[utoipa::path(
    post,
    path = "/api/move",
    tag = "pages",
    request_body = MovePage,
    responses(
        (status = 200, description = "The page at its new slug", body = PageView),
        (status = 400, description = "One of the slugs is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at the source slug", body = crate::error::ErrorResponse),
        (status = 409, description = "A page already exists at the destination", body = crate::error::ErrorResponse),
    ),
)]
pub async fn move_page(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<MovePage>,
) -> AppResult<Json<PageView>> {
    let page = state.store.move_page(&request.from, &request.to).await?;

    state.index.remove(&request.from).await?;
    state.index.upsert(&page).await?;
    // Unlike inbound links, a pin follows the page. A link is something another
    // page said and is not ours to rewrite; a pin is a bookmark, and a bookmark
    // that stopped working because you renamed the thing it points at is a bug.
    state.index.repin(&request.from, &request.to).await?;

    Ok(Json(PageView::new(&page, false)))
}

/// Render markdown to HTML without storing it.
///
/// `GET /api/pages/{slug}?render=true` renders what is *saved*, which is no use
/// to an editor showing an unsaved draft. This renders whatever it is handed,
/// through the same renderer, so a preview cannot disagree with what the page
/// will look like once written. Rendering in the client instead would mean a
/// second markdown implementation that does not know about wikilinks, and it
/// would be wrong in exactly the places this wiki cares about.
///
/// Nothing is read or written, so this is safe to call on every keystroke.
#[utoipa::path(
    post,
    path = "/api/render",
    tag = "pages",
    request_body = RenderRequest,
    responses(
        (status = 200, description = "The rendered HTML", body = RenderedHtml),
        (status = 400, description = "The request body is not valid", body = crate::error::ErrorResponse),
    ),
)]
pub async fn render_markdown(
    JsonBody(request): JsonBody<RenderRequest>,
) -> AppResult<Json<RenderedHtml>> {
    Ok(Json(RenderedHtml {
        html: markdown::render(request.slug.as_ref(), &request.content),
    }))
}

// ------------------------------------------------------------------ helpers

/// 201 with a `Location` header, so a caller learns the page's URL without
/// having to build it.
fn created(page: &Page) -> Response {
    let location = format!("/api/pages/{}", page.slug);
    let mut response = (StatusCode::CREATED, Json(PageView::new(page, false))).into_response();

    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

pub(crate) fn parse_slug(raw: &str) -> AppResult<Slug> {
    Slug::parse(raw).map_err(|source| AppError::InvalidSlug {
        raw: raw.to_owned(),
        source,
    })
}

fn parse_sort(raw: Option<&str>) -> AppResult<SortBy> {
    match raw {
        None => Ok(SortBy::default()),
        Some("slug") => Ok(SortBy::Slug),
        Some("title") => Ok(SortBy::Title),
        Some("created") => Ok(SortBy::Created),
        Some("updated") => Ok(SortBy::Updated),
        Some(other) => Err(AppError::InvalidParameter {
            parameter: "sort",
            value: other.to_owned(),
            allowed: &SORT_KEYS,
        }),
    }
}

fn parse_order(raw: Option<&str>) -> AppResult<SortOrder> {
    match raw {
        None => Ok(SortOrder::default()),
        Some("asc") => Ok(SortOrder::Ascending),
        Some("desc") => Ok(SortOrder::Descending),
        Some(other) => Err(AppError::InvalidParameter {
            parameter: "order",
            value: other.to_owned(),
            allowed: &ORDER_KEYS,
        }),
    }
}

fn parse_fields(raw: Option<&str>) -> AppResult<Option<Vec<String>>> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    let requested: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(str::to_owned)
        .collect();

    if requested.is_empty() {
        return Ok(None);
    }

    let unknown: Vec<String> = requested
        .iter()
        .filter(|field| !SUMMARY_FIELDS.contains(&field.as_str()))
        .cloned()
        .collect();

    if !unknown.is_empty() {
        return Err(AppError::UnknownFields {
            unknown,
            valid: &SUMMARY_FIELDS,
        });
    }

    Ok(Some(requested))
}

/// Narrow each entry of a serialised listing to the requested keys.
fn project(value: &mut Value, fields: &[String]) {
    let Some(pages) = value.get_mut("pages").and_then(Value::as_array_mut) else {
        return;
    };

    for page in pages {
        let Some(object) = page.as_object_mut() else {
            continue;
        };
        object.retain(|key, _| fields.iter().any(|field| field == key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document promises this output, so the renderer had better produce it.
    /// Examples that quietly stop being true are worse than no examples: a
    /// reader has no way to tell which ones still hold.
    #[test]
    fn the_documented_html_is_what_the_renderer_produces() {
        let slug = Slug::parse("notes/rust/async").expect("valid slug");

        assert_eq!(
            markdown::render(Some(&slug), example_body()),
            example_html()
        );
    }

    #[test]
    fn sort_and_order_accept_their_keys() {
        assert_eq!(parse_sort(None).unwrap(), SortBy::Slug);
        assert_eq!(parse_sort(Some("title")).unwrap(), SortBy::Title);
        assert_eq!(parse_order(Some("desc")).unwrap(), SortOrder::Descending);
    }

    /// A caller that guesses wrong should be told what was allowed.
    #[test]
    fn an_unknown_sort_key_names_the_alternatives() {
        let error = parse_sort(Some("size")).unwrap_err();

        assert_eq!(error.code(), "invalid_parameter");
        let rendered = serde_json::to_value(error.to_string()).unwrap();
        assert!(rendered.as_str().unwrap().contains("slug"));
    }

    #[test]
    fn fields_are_parsed_and_validated() {
        assert_eq!(parse_fields(None).unwrap(), None);
        assert_eq!(
            parse_fields(Some("slug,title")).unwrap(),
            Some(vec!["slug".to_owned(), "title".to_owned()])
        );
        // Whitespace and empty entries are tolerated.
        assert_eq!(
            parse_fields(Some(" slug , , title ")).unwrap(),
            Some(vec!["slug".to_owned(), "title".to_owned()])
        );
        assert_eq!(parse_fields(Some("")).unwrap(), None);
    }

    #[test]
    fn an_unknown_field_is_rejected_by_name() {
        let error = parse_fields(Some("slug,body,nope")).unwrap_err();

        match error {
            AppError::UnknownFields { unknown, valid } => {
                assert_eq!(unknown, ["body", "nope"]);
                assert!(valid.contains(&"slug"));
            }
            other => panic!("expected UnknownFields, got {other:?}"),
        }
    }

    #[test]
    fn projection_keeps_only_the_requested_keys() {
        let mut value = serde_json::json!({
            "pages": [{ "slug": "a", "title": "A", "tags": [], "size": 1 }],
            "total": 1,
        });

        project(&mut value, &["slug".to_owned(), "tags".to_owned()]);

        let page = &value["pages"][0];
        assert!(page.get("slug").is_some());
        assert!(page.get("tags").is_some());
        assert!(page.get("title").is_none());
        assert!(page.get("size").is_none());
        // Envelope fields are untouched.
        assert_eq!(value["total"], 1);
    }

    /// `null` and "absent" have to stay distinguishable, or a title can never
    /// be cleared.
    #[test]
    fn patch_distinguishes_a_null_title_from_an_omitted_one() {
        let omitted: PatchPage = serde_json::from_str(r#"{"content":"x"}"#).unwrap();
        assert_eq!(omitted.title, None);

        let cleared: PatchPage = serde_json::from_str(r#"{"title":null}"#).unwrap();
        assert_eq!(cleared.title, Some(None));

        let set: PatchPage = serde_json::from_str(r#"{"title":"New"}"#).unwrap();
        assert_eq!(set.title, Some(Some("New".to_owned())));
    }
}
