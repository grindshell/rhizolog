//! Turning a pile of time entries into the numbers a dashboard shows.
//!
//! ## Why this is Rust and not SQL
//!
//! Two things make the obvious `group by strftime(...)` the wrong tool.
//!
//! The first is that **an entry can span a bucket boundary**. A session from
//! 23:00 to 01:30 is not two and a half hours on Tuesday; it is one hour on
//! Tuesday and ninety minutes on Wednesday, and on an hour-of-day heat map it
//! should light up three cells, not one. Attributing a whole entry to the
//! bucket its `start` falls in — which is what grouping by a formatted start
//! does, and what most time trackers settle for — makes the heat map say the
//! wrong thing about exactly the sessions worth looking at. Everything here
//! splits durations across boundaries instead.
//!
//! The second is **the caller's timezone**. Entries are stored in UTC because
//! an instant is an instant, but "how much did I work today" and "when am I
//! usually working" are questions about local wall-clock time. The offset comes
//! in as a parameter and every boundary below is computed in it.
//!
//! That offset is a fixed number of minutes, not a timezone, which is a real
//! limitation: a window straddling a daylight-saving change is bucketed with
//! today's offset throughout, so one day in the past may come out an hour wide
//! or three. Carrying a proper IANA zone would mean another dependency and a
//! zone database to keep current, for a single-user tool where the affected
//! numbers are two Sundays a year. It is a deliberate trade, not an oversight.

use std::collections::HashMap;

use chrono::{
    DateTime, Datelike, Days, FixedOffset, Months, NaiveDate, TimeDelta, TimeZone, Timelike, Utc,
};

use crate::slug::Slug;

/// How many entries the top-N lists in a period carry.
pub const TOP_N: usize = 10;

/// Cells in the heat map: seven weekdays by twenty-four hours.
pub const HEATMAP_CELLS: usize = 7 * 24;

/// One entry, reduced to what the arithmetic needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub name: String,
    pub start: DateTime<Utc>,
    /// `None` while the timer runs, in which case it counts up to `at`.
    pub end: Option<DateTime<Utc>>,
    /// The pages this time is attached to, with their titles.
    pub pages: Vec<SamplePage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamplePage {
    pub slug: Slug,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Year];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }
}

/// One column of a period's chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub start: DateTime<Utc>,
    pub seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameTotal {
    pub name: String,
    pub seconds: u64,
    pub entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTotal {
    pub slug: Slug,
    pub title: String,
    pub seconds: u64,
    pub entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodStats {
    pub period: Period,
    /// The window, in UTC. Its edges are local midnights.
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// Time falling inside the window, not the full length of entries that
    /// touch it.
    pub seconds: u64,
    /// Entries overlapping the window at all.
    pub entries: usize,
    /// Busiest activities first, capped at [`TOP_N`].
    pub names: Vec<NameTotal>,
    /// Busiest pages first, capped at [`TOP_N`].
    pub pages: Vec<PageTotal>,
    /// Hours for a day, days for a week or month, months for a year.
    pub buckets: Vec<Bucket>,
}

/// One square of the heat map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeatCell {
    /// 0 is Monday, matching the week the periods use.
    pub weekday: u8,
    pub hour: u8,
    pub seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeStats {
    pub at: DateTime<Utc>,
    pub offset_minutes: i32,
    /// Day, week, month, year, in that order.
    pub periods: Vec<PeriodStats>,
    /// All [`HEATMAP_CELLS`] cells, including the empty ones, so a client can
    /// draw the grid without filling gaps itself. Over the year window.
    pub heatmap: Vec<HeatCell>,
}

/// Build every figure the dashboard's time section shows.
///
/// `at` is "now": it closes running entries and decides which day, week, month
/// and year the periods describe. Passing it in rather than reading the clock
/// keeps this a pure function, which is the only reason the boundary cases
/// below are testable at all.
pub fn build(samples: &[Sample], at: DateTime<Utc>, offset_minutes: i32) -> TimeStats {
    let zone = zone(offset_minutes);
    let periods: Vec<PeriodStats> = Period::ALL
        .iter()
        .map(|period| period_stats(*period, samples, at, zone))
        .collect();

    // Over the year, which is the window that makes "when do I usually work"
    // worth asking. The periods are in `Period::ALL` order, so the year is last.
    let year = periods.last().expect("Period::ALL is not empty");
    let heatmap = heatmap(samples, at, zone, year.from, year.to);

    TimeStats {
        at,
        offset_minutes,
        periods,
        heatmap,
    }
}

/// The caller's offset as a timezone.
///
/// Anything outside ±24 hours is nonsense a client should not have sent, and UTC
/// is a better answer to it than a panic.
fn zone(offset_minutes: i32) -> FixedOffset {
    // The multiplication is checked as well as the result. A large enough number
    // of minutes overflows on the way to seconds, which in a debug build panics
    // before there is anything for `east_opt` to reject.
    offset_minutes
        .checked_mul(60)
        .and_then(FixedOffset::east_opt)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC is a valid offset"))
}

fn period_stats(
    period: Period,
    samples: &[Sample],
    at: DateTime<Utc>,
    zone: FixedOffset,
) -> PeriodStats {
    let edges = bucket_edges(period, at, zone);
    let from = *edges.first().expect("a period has at least one bucket");
    let to = *edges.last().expect("a period has at least one bucket");

    let mut seconds = 0_u64;
    let mut entries = 0_usize;
    let mut names: HashMap<&str, (u64, usize)> = HashMap::new();
    let mut pages: HashMap<&Slug, (&str, u64, usize)> = HashMap::new();
    let mut buckets = vec![0_u64; edges.len() - 1];

    for sample in samples {
        let (start, end) = span(sample, at);
        let inside = overlap(start, end, from, to);
        if inside == 0 {
            continue;
        }

        seconds += inside;
        entries += 1;

        let name = names.entry(sample.name.as_str()).or_insert((0, 0));
        name.0 += inside;
        name.1 += 1;

        for page in &sample.pages {
            let total = pages
                .entry(&page.slug)
                .or_insert((page.title.as_str(), 0, 0));
            total.1 += inside;
            total.2 += 1;
        }

        for (index, window) in edges.windows(2).enumerate() {
            buckets[index] += overlap(start, end, window[0], window[1]);
        }
    }

    let mut names: Vec<NameTotal> = names
        .into_iter()
        .map(|(name, (seconds, entries))| NameTotal {
            name: name.to_owned(),
            seconds,
            entries,
        })
        .collect();
    names.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.name.cmp(&b.name)));
    names.truncate(TOP_N);

    let mut pages: Vec<PageTotal> = pages
        .into_iter()
        .map(|(slug, (title, seconds, entries))| PageTotal {
            slug: slug.clone(),
            title: title.to_owned(),
            seconds,
            entries,
        })
        .collect();
    pages.sort_by(|a, b| b.seconds.cmp(&a.seconds).then_with(|| a.slug.cmp(&b.slug)));
    pages.truncate(TOP_N);

    PeriodStats {
        period,
        from,
        to,
        seconds,
        entries,
        names,
        pages,
        buckets: edges
            .iter()
            .zip(buckets)
            .map(|(start, seconds)| Bucket {
                start: *start,
                seconds,
            })
            .collect(),
    }
}

/// The narrowest window containing every period.
///
/// Callers load entries once and hand the whole lot to [`build`], so they need
/// to know what to load. It is not simply the year: the week containing New
/// Year's Day starts in December, so a naive year window would quietly leave
/// half of "this week" out of the numbers every January.
pub fn covering_window(at: DateTime<Utc>, offset_minutes: i32) -> (DateTime<Utc>, DateTime<Utc>) {
    let zone = zone(offset_minutes);
    let edges: Vec<Vec<DateTime<Utc>>> = Period::ALL
        .iter()
        .map(|period| bucket_edges(*period, at, zone))
        .collect();

    let first = edges
        .iter()
        .filter_map(|period| period.first().copied())
        .min()
        .unwrap_or(at);
    let last = edges
        .iter()
        .filter_map(|period| period.last().copied())
        .max()
        .unwrap_or(at);

    (first, last)
}

/// The instants separating a period's buckets: one more than there are buckets.
///
/// Every edge is built from a *date* rather than by adding a duration to the
/// first one. Days and months are calendar spans, not fixed lengths, and going
/// through the calendar is what makes February twenty-eight buckets one year
/// and twenty-nine the next without a special case.
fn bucket_edges(period: Period, at: DateTime<Utc>, zone: FixedOffset) -> Vec<DateTime<Utc>> {
    let today = at.with_timezone(&zone).date_naive();

    let midnight = |date: NaiveDate| local_midnight(date, zone).with_timezone(&Utc);

    match period {
        Period::Day => {
            let start = local_midnight(today, zone);
            (0..=24)
                .map(|hour| (start + TimeDelta::hours(hour)).with_timezone(&Utc))
                .collect()
        }
        Period::Week => {
            // Monday, matching ISO weeks. A dashboard that starts its week on
            // Sunday and a calendar that does not would disagree about what
            // "this week" means every Sunday.
            let monday = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
            (0..=7)
                .map(|day| midnight(monday + Days::new(day)))
                .collect()
        }
        Period::Month => {
            let first = today.with_day(1).unwrap_or(today);
            (0..=u64::from(days_in_month(today)))
                .map(|day| midnight(first + Days::new(day)))
                .collect()
        }
        Period::Year => {
            let january = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today);
            (0..=12)
                .map(|month| midnight(january + Months::new(month)))
                .collect()
        }
    }
}

fn days_in_month(date: NaiveDate) -> u32 {
    let first = date.with_day(1).unwrap_or(date);
    let next = first + Months::new(1);
    (next - first).num_days().unsigned_abs() as u32
}

/// Local midnight at the start of `date`, as an instant.
fn local_midnight(date: NaiveDate, zone: FixedOffset) -> DateTime<FixedOffset> {
    let naive = date.and_hms_opt(0, 0, 0).expect("midnight always exists");
    // A fixed offset has no gaps or repeats, so a local time always maps to
    // exactly one instant.
    zone.from_local_datetime(&naive)
        .single()
        .unwrap_or_else(|| DateTime::from_naive_utc_and_offset(naive, zone))
}

/// The span an entry occupies, with a running one closed at `at`.
fn span(sample: &Sample, at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let end = sample.end.unwrap_or(at);
    // A hand-edited file can put `end` before `start`; that entry contributes
    // nothing rather than subtracting from every total it touches.
    (sample.start, end.max(sample.start))
}

/// Seconds two spans share.
fn overlap(
    a_start: DateTime<Utc>,
    a_end: DateTime<Utc>,
    b_start: DateTime<Utc>,
    b_end: DateTime<Utc>,
) -> u64 {
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    (end - start).num_seconds().max(0).unsigned_abs()
}

/// Seconds worked in each weekday-and-hour square, over `[from, to)`.
fn heatmap(
    samples: &[Sample],
    at: DateTime<Utc>,
    zone: FixedOffset,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<HeatCell> {
    let mut cells = vec![0_u64; HEATMAP_CELLS];

    for sample in samples {
        let (start, end) = span(sample, at);
        let mut cursor = start.max(from);
        let stop = end.min(to);

        while cursor < stop {
            let local = cursor.with_timezone(&zone);
            let next = next_hour(local).with_timezone(&Utc);
            // Offsets are whole minutes, so an hour boundary is always ahead of
            // the cursor; the guard is here so a future change cannot spin.
            if next <= cursor {
                break;
            }

            let index = usize::try_from(local.weekday().num_days_from_monday()).unwrap_or(0) * 24
                + usize::try_from(local.hour()).unwrap_or(0);
            if let Some(cell) = cells.get_mut(index) {
                *cell += overlap(cursor, stop, cursor, next);
            }

            cursor = next;
        }
    }

    cells
        .into_iter()
        .enumerate()
        .map(|(index, seconds)| HeatCell {
            weekday: (index / 24) as u8,
            hour: (index % 24) as u8,
            seconds,
        })
        .collect()
}

/// The start of the local hour after this one.
fn next_hour(local: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    let truncated = local
        .with_minute(0)
        .and_then(|at| at.with_second(0))
        .and_then(|at| at.with_nanosecond(0))
        .unwrap_or(local);
    truncated + TimeDelta::hours(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn sample(name: &str, start: &str, end: Option<&str>) -> Sample {
        Sample {
            name: name.to_owned(),
            start: at(start),
            end: end.map(at),
            pages: Vec::new(),
        }
    }

    fn on(name: &str, start: &str, end: &str, page: &str) -> Sample {
        Sample {
            pages: vec![SamplePage {
                slug: Slug::parse(page).expect("valid slug"),
                title: page.to_uppercase(),
            }],
            ..sample(name, start, Some(end))
        }
    }

    fn period(stats: &TimeStats, period: Period) -> &PeriodStats {
        stats
            .periods
            .iter()
            .find(|entry| entry.period == period)
            .expect("period is present")
    }

    #[test]
    fn totals_the_current_day_in_the_callers_offset() {
        // 08:00 to 10:00 UTC is 01:00 to 03:00 in UTC-7.
        let samples = [sample(
            "Deep work",
            "2026-08-06T08:00:00Z",
            Some("2026-08-06T10:00:00Z"),
        )];
        let stats = build(&samples, at("2026-08-06T20:00:00Z"), -420);

        let day = period(&stats, Period::Day);
        assert_eq!(day.seconds, 2 * 3600);
        assert_eq!(day.entries, 1);
        assert_eq!(day.buckets.len(), 24);
        // Local hours 1 and 2, not UTC 8 and 9.
        assert_eq!(day.buckets[1].seconds, 3600);
        assert_eq!(day.buckets[2].seconds, 3600);
        assert_eq!(day.buckets[8].seconds, 0);
    }

    /// The same instants, read from a different desk, land on a different day.
    #[test]
    fn the_offset_decides_which_day_an_entry_falls_in() {
        let samples = [sample(
            "Late",
            "2026-08-06T23:30:00Z",
            Some("2026-08-07T00:30:00Z"),
        )];

        let utc = build(&samples, at("2026-08-07T12:00:00Z"), 0);
        assert_eq!(
            period(&utc, Period::Day).seconds,
            1800,
            "half after midnight"
        );

        // In UTC+9 the whole hour is already the 7th, locally 08:30 to 09:30.
        let tokyo = build(&samples, at("2026-08-07T12:00:00Z"), 540);
        assert_eq!(period(&tokyo, Period::Day).seconds, 3600);
    }

    /// The reason this is not a `group by` on the start time.
    #[test]
    fn an_entry_spanning_midnight_is_split_between_the_days_it_touches() {
        let samples = [sample(
            "Night",
            "2026-08-05T23:00:00Z",
            Some("2026-08-06T01:30:00Z"),
        )];
        let stats = build(&samples, at("2026-08-06T12:00:00Z"), 0);

        let day = period(&stats, Period::Day);
        assert_eq!(day.seconds, 90 * 60, "only today's share counts");
        assert_eq!(day.entries, 1);
        assert_eq!(day.buckets[0].seconds, 3600);
        assert_eq!(day.buckets[1].seconds, 30 * 60);

        // The week holds both halves.
        assert_eq!(period(&stats, Period::Week).seconds, 150 * 60);
    }

    #[test]
    fn a_week_starts_on_monday_and_has_one_bucket_per_day() {
        // 2026-08-06 is a Thursday.
        let samples = [sample(
            "Deep work",
            "2026-08-06T09:00:00Z",
            Some("2026-08-06T10:00:00Z"),
        )];
        let stats = build(&samples, at("2026-08-06T20:00:00Z"), 0);

        let week = period(&stats, Period::Week);
        assert_eq!(week.buckets.len(), 7);
        assert_eq!(week.from, at("2026-08-03T00:00:00Z"), "the Monday");
        assert_eq!(week.to, at("2026-08-10T00:00:00Z"));
        assert_eq!(week.buckets[3].seconds, 3600, "Thursday is the fourth day");
    }

    #[test]
    fn a_month_has_a_bucket_per_day_and_a_year_a_bucket_per_month() {
        let stats = build(&[], at("2026-02-15T12:00:00Z"), 0);

        assert_eq!(period(&stats, Period::Month).buckets.len(), 28);
        assert_eq!(
            period(&stats, Period::Month).from,
            at("2026-02-01T00:00:00Z")
        );
        assert_eq!(period(&stats, Period::Month).to, at("2026-03-01T00:00:00Z"));

        let year = period(&stats, Period::Year);
        assert_eq!(year.buckets.len(), 12);
        assert_eq!(year.from, at("2026-01-01T00:00:00Z"));
        assert_eq!(year.to, at("2027-01-01T00:00:00Z"));
        assert_eq!(year.buckets[1].start, at("2026-02-01T00:00:00Z"));
    }

    #[test]
    fn a_leap_february_gets_its_extra_day() {
        let stats = build(&[], at("2028-02-15T12:00:00Z"), 0);
        assert_eq!(period(&stats, Period::Month).buckets.len(), 29);
    }

    #[test]
    fn ranks_the_most_used_names() {
        let samples = [
            sample(
                "Deep work",
                "2026-08-06T08:00:00Z",
                Some("2026-08-06T10:00:00Z"),
            ),
            sample(
                "Email",
                "2026-08-06T10:00:00Z",
                Some("2026-08-06T10:30:00Z"),
            ),
            sample(
                "Deep work",
                "2026-08-06T11:00:00Z",
                Some("2026-08-06T12:00:00Z"),
            ),
        ];
        let stats = build(&samples, at("2026-08-06T20:00:00Z"), 0);

        let day = period(&stats, Period::Day);
        assert_eq!(day.names[0].name, "Deep work");
        assert_eq!(day.names[0].seconds, 3 * 3600);
        assert_eq!(day.names[0].entries, 2);
        assert_eq!(day.names[1].name, "Email");
    }

    #[test]
    fn attributes_time_to_the_pages_it_was_attached_to() {
        let samples = [
            on(
                "Deep work",
                "2026-08-06T08:00:00Z",
                "2026-08-06T10:00:00Z",
                "notes/a",
            ),
            on(
                "Reading",
                "2026-08-06T10:00:00Z",
                "2026-08-06T10:30:00Z",
                "notes/b",
            ),
        ];
        let stats = build(&samples, at("2026-08-06T20:00:00Z"), 0);

        let day = period(&stats, Period::Day);
        assert_eq!(day.pages[0].slug.as_str(), "notes/a");
        assert_eq!(day.pages[0].seconds, 2 * 3600);
        assert_eq!(day.pages[0].title, "NOTES/A");
        assert_eq!(day.pages[1].slug.as_str(), "notes/b");
    }

    #[test]
    fn a_running_entry_counts_up_to_now() {
        let samples = [sample("Deep work", "2026-08-06T09:00:00Z", None)];
        let stats = build(&samples, at("2026-08-06T09:45:00Z"), 0);

        assert_eq!(period(&stats, Period::Day).seconds, 45 * 60);
    }

    #[test]
    fn the_heat_map_lights_every_hour_a_session_touched() {
        let samples = [sample(
            "Deep work",
            "2026-08-06T22:30:00Z",
            Some("2026-08-07T01:00:00Z"),
        )];
        let stats = build(&samples, at("2026-08-07T12:00:00Z"), 0);

        let lit: Vec<(u8, u8, u64)> = stats
            .heatmap
            .iter()
            .filter(|cell| cell.seconds > 0)
            .map(|cell| (cell.weekday, cell.hour, cell.seconds))
            .collect();

        // 2026-08-06 is a Thursday (weekday 3), the 7th a Friday (weekday 4).
        assert_eq!(
            lit,
            [(3, 22, 1800), (3, 23, 3600), (4, 0, 3600)],
            "an overnight session should light three cells"
        );
    }

    #[test]
    fn the_heat_map_always_has_every_cell() {
        let stats = build(&[], at("2026-08-06T12:00:00Z"), 0);

        assert_eq!(stats.heatmap.len(), HEATMAP_CELLS);
        assert!(stats.heatmap.iter().all(|cell| cell.seconds == 0));
        assert_eq!(stats.heatmap[0].weekday, 0);
        assert_eq!(stats.heatmap[0].hour, 0);
        assert_eq!(stats.heatmap[HEATMAP_CELLS - 1].weekday, 6);
        assert_eq!(stats.heatmap[HEATMAP_CELLS - 1].hour, 23);
    }

    /// Half-hour offsets are real places, and truncating to the local hour has
    /// to respect them.
    #[test]
    fn a_half_hour_offset_shifts_the_heat_map_by_half_an_hour() {
        // 03:00-05:00 UTC is 08:30-10:30 in UTC+5:30.
        let samples = [sample(
            "Deep work",
            "2026-08-06T03:00:00Z",
            Some("2026-08-06T05:00:00Z"),
        )];
        let stats = build(&samples, at("2026-08-06T20:00:00Z"), 330);

        let lit: Vec<(u8, u64)> = stats
            .heatmap
            .iter()
            .filter(|cell| cell.seconds > 0)
            .map(|cell| (cell.hour, cell.seconds))
            .collect();
        assert_eq!(lit, [(8, 1800), (9, 3600), (10, 1800)]);
    }

    #[test]
    fn an_entry_outside_every_window_contributes_nothing() {
        let samples = [sample(
            "Old",
            "2020-01-01T09:00:00Z",
            Some("2020-01-01T17:00:00Z"),
        )];
        let stats = build(&samples, at("2026-08-06T12:00:00Z"), 0);

        for entry in &stats.periods {
            assert_eq!(entry.seconds, 0, "{:?}", entry.period);
            assert_eq!(entry.entries, 0);
        }
        assert!(stats.heatmap.iter().all(|cell| cell.seconds == 0));
    }

    #[test]
    fn an_end_before_its_start_is_ignored_rather_than_subtracted() {
        let samples = [
            sample("Good", "2026-08-06T09:00:00Z", Some("2026-08-06T10:00:00Z")),
            sample(
                "Backwards",
                "2026-08-06T14:00:00Z",
                Some("2026-08-06T13:00:00Z"),
            ),
        ];
        let stats = build(&samples, at("2026-08-06T20:00:00Z"), 0);

        assert_eq!(period(&stats, Period::Day).seconds, 3600);
    }

    /// The week straddling New Year is the case that makes the covering window
    /// more than just "the year".
    #[test]
    fn the_covering_window_reaches_back_into_last_year_for_this_week() {
        // 2027-01-01 is a Friday, so the week began on 2026-12-28.
        let (from, to) = covering_window(at("2027-01-01T12:00:00Z"), 0);

        assert_eq!(from, at("2026-12-28T00:00:00Z"));
        assert_eq!(to, at("2028-01-01T00:00:00Z"));
    }

    #[test]
    fn the_covering_window_is_the_year_in_the_ordinary_case() {
        let (from, to) = covering_window(at("2026-08-06T12:00:00Z"), 0);
        assert_eq!(from, at("2026-01-01T00:00:00Z"));
        assert_eq!(to, at("2027-01-01T00:00:00Z"));
    }

    #[test]
    fn a_nonsense_offset_falls_back_to_utc_rather_than_panicking() {
        let stats = build(&[], at("2026-08-06T12:00:00Z"), 100_000);
        assert_eq!(period(&stats, Period::Day).from, at("2026-08-06T00:00:00Z"));
    }
}
