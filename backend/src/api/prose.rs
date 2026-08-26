//! `/api/prose`: the rules, and what they have to say.
//!
//! Three endpoints, and the third is the one that makes the other two mean
//! anything. `.rhizolog/prose.toml` sits outside the page API and outside the
//! wiki walker, so without `GET /api/prose/rules` a remote caller could receive
//! findings and have no way to see what produced them. The assistant reads the
//! manuscript **and the rules** the same way the dashboard does, or the promise
//! this feature makes is false.
//!
//! It is `/api/prose` with query parameters rather than
//! `/api/pages/{slug}/prose` for the reason that already produced `/api/move`
//! and `/api/compile`: `matchit` requires a catch-all to be the final segment,
//! and a slug is a catch-all. `/api/prose/rules` is a fixed segment underneath
//! and cannot collide with anything, because `/api/prose` takes no path
//! parameter at all.
//!
//! The analysis itself is in [`crate::prose`], which knows nothing about HTTP,
//! the store or who is asking.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::api::compile::Readable;
use crate::api::extract::Json as JsonBody;
use crate::api::pages::{parse_slug, readable};
use crate::auth::Viewer;
use crate::compile::{self, CompileError, Section, Status};
use crate::error::{AppError, AppResult};
use crate::prose::{self, Analysis, Finding, NormalizedRule};
use crate::store::StoreError;

/// What the offsets in a report index.
///
/// Two answers, and a caller has to be told which it got. Findings over one page
/// are offsets into that page's body, which is what the editor's textarea holds.
/// Findings over a manuscript are offsets into the **compiled document**, which
/// is not the same thing as any one page's source: the heading shift moves bytes
/// inside a section. Each finding still names the page it fell in, so the way to
/// get an exact source offset is to ask again for that page on its own.
const IN_PAGE: &str = "page";
const IN_DOCUMENT: &str = "document";

// -------------------------------------------------------------------- request

/// Markdown to check, for an editor that has not saved yet.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProseRequest {
    /// Markdown body, without frontmatter.
    #[schema(example = "The ferry was late, and being late was all it had ever been.")]
    pub content: String,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProseQuery {
    /// The page to check.
    #[param(example = "book/one/the-ferry")]
    pub slug: String,
    /// Check what this page **compiles to** rather than its own body.
    ///
    /// The whole manuscript, assembled exactly as `GET /api/compile` assembles
    /// it. Two of the five rules are cross-page questions by nature: a name
    /// spelled two ways in two chapters, and a word echoed across a section
    /// break, are invisible to anything reading one page at a time.
    ///
    /// It moves what the offsets mean. See `offsets` on the response.
    #[serde(default)]
    #[param(example = true)]
    pub compiled: bool,
}

// ------------------------------------------------------------------- response

#[derive(Debug, Serialize, ToSchema)]
pub struct SpanView {
    /// Byte offset, not a character index and not a position in rendered HTML.
    #[schema(example = 1840)]
    pub start: usize,
    #[schema(example = 1908)]
    pub end: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FindingView {
    /// The `id` of the rule that fired.
    #[schema(example = "echo")]
    pub rule: String,
    /// `error` or `warn`, as the rule declares it. It carries no behaviour:
    /// nothing here blocks a save.
    #[schema(example = "warn")]
    pub severity: String,
    /// Which page this fell in. Only on a compiled report, where it is the whole
    /// point: a finding over a whole book is no use if it cannot say which
    /// chapter owns it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "book/one/the-ferry")]
    pub slug: Option<String>,
    pub span: SpanView,
    /// The text the rule fired on, cut from the source at exactly `span`.
    ///
    /// Nothing around it: a caller that wants context has the offsets and the
    /// text, and a context window would be a number nobody asked for. Render it
    /// as text and never as HTML. Page content is what agents write.
    #[schema(example = "the ferry was late, and being late was the only thing it had ever been")]
    pub quote: String,
    #[schema(example = "late repeated within 12 words")]
    pub message: String,
    /// The numbers the rule actually compared.
    ///
    /// Without it, "late repeated within 12 words" is a sentence asking to be
    /// believed rather than an arithmetic anybody can check. Its shape depends
    /// on the rule: token positions and a distance for `echo`, sentence lengths
    /// and a mean for `uniformity`, both spellings and both counts for
    /// `consistent`, the matched literal for `forbid` and `phrase`.
    #[schema(example = json!({
        "token": "late", "first": 1840, "second": 1889, "distance": 11, "within": 40
    }))]
    pub receipt: Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProseReport {
    /// Which analyzer produced this. Changing any rule's arithmetic, the
    /// tokenizer or the sentence splitter is a version change here, following
    /// `tfidf/v1`.
    #[schema(example = "prose/v1")]
    pub analyzer: &'static str,
    /// The stamp of the ruleset these findings came from.
    ///
    /// The same string `GET /api/prose/rules` reports. A finding and a ruleset
    /// that disagree on it were produced from different rules, which is
    /// otherwise an invisible way to be confidently wrong about why something
    /// fired.
    #[schema(example = "sha256:9f2bcd00")]
    pub rules_digest: String,
    /// How many rules were applied.
    ///
    /// **Zero and no findings is not a clean page**, it is a wiki that has never
    /// written a rules file, and without this a caller cannot tell the two
    /// apart. `GET /api/prose/rules` answers an empty ruleset rather than a
    /// `404` for the same reason: no rules is a state a wiki is genuinely in,
    /// and it is worth being able to say so.
    #[schema(example = 5)]
    pub rules: usize,
    /// What `span` indexes: `page` for one page's body, `document` for a
    /// compiled manuscript.
    #[schema(example = "page")]
    pub offsets: &'static str,
    /// The page asked about. Absent when the body was posted rather than stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "book/one/the-ferry")]
    pub slug: Option<String>,
    /// Ordered by start offset, then by rule id.
    pub findings: Vec<FindingView>,
    #[schema(example = 0)]
    pub errors: usize,
    #[schema(example = 4)]
    pub warnings: usize,
    /// Whether the list was cut short.
    ///
    /// A `forbid` rule naming one common letter would otherwise return a finding
    /// per occurrence across a whole book. Unlike a compile, a short answer here
    /// is safe to give because it says it is short.
    pub truncated: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RulesView {
    #[schema(example = "prose/v1")]
    pub analyzer: &'static str,
    /// Taken over the normalized rules below rather than over the file's bytes,
    /// so a caller can recompute it from this response and check.
    #[schema(example = "sha256:9f2bcd00")]
    pub rules_digest: String,
    /// Every rule, sorted by `id`, with its options resolved and its defaults
    /// filled in. A wiki that has never written rules answers an empty list
    /// rather than a `404`: no rules is a state a wiki is genuinely in.
    pub rules: Vec<NormalizedRule>,
}

// ------------------------------------------------------------------- handlers

/// Check markdown without storing it.
///
/// The editor's path, and it matches `POST /api/render`, which already takes
/// markdown and returns something derived from it. Nothing is read or written
/// except the rules file, so this is safe to call on a debounce.
///
/// A wiki with no rules file is not an error here. It is the ordinary case, and
/// the answer is no findings.
#[utoipa::path(
    post,
    path = "/api/prose",
    tag = "prose",
    request_body = ProseRequest,
    responses(
        (status = 200, description = "The findings over the submitted body", body = ProseReport),
        (status = 400, description = "The request body is not valid", body = crate::error::ErrorResponse),
        (status = 422, description = "The rules file will not parse", body = crate::error::ErrorResponse),
    ),
)]
pub async fn check_prose(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<ProseRequest>,
) -> AppResult<Json<ProseReport>> {
    let ruleset = prose::load(state.store.root()).await?;
    let analysis = prose::analyze(&request.content, &ruleset);

    Ok(Json(report(&ruleset, IN_PAGE, None, analysis, &[])))
}

/// Check a page, or everything it compiles to.
#[utoipa::path(
    get,
    path = "/api/prose",
    tag = "prose",
    params(ProseQuery),
    responses(
        (status = 200, description = "The findings", body = ProseReport),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page there, or none this caller may read", body = crate::error::ErrorResponse),
        (status = 413, description = "Compiling it exceeded a limit", body = crate::error::ErrorResponse),
        (status = 422, description = "The rules file will not parse", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_prose(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<ProseQuery>,
) -> AppResult<Json<ProseReport>> {
    let slug = parse_slug(&query.slug)?;
    let ruleset = prose::load(state.store.root()).await?;

    if query.compiled {
        let pages = Readable {
            store: &state.store,
            viewer: &viewer,
        };

        let compiled =
            compile::compile(&slug, None, &pages)
                .await
                .map_err(|error| match error {
                    CompileError::RootNotFound { slug } => AppError::CompileRootNotFound { slug },
                    CompileError::TooLarge { limit, at } => AppError::CompileTooLarge {
                        limit: limit.as_str(),
                        ceiling: limit.ceiling(),
                        at,
                    },
                })?;

        let analysis = prose::analyze(&compiled.markdown, &ruleset);

        return Ok(Json(report(
            &ruleset,
            IN_DOCUMENT,
            Some(slug.to_string()),
            analysis,
            &compiled.sections,
        )));
    }

    let page = state.store.read(&slug).await?;

    // Checked against the file that was just read rather than against the index,
    // exactly as `GET /api/pages/{slug}` does, and a 404 rather than a 403 for
    // the same reason: a 403 confirms that a page exists at a slug somebody
    // guessed.
    if !readable(&page, &viewer) {
        return Err(AppError::Store(StoreError::NotFound { slug }));
    }

    let analysis = prose::analyze(&page.body, &ruleset);

    Ok(Json(report(
        &ruleset,
        IN_PAGE,
        Some(slug.to_string()),
        analysis,
        &[],
    )))
}

/// The rules, as the analyzer resolved them.
///
/// Normalized rather than the file's bytes, because a caller wanting to
/// reproduce a finding needs the values the analyzer used, and TOML has more
/// than one way to write most of them.
///
/// Not writable through the API in this version. The file is authored
/// configuration, editing it is a text edit, and a second way to write it would
/// be a second place for it to be wrong.
#[utoipa::path(
    get,
    path = "/api/prose/rules",
    tag = "prose",
    responses(
        (status = 200, description = "The normalized ruleset and its digest", body = RulesView),
        (status = 422, description = "The rules file will not parse", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_prose_rules(State(state): State<AppState>) -> AppResult<Json<RulesView>> {
    let ruleset = prose::load(state.store.root()).await?;

    Ok(Json(RulesView {
        analyzer: prose::ANALYZER,
        rules_digest: ruleset.digest().to_owned(),
        rules: ruleset.normalized(),
    }))
}

// -------------------------------------------------------------------- helpers

fn report(
    ruleset: &prose::Ruleset,
    offsets: &'static str,
    slug: Option<String>,
    analysis: Analysis,
    sections: &[Section],
) -> ProseReport {
    let errors = analysis
        .findings
        .iter()
        .filter(|finding| finding.severity == prose::Severity::Error)
        .count();

    ProseReport {
        analyzer: prose::ANALYZER,
        rules_digest: ruleset.digest().to_owned(),
        rules: ruleset.rules().len(),
        offsets,
        slug,
        warnings: analysis.findings.len() - errors,
        errors,
        truncated: analysis.truncated,
        findings: analysis
            .findings
            .into_iter()
            .map(|finding| view(finding, sections))
            .collect(),
    }
}

fn view(finding: Finding, sections: &[Section]) -> FindingView {
    FindingView {
        rule: finding.rule,
        severity: finding.severity.as_str().to_owned(),
        slug: owner(finding.start, sections),
        span: SpanView {
            start: finding.start,
            end: finding.end,
        },
        quote: finding.quote,
        message: finding.message,
        receipt: finding.receipt,
    }
}

/// Which section of a compiled document an offset fell in.
///
/// A linear scan, because the manifest is at most a couple of thousand entries
/// and the findings are already sorted. Sections that were not included have no
/// bytes, so they can never claim one: a gap in the manuscript owns nothing.
fn owner(at: usize, sections: &[Section]) -> Option<String> {
    sections
        .iter()
        .find(|section| {
            section.status == Status::Included
                && at >= section.offset
                && at < section.offset + section.length
        })
        .map(|section| section.slug.clone())
}
