//! Pinned pages.
//!
//! A wiki that branches chaotically still has two or three pages you touch
//! every day — a scratch pad, a running index, whatever the current project is.
//! Pins are the shortcut to those, and they are server-side rather than a
//! browser preference for the same reason everything else here is: the API is
//! the product, and "which pages does this wiki revolve around" is a question an
//! agent should be able to ask and answer.
//!
//! The slug routes are wildcards for the reason the page routes are — see
//! [`crate::api::pages`].
//!
//! The handlers are named `list_pins`/`pin_page`/`unpin_page` rather than
//! `list`/`create`/`delete`. utoipa takes the operation id straight from the
//! function name, and those ids are global to the document: the short names
//! would collide with [`crate::api::pages`]'s and produce two operations with
//! one id, which a generated client resolves by dropping one of them.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;
use crate::api::pages::{parse_slug, readable};
use crate::auth::Viewer;
use crate::error::{AppError, AppResult};
use crate::index::Pin;
use crate::slug::Slug;
use crate::store::StoreError;

/// How many pages may be pinned at once.
///
/// Pins are a shortcut menu, not a second listing: past a certain length the
/// menu is slower to use than the search box it was meant to save you. The cap
/// is generous enough that nobody reaches it by hand and low enough that a
/// script cannot turn the menu into the whole wiki.
pub const MAX_PINS: usize = 50;

#[derive(Debug, Serialize, ToSchema)]
pub struct PinView {
    pub slug: Slug,
    /// The pinned page's title, falling back to the slug when no page is there.
    #[schema(example = "Async in Rust")]
    pub title: String,
    /// Whether a page still exists at this slug.
    ///
    /// A pin can outlive its page — a file removed or renamed outside Rhizolog
    /// leaves one behind. That is reported rather than cleaned up silently, so
    /// the pin can be removed deliberately.
    pub exists: bool,
    /// When it was pinned. Pinning an already-pinned page does not change this.
    pub pinned_at: DateTime<Utc>,
}

impl From<Pin> for PinView {
    fn from(pin: Pin) -> Self {
        Self {
            exists: pin.exists(),
            title: pin.title.unwrap_or_else(|| pin.slug.to_string()),
            slug: pin.slug,
            pinned_at: pin.pinned_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PinsResponse {
    /// Oldest pin first. The order is stable: pinning something new appends to
    /// it and re-pinning moves nothing.
    pub pins: Vec<PinView>,
    /// How many pins there may be in total, so a client can say why a pin was
    /// refused before it tries.
    #[schema(example = 50)]
    pub limit: usize,
}

/// Every pinned page.
#[utoipa::path(
    get,
    path = "/api/pins",
    tag = "pins",
    responses((status = 200, description = "The pinned pages, oldest first", body = PinsResponse)),
)]
pub async fn list_pins(
    State(state): State<AppState>,
    viewer: Viewer,
) -> AppResult<Json<PinsResponse>> {
    Ok(Json(PinsResponse {
        pins: state
            .index
            .pins(&viewer.audience())
            .await?
            .into_iter()
            .map(PinView::from)
            .collect(),
        limit: MAX_PINS,
    }))
}

/// Pin a page.
///
/// Idempotent: pinning a page that is already pinned succeeds and leaves its
/// position alone, so a client does not have to check first.
#[utoipa::path(
    put,
    path = "/api/pins/{*slug}",
    tag = "pins",
    params(("slug" = String, Path, description = "Page slug", example = "notes/rust/async")),
    responses(
        (status = 200, description = "The page is pinned", body = PinView),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at that slug", body = crate::error::ErrorResponse),
        (status = 409, description = "The pin limit is already reached", body = crate::error::ErrorResponse),
    ),
)]
pub async fn pin_page(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<PinView>> {
    let slug = parse_slug(&raw)?;

    // Pinning something that is not there would be a typo every time, and
    // pinning something you may not read would answer the one question its slug
    // was hiding — so both come back as the same 404. The store answers rather
    // than the index, because the store is the source of truth and a page
    // written a moment ago is on disk before it is indexed.
    match state.store.read(&slug).await {
        Ok(page) if !readable(&page, &viewer) => {
            return Err(AppError::Store(StoreError::NotFound { slug }));
        }
        // A page whose frontmatter will not parse has no visibility to read, and
        // it is still a page. Pinning it is how somebody gets back to it.
        Ok(_) | Err(StoreError::Malformed { .. }) => {}
        Err(error) => return Err(error.into()),
    }

    // Checked before inserting, and skipped when the page is already pinned so
    // that a re-pin at the limit is not refused for adding nothing.
    if !state.index.is_pinned(&slug).await? && state.index.count_pins().await? >= MAX_PINS {
        return Err(AppError::TooManyPins { limit: MAX_PINS });
    }

    state.index.pin(&slug, Utc::now()).await?;

    let pin = state
        .index
        .pins(&viewer.audience())
        .await?
        .into_iter()
        .find(|pin| pin.slug == slug)
        .ok_or_else(|| AppError::internal("the pin vanished between writing and reading it"))?;

    Ok(Json(PinView::from(pin)))
}

/// Unpin a page.
///
/// Unpinning something that was not pinned is a `404`, matching `DELETE` on a
/// page: an operation that did nothing is worth knowing about. The page itself
/// is untouched — this removes the shortcut, not the wiki entry.
#[utoipa::path(
    delete,
    path = "/api/pins/{*slug}",
    tag = "pins",
    params(("slug" = String, Path, description = "Page slug", example = "notes/rust/async")),
    responses(
        (status = 204, description = "The pin was removed"),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "That page was not pinned", body = crate::error::ErrorResponse),
    ),
)]
pub async fn unpin_page(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> AppResult<StatusCode> {
    let slug = parse_slug(&raw)?;

    if state.index.unpin(&slug).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::PinNotFound { slug })
    }
}
