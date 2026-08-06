//! Server status.

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;
use crate::error::AppResult;

#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    /// Always `"ok"` — a response at all is the liveness signal.
    #[schema(example = "ok")]
    pub status: String,

    /// The running Rhizolog version.
    #[schema(example = "0.1.0")]
    pub version: String,

    /// Absolute path of the wiki directory being served.
    #[schema(example = "/home/tim/wiki")]
    pub wiki_root: String,

    /// Number of pages currently indexed.
    #[schema(example = 42)]
    pub pages: usize,

    /// Number of time entries currently indexed.
    #[schema(example = 312)]
    pub times: usize,

    /// Timers running right now. Several may run at once.
    #[schema(example = 1)]
    pub running_timers: usize,

    /// When the index was last reconciled with the files on disk. Null if it
    /// has not been scanned yet.
    pub last_indexed: Option<DateTime<Utc>>,
}

/// Report that the server is running, which wiki it is serving, and how fresh
/// its index is.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "meta",
    responses(
        (status = 200, description = "The server is running", body = Health),
    ),
)]
pub async fn health(State(state): State<AppState>) -> AppResult<Json<Health>> {
    let totals = state.index.time_totals(Utc::now()).await?;

    Ok(Json(Health {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        wiki_root: state.store.root_display(),
        pages: state.index.count().await?,
        times: totals.entries,
        running_timers: totals.running,
        last_indexed: state.index.last_sync().await?,
    }))
}
