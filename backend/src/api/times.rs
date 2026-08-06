//! Time tracking: timers, manual entries, groups, and statistics.
//!
//! ## One endpoint starts a timer and logs an hour you forgot
//!
//! `POST /api/times` with a `name` and nothing else starts a timer now.  The
//! same call with a `start` and an `end` records a finished entry. There is no
//! separate `/api/times/start`, because there is no difference worth an
//! endpoint: a running entry is one whose `end` has not been written yet, and
//! saying so with an absent field is more honest than a mode flag.
//!
//! `POST /api/times/{id}/stop` is the exception, and it earns its place by
//! being the one operation whose whole content is "now". Doing it with a
//! `PATCH` would mean every client reading the clock and sending a timestamp,
//! and a client whose clock is wrong writing it down.
//!
//! ## Searching the log is `?q=`, not `/api/times/search`
//!
//! Pages get a listing and a search as two endpoints, because a search hit is
//! not a page record — it carries a snippet and a relevance score, and it comes
//! back in a different order. A time search is not like that. It is one more
//! way to narrow the log, and the useful questions are intersections: what did
//! I write about the poll loop, last week, under `Deep work`. Splitting it off
//! would mean either duplicating five filters on the second endpoint or being
//! unable to ask.
//!
//! It also spares `/api/times/{id}` a sibling that looks like an id and is not.
//!
//! ## Ids are not slugs, so these routes are not wildcards
//!
//! The page routes capture `{*slug}` because a slug contains `/`. A
//! [`TimeId`] cannot, which is what lets `/api/times/{id}/stop` exist at all —
//! `matchit` requires a catch-all to be the last segment, and that restriction
//! is the reason moving a page had to become `/api/move`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::api::extract::Json as JsonBody;
use crate::api::pages::present_or_absent;
use crate::error::{AppError, AppResult};
use crate::index::{SortOrder, TimeGroup, TimeListOptions, TimeRecord, TimeSortBy, TimeTotals};
use crate::markdown;
use crate::slug::Slug;
use crate::times::stats::{self, HeatCell, Period, PeriodStats};
use crate::times::{TimeDraft, TimeEntry, TimeId};

const SORT_KEYS: [&str; 3] = ["start", "name", "duration"];
const ORDER_KEYS: [&str; 2] = ["asc", "desc"];

const DEFAULT_LIMIT: usize = 50;
/// Caps how much one listing can return. A busy year is thousands of entries,
/// so a caller that wants all of them pages through rather than asking once.
const MAX_LIMIT: usize = 500;

/// Longest an activity name may be.
///
/// A name is the group, and a group is a menu entry and a chart label. This is
/// generous for anything a person types and mean enough that a script cannot
/// turn the group list into a wall of prose.
pub const MAX_NAME_LEN: usize = 200;

/// The widest offset a caller may claim to be at, in minutes.
///
/// Real offsets top out at UTC+14. Anything past a day is not a timezone.
const MAX_OFFSET_MINUTES: i32 = 24 * 60;

// ---------------------------------------------------------------- responses

fn example_name() -> &'static str {
    "Deep work"
}

fn example_note() -> &'static str {
    "Chased down a lifetime error in the poll loop.\n"
}

/// A page a time entry is attached to.
#[derive(Debug, Serialize, ToSchema)]
pub struct TimePageView {
    pub slug: Slug,
    /// The page's title, or `null` if nothing is written there yet.
    #[schema(example = "Async in Rust")]
    pub title: Option<String>,
    /// Whether a page exists at that slug.
    ///
    /// Time can be tracked against a page before it is written, exactly as a
    /// link can point at one. It attaches itself when the page appears, with
    /// nothing to reindex.
    pub exists: bool,
}

/// A time entry without its note.
#[derive(Debug, Serialize, ToSchema)]
pub struct TimeSummary {
    pub id: TimeId,
    /// The activity. Entries are grouped by this, spelled exactly as written.
    #[schema(example = example_name)]
    pub name: String,
    pub start: DateTime<Utc>,
    /// `null` while the timer is running.
    pub end: Option<DateTime<Utc>>,
    /// Whether the timer is still running. Several may run at once, and they
    /// are allowed to overlap.
    pub running: bool,
    /// Elapsed seconds, counting up to now while the entry runs.
    #[schema(example = 4470)]
    pub seconds: u64,
    /// The pages this time is attached to.
    pub pages: Vec<TimePageView>,
    /// Whether the entry carries a note. Fetch the entry itself to read it.
    pub has_note: bool,
    /// An excerpt of the note with the matched terms wrapped in `<mark>`,
    /// present only when a `q=` search is what turned this entry up and the
    /// note is what matched it. Absent when the name matched instead — that is
    /// already in `name`.
    ///
    /// Only the marks are markup: the text around them is the note verbatim and
    /// is **not** escaped, exactly as in a search hit over pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Chased down a lifetime error in the <mark>poll</mark> loop")]
    pub snippet: Option<String>,
    /// The file's modification time.
    pub updated: DateTime<Utc>,
    /// Size of the entry's file on disk, in bytes.
    #[schema(example = 148)]
    pub size: u64,
}

/// A time entry and its note.
#[derive(Debug, Serialize, ToSchema)]
pub struct TimeView {
    pub id: TimeId,
    #[schema(example = example_name)]
    pub name: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub running: bool,
    #[schema(example = 4470)]
    pub seconds: u64,
    pub pages: Vec<TimePageView>,
    /// The note as markdown, without its frontmatter. Usually empty.
    #[schema(example = example_note)]
    pub note: String,
    /// The note rendered to HTML. Present only when `render=true` was asked
    /// for. Wikilinks in a note resolve like anywhere else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    pub updated: DateTime<Utc>,
    #[schema(example = 148)]
    pub size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeListResponse {
    /// Notes are never included here. Newest first unless you say otherwise.
    pub times: Vec<TimeSummary>,
    /// Total matching entries, not the number returned.
    #[schema(example = 312)]
    pub total: usize,
    /// The limit that was applied, after clamping.
    #[schema(example = 50)]
    pub limit: usize,
    #[schema(example = 0)]
    pub offset: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeGroupView {
    /// The activity name, which is the group.
    #[schema(example = example_name)]
    pub name: String,
    #[schema(example = 42)]
    pub entries: usize,
    /// Total tracked, with running entries counted up to now.
    #[schema(example = 151200)]
    pub seconds: u64,
    /// How many of this group's entries are running.
    #[schema(example = 1)]
    pub running: usize,
    pub first_start: DateTime<Utc>,
    pub last_start: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeGroupsResponse {
    /// Most time first.
    pub groups: Vec<TimeGroupView>,
    /// Every group, every entry, all of it.
    pub totals: TimeTotalsView,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeTotalsView {
    #[schema(example = 312)]
    pub entries: usize,
    /// Distinct activity names.
    #[schema(example = 9)]
    pub groups: usize,
    #[schema(example = 1512000)]
    pub seconds: u64,
    #[schema(example = 1)]
    pub running: usize,
    /// The earliest entry's start, or `null` for an empty log.
    pub first_start: Option<DateTime<Utc>>,
    pub last_start: Option<DateTime<Utc>>,
}

impl From<TimeTotals> for TimeTotalsView {
    fn from(totals: TimeTotals) -> Self {
        Self {
            entries: totals.entries,
            groups: totals.groups,
            seconds: totals.seconds,
            running: totals.running,
            first_start: totals.first_start,
            last_start: totals.last_start,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NameTotalView {
    #[schema(example = example_name)]
    pub name: String,
    #[schema(example = 7200)]
    pub seconds: u64,
    #[schema(example = 3)]
    pub entries: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PageTotalView {
    pub slug: Slug,
    #[schema(example = "Async in Rust")]
    pub title: String,
    #[schema(example = 7200)]
    pub seconds: u64,
    #[schema(example = 3)]
    pub entries: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BucketView {
    /// The instant the bucket opens. Buckets are contiguous and equal in
    /// calendar terms, not necessarily in length.
    pub start: DateTime<Utc>,
    #[schema(example = 3600)]
    pub seconds: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PeriodStatsView {
    /// `day`, `week`, `month` or `year`.
    #[schema(example = "week")]
    pub period: String,
    /// The window, in UTC. Its edges are local midnights in the offset given.
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Time falling inside the window, not the full length of every entry that
    /// touches it: a session spanning midnight is split between the two days.
    #[schema(example = 68400)]
    pub seconds: u64,
    /// Entries overlapping the window at all.
    #[schema(example = 24)]
    pub entries: usize,
    /// The most-used activities, busiest first.
    pub names: Vec<NameTotalView>,
    /// The pages the most time was attached to.
    pub pages: Vec<PageTotalView>,
    /// Hours for a day, days for a week or a month, months for a year.
    pub buckets: Vec<BucketView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeatCellView {
    /// 0 is Monday, matching the week the periods use.
    #[schema(example = 3)]
    pub weekday: u8,
    /// Local hour, 0 to 23.
    #[schema(example = 14)]
    pub hour: u8,
    #[schema(example = 5400)]
    pub seconds: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HeatmapView {
    /// The window the map covers: the same one as the `year` period.
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// All 168 cells, including the empty ones, so a client can draw the grid
    /// without filling gaps itself. Monday 00:00 first, then by hour.
    pub cells: Vec<HeatCellView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeStatsResponse {
    /// The instant everything was computed against. Running entries are
    /// counted up to here.
    pub at: DateTime<Utc>,
    /// The offset the windows were cut in, as it was applied after clamping.
    #[schema(example = -420)]
    pub offset_minutes: i32,
    /// The whole log, ignoring windows.
    pub all_time: TimeTotalsView,
    /// Today, this week, this month and this year, in that order.
    pub periods: Vec<PeriodStatsView>,
    /// When the hours actually go, over the year.
    pub heatmap: HeatmapView,
}

// ----------------------------------------------------------------- requests

/// A new entry. With no `start` it begins now; with no `end` it keeps running.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTime {
    /// The activity. Entries are grouped by this, and it is not normalised —
    /// `Deep work` and `deep work` are two groups, the same way two tags spelled
    /// differently are two tags.
    #[schema(example = example_name)]
    pub name: String,
    /// Defaults to now, which is what starting a timer means.
    #[serde(default)]
    pub start: Option<DateTime<Utc>>,
    /// Omit it to start a timer; send it to log time that is already over.
    #[serde(default)]
    pub end: Option<DateTime<Utc>>,
    /// Pages this time was spent on. They need not exist yet.
    #[serde(default)]
    #[schema(example = json!(["notes/rust/async"]))]
    pub pages: Vec<Slug>,
    /// A markdown note, without frontmatter.
    #[serde(default)]
    #[schema(example = example_note)]
    pub note: String,
}

/// A partial update. Omitted fields are left alone.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PatchTime {
    #[serde(default)]
    #[schema(example = example_name)]
    pub name: Option<String>,
    #[serde(default)]
    pub start: Option<DateTime<Utc>>,
    /// Omit to leave the end alone; send `null` to clear it, which sets the
    /// entry running again.
    #[serde(default, deserialize_with = "present_or_absent")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    pub end: Option<Option<DateTime<Utc>>>,
    /// Replaces the whole list when present.
    #[serde(default)]
    #[schema(example = json!(["notes/rust/async"]))]
    pub pages: Option<Vec<Slug>>,
    #[serde(default)]
    #[schema(example = example_note)]
    pub note: Option<String>,
}

// -------------------------------------------------------------- query types

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimeListQuery {
    /// Search the entries' names and notes. Terms are matched literally and
    /// combined with AND, a trailing `*` searches by prefix, and punctuation is
    /// safe to include — the same rules `/api/search` follows.
    ///
    /// It is a filter, so it intersects with everything else here rather than
    /// replacing it, and it does not reorder the log. Matching entries carry a
    /// `snippet`.
    #[param(example = "poll loop")]
    pub q: Option<String>,
    /// Only entries in this group, matched **exactly**. `q` is the fuzzy one;
    /// this is the group, spelled as written.
    #[param(example = "Deep work")]
    pub name: Option<String>,
    /// Only entries attached to this page.
    #[param(example = "notes/rust/async")]
    pub page: Option<String>,
    /// `true` for running entries only, `false` for finished ones.
    #[param(example = true)]
    pub running: Option<bool>,
    /// Only entries **overlapping** `[from, to)`, not only those starting
    /// inside it — a session that began last night and is still going is time
    /// being spent today.
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// One of `start`, `name`, `duration`. Defaults to `start`.
    #[param(example = "start")]
    pub sort: Option<String>,
    /// `asc` or `desc`. Defaults to `desc`, because a log reads newest first.
    #[param(example = "desc")]
    pub order: Option<String>,
    /// Defaults to 50, capped at 500.
    #[param(example = 50)]
    pub limit: Option<usize>,
    #[param(example = 0)]
    pub offset: Option<usize>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReadTimeQuery {
    /// Also return the note rendered to HTML, in an `html` field.
    #[serde(default)]
    #[param(example = true)]
    pub render: bool,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimeStatsQuery {
    /// Minutes **east** of UTC, which is `-new Date().getTimezoneOffset()` in a
    /// browser. Defaults to 0.
    ///
    /// Entries are stored in UTC, but "today" and "when do I usually work" are
    /// questions about a wall clock, so every window and every heat map row is
    /// cut in this offset. It is a fixed offset rather than a timezone: a
    /// window straddling a daylight-saving change is bucketed throughout with
    /// the offset you sent, which can make one past day an hour short or long.
    #[param(example = -420)]
    pub offset: Option<i32>,
    /// The instant to compute against. Defaults to now; useful for asking what
    /// last Tuesday looked like.
    pub at: Option<DateTime<Utc>>,
}

// ----------------------------------------------------------------- handlers

/// List time entries, without their notes.
#[utoipa::path(
    get,
    path = "/api/times",
    tag = "times",
    params(TimeListQuery),
    responses(
        (status = 200, description = "Matching entries", body = TimeListResponse),
        (status = 400, description = "Unknown sort key or order", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_times(
    State(state): State<AppState>,
    Query(query): Query<TimeListQuery>,
) -> AppResult<Json<TimeListResponse>> {
    let now = Utc::now();
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = query.offset.unwrap_or(0);

    let list = state
        .index
        .list_times(
            TimeListOptions {
                query: query.q,
                name: query.name,
                page: query.page,
                running: query.running,
                from: query.from,
                to: query.to,
                sort: parse_sort(query.sort.as_deref())?,
                order: parse_order(query.order.as_deref())?,
                limit,
                offset,
            },
            now,
        )
        .await?;

    Ok(Json(TimeListResponse {
        times: list
            .times
            .into_iter()
            .map(|record| summary(record, now))
            .collect(),
        total: list.total,
        limit,
        offset,
    }))
}

/// Start a timer, or log time that is already over.
#[utoipa::path(
    post,
    path = "/api/times",
    tag = "times",
    request_body = CreateTime,
    responses(
        (status = 201, description = "The entry as written", body = TimeView),
        (status = 400, description = "The name, a slug, or the range is not valid", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_time(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<CreateTime>,
) -> AppResult<Response> {
    let now = Utc::now();
    let start = request.start.unwrap_or(now);
    let draft = TimeDraft {
        name: check_name(request.name)?,
        start,
        end: check_range(start, request.end)?,
        pages: request.pages,
        note: request.note,
    };

    // The file first, then the index, in that order and before responding: the
    // index is derived, so a crash between the two loses nothing the next scan
    // will not repair, but a read straight after a write must see the write.
    let entry = state.times.create(draft).await?;
    state.index.upsert_time(&entry).await?;

    let location = format!("/api/times/{}", entry.id);
    let mut response = (
        StatusCode::CREATED,
        Json(view(&entry, &state, now, false).await?),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}

/// Fetch one entry, with its note.
#[utoipa::path(
    get,
    path = "/api/times/{id}",
    tag = "times",
    params(
        ("id" = String, Path, description = "Time entry id", example = "20260806T142530-123456789"),
        ReadTimeQuery,
    ),
    responses(
        (status = 200, description = "The entry", body = TimeView),
        (status = 400, description = "The id is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No entry with that id", body = crate::error::ErrorResponse),
        (status = 422, description = "The entry exists but could not be parsed", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_time(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    Query(query): Query<ReadTimeQuery>,
) -> AppResult<Json<TimeView>> {
    let id = parse_id(&raw)?;
    let entry = state.times.read(&id).await?;
    Ok(Json(view(&entry, &state, Utc::now(), query.render).await?))
}

/// Update part of an entry, leaving the rest alone.
#[utoipa::path(
    patch,
    path = "/api/times/{id}",
    tag = "times",
    params(("id" = String, Path, description = "Time entry id", example = "20260806T142530-123456789")),
    request_body = PatchTime,
    responses(
        (status = 200, description = "The entry as updated", body = TimeView),
        (status = 400, description = "The id, the name, or the range is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No entry with that id", body = crate::error::ErrorResponse),
    ),
)]
pub async fn patch_time(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    JsonBody(request): JsonBody<PatchTime>,
) -> AppResult<Json<TimeView>> {
    let id = parse_id(&raw)?;
    let existing = state.times.read(&id).await?;

    let start = request.start.unwrap_or(existing.start);
    let end = match request.end {
        Some(end) => end,
        None => existing.end,
    };

    let draft = TimeDraft {
        name: match request.name {
            Some(name) => check_name(name)?,
            None => existing.name,
        },
        start,
        end: check_range(start, end)?,
        pages: request.pages.unwrap_or(existing.pages),
        note: request.note.unwrap_or(existing.note),
    };

    // Written back under the same id even when `start` moved. An id is a name,
    // not a claim about the entry's contents, and a PATCH that silently handed
    // back a different one would break every reference a caller held.
    let entry = state.times.write(&id, draft).await?;
    state.index.upsert_time(&entry).await?;

    Ok(Json(view(&entry, &state, Utc::now(), false).await?))
}

/// Stop a running timer, now.
///
/// A `409` if it has already stopped, rather than a silent success: a stop that
/// did nothing usually means a second tab got there first, and a caller that
/// could not tell would show the wrong duration.
#[utoipa::path(
    post,
    path = "/api/times/{id}/stop",
    tag = "times",
    params(("id" = String, Path, description = "Time entry id", example = "20260806T142530-123456789")),
    responses(
        (status = 200, description = "The entry, now finished", body = TimeView),
        (status = 400, description = "The id is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No entry with that id", body = crate::error::ErrorResponse),
        (status = 409, description = "That entry was not running", body = crate::error::ErrorResponse),
    ),
)]
pub async fn stop_time(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> AppResult<Json<TimeView>> {
    let id = parse_id(&raw)?;
    let existing = state.times.read(&id).await?;

    if !existing.is_running() {
        return Err(AppError::TimeNotRunning { id });
    }

    let now = Utc::now();
    let entry = state
        .times
        .write(
            &id,
            TimeDraft {
                name: existing.name,
                start: existing.start,
                // A clock that has gone backwards since the timer started would
                // otherwise write an inverted entry. Zero is the honest answer.
                end: Some(now.max(existing.start)),
                pages: existing.pages,
                note: existing.note,
            },
        )
        .await?;
    state.index.upsert_time(&entry).await?;

    Ok(Json(view(&entry, &state, now, false).await?))
}

/// Delete a time entry.
#[utoipa::path(
    delete,
    path = "/api/times/{id}",
    tag = "times",
    params(("id" = String, Path, description = "Time entry id", example = "20260806T142530-123456789")),
    responses(
        (status = 204, description = "The entry was deleted"),
        (status = 400, description = "The id is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No entry with that id", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_time(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> AppResult<StatusCode> {
    let id = parse_id(&raw)?;
    state.times.delete(&id).await?;
    state.index.remove_time(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every group, with its totals.
///
/// A group is an activity name. Nothing creates or deletes one: a group exists
/// because entries carry its name, and it is gone when the last of them is.
#[utoipa::path(
    get,
    path = "/api/time-groups",
    tag = "times",
    responses((status = 200, description = "Groups, most time first", body = TimeGroupsResponse)),
)]
pub async fn list_time_groups(
    State(state): State<AppState>,
) -> AppResult<Json<TimeGroupsResponse>> {
    let now = Utc::now();

    Ok(Json(TimeGroupsResponse {
        groups: state
            .index
            .time_groups(now)
            .await?
            .into_iter()
            .map(group_view)
            .collect(),
        totals: state.index.time_totals(now).await?.into(),
    }))
}

/// Where the time went: today, this week, this month, this year.
///
/// Entries are split across bucket boundaries rather than attributed whole to
/// the bucket they started in, so an overnight session lands on both days and
/// lights every hour it touched on the heat map.
#[utoipa::path(
    get,
    path = "/api/time-stats",
    tag = "times",
    params(TimeStatsQuery),
    responses((status = 200, description = "Time statistics", body = TimeStatsResponse)),
)]
pub async fn time_statistics(
    State(state): State<AppState>,
    Query(query): Query<TimeStatsQuery>,
) -> AppResult<Json<TimeStatsResponse>> {
    let at = query.at.unwrap_or_else(Utc::now);
    let offset = query
        .offset
        .unwrap_or(0)
        .clamp(-MAX_OFFSET_MINUTES, MAX_OFFSET_MINUTES);

    // Loaded once for the widest window any period needs — which is not simply
    // the year, because the week containing New Year's Day starts in December.
    let (from, to) = stats::covering_window(at, offset);
    let samples = state.index.time_samples(from, to).await?;
    let computed = stats::build(&samples, at, offset);

    let heatmap = {
        let year = computed
            .periods
            .iter()
            .find(|period| period.period == Period::Year);
        HeatmapView {
            from: year.map_or(from, |period| period.from),
            to: year.map_or(to, |period| period.to),
            cells: computed.heatmap.iter().copied().map(cell_view).collect(),
        }
    };

    Ok(Json(TimeStatsResponse {
        at: computed.at,
        offset_minutes: computed.offset_minutes,
        all_time: state.index.time_totals(at).await?.into(),
        periods: computed.periods.iter().map(period_view).collect(),
        heatmap,
    }))
}

// ------------------------------------------------------------------ helpers

fn summary(record: TimeRecord, now: DateTime<Utc>) -> TimeSummary {
    TimeSummary {
        running: record.is_running(),
        seconds: record.seconds(now),
        pages: record
            .pages
            .iter()
            .map(|page| TimePageView {
                slug: page.slug.clone(),
                exists: page.title.is_some(),
                title: page.title.clone(),
            })
            .collect(),
        id: record.id,
        name: record.name,
        start: record.start,
        end: record.end,
        has_note: record.has_note,
        snippet: record.snippet,
        updated: record.updated,
        size: record.size,
    }
}

/// Build the full view of an entry read from disk.
///
/// Page titles come from the index rather than from the file, because the file
/// only holds slugs — the same read-time join a pin and a link both use, and
/// for the same reason: renaming a page relabels every reference with nothing
/// to reindex.
async fn view(
    entry: &TimeEntry,
    state: &AppState,
    now: DateTime<Utc>,
    render: bool,
) -> AppResult<TimeView> {
    let indexed = state.index.time_record(&entry.id).await?;
    let titles: Vec<TimePageView> = match indexed {
        Some(record) => record
            .pages
            .into_iter()
            .map(|page| TimePageView {
                exists: page.title.is_some(),
                slug: page.slug,
                title: page.title,
            })
            .collect(),
        // Not indexed yet, which a caller should not be able to observe but a
        // slow scan could produce. The slugs are still the truth.
        None => entry
            .pages
            .iter()
            .map(|slug| TimePageView {
                slug: slug.clone(),
                title: None,
                exists: false,
            })
            .collect(),
    };

    Ok(TimeView {
        id: entry.id.clone(),
        name: entry.name.clone(),
        start: entry.start,
        end: entry.end,
        running: entry.is_running(),
        seconds: entry.seconds(now),
        pages: titles,
        note: entry.note.clone(),
        // Rendered as though it sat at the wiki root: a note has no slug of its
        // own, so a relative markdown link in one has nothing to be relative
        // to. Wikilinks are absolute and work as they do anywhere.
        html: render.then(|| markdown::render(None, &entry.note)),
        updated: entry.updated,
        size: entry.size,
    })
}

fn group_view(group: TimeGroup) -> TimeGroupView {
    TimeGroupView {
        name: group.name,
        entries: group.entries,
        seconds: group.seconds,
        running: group.running,
        first_start: group.first_start,
        last_start: group.last_start,
    }
}

fn period_view(period: &PeriodStats) -> PeriodStatsView {
    PeriodStatsView {
        period: period.period.as_str().to_owned(),
        from: period.from,
        to: period.to,
        seconds: period.seconds,
        entries: period.entries,
        names: period
            .names
            .iter()
            .map(|total| NameTotalView {
                name: total.name.clone(),
                seconds: total.seconds,
                entries: total.entries,
            })
            .collect(),
        pages: period
            .pages
            .iter()
            .map(|total| PageTotalView {
                slug: total.slug.clone(),
                title: total.title.clone(),
                seconds: total.seconds,
                entries: total.entries,
            })
            .collect(),
        buckets: period
            .buckets
            .iter()
            .map(|bucket| BucketView {
                start: bucket.start,
                seconds: bucket.seconds,
            })
            .collect(),
    }
}

fn cell_view(cell: HeatCell) -> HeatCellView {
    HeatCellView {
        weekday: cell.weekday,
        hour: cell.hour,
        seconds: cell.seconds,
    }
}

pub(crate) fn parse_id(raw: &str) -> AppResult<TimeId> {
    TimeId::parse(raw).map_err(|source| AppError::InvalidTimeId {
        raw: raw.to_owned(),
        source,
    })
}

/// Trim a name and refuse the two ways it can be useless.
fn check_name(name: String) -> AppResult<String> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err(AppError::InvalidRequestBody {
            message: "a time entry needs a name; it is what the entry is grouped under".to_owned(),
            kind: "time_name",
        });
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(AppError::InvalidRequestBody {
            message: format!("a time entry's name may be at most {MAX_NAME_LEN} bytes"),
            kind: "time_name",
        });
    }

    Ok(trimmed.to_owned())
}

/// Refuse an entry that ends before it starts.
fn check_range(
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
) -> AppResult<Option<DateTime<Utc>>> {
    match end {
        Some(end) if end < start => Err(AppError::TimeRangeInverted { start, end }),
        other => Ok(other),
    }
}

fn parse_sort(raw: Option<&str>) -> AppResult<TimeSortBy> {
    match raw {
        None => Ok(TimeSortBy::default()),
        Some("start") => Ok(TimeSortBy::Start),
        Some("name") => Ok(TimeSortBy::Name),
        Some("duration") => Ok(TimeSortBy::Duration),
        Some(other) => Err(AppError::InvalidParameter {
            parameter: "sort",
            value: other.to_owned(),
            allowed: &SORT_KEYS,
        }),
    }
}

fn parse_order(raw: Option<&str>) -> AppResult<SortOrder> {
    match raw {
        // Newest first, unlike every other listing here. A log is read from
        // the end.
        None => Ok(SortOrder::Descending),
        Some("asc") => Ok(SortOrder::Ascending),
        Some("desc") => Ok(SortOrder::Descending),
        Some(other) => Err(AppError::InvalidParameter {
            parameter: "order",
            value: other.to_owned(),
            allowed: &ORDER_KEYS,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn a_name_is_trimmed_and_cannot_be_blank() {
        assert_eq!(
            check_name("  Deep work \n".to_owned()).unwrap(),
            "Deep work"
        );
        assert_eq!(
            check_name("   ".to_owned()).unwrap_err().code(),
            "invalid_request_body"
        );
        assert_eq!(
            check_name("x".repeat(MAX_NAME_LEN + 1)).unwrap_err().code(),
            "invalid_request_body"
        );
    }

    #[test]
    fn a_range_that_runs_backwards_is_refused() {
        let start = at("2026-08-06T14:00:00Z");

        assert!(check_range(start, None).unwrap().is_none());
        assert!(check_range(start, Some(start)).unwrap().is_some());
        assert_eq!(
            check_range(start, Some(at("2026-08-06T13:00:00Z")))
                .unwrap_err()
                .code(),
            "time_range_inverted"
        );
    }

    /// The one listing in Rhizolog that defaults to descending.
    #[test]
    fn listing_defaults_to_newest_first() {
        assert_eq!(parse_order(None).unwrap(), SortOrder::Descending);
        assert_eq!(parse_sort(None).unwrap(), TimeSortBy::Start);
    }

    #[test]
    fn an_unknown_sort_key_names_the_alternatives() {
        let error = parse_sort(Some("size")).unwrap_err();
        assert_eq!(error.code(), "invalid_parameter");
        assert!(error.to_string().contains("duration"));
    }

    /// An id arrives from a URL and becomes a path, so a rejection has to name
    /// the rule rather than merely refusing.
    #[test]
    fn a_rejected_id_explains_itself() {
        let error = parse_id("../../etc/passwd").unwrap_err();
        assert_eq!(error.code(), "invalid_time_id");
    }
}
