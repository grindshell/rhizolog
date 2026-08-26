//! `GET /api/pace`: where a manuscript stands against its target and its day.
//!
//! An endpoint of its own rather than a flag on `/api/compile`, and the reason
//! is the gate rather than the shape. A compile is a document, served under the
//! ordinary page rules; a pace is read off the **word log**, which is working
//! state and is never handed to a caller who has not signed in, even under
//! `RHIZOLOG_ANONYMOUS_READ`. Two different answers to "who may ask this" is two
//! endpoints, because a query parameter that quietly changes the audience of a
//! response is the kind of thing that is right until somebody adds a caller.
//!
//! It is `/api/pace?root=` rather than `/api/pages/{slug}/pace` for the reason
//! that already produced `/api/move` and `/api/compile`: `matchit` requires a
//! catch-all to be the final segment, and a slug is a catch-all.
//!
//! The arithmetic is in [`crate::pace`], which knows nothing about HTTP, the
//! store or who is asking. This module is the seam.

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::api::AppState;
use crate::api::compile::Readable;
use crate::api::pages::parse_slug;
use crate::api::words::MAX_OFFSET_MINUTES;
use crate::auth::Viewer;
use crate::compile::{self, CompileError};
use crate::error::{AppError, AppResult};
use crate::pace::{self, DEFAULT_WINDOW_DAYS, Pace, Question};

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PaceQuery {
    /// The manuscript to measure. Its `target` and `due` are the ones that
    /// count, and its compiled total is what they are measured against.
    #[param(example = "book")]
    pub root: String,
    /// The instant to read "now" as. Defaults to the server's clock.
    ///
    /// It decides both the deadline arithmetic and where the window ends, so
    /// passing it is what makes a figure reproducible: every number below is a
    /// function of this instant and the files on disk.
    pub at: Option<DateTime<Utc>>,
    /// Minutes east of UTC, which is what `-new Date().getTimezoneOffset()`
    /// gives. It decides which local day an observation was written on, and
    /// where the trailing window ends.
    ///
    /// It does **not** move the deadline: `due` names a UTC day. See
    /// `days_remaining` on the response.
    #[param(example = -420)]
    pub offset: Option<i32>,
    /// How many days back to measure the observed rate over. Fourteen by
    /// default, clamped to between one and four hundred.
    #[param(example = 14)]
    pub days: Option<u32>,
}

/// Where a manuscript stands, and the arithmetic behind it.
///
/// Words remaining over days remaining, against words a day over the last
/// fortnight. **Refused for a caller who has not signed in**, on a wiki with
/// accounts, because half of it is read off the word log and a writing history
/// is working state rather than published content.
///
/// Nothing here is a verdict. Two rates come back in the same unit and the
/// reader compares them; there is no streak, no completion badge and no change
/// of tone when a number goes up.
#[utoipa::path(
    get,
    path = "/api/pace",
    tag = "words",
    params(PaceQuery),
    responses(
        (status = 200, description = "Where the manuscript stands", body = Pace),
        (status = 400, description = "The root slug is not valid", body = crate::error::ErrorResponse),
        (status = 401, description = "This wiki has accounts and the request named none", body = crate::error::ErrorResponse),
        (status = 404, description = "No page at the root slug", body = crate::error::ErrorResponse),
        (status = 413, description = "Depth, section or byte limit exceeded", body = crate::error::ErrorResponse),
    ),
)]
pub async fn pace_of(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<PaceQuery>,
) -> AppResult<Json<Pace>> {
    viewer.require_account()?;

    let root = parse_slug(&query.root)?;
    let at = query.at.unwrap_or_else(Utc::now);
    let offset = query
        .offset
        .unwrap_or(0)
        .clamp(-MAX_OFFSET_MINUTES, MAX_OFFSET_MINUTES);
    let window_days = query.days.unwrap_or(DEFAULT_WINDOW_DAYS);

    // The same walk the manifest and the assembled document do, and the same
    // audience rules with it: a chapter this caller may not read is a gap here
    // exactly as it is there, so its words are not in the total and not in the
    // rate.
    let pages = Readable {
        store: &state.store,
        viewer: &viewer,
    };
    let compiled = compile::compile(&root, None, &pages)
        .await
        .map_err(|error| match error {
            CompileError::RootNotFound { slug } => AppError::CompileRootNotFound { slug },
            CompileError::TooLarge { limit, at } => AppError::CompileTooLarge {
                limit: limit.as_str(),
                ceiling: limit.ceiling(),
                at,
            },
        })?;

    // One reading of "the last fortnight", used to ask the question and again to
    // answer it. Two would be two windows that agree until somebody changes one.
    let (from, to) = pace::window(at, offset, window_days);
    let samples = state
        .index
        .word_samples(from, to, &viewer.audience())
        .await?;

    let question = Question {
        root: root.to_string(),
        at,
        offset_minutes: offset,
        window_days,
    };

    Ok(Json(pace::build(&question, &compiled, &samples)))
}
