//! What state an idea is in, and how much momentum it has: `idea-momentum/v1`.
//!
//! A pure function of the folded authored evidence and an explicit instant.
//! Neither the label nor the score is stored anywhere, which is not an
//! optimisation: an idea becomes dormant because time passes and nothing
//! happens, and a value written into a table would be an answer to a question
//! that had not been asked yet. Computing it on read is also what removes the
//! need for a background scheduler.
//!
//! ## Integrity comes first
//!
//! An idea whose captures have been deleted from underneath it has no authored
//! evidence left. It gets [`Integrity::EvidenceMissing`], the ids of what it
//! lost, and no state and no momentum at all. Manufacturing a lifecycle answer
//! out of absent evidence is the one thing this feature must never do, so there
//! is no path here that does it. A *retired* idea is exempt, because retiring is
//! itself a decision somebody took and the state is that decision rather than an
//! inference from captures.
//!
//! ## Every number in the receipt is checkable from the receipt
//!
//! [`Receipt`] carries the components, the window boundaries they were measured
//! against, and every capture and event that was counted with a flag saying
//! which window it fell in. A reader can recompute `momentum` from the response
//! without seeing this file, which is the gate the whole phase is measured
//! against.

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ideas::{CaptureId, EventId, EventKind};

/// The name this ruleset answers to, returned with every receipt.
pub const RULESET: &str = "idea-momentum/v1";

/// The short window. Three captures inside it is a burst.
pub const RECENT_DAYS: i64 = 14;

/// The long window. One capture inside it is a sign of life, and it is also how
/// far back an affirmation counts.
pub const WINDOW_DAYS: i64 = 30;

/// How long nothing has to happen before an idea is dormant.
pub const DORMANT_DAYS: i64 = 60;

/// How many captures inside [`RECENT_DAYS`] make a burst.
pub const BURST: usize = 3;

/// The most `base` can be worth, however many captures an idea holds.
pub const MAX_BASE: usize = 4;

/// The momentum an idea needs to count as active.
pub const ACTIVE_MOMENTUM: u32 = 4;

/// The ceiling on momentum.
///
/// Version 1 cannot reach it: `base` tops out at 4, `recency` at 2 and
/// `affirmation` at 1, so seven is the most any idea can score. The cap is here
/// because the ruleset defines it, and because a component added later must not
/// silently change what the top of the scale means.
pub const MAX_MOMENTUM: u32 = 10;

/// Where an idea is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// Set aside by an explicit decision. Nothing else is consulted.
    Retired,
    /// Nothing has happened for 60 days. This is what rediscovery looks for.
    Dormant,
    /// Exactly one capture connected: a thought, not yet a pattern.
    New,
    /// Momentum of 4 or more.
    Active,
    /// Coming back, but not right now.
    Recurring,
}

impl Lifecycle {
    /// Every spelling, for an error that has to say what would have worked.
    pub const NAMES: &'static [&'static str] =
        &["retired", "dormant", "new", "active", "recurring"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retired => "retired",
            Self::Dormant => "dormant",
            Self::New => "new",
            Self::Active => "active",
            Self::Recurring => "recurring",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "retired" => Some(Self::Retired),
            "dormant" => Some(Self::Dormant),
            "new" => Some(Self::New),
            "active" => Some(Self::Active),
            "recurring" => Some(Self::Recurring),
            _ => None,
        }
    }
}

impl std::fmt::Display for Lifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether an idea still has the evidence it rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    /// Every connected capture is still readable.
    Sound,
    /// Captures this idea holds have been deleted, and nothing live is left. The
    /// dashboard groups these under Needs repair.
    EvidenceMissing,
}

impl Integrity {
    pub const NAMES: &'static [&'static str] = &["sound", "evidence_missing"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sound => "sound",
            Self::EvidenceMissing => "evidence_missing",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "sound" => Some(Self::Sound),
            "evidence_missing" => Some(Self::EvidenceMissing),
            _ => None,
        }
    }
}

impl std::fmt::Display for Integrity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One connected capture, and when the thought was had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureMoment {
    pub id: CaptureId,
    pub created: DateTime<Utc>,
}

/// One `interest_affirmed` or `idea_reopened`: somebody saying, at a moment,
/// that they still care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Affirmation {
    pub id: EventId,
    pub kind: EventKind,
    pub created: DateTime<Utc>,
}

/// Everything the rules read about one idea.
///
/// Assembled from the folded index and passed in whole, so the rules below are a
/// pure function of stated inputs and a test can write down four timestamps and
/// the exact momentum they produce.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    /// Whether the latest retire-or-reopen decision says retired.
    pub retired: bool,
    /// The latest of a connected capture's creation and an affirm, connect,
    /// reopen or promote event. Folded by the index, not recomputed here.
    pub last_signal: Option<DateTime<Utc>>,
    /// Connected captures whose files are still there, oldest first.
    pub captures: Vec<CaptureMoment>,
    /// Connected captures whose files are gone.
    pub missing: Vec<CaptureId>,
    /// Every affirmation and reopening, in decision order.
    pub affirmations: Vec<Affirmation>,
}

/// The arithmetic behind a momentum score.
// Serialized as it stands. This struct and the three below are the receipt's
// wire shape as well as its internal one, so there is no second spelling to keep
// in step and no chance of the API reporting a number the rules did not compute.
// Their doc comments become OpenAPI descriptions, which is why none of them
// carries a rustdoc link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct Components {
    /// Connected captures whose files are still there. Archived ones count:
    /// archive means processed, not "this thought never happened". Deleted ones
    /// do not, because there is no authored evidence left.
    pub total: usize,
    /// How many of them were captured in the 14 days up to the instant asked
    /// about.
    pub recent_14: usize,
    /// How many in the 30 days up to it.
    pub recent_30: usize,
    /// `min(total, 4)`.
    pub base: u32,
    /// `2` for a burst, `1` for any sign of life in the long window, else `0`.
    pub recency: u32,
    /// `1` for an affirmation or reopening inside the long window.
    pub affirmation: u32,
    /// `min(base + recency + affirmation, 10)`.
    pub momentum: u32,
}

/// Where each window starts, so every flag in a receipt can be checked against
/// the timestamps beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct Boundaries {
    /// `at - 14 days`. A capture at or after this counts toward `recent_14`.
    pub recent_14: DateTime<Utc>,
    /// `at - 30 days`.
    pub recent_30: DateTime<Utc>,
    /// `at - 60 days`. A `last_signal` at or before this is dormant.
    pub dormant: DateTime<Utc>,
}

impl Boundaries {
    pub fn at(at: DateTime<Utc>) -> Self {
        Self {
            recent_14: days_before(at, RECENT_DAYS),
            recent_30: days_before(at, WINDOW_DAYS),
            dormant: days_before(at, DORMANT_DAYS),
        }
    }
}

/// `at` minus some days, saturating rather than panicking.
///
/// `at` comes off a query string, and chrono's subtraction panics on overflow.
/// A timestamp near the start of the representable range would otherwise take
/// the request handler down with it.
fn days_before(at: DateTime<Utc>, days: i64) -> DateTime<Utc> {
    at.checked_sub_signed(TimeDelta::days(days))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

/// One capture, and which windows it fell in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CountedCapture {
    pub id: CaptureId,
    pub created: DateTime<Utc>,
    pub within_14_days: bool,
    pub within_30_days: bool,
}

/// One affirmation, and whether it was recent enough to count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CountedAffirmation {
    pub id: EventId,
    pub kind: EventKind,
    pub created: DateTime<Utc>,
    pub within_30_days: bool,
}

/// The rules, the numbers and the evidence behind one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    /// The ruleset that produced this, and the only thing that changes it.
    pub ruleset: &'static str,
    /// The instant this is an answer *about*, which is the caller's `at` or the
    /// server's now.
    pub computed_at: DateTime<Utc>,
    pub boundaries: Boundaries,
    pub integrity: Integrity,
    /// Absent when the evidence is missing. There is nothing to derive it from.
    pub state: Option<Lifecycle>,
    /// Absent for the same reason.
    pub momentum: Option<u32>,
    pub components: Option<Components>,
    pub last_signal: Option<DateTime<Utc>>,
    /// Every connected capture that is still readable, oldest first.
    pub captures: Vec<CountedCapture>,
    /// Every affirmation and reopening, in decision order.
    pub affirmations: Vec<CountedAffirmation>,
    /// Connected captures whose files are gone.
    pub missing: Vec<CaptureId>,
    /// One sentence per line, each from a fixed template. Prose for a reader;
    /// the arithmetic is in `components`.
    pub explanation: Vec<String>,
}

impl Receipt {
    pub fn needs_repair(&self) -> bool {
        self.integrity == Integrity::EvidenceMissing
    }
}

/// Work out where an idea stands at a given instant.
pub fn assess(evidence: &Evidence, at: DateTime<Utc>) -> Receipt {
    let boundaries = Boundaries::at(at);

    let captures: Vec<CountedCapture> = evidence
        .captures
        .iter()
        .map(|capture| CountedCapture {
            id: capture.id.clone(),
            created: capture.created,
            within_14_days: within(capture.created, boundaries.recent_14, at),
            within_30_days: within(capture.created, boundaries.recent_30, at),
        })
        .collect();

    let affirmations: Vec<CountedAffirmation> = evidence
        .affirmations
        .iter()
        .map(|affirmation| CountedAffirmation {
            id: affirmation.id.clone(),
            kind: affirmation.kind,
            created: affirmation.created,
            within_30_days: within(affirmation.created, boundaries.recent_30, at),
        })
        .collect();

    // Integrity first. A non-retired idea holding nothing readable is not a
    // dormant idea or a new one; it is an idea with no evidence, and the answer
    // is to say so rather than to derive something from the absence.
    if !evidence.retired && captures.is_empty() {
        return Receipt {
            ruleset: RULESET,
            computed_at: at,
            boundaries,
            integrity: Integrity::EvidenceMissing,
            state: None,
            momentum: None,
            components: None,
            last_signal: evidence.last_signal,
            captures,
            affirmations,
            missing: evidence.missing.clone(),
            explanation: missing_evidence(evidence.missing.len()),
        };
    }

    let total = captures.len();
    let recent_14 = captures.iter().filter(|c| c.within_14_days).count();
    let recent_30 = captures.iter().filter(|c| c.within_30_days).count();

    let base = total.min(MAX_BASE) as u32;
    let recency = if recent_14 >= BURST {
        2
    } else if recent_30 >= 1 {
        1
    } else {
        0
    };
    let affirmation = u32::from(affirmations.iter().any(|a| a.within_30_days));
    let momentum = (base + recency + affirmation).min(MAX_MOMENTUM);

    let components = Components {
        total,
        recent_14,
        recent_30,
        base,
        recency,
        affirmation,
        momentum,
    };

    // In this order, and the order is the rule. Retirement is a decision and
    // beats everything derived; dormancy beats the capture count because an idea
    // nobody has touched for two months is dormant whether it holds one capture
    // or nine.
    let state = if evidence.retired {
        Lifecycle::Retired
    } else if evidence
        .last_signal
        .is_some_and(|signal| signal <= boundaries.dormant)
    {
        Lifecycle::Dormant
    } else if total == 1 {
        Lifecycle::New
    } else if momentum >= ACTIVE_MOMENTUM {
        Lifecycle::Active
    } else {
        Lifecycle::Recurring
    };

    Receipt {
        ruleset: RULESET,
        computed_at: at,
        boundaries,
        integrity: Integrity::Sound,
        state: Some(state),
        momentum: Some(momentum),
        components: Some(components),
        last_signal: evidence.last_signal,
        explanation: explain(state, &components, evidence.last_signal, at),
        captures,
        affirmations,
        missing: evidence.missing.clone(),
    }
}

/// Whether an instant falls in `[start, at]`, both ends included.
fn within(instant: DateTime<Utc>, start: DateTime<Utc>, at: DateTime<Utc>) -> bool {
    instant >= start && instant <= at
}

fn missing_evidence(missing: usize) -> Vec<String> {
    vec![
        format!(
            "This idea holds {missing} {} whose files are gone and nothing that is still \
             readable, so there is no authored evidence to derive a state from.",
            plural(missing, "capture", "captures"),
        ),
        "Restore one of them, connect another capture, or retire the idea.".to_owned(),
    ]
}

/// Two sentences, both from fixed templates: what the state is, and how the
/// momentum was arrived at.
fn explain(
    state: Lifecycle,
    components: &Components,
    last_signal: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
) -> Vec<String> {
    let Components {
        total,
        recent_14,
        recent_30,
        base,
        recency,
        affirmation,
        momentum,
    } = *components;

    let heading = match state {
        Lifecycle::Retired => {
            "Retired: the latest decision taken about this idea was to retire it.".to_owned()
        }
        Lifecycle::Dormant => {
            let days = last_signal.map_or(0, |signal| {
                at.signed_duration_since(signal).num_days().max(0)
            });
            format!(
                "Dormant: the last signal was {days} {} ago, and an idea goes dormant after \
                 {DORMANT_DAYS}.",
                plural(days as usize, "day", "days"),
            )
        }
        Lifecycle::New => "New: exactly one capture is connected.".to_owned(),
        Lifecycle::Active => {
            format!("Active: momentum is {momentum}, and {ACTIVE_MOMENTUM} or more is active.")
        }
        Lifecycle::Recurring => format!(
            "Recurring: momentum is {momentum}, below the {ACTIVE_MOMENTUM} an active idea needs."
        ),
    };

    let held = if total > MAX_BASE {
        format!("{base} for {total} connected captures, capped at {MAX_BASE}")
    } else {
        format!(
            "{base} for {total} connected {}",
            plural(total, "capture", "captures")
        )
    };
    let recent = match recency {
        2 => format!("{recency} because {recent_14} arrived in the last {RECENT_DAYS} days"),
        1 => format!(
            "{recency} because {recent_30} {} in the last {WINDOW_DAYS} days",
            plural(recent_30, "arrived", "arrived"),
        ),
        _ => format!("{recency} because none arrived in the last {WINDOW_DAYS} days"),
    };
    let affirmed = if affirmation == 1 {
        format!("{affirmation} for an affirmation in the last {WINDOW_DAYS} days")
    } else {
        format!("{affirmation} with no affirmation in the last {WINDOW_DAYS} days")
    };

    vec![
        heading,
        format!("Momentum {momentum} = {held}; {recent}; {affirmed}."),
    ]
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    /// The instant every test asks about.
    fn now() -> DateTime<Utc> {
        at("2026-08-20T12:00:00Z")
    }

    fn capture_id(raw: &str) -> CaptureId {
        CaptureId::parse(raw).expect("valid capture id")
    }

    /// A capture made `days` before [`now`], numbered so ids stay distinct.
    fn made(n: u32, days: i64) -> CaptureMoment {
        CaptureMoment {
            id: capture_id(&format!("20260820T1200{n:02}-000000000")),
            created: now() - TimeDelta::days(days),
        }
    }

    fn affirmed(days: i64) -> Affirmation {
        let created = now() - TimeDelta::days(days);
        Affirmation {
            id: EventId::mint(created, 0),
            kind: EventKind::InterestAffirmed,
            created,
        }
    }

    /// Evidence whose `last_signal` is the newest capture, which is what the
    /// index folds when nothing else has happened.
    fn held(captures: Vec<CaptureMoment>) -> Evidence {
        Evidence {
            last_signal: captures.iter().map(|c| c.created).max(),
            captures,
            ..Evidence::default()
        }
    }

    fn assessed(evidence: &Evidence) -> Receipt {
        assess(evidence, now())
    }

    #[test]
    fn one_capture_is_new() {
        let receipt = assessed(&held(vec![made(1, 0)]));

        assert_eq!(receipt.state, Some(Lifecycle::New));
        assert_eq!(receipt.momentum, Some(2), "1 base + 1 recency");
        assert_eq!(receipt.integrity, Integrity::Sound);
        assert_eq!(receipt.ruleset, RULESET);
        assert_eq!(receipt.computed_at, now());
    }

    /// The arithmetic, written out: four captures inside 14 days is a burst.
    #[test]
    fn a_burst_of_recent_captures_is_active() {
        let receipt = assessed(&held(vec![made(1, 1), made(2, 3), made(3, 5), made(4, 40)]));

        let components = receipt.components.expect("components");
        assert_eq!(components.total, 4);
        assert_eq!(components.recent_14, 3);
        assert_eq!(components.recent_30, 3);
        assert_eq!(components.base, 4);
        assert_eq!(components.recency, 2);
        assert_eq!(components.affirmation, 0);
        assert_eq!(components.momentum, 6);
        assert_eq!(receipt.state, Some(Lifecycle::Active));
    }

    #[test]
    fn base_is_capped_at_four_captures() {
        let captures: Vec<_> = (1..=9).map(|n| made(n, i64::from(n) * 100)).collect();
        let mut evidence = held(captures);
        // Otherwise this would be dormant, which is decided before the count.
        evidence.last_signal = Some(now());

        let components = assessed(&evidence).components.expect("components");

        assert_eq!(components.total, 9);
        assert_eq!(components.base, 4);
        assert_eq!(components.recent_30, 0);
        assert_eq!(components.recency, 0);
        assert_eq!(components.momentum, 4);
    }

    /// The three boundaries, tested exactly on them. Both ends are inclusive.
    #[test]
    fn a_capture_exactly_fourteen_days_old_still_counts_as_recent() {
        let components = assessed(&held(vec![made(1, 14), made(2, 14), made(3, 14)]))
            .components
            .expect("components");

        assert_eq!(components.recent_14, 3);
        assert_eq!(components.recency, 2);
    }

    #[test]
    fn a_capture_a_day_past_fourteen_is_not_a_burst() {
        let components = assessed(&held(vec![made(1, 14), made(2, 14), made(3, 15)]))
            .components
            .expect("components");

        assert_eq!(components.recent_14, 2);
        assert_eq!(components.recent_30, 3);
        assert_eq!(components.recency, 1);
    }

    #[test]
    fn a_capture_exactly_thirty_days_old_is_still_a_sign_of_life() {
        let components = assessed(&held(vec![made(1, 30), made(2, 100)]))
            .components
            .expect("components");

        assert_eq!(components.recent_30, 1);
        assert_eq!(components.recency, 1);
    }

    #[test]
    fn a_capture_a_day_past_thirty_is_not() {
        let components = assessed(&held(vec![made(1, 31), made(2, 100)]))
            .components
            .expect("components");

        assert_eq!(components.recent_30, 0);
        assert_eq!(components.recency, 0);
    }

    #[test]
    fn nothing_for_exactly_sixty_days_is_dormant() {
        let mut evidence = held(vec![made(1, 60), made(2, 200)]);
        evidence.last_signal = Some(now() - TimeDelta::days(60));

        assert_eq!(assessed(&evidence).state, Some(Lifecycle::Dormant));

        evidence.last_signal = Some(now() - TimeDelta::days(59));
        assert_ne!(assessed(&evidence).state, Some(Lifecycle::Dormant));
    }

    /// A capture dated in the future counts toward the total and toward neither
    /// window, which keeps a mistyped timestamp from inventing recency.
    #[test]
    fn a_capture_after_the_instant_asked_about_counts_for_nothing_recent() {
        let mut evidence = held(vec![made(1, 100), made(2, -5)]);
        evidence.last_signal = Some(now());

        let components = assessed(&evidence).components.expect("components");

        assert_eq!(components.total, 2);
        assert_eq!(components.recent_14, 0);
        assert_eq!(components.recent_30, 0);
    }

    #[test]
    fn an_affirmation_inside_thirty_days_is_worth_a_point() {
        let mut evidence = held(vec![made(1, 100), made(2, 200)]);
        evidence.last_signal = Some(now() - TimeDelta::days(1));
        assert_eq!(assessed(&evidence).momentum, Some(2), "2 base, no recency");

        evidence.affirmations = vec![affirmed(30)];
        let receipt = assessed(&evidence);
        assert_eq!(receipt.momentum, Some(3));
        assert!(receipt.affirmations[0].within_30_days);

        evidence.affirmations = vec![affirmed(31)];
        let receipt = assessed(&evidence);
        assert_eq!(receipt.momentum, Some(2));
        assert!(!receipt.affirmations[0].within_30_days);
        assert_eq!(
            receipt.affirmations.len(),
            1,
            "an affirmation that did not count is still evidence and is still named"
        );
    }

    /// Reopening is a way of saying you still care, so it counts as one.
    #[test]
    fn a_reopening_counts_as_an_affirmation() {
        let mut evidence = held(vec![made(1, 100), made(2, 200)]);
        evidence.last_signal = Some(now() - TimeDelta::days(1));
        evidence.affirmations = vec![Affirmation {
            kind: EventKind::IdeaReopened,
            ..affirmed(2)
        }];

        assert_eq!(assessed(&evidence).components.unwrap().affirmation, 1);
    }

    #[test]
    fn retirement_beats_everything_derived() {
        let mut evidence = held(vec![made(1, 1), made(2, 2), made(3, 3)]);
        evidence.retired = true;

        let receipt = assessed(&evidence);

        assert_eq!(receipt.state, Some(Lifecycle::Retired));
        // The arithmetic is still reported, so reopening it is a known quantity.
        assert_eq!(receipt.momentum, Some(5));
    }

    /// Dormancy is decided before the capture count, so an idea with one capture
    /// and nothing since is dormant rather than new.
    #[test]
    fn dormancy_beats_the_capture_count() {
        let receipt = assessed(&held(vec![made(1, 90)]));

        assert_eq!(receipt.state, Some(Lifecycle::Dormant));
        assert_eq!(receipt.momentum, Some(1));
    }

    #[test]
    fn two_quiet_captures_are_recurring() {
        let mut evidence = held(vec![made(1, 40), made(2, 45)]);
        evidence.last_signal = Some(now() - TimeDelta::days(1));

        let receipt = assessed(&evidence);

        assert_eq!(receipt.state, Some(Lifecycle::Recurring));
        assert_eq!(receipt.momentum, Some(2));
    }

    /// The one thing this must never do: derive an answer from absent evidence.
    #[test]
    fn a_thread_whose_captures_are_gone_gets_no_state_and_no_score() {
        let evidence = Evidence {
            missing: vec![capture_id("20260820T120001-000000000")],
            last_signal: None,
            ..Evidence::default()
        };

        let receipt = assess(&evidence, now());

        assert_eq!(receipt.integrity, Integrity::EvidenceMissing);
        assert!(receipt.needs_repair());
        assert_eq!(receipt.state, None);
        assert_eq!(receipt.momentum, None);
        assert_eq!(receipt.components, None);
        assert_eq!(receipt.missing.len(), 1);
        assert!(receipt.explanation[0].contains("no authored evidence"));
    }

    /// Retiring is the way out of Needs repair, and it is a decision rather than
    /// an inference, so it does not need evidence to stand on.
    #[test]
    fn a_retired_thread_with_nothing_left_is_retired_rather_than_broken() {
        let evidence = Evidence {
            retired: true,
            missing: vec![capture_id("20260820T120001-000000000")],
            ..Evidence::default()
        };

        let receipt = assess(&evidence, now());

        assert_eq!(receipt.integrity, Integrity::Sound);
        assert_eq!(receipt.state, Some(Lifecycle::Retired));
        assert_eq!(receipt.momentum, Some(0));
    }

    /// The whole point of taking `at`: the same files answer differently at two
    /// moments, and neither answer is stored.
    #[test]
    fn the_same_evidence_answers_differently_at_two_instants() {
        let evidence = held(vec![made(1, 0), made(2, 1), made(3, 2)]);

        assert_eq!(assess(&evidence, now()).state, Some(Lifecycle::Active));
        assert_eq!(
            assess(&evidence, now() + TimeDelta::days(90)).state,
            Some(Lifecycle::Dormant)
        );
    }

    /// Version 1 cannot reach the ceiling, which is worth knowing before
    /// somebody reads `min(.., 10)` and designs a bar chart around it.
    #[test]
    fn the_highest_momentum_version_one_can_produce_is_seven() {
        let mut evidence = held(vec![
            made(1, 0),
            made(2, 1),
            made(3, 2),
            made(4, 3),
            made(5, 4),
        ]);
        evidence.affirmations = vec![affirmed(0)];

        let components = assessed(&evidence).components.expect("components");

        assert_eq!(components.base, 4);
        assert_eq!(components.recency, 2);
        assert_eq!(components.affirmation, 1);
        assert_eq!(components.momentum, 7);
        assert!(components.momentum < MAX_MOMENTUM);
    }

    /// Every window boundary is in the receipt, and every flag beside it agrees
    /// with the timestamps, so the components can be recomputed from the answer.
    #[test]
    fn a_receipt_carries_the_boundaries_its_flags_were_measured_against() {
        let receipt = assessed(&held(vec![made(1, 2), made(2, 20), made(3, 200)]));

        assert_eq!(receipt.boundaries.recent_14, now() - TimeDelta::days(14));
        assert_eq!(receipt.boundaries.recent_30, now() - TimeDelta::days(30));
        assert_eq!(receipt.boundaries.dormant, now() - TimeDelta::days(60));

        for capture in &receipt.captures {
            assert_eq!(
                capture.within_14_days,
                capture.created >= receipt.boundaries.recent_14 && capture.created <= now()
            );
            assert_eq!(
                capture.within_30_days,
                capture.created >= receipt.boundaries.recent_30 && capture.created <= now()
            );
        }

        let components = receipt.components.expect("components");
        assert_eq!(
            components.recent_14,
            receipt.captures.iter().filter(|c| c.within_14_days).count()
        );
        assert_eq!(
            components.recent_30,
            receipt.captures.iter().filter(|c| c.within_30_days).count()
        );
    }

    #[test]
    fn the_explanation_says_the_state_and_the_arithmetic() {
        let receipt = assessed(&held(vec![made(1, 1), made(2, 2), made(3, 3)]));

        assert_eq!(receipt.explanation.len(), 2);
        assert!(
            receipt.explanation[0].starts_with("Active:"),
            "{receipt:#?}"
        );
        assert!(
            receipt.explanation[1].starts_with("Momentum 5 = 3 for 3 connected captures; 2 because 3 arrived in the last 14 days; 0 with no affirmation"),
            "{}",
            receipt.explanation[1]
        );
    }

    /// A `at` near the start of representable time must not take the request
    /// down with it, which subtracting sixty days from it otherwise would.
    #[test]
    fn an_absurd_instant_saturates_rather_than_panicking() {
        let boundaries = Boundaries::at(DateTime::<Utc>::MIN_UTC);

        assert_eq!(boundaries.recent_14, DateTime::<Utc>::MIN_UTC);
        assert_eq!(boundaries.dormant, DateTime::<Utc>::MIN_UTC);
        assert_eq!(
            assess(&Evidence::default(), DateTime::<Utc>::MIN_UTC).integrity,
            Integrity::EvidenceMissing
        );
    }

    #[test]
    fn every_state_and_integrity_name_round_trips() {
        for name in Lifecycle::NAMES {
            assert_eq!(Lifecycle::parse(name).map(Lifecycle::as_str), Some(*name));
        }
        for name in Integrity::NAMES {
            assert_eq!(Integrity::parse(name).map(Integrity::as_str), Some(*name));
        }
        assert_eq!(Lifecycle::parse("dormantish"), None);
        assert_eq!(Integrity::parse("broken"), None);
    }
}
