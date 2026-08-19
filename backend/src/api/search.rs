//! Full-text search over pages, and rebuilding the index it reads from.
//!
//! Time entries are searched through `GET /api/times?q=`, not here — see
//! [`crate::api::times`] for why the log carries its own.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::auth::Viewer;
use crate::error::AppResult;
use crate::index::SyncCounts;
use crate::index::sync::rebuild;
use crate::slug::Slug;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Terms to search for. Matched literally and combined with AND; a
    /// trailing `*` on a term searches by prefix. Punctuation is safe to
    /// include — it is never interpreted as query syntax.
    #[param(example = "futures lazy*")]
    pub q: String,
    /// Defaults to 20, capped at 100.
    #[param(example = 20)]
    pub limit: Option<usize>,
    /// How many matches to skip. Pair it with `total` in the response.
    #[param(example = 0)]
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHitView {
    pub slug: Slug,
    /// The page's effective title.
    #[schema(example = "Async in Rust")]
    pub title: String,
    /// Its tags, so a caller can filter results without fetching each page.
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Vec<String>,
    /// An excerpt of the body with matched terms wrapped in `<mark>`. This is
    /// here so a caller can judge which hits are worth fetching without
    /// pulling every body.
    ///
    /// Only the marks are markup: the text around them is the page body
    /// verbatim and is **not** escaped, so a client rendering this as HTML
    /// would be rendering whatever the page contains.
    #[schema(example = "Futures are <mark>lazy</mark>. See")]
    pub snippet: String,
    /// Relevance. Higher is better; comparable only within one result set.
    #[schema(example = 1.87)]
    pub score: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    /// Best first.
    pub hits: Vec<SearchHitView>,
    /// Total matches, not the number returned.
    #[schema(example = 3)]
    pub total: usize,
    /// The limit that was applied, after clamping.
    #[schema(example = 20)]
    pub limit: usize,
    /// The offset that was applied.
    #[schema(example = 0)]
    pub offset: usize,
}

/// Search page titles and bodies.
///
/// Time entries are not included. Their names and notes are searchable through
/// `GET /api/times?q=`, where the search intersects with the log's own filters.
#[utoipa::path(
    get,
    path = "/api/search",
    tag = "search",
    params(SearchQuery),
    responses(
        (status = 200, description = "Matching pages, best first", body = SearchResponse),
    ),
)]
pub async fn search(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<SearchResponse>> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);

    // Search is the sharpest of the leaks visibility has to cover: a hit carries
    // the slug, the title, the tags *and* an excerpt of the body with the match
    // highlighted in it. An unfiltered search over a wiki with private pages in
    // it is a way to read them a dozen words at a time.
    let results = state
        .index
        .search(&query.q, limit, offset, &viewer.audience())
        .await?;

    Ok(Json(SearchResponse {
        hits: results
            .hits
            .into_iter()
            .map(|hit| SearchHitView {
                slug: hit.slug,
                title: hit.title,
                tags: hit.tags,
                snippet: hit.snippet,
                score: hit.score,
            })
            .collect(),
        total: results.total,
        limit,
        offset,
    }))
}

/// What one scan of one tree found.
#[derive(Debug, Serialize, ToSchema)]
pub struct SyncCountsView {
    /// Files found on disk.
    #[schema(example = 6)]
    pub scanned: usize,
    /// Files read and written to the index.
    #[schema(example = 6)]
    pub indexed: usize,
    /// Files already indexed with a matching mtime and size.
    #[schema(example = 0)]
    pub unchanged: usize,
    /// Rows dropped because the file is no longer on disk.
    #[schema(example = 0)]
    pub removed: usize,
    /// Files on disk that could not be read, and so are not in the index.
    #[schema(example = 0)]
    pub failed: usize,
}

impl From<SyncCounts> for SyncCountsView {
    fn from(counts: SyncCounts) -> Self {
        Self {
            scanned: counts.scanned,
            indexed: counts.indexed,
            unchanged: counts.unchanged,
            removed: counts.removed,
            failed: counts.failed,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReindexResponse {
    /// The markdown pages.
    pub pages: SyncCountsView,
    /// The time log under `.rhizolog/times/`, which is scanned the same way
    /// and for the same reason.
    pub times: SyncCountsView,
}

/// Rebuild the index from the files on disk.
///
/// The index holds nothing that is not already in a file, so this is always
/// safe and never loses anything. It is the escape hatch for the one case
/// incremental scanning can miss: an edit that leaves both mtime and size
/// unchanged.
#[utoipa::path(
    post,
    path = "/api/reindex",
    tag = "search",
    responses(
        (status = 200, description = "What the rebuild found", body = ReindexResponse),
    ),
)]
pub async fn reindex(State(state): State<AppState>) -> AppResult<Json<ReindexResponse>> {
    let report = rebuild(&state.store, &state.times, &state.index).await?;

    tracing::info!(
        pages = report.pages.indexed,
        times = report.times.indexed,
        removed = report.pages.removed + report.times.removed,
        failed = report.pages.failed + report.times.failed,
        "index rebuilt on request"
    );

    Ok(Json(ReindexResponse {
        pages: report.pages.into(),
        times: report.times.into(),
    }))
}
