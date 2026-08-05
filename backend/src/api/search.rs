//! Full-text search, and rebuilding the index it reads from.

use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::error::AppResult;
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
    #[param(example = "rhizome branch*")]
    pub q: String,
    /// Defaults to 20, capped at 100.
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHitView {
    pub slug: Slug,
    pub title: String,
    pub tags: Vec<String>,
    /// An excerpt of the body with matched terms wrapped in `<mark>`. This is
    /// here so a caller can judge which hits are worth fetching without
    /// pulling every body.
    #[schema(example = "knowledge branches off <mark>chaotically</mark>")]
    pub snippet: String,
    /// Relevance. Higher is better; comparable only within one result set.
    pub score: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub hits: Vec<SearchHitView>,
    /// Total matches, not the number returned.
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

/// Search page titles and bodies.
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
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<SearchResponse>> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);

    let results = state.index.search(&query.q, limit, offset).await?;

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

#[derive(Debug, Serialize, ToSchema)]
pub struct ReindexResponse {
    /// Pages found on disk.
    pub scanned: usize,
    /// Pages read and written to the index.
    pub indexed: usize,
    /// Pages dropped because they are no longer on disk.
    pub removed: usize,
    /// Pages on disk that could not be read, and so are not searchable.
    pub failed: usize,
}

/// Rebuild the search index from the wiki directory.
///
/// The index holds nothing that is not already on disk, so this is always safe
/// and never loses anything. It is the escape hatch for the one case
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
    let report = rebuild(&state.store, &state.index).await?;

    tracing::info!(
        scanned = report.scanned,
        indexed = report.indexed,
        removed = report.removed,
        failed = report.failed,
        "index rebuilt on request"
    );

    Ok(Json(ReindexResponse {
        scanned: report.scanned,
        indexed: report.indexed,
        removed: report.removed,
        failed: report.failed,
    }))
}
