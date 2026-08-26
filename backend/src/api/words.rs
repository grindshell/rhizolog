//! `GET /api/word-stats`: where the words went.
//!
//! A chart, on the same terms as the hours heat map, and nothing that
//! congratulates you. There are no streaks here and no goals with encouragement
//! attached.

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Days, FixedOffset, NaiveDate, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::auth::Viewer;
use crate::error::AppResult;
use crate::words::stats::{self, MAX_DAYS};

/// How many days a request that named no window gets.
///
/// A quarter: long enough to see a habit, short enough that the first thing the
/// dashboard draws is not mostly empty.
pub const DEFAULT_DAYS: u64 = 90;

/// The largest offset a client can be in, in minutes.
///
/// Real zones run from about minus twelve to plus fourteen hours. A day either
/// side of that is generous and keeps a nonsense value from producing a window
/// nobody asked for.
pub const MAX_OFFSET_MINUTES: i32 = 24 * 60;

/// What resolution the series is at.
///
/// **Not keystroke history**, and the field is here to say so rather than to be
/// branched on. An observation is not a save: the file watcher debounces at
/// 500 ms and collapses a burst into one batch, edits made while the server was
/// down are one observation at the next startup, and only an API write is
/// genuinely one per save. The diff is right in every one of those cases; what
/// is lost is resolution in time, and churn inside a window is invisible.
pub const RESOLUTION: &str = "observed";

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WordStatsQuery {
    /// The instant to read "now" as. Defaults to the server's clock.
    ///
    /// It decides which local day the window ends on, and passing it is what
    /// makes a figure captured for the product site reproducible.
    pub at: Option<DateTime<Utc>>,
    /// Minutes east of UTC, which is what `-new Date().getTimezoneOffset()`
    /// gives. It decides which local day an observation falls in.
    #[param(example = -420)]
    pub offset: Option<i32>,
    /// The start of the window. Defaults to ninety local days back.
    pub from: Option<DateTime<Utc>>,
    /// The end of the window, exclusive. Defaults to the end of the local day
    /// holding `at`.
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayView {
    /// The local date, in the offset that was asked for.
    #[schema(example = "2026-08-25")]
    pub date: String,
    #[schema(example = 1900)]
    pub added: u64,
    #[schema(example = 2000)]
    pub removed: u64,
    /// `added - removed`. Arithmetic over the two rather than a stored value,
    /// which is the whole point: a day of revision is not "minus one hundred".
    #[schema(example = -100)]
    pub delta: i64,
    pub observations: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ActorWordsView {
    /// Which tool. **A claim rather than a proof**: anything that can write can
    /// send the header, which is fine, because the question is bookkeeping about
    /// your own tools rather than security.
    #[schema(example = "claude-code")]
    pub actor: String,
    pub added: u64,
    pub removed: u64,
    pub delta: i64,
    pub observations: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PageWordsView {
    #[schema(example = "book/one/the-ferry")]
    pub slug: String,
    /// The page's title, or its slug when there is none to show.
    ///
    /// A page this caller may not read is ranked under its slug, which is the
    /// label an unwritten page already gets. The row itself stays: the words
    /// really were written, and hiding somebody's own working history from them
    /// would be the wrong reading of a rule that protects other people's pages.
    #[schema(example = "The Ferry")]
    pub title: String,
    pub added: u64,
    pub removed: u64,
    pub delta: i64,
    pub observations: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WordTotalsView {
    pub added: u64,
    pub removed: u64,
    pub delta: i64,
    pub observations: usize,
    /// Distinct pages written to in the window.
    pub pages: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WordStatsResponse {
    /// The instant the window was computed against.
    pub at: DateTime<Utc>,
    /// The offset the days were cut in, as it was applied after clamping.
    #[schema(example = -420)]
    pub offset_minutes: i32,
    pub from: DateTime<Utc>,
    /// Exclusive.
    pub to: DateTime<Utc>,
    /// Always `observed`, and the field is here to say what that means rather
    /// than to be branched on: **this is not keystroke history**. The watcher
    /// debounces at 500 ms and collapses a burst into one batch, edits made
    /// while the server was down are one observation at the next startup, and
    /// only an API write is genuinely one per save. The counts are right in
    /// every one of those cases; what is lost is resolution in time.
    #[schema(example = "observed")]
    pub resolution: &'static str,
    /// Every local day in the window, including the empty ones, so a client can
    /// draw the chart without filling gaps itself.
    pub days: Vec<DayView>,
    /// Busiest tools first, capped at ten.
    pub actors: Vec<ActorWordsView>,
    /// Busiest pages first, capped at ten.
    pub pages: Vec<PageWordsView>,
    pub totals: WordTotalsView,
}

/// Where the words went, by day and by tool.
///
/// **Refused for a caller who has not signed in**, on a wiki with accounts, even
/// under `RHIZOLOG_ANONYMOUS_READ`. A writing history is working state rather
/// than published content, and that variable exists to publish pages marked
/// `public`.
#[utoipa::path(
    get,
    path = "/api/word-stats",
    tag = "words",
    params(WordStatsQuery),
    responses(
        (status = 200, description = "The series", body = WordStatsResponse),
        (status = 401, description = "This wiki has accounts and the request named none", body = crate::error::ErrorResponse),
    ),
)]
pub async fn word_statistics(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<WordStatsQuery>,
) -> AppResult<Json<WordStatsResponse>> {
    viewer.require_account()?;

    let at = query.at.unwrap_or_else(Utc::now);
    let offset = query
        .offset
        .unwrap_or(0)
        .clamp(-MAX_OFFSET_MINUTES, MAX_OFFSET_MINUTES);

    let (from, to) = window(at, offset, query.from, query.to);

    let samples = state
        .index
        .word_samples(from, to, &viewer.audience())
        .await?;
    let computed = stats::build(&samples, from, to, offset);

    Ok(Json(WordStatsResponse {
        at,
        offset_minutes: computed.offset_minutes,
        from: computed.from,
        to: computed.to,
        resolution: RESOLUTION,
        days: computed
            .days
            .iter()
            .map(|day| DayView {
                date: day.date.format("%Y-%m-%d").to_string(),
                added: day.added,
                removed: day.removed,
                delta: day.delta(),
                observations: day.observations,
            })
            .collect(),
        actors: computed
            .actors
            .iter()
            .map(|total| ActorWordsView {
                actor: total.actor.clone(),
                added: total.added,
                removed: total.removed,
                delta: total.added as i64 - total.removed as i64,
                observations: total.observations,
            })
            .collect(),
        pages: computed
            .pages
            .iter()
            .map(|total| PageWordsView {
                slug: total.slug.to_string(),
                title: total.title.clone(),
                added: total.added,
                removed: total.removed,
                delta: total.added as i64 - total.removed as i64,
                observations: total.observations,
            })
            .collect(),
        totals: WordTotalsView {
            added: computed.totals.added,
            removed: computed.totals.removed,
            delta: computed.totals.delta(),
            observations: computed.totals.observations,
            pages: computed.totals.pages,
        },
    }))
}

/// The window to read, from whatever the caller did or did not say.
///
/// The default ends at the **end** of the local day holding `at`, not at `at`
/// itself, so today is a whole column rather than a half-finished one. It starts
/// [`DEFAULT_DAYS`] before that.
///
/// A window longer than [`MAX_DAYS`] is clamped by moving its **start** forward
/// rather than its end back, because a chart wants the recent end of a range it
/// cannot draw all of.
fn window(
    at: DateTime<Utc>,
    offset_minutes: i32,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    // The same reading of an offset the bucketing uses, so a window and the days
    // inside it can never be cut in two different zones.
    let zone = stats::zone(offset_minutes);

    let end_of_today = at
        .with_timezone(&zone)
        .date_naive()
        .succ_opt()
        .map_or(at, |tomorrow| local_midnight(tomorrow, zone));

    let to = to.unwrap_or(end_of_today);
    let from = from.unwrap_or_else(|| to - Days::new(DEFAULT_DAYS));

    // Clamp, not refuse: a window this long is a client that has not thought
    // about it rather than a mistake worth an error.
    let earliest = to - TimeDelta::days(MAX_DAYS as i64);
    (from.max(earliest), to)
}

/// Local midnight at the start of `date`, as an instant.
///
/// A fixed offset has no gaps or repeats, so a local time always maps to exactly
/// one instant. The fallback is unreachable and is there because `single()` is
/// honest about zones that do have them.
fn local_midnight(date: NaiveDate, zone: FixedOffset) -> DateTime<Utc> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time of day");

    naive
        .and_local_timezone(zone)
        .single()
        .map_or_else(|| naive.and_utc(), |local| local.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    /// Today is a whole column, not a half-finished one.
    #[test]
    fn the_default_window_ends_at_the_end_of_the_local_day() {
        let (from, to) = window(at("2026-08-25T14:00:00Z"), 0, None, None);

        assert_eq!(to, at("2026-08-26T00:00:00Z"));
        assert_eq!(from, at("2026-05-28T00:00:00Z"));
    }

    /// Seven hours behind, the afternoon of the 25th in UTC is still the morning
    /// of the 25th locally, and the window ends at local midnight.
    #[test]
    fn the_offset_moves_the_end_of_the_window() {
        let (_, to) = window(at("2026-08-25T14:00:00Z"), -420, None, None);
        assert_eq!(to, at("2026-08-26T07:00:00Z"));
    }

    #[test]
    fn a_window_the_caller_named_is_used_as_written() {
        let (from, to) = window(
            at("2026-08-25T14:00:00Z"),
            0,
            Some(at("2026-08-01T00:00:00Z")),
            Some(at("2026-08-10T00:00:00Z")),
        );

        assert_eq!(from, at("2026-08-01T00:00:00Z"));
        assert_eq!(to, at("2026-08-10T00:00:00Z"));
    }

    /// A chart wants the recent end of a range it cannot draw all of.
    #[test]
    fn an_enormous_window_is_clamped_at_its_start() {
        let (from, to) = window(
            at("2026-08-25T14:00:00Z"),
            0,
            Some(at("1970-01-01T00:00:00Z")),
            Some(at("2026-08-26T00:00:00Z")),
        );

        assert_eq!(to, at("2026-08-26T00:00:00Z"));
        assert_eq!((to - from).num_days(), MAX_DAYS as i64);
    }
}
