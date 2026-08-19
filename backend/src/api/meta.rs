//! Server status.
//!
//! ## Why this endpoint is reachable without an account
//!
//! [`crate::endpoint::live`] confirms a published `server.json` by asking here
//! and comparing the `wiki_root` it gets back. That happens before anybody could
//! have signed in — it is how a second copy of the desktop app finds out that a
//! wiki is already open — so gating it would break the single-instance check on
//! every wiki that has accounts.
//!
//! What it *says* is gated instead. To an anonymous caller on a wiki that
//! requires authentication, the counts are omitted: how many pages a wiki has
//! and whether somebody is running a timer right now are facts about its
//! contents, and none of them are needed to answer "is a server alive here, and
//! is it serving this directory".
//!
//! `wiki_root` survives that trim, and it is a filesystem path disclosed to
//! anybody who can reach the port. That is a real cost, accepted because the
//! handshake above is built on it and a path is the least interesting thing
//! behind this door. It is listed in `TODO.md`.

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;
use crate::auth::Viewer;
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

    /// Whether this wiki has accounts, and so wants callers to sign in.
    ///
    /// False is the state of a fresh wiki: no login page, nothing refused, every
    /// request treated as the single user. Reported to anonymous callers on
    /// purpose — a client cannot decide whether to show a sign-in prompt without
    /// being told, and the answer is one bit that is obvious from the response
    /// to any other request anyway.
    pub authentication_required: bool,

    /// Number of pages currently indexed. Omitted for an anonymous caller on a
    /// wiki that requires authentication.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 42)]
    pub pages: Option<usize>,

    /// Number of time entries currently indexed. Omitted as `pages` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 312)]
    pub times: Option<usize>,

    /// Timers running right now. Several may run at once. Omitted as `pages` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1)]
    pub running_timers: Option<usize>,

    /// When the index was last reconciled with the files on disk.
    ///
    /// **Null** if it has not been scanned yet; **absent** for an anonymous
    /// caller, as `pages` is. The two are different answers, which is why this
    /// is nested rather than a plain optional timestamp that would collapse
    /// "never scanned" into "not telling you".
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    pub last_indexed: Option<Option<DateTime<Utc>>>,
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
pub async fn health(State(state): State<AppState>, viewer: Viewer) -> AppResult<Json<Health>> {
    let mut health = Health {
        status: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        wiki_root: state.store.root_display(),
        authentication_required: viewer.authentication_required(),
        pages: None,
        times: None,
        running_timers: None,
        last_indexed: None,
    };

    // The liveness half is above and is answered for anybody. The rest describes
    // what is *in* the wiki, and is not part of the discovery handshake.
    if viewer.is_permitted() {
        let totals = state.index.time_totals(Utc::now()).await?;

        health.pages = Some(state.index.count().await?);
        health.times = Some(totals.entries);
        health.running_timers = Some(totals.running);
        health.last_indexed = Some(state.index.last_sync().await?);
    }

    Ok(Json(health))
}
