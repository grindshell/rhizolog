//! Turning a pile of observations into the series a chart draws.
//!
//! Much simpler than [`crate::times::stats`], and for one reason: a time entry
//! has a duration and can straddle midnight, so it has to be split across
//! buckets. An observation is an **instant**, so it falls in exactly one day and
//! the whole of that module's boundary arithmetic is unnecessary here.
//!
//! What does carry over is why this is Rust and not SQL. "How much did I write
//! today" is a question about a local wall clock, and the log stores instants,
//! so the day boundaries are computed from an offset the caller supplies. A
//! precomputed day column would bake in a guess about where the writer was.
//!
//! The same limitation applies as there: the offset is a fixed number of
//! minutes rather than a zone, so a window straddling a daylight-saving change
//! is bucketed with one offset throughout. Two Sundays a year, on a single-user
//! tool, against another dependency and a zone database to keep current.

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeDelta, Utc};

use super::Kind;
use crate::slug::Slug;

/// How many entries the actor and page lists carry.
pub const TOP_N: usize = 10;

/// The longest window that will be bucketed, in days.
///
/// A caller asking for a century would otherwise be handed thirty-six thousand
/// buckets, every one of them empty. A year and a bit is what a chart holds.
pub const MAX_DAYS: usize = 400;

/// One observation, reduced to what the arithmetic needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub at: DateTime<Utc>,
    pub slug: Slug,
    /// The page's title, or `None` when the caller may not read it.
    ///
    /// The slug is the observation's own content and stays either way. Hiding
    /// the row would be hiding somebody's own working history from them, which
    /// is the rule the time log already settled.
    pub title: Option<String>,
    pub actor: String,
    pub kind: Kind,
    pub added: u64,
    pub removed: u64,
}

/// One column of the chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    pub added: u64,
    pub removed: u64,
    pub observations: usize,
}

impl Day {
    pub fn delta(&self) -> i64 {
        self.added as i64 - self.removed as i64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorTotal {
    pub actor: String,
    pub added: u64,
    pub removed: u64,
    pub observations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTotal {
    pub slug: Slug,
    /// The title, or the slug when there is none to show: an unwritten page and
    /// an unreadable one both get the honest label.
    pub title: String,
    pub added: u64,
    pub removed: u64,
    pub observations: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub added: u64,
    pub removed: u64,
    pub observations: usize,
    /// Distinct pages written to in the window.
    pub pages: usize,
}

impl Totals {
    pub fn delta(&self) -> i64 {
        self.added as i64 - self.removed as i64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordStats {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub offset_minutes: i32,
    /// Every local day in the window, including the empty ones, so a client can
    /// draw the chart without filling gaps itself.
    pub days: Vec<Day>,
    /// Busiest tools first, capped at [`TOP_N`].
    pub actors: Vec<ActorTotal>,
    /// Busiest pages first, capped at [`TOP_N`].
    pub pages: Vec<PageTotal>,
    pub totals: Totals,
}

/// Build the series over `[from, to)`.
///
/// Only observations that report **writing** are counted. A baseline, a move and
/// a delete are bookkeeping: they exist so the series can be read correctly, and
/// counting them would put a page's whole length into the day somebody first
/// pointed the server at it.
pub fn build(
    samples: &[Sample],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    offset_minutes: i32,
) -> WordStats {
    let zone = zone(offset_minutes);

    let mut by_day: HashMap<NaiveDate, Day> = HashMap::new();
    let mut by_actor: HashMap<&str, ActorTotal> = HashMap::new();
    let mut by_page: HashMap<&Slug, PageTotal> = HashMap::new();
    let mut totals = Totals::default();

    for sample in samples {
        if !sample.kind.is_work() || sample.at < from || sample.at >= to {
            continue;
        }

        let date = sample.at.with_timezone(&zone).date_naive();
        let day = by_day.entry(date).or_insert(Day {
            date,
            added: 0,
            removed: 0,
            observations: 0,
        });
        day.added += sample.added;
        day.removed += sample.removed;
        day.observations += 1;

        let actor = by_actor
            .entry(sample.actor.as_str())
            .or_insert_with(|| ActorTotal {
                actor: sample.actor.clone(),
                added: 0,
                removed: 0,
                observations: 0,
            });
        actor.added += sample.added;
        actor.removed += sample.removed;
        actor.observations += 1;

        let page = by_page.entry(&sample.slug).or_insert_with(|| PageTotal {
            slug: sample.slug.clone(),
            title: sample
                .title
                .clone()
                .unwrap_or_else(|| sample.slug.to_string()),
            added: 0,
            removed: 0,
            observations: 0,
        });
        page.added += sample.added;
        page.removed += sample.removed;
        page.observations += 1;

        totals.added += sample.added;
        totals.removed += sample.removed;
        totals.observations += 1;
    }

    totals.pages = by_page.len();

    WordStats {
        days: calendar(from, to, zone, &by_day),
        actors: ranked(by_actor.into_values().collect(), |total| {
            (total.added, total.removed)
        }),
        pages: ranked(by_page.into_values().collect(), |total| {
            (total.added, total.removed)
        }),
        totals,
        from,
        to,
        offset_minutes,
    }
}

/// Every local day the window touches, in order, with the empty ones filled in.
fn calendar(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: FixedOffset,
    found: &HashMap<NaiveDate, Day>,
) -> Vec<Day> {
    if to <= from {
        return Vec::new();
    }

    let first = from.with_timezone(&zone).date_naive();
    // The last instant inside the window rather than the boundary itself, so a
    // window ending at local midnight does not grow a day nothing can fall in.
    let last = (to - TimeDelta::nanoseconds(1))
        .with_timezone(&zone)
        .date_naive();

    let mut days = Vec::new();
    let mut date = first;

    while date <= last && days.len() < MAX_DAYS {
        days.push(found.get(&date).copied().unwrap_or(Day {
            date,
            added: 0,
            removed: 0,
            observations: 0,
        }));

        let Some(next) = date.succ_opt() else { break };
        date = next;
    }

    days
}

/// Busiest first, then by whatever the tie-break in `key` leaves, capped.
///
/// Sorted by `added` before `removed`, because the question is what was written.
/// The final tie-break is the name, so two runs over the same log never disagree
/// about the order.
fn ranked<T, K>(mut totals: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T>
where
    K: Ord,
    T: Named,
{
    totals.sort_by(|left, right| {
        key(right)
            .cmp(&key(left))
            .then_with(|| left.name().cmp(right.name()))
    });
    totals.truncate(TOP_N);
    totals
}

/// What a ranked total is called, for the tie-break.
trait Named {
    fn name(&self) -> &str;
}

impl Named for ActorTotal {
    fn name(&self) -> &str {
        &self.actor
    }
}

impl Named for PageTotal {
    fn name(&self) -> &str {
        self.slug.as_str()
    }
}

/// The caller's offset as a timezone.
///
/// Anything outside twenty-four hours is nonsense a client should not have sent,
/// and UTC is a better answer to it than a panic. The same rule
/// [`crate::times::stats`] applies, with the multiplication checked as well as
/// the result: a large enough number of minutes overflows on the way to seconds,
/// which in a debug build is a panic before there is anything to reject.
pub fn zone(offset_minutes: i32) -> FixedOffset {
    offset_minutes
        .checked_mul(60)
        .and_then(FixedOffset::east_opt)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC is a valid offset"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn sample(when: &str, slug: &str, actor: &str, added: u64, removed: u64) -> Sample {
        Sample {
            at: at(when),
            slug: Slug::parse(slug).expect("valid slug"),
            title: Some(slug.to_uppercase()),
            actor: actor.to_owned(),
            kind: Kind::Observed,
            added,
            removed,
        }
    }

    fn day(stats: &WordStats, date: &str) -> Day {
        let wanted: NaiveDate = date.parse().expect("valid date");
        *stats
            .days
            .iter()
            .find(|day| day.date == wanted)
            .unwrap_or_else(|| panic!("{date} is not in the window"))
    }

    #[test]
    fn a_day_carries_both_halves_and_leaves_the_subtraction_to_the_reader() {
        let samples = [
            sample("2026-08-25T09:00:00Z", "a", "claude-code", 1900, 2000),
            sample("2026-08-25T17:00:00Z", "b", "web", 300, 0),
        ];

        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        let day = day(&stats, "2026-08-25");
        assert_eq!((day.added, day.removed), (2200, 2000));
        assert_eq!(day.delta(), 200);
        assert_eq!(day.observations, 2);
        assert_eq!(stats.totals.pages, 2);
    }

    /// The reason the bucketing is not a `group by` in SQL.
    #[test]
    fn the_offset_decides_which_day_an_observation_falls_in() {
        let samples = [sample("2026-08-26T04:00:00Z", "a", "web", 100, 0)];
        let (from, to) = (at("2026-08-24T00:00:00Z"), at("2026-08-28T00:00:00Z"));

        // In UTC it is the small hours of the 26th.
        assert_eq!(day(&build(&samples, from, to, 0), "2026-08-26").added, 100);
        // Seven hours behind, it is still the evening of the 25th.
        assert_eq!(
            day(&build(&samples, from, to, -420), "2026-08-25").added,
            100
        );
    }

    /// A chart has to be able to draw the gaps.
    #[test]
    fn every_day_in_the_window_is_present_including_the_empty_ones() {
        let samples = [sample("2026-08-25T09:00:00Z", "a", "web", 10, 0)];

        let stats = build(
            &samples,
            at("2026-08-23T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.days.len(), 3);
        assert_eq!(day(&stats, "2026-08-23").observations, 0);
        assert_eq!(day(&stats, "2026-08-25").observations, 1);
    }

    /// A window that ends at local midnight must not grow a day nothing can fall
    /// in.
    #[test]
    fn a_window_ending_at_midnight_stops_the_day_before() {
        let stats = build(
            &[],
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.days.len(), 1);
        assert_eq!(stats.days[0].date.to_string(), "2026-08-25");
    }

    #[test]
    fn observations_outside_the_window_are_not_counted() {
        let samples = [
            sample("2026-08-24T23:59:59Z", "a", "web", 500, 0),
            sample("2026-08-25T12:00:00Z", "a", "web", 10, 0),
            sample("2026-08-26T00:00:00Z", "a", "web", 700, 0),
        ];

        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.totals.added, 10);
        assert_eq!(stats.totals.observations, 1);
    }

    /// The question is "how much of today came through Claude", which is what
    /// the actor split is for.
    #[test]
    fn actors_are_ranked_by_what_they_wrote() {
        let samples = [
            sample("2026-08-25T09:00:00Z", "a", "claude-code", 1900, 2000),
            sample("2026-08-25T10:00:00Z", "a", "web", 120, 5),
            sample("2026-08-25T11:00:00Z", "b", "web", 80, 0),
        ];

        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.actors.len(), 2);
        assert_eq!(stats.actors[0].actor, "claude-code");
        assert_eq!(stats.actors[0].added, 1900);
        assert_eq!(stats.actors[1].actor, "web");
        assert_eq!(stats.actors[1].added, 200);
        assert_eq!(stats.actors[1].observations, 2);
    }

    /// Bookkeeping is not writing. A baseline putting a page's whole length into
    /// the day the server first saw it is the failure this guards against.
    #[test]
    fn a_baseline_a_move_and_a_delete_are_not_words_written() {
        let samples = [
            Sample {
                kind: Kind::Baseline,
                ..sample("2026-08-25T09:00:00Z", "a", "scan", 0, 0)
            },
            Sample {
                kind: Kind::Moved,
                ..sample("2026-08-25T10:00:00Z", "b", "web", 0, 0)
            },
            Sample {
                kind: Kind::Deleted,
                ..sample("2026-08-25T11:00:00Z", "c", "web", 0, 0)
            },
            sample("2026-08-25T12:00:00Z", "d", "web", 40, 0),
        ];

        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.totals.observations, 1);
        assert_eq!(stats.totals.added, 40);
        assert_eq!(stats.totals.pages, 1);
        assert_eq!(stats.pages.len(), 1);
    }

    /// A net is still writing, and is counted as such. It is labelled in the log
    /// so a reader knows the split is by sign rather than by diff.
    #[test]
    fn a_net_observation_counts_as_work() {
        let samples = [Sample {
            kind: Kind::Net,
            ..sample("2026-08-25T09:00:00Z", "a", "scan", 0, 300)
        }];

        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.totals.removed, 300);
        assert_eq!(stats.totals.observations, 1);
    }

    /// A page the caller may not read keeps its slug and loses its title, which
    /// is the time log's rule and not a new one.
    #[test]
    fn a_page_with_no_title_is_ranked_under_its_slug() {
        let samples = [Sample {
            title: None,
            ..sample("2026-08-25T09:00:00Z", "private/diary", "web", 900, 0)
        }];

        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            0,
        );

        assert_eq!(stats.pages[0].title, "private/diary");
        assert_eq!(stats.pages[0].added, 900, "the words really were written");
    }

    #[test]
    fn a_window_nobody_wrote_in_is_a_series_of_zeroes_rather_than_nothing() {
        let stats = build(
            &[],
            at("2026-08-24T00:00:00Z"),
            at("2026-08-27T00:00:00Z"),
            0,
        );

        assert_eq!(stats.days.len(), 3);
        assert_eq!(stats.totals, Totals::default());
        assert!(stats.actors.is_empty());
        assert!(stats.pages.is_empty());
    }

    #[test]
    fn an_impossible_window_is_empty_rather_than_a_panic() {
        let stats = build(
            &[],
            at("2026-08-27T00:00:00Z"),
            at("2026-08-24T00:00:00Z"),
            0,
        );
        assert!(stats.days.is_empty());
    }

    #[test]
    fn a_nonsense_offset_falls_back_to_utc() {
        let samples = [sample("2026-08-25T12:00:00Z", "a", "web", 10, 0)];
        let stats = build(
            &samples,
            at("2026-08-25T00:00:00Z"),
            at("2026-08-26T00:00:00Z"),
            i32::MAX,
        );

        assert_eq!(day(&stats, "2026-08-25").added, 10);
    }

    /// A caller asking for a century gets a year and a bit rather than thirty-six
    /// thousand empty buckets.
    #[test]
    fn the_window_is_capped_at_a_length_a_chart_can_hold() {
        let stats = build(
            &[],
            at("2000-01-01T00:00:00Z"),
            at("2100-01-01T00:00:00Z"),
            0,
        );

        assert_eq!(stats.days.len(), MAX_DAYS);
    }
}
