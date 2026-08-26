//! Words remaining over days remaining: `pace/v1`.
//!
//! A pure function of a compiled manifest, a slice of word-log observations and
//! an explicit instant. Nothing here is stored: a manuscript falls behind
//! because days pass and nothing is written, so a number in a table would be an
//! answer to a question nobody had asked yet. That is the same argument
//! [`crate::ideas::lifecycle`] makes for computing momentum on read, and it
//! removes the same background scheduler.
//!
//! ## It is arithmetic, not encouragement
//!
//! There is no verdict here. No `on_track`, no streak, no colour with a mood
//! attached, and nothing that changes tone when a number goes up. What the
//! response carries is two rates in the same unit, [`Pace::required_per_day`]
//! and [`PaceWindow::per_day`], and the reader compares them. A tool that told
//! somebody they were behind would be having an opinion about their week, which
//! is what [`crate::ideas::lifecycle`] refuses when it declines to move an
//! idea's lifecycle on its own.
//!
//! ## Every number is checkable from the response
//!
//! Each figure travels with the values it was divided from: `remaining` with
//! `target` and `words`, `required_per_day` with `remaining` and
//! `days_remaining`, `per_day` with the window's `net` and `days`. A reader can
//! recompute the lot without seeing this file, which is the gate the whole
//! feature is measured against.
//!
//! ## Net is the right number here, and only here
//!
//! The word log deliberately never stores a difference: a rewrite of two
//! thousand words into nineteen hundred is not "minus one hundred", and the
//! whole of [`crate::words`] exists to keep the two halves apart. Pacing
//! computes a net anyway, because a `target` **is** a net quantity: it is how
//! long the work should end up, and cutting two hundred words moves you away
//! from it exactly as surely as writing two hundred moves you toward it. So the
//! difference is taken here, at the point where it is the question, and both
//! halves come back beside it so nobody has to take it on trust.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::compile::{Compiled, Status};
use crate::words::stats::{self, Sample, TOP_N};

/// The name this ruleset answers to, returned with every response.
///
/// Changing how the pace is computed is a version change here, following
/// `compile/v1`, `prose/v1` and `idea-momentum/v1`.
pub const RULESET: &str = "pace/v1";

/// How far back a request that named no window looks.
///
/// A fortnight: long enough that one day off does not halve the rate, short
/// enough that it describes what somebody is doing now rather than what they did
/// in the spring. The same figure `idea-momentum/v1` calls recent, and for the
/// same reason rather than by coincidence.
pub const DEFAULT_WINDOW_DAYS: u32 = 14;

/// The longest window that will be measured.
///
/// [`stats::MAX_DAYS`], because the two are answering over the same log and a
/// pace window longer than the chart can draw is a rate nothing can be checked
/// against.
pub const MAX_WINDOW_DAYS: u32 = stats::MAX_DAYS as u32;

/// How far ahead a projection is worth making, in days.
///
/// A century. Past this the finish date is not a figure, it is what dividing by
/// a rate near zero produces, and [`Pace::projected_finish`] is absent instead.
/// [`Pace::projected_days`] still carries the number, so the reason is visible
/// rather than silent.
pub const MAX_PROJECTION_DAYS: i64 = 36_500;

/// What was written at one page inside the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PacePage {
    #[schema(example = "book/one/the-ferry")]
    pub slug: String,
    /// The page's title, or absent when this caller may not read it.
    ///
    /// The row itself stays either way. The slug is the observation's own
    /// content, and hiding somebody's working history from them would be the
    /// wrong reading of a rule that exists to protect other people's pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "The Ferry")]
    pub title: Option<String>,
    pub added: u64,
    pub removed: u64,
    /// `added - removed`.
    pub net: i64,
    pub observations: usize,
}

/// What was written over some set of pages, and where.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct PaceUncounted {
    pub added: u64,
    pub removed: u64,
    /// `added - removed`. See the module docs for why a net is the right figure
    /// here and nowhere else in the word log.
    pub net: i64,
    pub observations: usize,
    /// The busiest pages first, capped at ten.
    ///
    /// Provenance rather than the sum: the totals above are over **every** page,
    /// so a long manuscript does not quietly report a smaller number than it
    /// measured. `GET /api/word-stats` is where the whole list lives.
    pub pages: Vec<PacePage>,
}

/// The trailing window the observed rate was measured over.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct PaceWindow {
    /// How many days long, after clamping.
    #[schema(example = 14)]
    pub days: u32,
    pub from: DateTime<Utc>,
    /// Exclusive, and the end of the local day holding `at` rather than `at`
    /// itself, so today counts as a whole day and not a half-finished one.
    pub to: DateTime<Utc>,
    pub added: u64,
    pub removed: u64,
    pub net: i64,
    pub observations: usize,
    /// Distinct local days something was written on.
    ///
    /// Not what `per_day` divides by, and here so the other division is
    /// available to anybody who wants it. A projection is against a calendar, so
    /// the rate that projects has to be over calendar days; "what I do when I
    /// sit down" is a different and equally real question.
    #[schema(example = 4)]
    pub active_days: usize,
    /// `net / days`. Words a day, over the calendar rather than over the days
    /// somebody worked.
    #[schema(example = 43.285714285714285)]
    pub per_day: f64,
    /// The busiest pages in the manuscript first, capped at ten.
    pub pages: Vec<PacePage>,
}

/// Where a manuscript is against its target and its deadline.
// Serialized as it stands. This struct and the three above are the response's
// wire shape as well as this module's internal one, so there is no second
// spelling to keep in step and no chance of the API reporting a number the
// arithmetic did not compute. Their doc comments become OpenAPI descriptions,
// which is why none of them carries a rustdoc link.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Pace {
    /// Always `pace/v1`. Changing how the pace is computed changes this.
    #[schema(example = "pace/v1")]
    pub ruleset: &'static str,
    #[schema(example = "book")]
    pub root: String,
    /// The instant every figure here was computed against.
    pub at: DateTime<Utc>,
    /// The offset the window's days were cut in, as applied after clamping.
    #[schema(example = -420)]
    pub offset_minutes: i32,
    /// The compiled total: what a reader would actually get today.
    ///
    /// Not the sum of what was written. An excluded page's words are in the word
    /// log and not in the book, which is the one contrast worth knowing about
    /// here: see `uncounted` below.
    #[schema(example = 606)]
    pub words: u64,
    /// The root's `target`, if it names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 2000)]
    pub target: Option<u64>,
    /// `target - words`, **signed**.
    ///
    /// Negative on a manuscript past its target, which is worth seeing: a target
    /// is a length somebody is aiming at rather than a ceiling, and overshooting
    /// it by five thousand words is a fact about the work.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1394)]
    pub remaining: Option<i64>,
    /// The day the root is due, if its frontmatter names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<DateTime<Utc>>,
    /// Whole days from today to the due day, **counting today**.
    ///
    /// Zero once the day has passed, which is the only thing zero ever means:
    /// due today is one day, not none. Counted in UTC days, because `due` names
    /// a UTC day and reading it in a caller's own offset is what would show
    /// "due 30 September" as the 29th to anybody west of Greenwich. The window
    /// below is cut in the caller's offset instead, because which day an
    /// observation fell on is a question about a wall clock and this is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 56)]
    pub days_remaining: Option<i64>,
    /// `remaining / days_remaining`.
    ///
    /// Absent when there is nothing left to write or no day left to write it in,
    /// rather than infinite or negative. Compare it with the window's `per_day`,
    /// which is in the same unit and is the whole reason both are here.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 24.892857142857142)]
    pub required_per_day: Option<f64>,
    /// What was written in the manuscript over the trailing window.
    pub window: PaceWindow,
    /// `ceil(remaining / per_day)`: days at the observed rate.
    ///
    /// Absent when the target is met, or when the window's net is zero or
    /// negative. A fortnight of cutting projects no finish at all, and saying so
    /// is more use than a date arrived at by dividing by nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 33)]
    pub projected_days: Option<i64>,
    /// The day `projected_days` lands on, counting today as the first.
    ///
    /// Absent past a hundred years out, where a date stops being a figure and
    /// starts being what dividing by a rate near zero produces. The number above
    /// stays, so the reason is visible rather than silent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projected_finish: Option<DateTime<Utc>>,
    /// Words written in the window on pages this document does **not** carry.
    ///
    /// A cut scene, a chapter under an excluded part, a page deleted since. They
    /// were written and the word log counts them; they are not in `words` and
    /// they are not in the rate, because the rate has to be in the same currency
    /// as `remaining` or dividing one by the other means nothing: a day spent on
    /// a scene that is out of the book does not move the compiled total, and
    /// counting it would project a finish that never arrives.
    ///
    /// Reported rather than dropped so that contrast is visible. It is the
    /// commonest way to misread the two numbers.
    pub uncounted: PaceUncounted,
}

/// What is being asked, apart from the manuscript itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub root: String,
    pub at: DateTime<Utc>,
    /// Minutes east of UTC, already clamped by the caller.
    pub offset_minutes: i32,
    pub window_days: u32,
}

/// The trailing window `[from, to)` a request asks about.
///
/// Public because the caller has to run it **before** it can read the log and
/// again inside [`build`]: one function called twice rather than two readings of
/// "the last fortnight" that could drift apart.
pub fn window(
    at: DateTime<Utc>,
    offset_minutes: i32,
    window_days: u32,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let zone = stats::zone(offset_minutes);
    let to = stats::end_of_local_day(at, zone);
    let days = window_days.clamp(1, MAX_WINDOW_DAYS);

    let from = to
        .checked_sub_days(Days::new(u64::from(days)))
        .unwrap_or(to);

    (from, to)
}

/// Where `compiled` stands, given what the log says was written.
///
/// `samples` is expected to be the window's observations and is filtered again
/// anyway, so this is a function of its arguments and a test can hand it a
/// year's worth and get the fortnight back.
pub fn build(question: &Question, compiled: &Compiled, samples: &[Sample]) -> Pace {
    let days = question.window_days.clamp(1, MAX_WINDOW_DAYS);
    let (from, to) = window(question.at, question.offset_minutes, days);
    let zone = stats::zone(question.offset_minutes);

    // What the document actually carries. A `duplicate` entry names a page that
    // is included somewhere else, so it is already in here; a `wanted` or
    // `invalid` one names no page at all.
    let carried: HashSet<&str> = compiled
        .sections
        .iter()
        .filter(|section| section.status == Status::Included)
        .map(|section| section.slug.as_str())
        .collect();

    // Everything the spine names that the document left behind. Defined by
    // subtraction rather than by status, which is what makes an appendix listed
    // twice count once and stay out of here.
    let left_behind: HashSet<&str> = compiled
        .sections
        .iter()
        .map(|section| section.slug.as_str())
        .filter(|slug| !carried.contains(slug))
        .collect();

    let mut counted = Tally::default();
    let mut uncounted = Tally::default();
    let mut active: HashSet<NaiveDate> = HashSet::new();

    for sample in samples {
        // A baseline, a move and a delete are bookkeeping, exactly as they are
        // to the chart. Counting them would put a page's whole length into the
        // day somebody first pointed the server at it and report a fortnight's
        // work that nobody did.
        if !sample.kind.is_work() || sample.at < from || sample.at >= to {
            continue;
        }

        let slug = sample.slug.as_str();

        if carried.contains(slug) {
            counted.add(sample);
            active.insert(sample.at.with_timezone(&zone).date_naive());
        } else if left_behind.contains(slug) {
            uncounted.add(sample);
        }
    }

    let per_day = counted.net() as f64 / f64::from(days);
    // Saturating rather than `as`, which wraps. A hand-written `target:` larger
    // than an `i64` would otherwise come back as a *negative* remainder, which
    // reads as a manuscript past its target: precisely the wrong-way-round
    // failure `Frontmatter::target` is parsed strictly to avoid. An absurd
    // target should read as an absurd amount left to write.
    let remaining = compiled
        .target
        .map(|target| saturating(target).saturating_sub(saturating(compiled.words)));
    let days_remaining = compiled.due.map(|due| days_remaining(question.at, due));

    let required_per_day = match (remaining, days_remaining) {
        (Some(left), Some(days)) if left > 0 && days > 0 => Some(left as f64 / days as f64),
        _ => None,
    };

    let projected_days = match remaining {
        Some(left) if left > 0 && per_day > 0.0 => Some((left as f64 / per_day).ceil() as i64),
        _ => None,
    };

    Pace {
        ruleset: RULESET,
        root: question.root.clone(),
        at: question.at,
        offset_minutes: question.offset_minutes,
        words: compiled.words,
        target: compiled.target,
        remaining,
        due: compiled.due,
        days_remaining,
        required_per_day,
        window: PaceWindow {
            days,
            from,
            to,
            added: counted.added,
            removed: counted.removed,
            net: counted.net(),
            observations: counted.observations,
            active_days: active.len(),
            per_day,
            pages: counted.ranked(),
        },
        projected_days,
        projected_finish: projected_days.and_then(|ahead| finish(question.at, ahead)),
        uncounted: PaceUncounted {
            added: uncounted.added,
            removed: uncounted.removed,
            net: uncounted.net(),
            observations: uncounted.observations,
            pages: uncounted.ranked(),
        },
    }
}

/// A word count as a signed number, with anything past the ceiling held there.
fn saturating(words: u64) -> i64 {
    i64::try_from(words).unwrap_or(i64::MAX)
}

/// Whole days from the day holding `at` to the day holding `due`, counting both.
///
/// Both read in UTC. See [`Pace::days_remaining`] for why this one question is
/// not asked in the caller's offset.
fn days_remaining(at: DateTime<Utc>, due: DateTime<Utc>) -> i64 {
    let gap = (due.date_naive() - at.date_naive()).num_days();
    (gap + 1).max(0)
}

/// The day a projection lands on, counting today as the first of them.
///
/// `None` past [`MAX_PROJECTION_DAYS`], and `None` again if the addition would
/// leave the calendar, which is the same answer for the same reason.
fn finish(at: DateTime<Utc>, ahead: i64) -> Option<DateTime<Utc>> {
    if !(1..=MAX_PROJECTION_DAYS).contains(&ahead) {
        return None;
    }

    let day = at
        .date_naive()
        .checked_add_days(Days::new((ahead - 1) as u64))?;

    Some(stats::local_midnight(day, stats::zone(0)))
}

/// Running totals over one set of pages.
#[derive(Debug, Default)]
struct Tally {
    added: u64,
    removed: u64,
    observations: usize,
    pages: HashMap<String, PacePage>,
}

impl Tally {
    fn add(&mut self, sample: &Sample) {
        self.added += sample.added;
        self.removed += sample.removed;
        self.observations += 1;

        let page = self
            .pages
            .entry(sample.slug.to_string())
            .or_insert_with(|| PacePage {
                slug: sample.slug.to_string(),
                title: sample.title.clone(),
                added: 0,
                removed: 0,
                net: 0,
                observations: 0,
            });

        page.added += sample.added;
        page.removed += sample.removed;
        page.net = page.added as i64 - page.removed as i64;
        page.observations += 1;
    }

    fn net(&self) -> i64 {
        self.added as i64 - self.removed as i64
    }

    /// Busiest first, capped, with the slug as the final tie-break so two runs
    /// over one log never disagree about the order.
    fn ranked(self) -> Vec<PacePage> {
        let mut pages: Vec<PacePage> = self.pages.into_values().collect();

        pages.sort_by(|left, right| {
            (right.added, right.removed)
                .cmp(&(left.added, left.removed))
                .then_with(|| left.slug.cmp(&right.slug))
        });
        pages.truncate(TOP_N);
        pages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::Section;
    use crate::slug::Slug;
    use crate::words::Kind;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn section(slug: &str, status: Status, words: u64) -> Section {
        Section {
            slug: slug.to_owned(),
            title: None,
            synopsis: None,
            stage: None,
            target: None,
            depth: 0,
            words,
            subtree: words,
            offset: 0,
            length: 0,
            status,
        }
    }

    fn compiled(
        words: u64,
        target: Option<u64>,
        due: Option<&str>,
        sections: Vec<Section>,
    ) -> Compiled {
        Compiled {
            markdown: String::new(),
            sections,
            words,
            target,
            due: due.map(at),
        }
    }

    fn sample(when: &str, slug: &str, added: u64, removed: u64) -> Sample {
        Sample {
            at: at(when),
            slug: Slug::parse(slug).expect("valid slug"),
            title: Some(slug.to_uppercase()),
            actor: "web".to_owned(),
            kind: Kind::Observed,
            added,
            removed,
        }
    }

    fn question(at_text: &str) -> Question {
        Question {
            root: "book".to_owned(),
            at: at(at_text),
            offset_minutes: 0,
            window_days: DEFAULT_WINDOW_DAYS,
        }
    }

    /// The whole feature in one case, and every number in it checkable from the
    /// three above it.
    #[test]
    fn words_remaining_over_days_remaining() {
        let book = compiled(
            600,
            Some(2_000),
            Some("2026-09-30T00:00:00Z"),
            vec![
                section("book", Status::Included, 100),
                section("book/one", Status::Included, 500),
            ],
        );
        let samples = [
            sample("2026-08-03T16:45:00Z", "book", 100, 0),
            sample("2026-08-05T11:15:00Z", "book/one", 620, 20),
        ];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.ruleset, "pace/v1");
        assert_eq!(pace.words, 600);
        assert_eq!(pace.remaining, Some(1_400));
        // 6 August to 30 September is 55 days, and today is one of them.
        assert_eq!(pace.days_remaining, Some(56));
        assert_eq!(pace.required_per_day, Some(1_400.0 / 56.0));

        assert_eq!((pace.window.added, pace.window.removed), (720, 20));
        assert_eq!(pace.window.net, 700);
        assert_eq!(pace.window.per_day, 700.0 / 14.0);
        assert_eq!(pace.window.active_days, 2);

        // 1,400 to go at 50 a day is 28 days, today being the first of them.
        assert_eq!(pace.projected_days, Some(28));
        assert_eq!(pace.projected_finish, Some(at("2026-09-02T00:00:00Z")));
    }

    /// The contrast the whole `uncounted` block exists for. A day spent on a
    /// scene that is out of the book is a day the compiled total did not move.
    #[test]
    fn words_written_into_a_cut_scene_are_reported_and_not_counted() {
        let book = compiled(
            100,
            Some(1_000),
            None,
            vec![
                section("book", Status::Included, 100),
                section("book/cut", Status::Excluded, 0),
            ],
        );
        let samples = [
            sample("2026-08-05T09:00:00Z", "book", 100, 0),
            sample("2026-08-05T14:20:00Z", "book/cut", 107, 0),
        ];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.net, 100, "only what a reader would get");
        assert_eq!(pace.uncounted.net, 107, "and they really were written");
        assert_eq!(pace.uncounted.observations, 1);
        assert_eq!(pace.uncounted.pages[0].slug, "book/cut");
        assert_eq!(
            pace.window.pages.len(),
            1,
            "the cut scene is in neither list twice"
        );
    }

    /// An appendix under two parts is one page, and the position that reported
    /// `duplicate` must not turn its words into words outside the book.
    #[test]
    fn a_page_listed_twice_is_counted_once_and_is_not_left_behind() {
        let book = compiled(
            80,
            None,
            None,
            vec![
                section("book", Status::Included, 0),
                section("book/appendix", Status::Included, 80),
                section("book/appendix", Status::Duplicate, 0),
            ],
        );
        let samples = [sample("2026-08-06T11:30:00Z", "book/appendix", 81, 1)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.observations, 1);
        assert_eq!(pace.window.net, 80);
        assert_eq!(pace.uncounted, PaceUncounted::default());
    }

    /// The case the subtraction is actually for, and the one `duplicate` above is
    /// not: an appendix under one excluded part and one included part is
    /// `excluded` at one position and `included` at the other. It is in the
    /// document, so it is in the rate, and it must not also be reported as words
    /// the document left behind.
    #[test]
    fn a_page_excluded_in_one_position_and_carried_in_another_is_counted_once() {
        let book = compiled(
            80,
            None,
            None,
            vec![
                section("book", Status::Included, 0),
                section("book/cut-part", Status::Excluded, 0),
                section("book/appendix", Status::Excluded, 0),
                section("book/two", Status::Included, 0),
                section("book/appendix", Status::Included, 80),
            ],
        );
        let samples = [sample("2026-08-06T11:30:00Z", "book/appendix", 81, 1)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.net, 80);
        assert_eq!(pace.window.observations, 1);
        assert_eq!(
            pace.uncounted,
            PaceUncounted::default(),
            "the appendix is in the book, so it is not also outside it"
        );
    }

    /// The rate is over the calendar, because the projection is against one.
    /// What somebody does when they sit down is a different question and gets a
    /// different field rather than a different rate.
    #[test]
    fn the_rate_divides_by_the_window_and_not_by_the_days_worked() {
        let book = compiled(0, None, None, vec![section("book", Status::Included, 0)]);
        let samples = [
            sample("2026-08-05T09:00:00Z", "book", 700, 0),
            sample("2026-08-06T09:00:00Z", "book", 700, 0),
        ];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.active_days, 2);
        assert_eq!(pace.window.per_day, 100.0, "1,400 over fourteen days");
    }

    /// Bookkeeping is not writing, which is the chart's rule and not a second
    /// one. A baseline is the case that would otherwise report a wiki somebody
    /// pointed a server at as a fortnight's work.
    #[test]
    fn a_baseline_is_neither_words_nor_a_day_somebody_wrote() {
        let book = compiled(0, None, None, vec![section("book", Status::Included, 0)]);
        let samples = [Sample {
            kind: Kind::Baseline,
            ..sample("2026-08-04T09:00:00Z", "book", 0, 0)
        }];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.observations, 0);
        assert_eq!(pace.window.active_days, 0);
        assert!(pace.window.pages.is_empty());
    }

    #[test]
    fn observations_before_the_window_are_not_in_the_rate() {
        let book = compiled(0, None, None, vec![section("book", Status::Included, 0)]);
        let samples = [
            sample("2026-07-20T09:00:00Z", "book", 5_000, 0),
            sample("2026-08-06T09:00:00Z", "book", 140, 0),
        ];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.net, 140);
        assert_eq!(pace.window.per_day, 10.0);
    }

    /// A fortnight spent cutting projects nothing, and saying nothing is more
    /// use than a date arrived at by dividing by zero.
    #[test]
    fn a_window_that_took_words_away_projects_no_finish() {
        let book = compiled(
            600,
            Some(2_000),
            None,
            vec![section("book", Status::Included, 600)],
        );
        let samples = [sample("2026-08-05T09:00:00Z", "book", 100, 400)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.net, -300);
        assert_eq!(pace.projected_days, None);
        assert_eq!(pace.projected_finish, None);
    }

    /// A `target:` nobody could mean is still not a manuscript that is finished.
    ///
    /// `as` would wrap this to a negative remainder, which reads as past its
    /// target, and reporting a page as finished is the exact failure the strict
    /// parse on `target` exists to prevent for a negative one.
    #[test]
    fn a_target_too_large_for_a_signed_count_is_held_at_the_ceiling() {
        let book = compiled(
            0,
            Some(u64::MAX),
            None,
            vec![section("book", Status::Included, 0)],
        );

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &[]);

        assert_eq!(pace.remaining, Some(i64::MAX));
        assert!(
            pace.remaining.is_some_and(|left| left > 0),
            "an absurd target is an absurd amount left, not a finished book"
        );
    }

    /// A target is a length somebody is aiming at rather than a ceiling.
    #[test]
    fn a_manuscript_past_its_target_reports_a_negative_remainder_and_no_rate() {
        let book = compiled(
            2_400,
            Some(2_000),
            Some("2026-09-30T00:00:00Z"),
            vec![section("book", Status::Included, 2_400)],
        );

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &[]);

        assert_eq!(pace.remaining, Some(-400));
        assert_eq!(pace.required_per_day, None);
        assert_eq!(pace.projected_days, None);
    }

    /// Due today is one day, not none. Zero means the day has gone.
    #[test]
    fn the_day_it_is_due_is_a_day_you_still_have() {
        let today = compiled(
            0,
            Some(100),
            Some("2026-08-06T00:00:00Z"),
            vec![section("book", Status::Included, 0)],
        );
        let pace = build(&question("2026-08-06T18:00:00Z"), &today, &[]);
        assert_eq!(pace.days_remaining, Some(1));
        assert_eq!(pace.required_per_day, Some(100.0));

        let yesterday = compiled(
            0,
            Some(100),
            Some("2026-08-05T00:00:00Z"),
            vec![section("book", Status::Included, 0)],
        );
        let pace = build(&question("2026-08-06T18:00:00Z"), &yesterday, &[]);
        assert_eq!(pace.days_remaining, Some(0));
        assert_eq!(
            pace.required_per_day, None,
            "no day left to write it in, rather than an infinite rate"
        );
    }

    /// A `due` in the small hours of a UTC day is that day, whatever offset the
    /// caller is reading their own week in.
    #[test]
    fn the_deadline_is_counted_in_utc_days_whatever_the_window_is_cut_in() {
        let book = compiled(
            0,
            Some(100),
            Some("2026-09-30T00:00:00Z"),
            vec![section("book", Status::Included, 0)],
        );

        let ahead = Question {
            offset_minutes: 780,
            ..question("2026-08-06T18:00:00Z")
        };
        let behind = Question {
            offset_minutes: -420,
            ..question("2026-08-06T18:00:00Z")
        };

        assert_eq!(build(&ahead, &book, &[]).days_remaining, Some(56));
        assert_eq!(build(&behind, &book, &[]).days_remaining, Some(56));
    }

    /// The window is cut in the caller's offset, which is where an offset does
    /// belong: which local day an observation fell on is a wall-clock question.
    #[test]
    fn the_offset_decides_which_local_day_an_observation_was_written_on() {
        let book = compiled(0, None, None, vec![section("book", Status::Included, 0)]);
        let samples = [
            sample("2026-08-06T04:00:00Z", "book", 100, 0),
            sample("2026-08-06T18:00:00Z", "book", 100, 0),
        ];

        let utc = build(&question("2026-08-06T20:00:00Z"), &book, &samples);
        assert_eq!(utc.window.active_days, 1, "both on the 6th");

        let west = Question {
            offset_minutes: -420,
            ..question("2026-08-06T20:00:00Z")
        };
        assert_eq!(
            build(&west, &book, &samples).window.active_days,
            2,
            "the small hours of the 6th are still the evening of the 5th"
        );
    }

    /// A book with neither a target nor a deadline still answers the half of the
    /// question the log can answer.
    #[test]
    fn a_manuscript_aiming_at_nothing_still_reports_what_was_written() {
        let book = compiled(
            400,
            None,
            None,
            vec![section("book", Status::Included, 400)],
        );
        let samples = [sample("2026-08-05T09:00:00Z", "book", 420, 20)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.target, None);
        assert_eq!(pace.remaining, None);
        assert_eq!(pace.days_remaining, None);
        assert_eq!(pace.required_per_day, None);
        assert_eq!(pace.window.net, 400);
    }

    /// A target and no deadline is the commonest shape, and the projection is
    /// the useful half of it.
    #[test]
    fn a_target_with_no_deadline_still_projects() {
        let book = compiled(
            100,
            Some(1_500),
            None,
            vec![section("book", Status::Included, 100)],
        );
        let samples = [sample("2026-08-05T09:00:00Z", "book", 140, 0)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.days_remaining, None);
        // 1,400 to go at ten a day, today being the first of the hundred and
        // forty.
        assert_eq!(pace.projected_days, Some(140));
        assert_eq!(pace.projected_finish, Some(at("2026-12-23T00:00:00Z")));
    }

    /// Past a century the date is not a figure, it is what dividing by a rate
    /// near zero produces. The day goes and the number stays, so the reason is
    /// visible.
    #[test]
    fn an_absurd_projection_keeps_its_number_and_loses_its_date() {
        let book = compiled(
            0,
            Some(1_000_000),
            None,
            vec![section("book", Status::Included, 0)],
        );
        // Fourteen words in a fortnight, which is a rate of one a day.
        let samples = [sample("2026-08-05T09:00:00Z", "book", 14, 0)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.projected_days, Some(1_000_000));
        assert_eq!(pace.projected_finish, None);
    }

    /// Provenance rather than the sum. The totals stay honest when the list is
    /// cut off, which is what makes the cap safe to have.
    #[test]
    fn the_page_list_is_capped_and_the_totals_are_not() {
        let mut sections = vec![section("book", Status::Included, 0)];
        let mut samples = Vec::new();

        for index in 0..(TOP_N + 5) {
            let slug = format!("book/chapter-{index:02}");
            sections.push(section(&slug, Status::Included, 0));
            samples.push(sample("2026-08-05T09:00:00Z", &slug, 10, 0));
        }

        let book = compiled(0, None, None, sections);
        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.pages.len(), TOP_N);
        assert_eq!(pace.window.observations, TOP_N + 5);
        assert_eq!(pace.window.added, (TOP_N as u64 + 5) * 10);
    }

    /// A page nobody wrote is in the manifest and in no log, and a page written
    /// somewhere else in the wiki is in the log and in neither figure.
    #[test]
    fn writing_outside_the_manuscript_is_in_neither_total() {
        let book = compiled(
            0,
            None,
            None,
            vec![
                section("book", Status::Included, 0),
                section("book/two/the-crossing", Status::Wanted, 0),
            ],
        );
        let samples = [sample("2026-08-05T09:00:00Z", "notes/rust", 900, 0)];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.net, 0);
        assert_eq!(pace.uncounted, PaceUncounted::default());
    }

    /// A chapter written and then deleted is a `wanted` gap whose words are in
    /// the log. They belong in `uncounted`, which is defined by what the
    /// document carries rather than by a status.
    #[test]
    fn words_at_a_gap_are_words_the_document_does_not_carry() {
        let book = compiled(
            0,
            None,
            None,
            vec![
                section("book", Status::Included, 0),
                section("book/two/the-crossing", Status::Wanted, 0),
            ],
        );
        let samples = [sample(
            "2026-08-05T09:00:00Z",
            "book/two/the-crossing",
            300,
            0,
        )];

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &samples);

        assert_eq!(pace.window.net, 0);
        assert_eq!(pace.uncounted.net, 300);
    }

    #[test]
    fn a_window_nobody_wrote_in_is_a_rate_of_zero_rather_than_nothing() {
        let book = compiled(
            600,
            Some(2_000),
            None,
            vec![section("book", Status::Included, 600)],
        );

        let pace = build(&question("2026-08-06T18:00:00Z"), &book, &[]);

        assert_eq!(pace.window.net, 0);
        assert_eq!(pace.window.per_day, 0.0);
        assert_eq!(pace.projected_days, None);
    }

    /// Clamped rather than refused, which is what the chart does with a window
    /// nobody thought about.
    #[test]
    fn an_impossible_window_length_is_clamped_at_both_ends() {
        let book = compiled(0, None, None, vec![section("book", Status::Included, 0)]);

        let none = Question {
            window_days: 0,
            ..question("2026-08-06T18:00:00Z")
        };
        assert_eq!(build(&none, &book, &[]).window.days, 1);

        let forever = Question {
            window_days: u32::MAX,
            ..question("2026-08-06T18:00:00Z")
        };
        assert_eq!(build(&forever, &book, &[]).window.days, MAX_WINDOW_DAYS);
    }

    /// Today is a whole day rather than a half-finished one, which is the same
    /// rule the chart's default window is on.
    #[test]
    fn the_window_ends_at_the_end_of_the_local_day() {
        let (from, to) = window(at("2026-08-06T18:00:00Z"), 0, 14);

        assert_eq!(to, at("2026-08-07T00:00:00Z"));
        assert_eq!(from, at("2026-07-24T00:00:00Z"));
    }
}
