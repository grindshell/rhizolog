//! Server status.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    /// Always `"ok"` — a response at all is the liveness signal.
    #[schema(example = "ok")]
    pub status: String,

    /// The running Rhizowiki version.
    #[schema(example = "0.1.0")]
    pub version: String,

    /// Absolute path of the wiki directory being served.
    pub wiki_root: String,
}

/// Report that the server is running, and which wiki it is serving.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "meta",
    responses(
        (status = 200, description = "The server is running", body = Health),
    ),
)]
pub async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        wiki_root: state.store.root_display(),
    })
}
