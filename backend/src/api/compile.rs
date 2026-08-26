//! `GET /api/compile`: a tree of pages as one document.
//!
//! It is `/api/compile` with a query parameter rather than
//! `/api/pages/{slug}/compiled` for the reason that already produced
//! `/api/move`: `matchit` requires a catch-all to be the final segment, and a
//! slug is a catch-all.
//!
//! The assembly itself is in [`crate::compile`], which knows nothing about
//! HTTP, the store or who is asking. This module is the seam: it turns a fetch
//! into a page the caller is allowed to see, and turns the result into JSON.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::api::pages::{parse_slug, readable};
use crate::auth::Viewer;
use crate::compile::{self, CompileError, Fetched, Pages};
use crate::error::{AppError, AppResult};
use crate::markdown;
use crate::slug::Slug;
use crate::store::{Store, StoreError};

/// Reads pages for one caller.
///
/// The audience check lives here rather than in [`crate::compile`] because it is
/// the only part of assembly that depends on who is asking. It is checked
/// against the file that was just read rather than against the index, exactly as
/// `GET /api/pages/{slug}` does, so a page whose frontmatter changed a moment
/// ago is not compiled under its old visibility.
pub(crate) struct Readable<'a> {
    pub store: &'a Store,
    pub viewer: &'a Viewer,
}

impl Pages for Readable<'_> {
    async fn fetch(&self, slug: &Slug) -> Fetched {
        match self.store.read(slug).await {
            // A page this caller may not read is `Missing`, and so is a slug
            // with nothing written at it. One answer on purpose: telling them
            // apart would confirm that a page exists at a slug somebody guessed,
            // which is what `404, never 403` exists to withhold.
            Ok(page) if !readable(&page, self.viewer) => Fetched::Missing,
            Ok(page) => Fetched::Page(Box::new(page)),
            // A page that will not parse is reported as itself. That read
            // already tells anybody who asks that the page is malformed, so
            // saying so here discloses nothing new, and hiding it would swallow
            // a real fault.
            Err(StoreError::Malformed { .. } | StoreError::NotUtf8 { .. }) => Fetched::Unreadable,
            Err(_) => Fetched::Missing,
        }
    }
}

/// What a document should come back as.
///
/// Read from a plain string in the handler rather than deserialised straight
/// into this enum, which is what `sort` and `order` on the page listing already
/// do. Two reasons, and the second is the one that bit: a caller who names a
/// format that does not exist gets the list of ones that do, and an enum
/// referenced only from a query parameter is not registered as a schema
/// component, so the generated document carries a `$ref` that resolves to
/// nothing and `pnpm gen:api` refuses the whole spec.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// The assembled markdown. The default.
    #[default]
    Markdown,
    /// The assembled markdown rendered to HTML.
    ///
    /// Rendered **after** assembly, so comrak sees one document and the shifted
    /// heading levels nest the way the manifest says they do.
    Html,
    /// The manifest and each section's text, for a caller that wants the parts
    /// rather than the whole.
    Json,
}

const FORMATS: [&str; 3] = ["markdown", "html", "json"];

fn parse_format(raw: Option<&str>) -> AppResult<Format> {
    match raw {
        None => Ok(Format::default()),
        Some("markdown") => Ok(Format::Markdown),
        Some("html") => Ok(Format::Html),
        Some("json") => Ok(Format::Json),
        Some(other) => Err(AppError::InvalidParameter {
            parameter: "format",
            value: other.to_owned(),
            allowed: &FORMATS,
        }),
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CompileQuery {
    /// The page to assemble from. Its body comes first, then the pages its
    /// `contents:` list names, recursively.
    #[param(example = "book")]
    pub root: String,
    /// `markdown` (the default), `html`, or `json`.
    #[param(example = "markdown")]
    #[serde(default)]
    pub format: Option<String>,
    /// A page to prepend as a preamble: the rules of the work, handed over in
    /// the same request as the work.
    ///
    /// A parameter rather than frontmatter because it is a property of who is
    /// asking rather than of the book. It appears in the manifest like anything
    /// else that is in the bytes.
    #[param(example = "rules/voice")]
    #[serde(default)]
    pub style: Option<String>,
}

/// One entry in the manifest.
#[derive(Debug, Serialize, ToSchema)]
pub struct SectionView {
    /// The slug as the contents list wrote it, so an `invalid` entry can be
    /// found and fixed.
    #[schema(example = "book/one/the-ferry")]
    pub slug: String,
    /// The page's title. Absent for anything that is not `included`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// How many contents lists deep this page sits. The root is zero.
    #[schema(example = 2)]
    pub depth: usize,
    #[schema(example = 2180)]
    pub words: u64,
    /// Where this section's bytes begin in the document.
    #[schema(example = 238)]
    pub offset: usize,
    /// How many bytes they run for. Zero for anything not `included`.
    #[schema(example = 12903)]
    pub length: usize,
    /// One of `included`, `wanted`, `invalid`, `duplicate`, `unreadable`.
    ///
    /// A section keeps its position whatever this says. A manuscript short of a
    /// chapter reports where the chapter was going to be, which is the whole
    /// difference between a gap and an omission.
    ///
    /// `wanted` covers a slug with nothing written at it **and** a page this
    /// caller may not read: the two are deliberately indistinguishable.
    /// `duplicate` is a page already emitted earlier, which covers a cycle and
    /// the commoner case that is not one, an appendix listed under two parts.
    #[schema(example = "included")]
    pub status: String,
    /// The section's own text. Only in the `json` format, and only when the
    /// section was `included`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CompiledView {
    /// Which assembly produced this. Changing how a document is put together is
    /// a version change here, following `tfidf/v1`.
    #[schema(example = "compile/v1")]
    pub compiler: &'static str,
    #[schema(example = "book")]
    pub root: String,
    /// The assembled document. Markdown or HTML depending on `format`, and
    /// absent for `json`, where each section carries its own text instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Every section in order, with where it landed.
    pub sections: Vec<SectionView>,
    /// Words across every included section.
    #[schema(example = 41230)]
    pub words: u64,
    /// The root's `target`, if it names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 90000)]
    pub target: Option<u64>,
}

/// The version this assembly is. See [`CompiledView::compiler`].
pub const COMPILER: &str = "compile/v1";

/// Assemble a manuscript.
#[utoipa::path(
    get,
    path = "/api/compile",
    tag = "pages",
    params(CompileQuery),
    responses(
        (status = 200, description = "The assembled document and its manifest", body = CompiledView),
        (status = 400, description = "The root or style slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at the root slug", body = crate::error::ErrorResponse),
        (status = 413, description = "Depth, section or byte limit exceeded", body = crate::error::ErrorResponse),
    ),
)]
pub async fn compile_pages(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<CompileQuery>,
) -> AppResult<Json<CompiledView>> {
    let root = parse_slug(&query.root)?;
    let style = query.style.as_deref().map(parse_slug).transpose()?;
    let format = parse_format(query.format.as_deref())?;

    let pages = Readable {
        store: &state.store,
        viewer: &viewer,
    };

    let compiled = compile::compile(&root, style.as_ref(), &pages)
        .await
        .map_err(|error| match error {
            CompileError::RootNotFound { slug } => AppError::CompileRootNotFound { slug },
            CompileError::TooLarge { limit, at } => AppError::CompileTooLarge {
                limit: limit.as_str(),
                ceiling: limit.ceiling(),
                at,
            },
        })?;

    let sections = compiled
        .sections
        .iter()
        .map(|section| SectionView {
            slug: section.slug.clone(),
            title: section.title.clone(),
            depth: section.depth,
            words: section.words,
            offset: section.offset,
            length: section.length,
            status: section.status.as_str().to_owned(),
            content: matches!(format, Format::Json)
                .then(|| {
                    compiled
                        .markdown
                        .get(section.offset..section.offset + section.length)
                        .map(str::to_owned)
                })
                .flatten(),
        })
        .collect();

    let content = match format {
        Format::Markdown => Some(compiled.markdown),
        // Rendered from the assembled document rather than page by page, so the
        // shifted headings nest and a footnote defined in one chapter is
        // resolvable from another. Links resolve against the root, which is the
        // page the document is addressed as.
        Format::Html => Some(markdown::render(Some(&root), &compiled.markdown)),
        Format::Json => None,
    };

    Ok(Json(CompiledView {
        compiler: COMPILER,
        root: root.to_string(),
        content,
        sections,
        words: compiled.words,
        target: compiled.target,
    }))
}
