//! Idea Inbox: captures, idea threads, and the decisions taken about them.
//!
//! Three kinds of file live under `<wiki>/.rhizolog/ideas/`, and between them
//! they are the whole of what Rhizolog knows about an unfinished thought.
//!
//! A **capture** is one timestamped piece of text, exactly as it was written:
//!
//! ```markdown
//! ---
//! created: 2026-08-20T14:15:30Z
//! owner: tim
//! ---
//!
//! Maybe dungeon quests should require finding particular seeds.
//! ```
//!
//! An **idea** is a thread somebody named, and the captures it grew from:
//!
//! ```markdown
//! ---
//! name: Dungeon seeds
//! created: 2026-08-20T14:20:00Z
//! owner: tim
//! captures:
//! - 20260820T141530-123456789
//! ---
//!
//! Optional working notes about the idea.
//! ```
//!
//! A **decision event** is one thing somebody did, and it is only its
//! frontmatter:
//!
//! ```markdown
//! ---
//! kind: capture_connected
//! created: 2026-08-20T14:20:30Z
//! actor: tim
//! idea: 20260820T142000-234567890
//! capture: 20260820T141530-123456789
//! ---
//! ```
//!
//! ## An idea's file says how it started, never how it stands
//!
//! The `captures:` list is the seed grouping and nothing else. Which captures
//! an idea currently holds, which candidates were rejected, whether it is
//! retired, and which page it produced are all *folded from the events*, so the
//! only way to change what an idea holds is to append another one. Reversing a
//! decision writes its inverse; nothing rewrites history.
//!
//! That is what makes creating an idea a single authored-file write rather than
//! a thread file plus a handful of event files that could half-succeed, and it
//! is why this module parses and serialises files without ever computing a
//! current state. The fold belongs to the derived index.
//!
//! ## None of this is disposable
//!
//! `.rhizolog/ideas/` shares a directory with `index.db`, which is derived and
//! may be deleted at any moment, and it is nothing like it: these files are the
//! only copy of the captures. They should be backed up, and unlike
//! `.rhizolog/users/` they hold no secrets, so they can be committed with the
//! wiki. Rhizolog does not gitignore them.
//!
//! See `knowledge-base/idea-inbox.md` for the reasoning behind all of it.

pub mod service;
pub mod store;

use std::fmt;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use thiserror::Error;
use utoipa::ToSchema;

use crate::frontmatter::{self, FrontmatterError};
use crate::slug::Slug;
use crate::users::Username;

pub use service::{IdeaService, IdeaServiceError};
pub use store::{CaptureDraft, EventDraft, IdeaDraft, IdeaStore, IdeaStoreError, IdeaWalkEntry};

/// Where Idea Inbox lives inside the wiki, under [`crate::store::INTERNAL_DIR`].
///
/// Under the dot-directory for the same reason the time log is: the page
/// walker, the slug validator and the file watcher already agree that nothing
/// there is a page, so a capture cannot be mistaken for one and a page cannot be
/// written over one.
pub const IDEAS_DIR: &str = "ideas";

/// Captures, filed by month: `captures/2026-08/<id>.md`.
pub const CAPTURES_DIR: &str = "captures";

/// Idea threads, flat: `threads/<id>.md`.
///
/// Flat because there are never many. Captures arrive at the speed of thought
/// and events at the speed of clicking; threads are named by hand, one at a
/// time, and bucketing a directory that holds tens of files buys nothing and
/// costs a directory listing that no longer reads as a list of ideas.
pub const THREADS_DIR: &str = "threads";

/// Decision events, filed by month: `events/2026-08/<id>.md`.
pub const EVENTS_DIR: &str = "events";

/// How many captures an idea may be started from.
///
/// The same ceiling the list endpoints take, because it is the same question
/// asked from the other side: a seed list is a page of captures somebody
/// selected, and one longer than a page could not have been selected from one.
/// It also keeps a thread file to something a person can read.
pub const MAX_SEEDS: usize = 200;

/// The number of characters in an id: `20260820T141530-123456789`.
const ID_LEN: usize = 25;

/// The number of characters in the timestamp half: `20260820T141530`.
const STAMP_LEN: usize = 15;

/// How the timestamp half of an id is spelled.
const ID_STAMP_FORMAT: &str = "%Y%m%dT%H%M%S";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    #[error("{kind} ids are {ID_LEN} characters like `20260820T141530-123456789`, not {length}")]
    Length { kind: &'static str, length: usize },

    #[error("{kind} ids may only contain digits, one `T` and one `-`")]
    Shape { kind: &'static str },

    #[error("`{stamp}` is not a real date and time")]
    Timestamp { stamp: String },
}

/// The shared validation behind every Idea Inbox id.
///
/// Twenty-five characters drawn from digits, one `T` and one `-` cannot express
/// a traversal, a drive prefix, a device name or a dot-segment, which is why an
/// id needs no list of hazards to exclude the way a [`Slug`] does. It is
/// generated, never typed, so it can be validated by exact shape.
fn validate_id(raw: &str, kind: &'static str) -> Result<(), IdError> {
    // Counting characters rather than bytes so a multi-byte character reports
    // the shape error below instead of a confusing length.
    let length = raw.chars().count();
    if length != ID_LEN {
        return Err(IdError::Length { kind, length });
    }

    let shaped = raw
        .as_bytes()
        .iter()
        .enumerate()
        .all(|(at, byte)| match at {
            8 => *byte == b'T',
            STAMP_LEN => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    if !shaped {
        return Err(IdError::Shape { kind });
    }

    // The shape allows `20261340T996060`. Parsing is what rejects it, and it
    // matters: the month in the id decides which directory the file lives in.
    let stamp = &raw[..STAMP_LEN];
    if NaiveDateTime::parse_from_str(stamp, ID_STAMP_FORMAT).is_err() {
        return Err(IdError::Timestamp {
            stamp: stamp.to_owned(),
        });
    }

    Ok(())
}

fn mint_id(at: DateTime<Utc>, nudge: u32) -> String {
    let nanos = (u64::from(at.timestamp_subsec_nanos()) + u64::from(nudge)) % 1_000_000_000;
    format!("{}-{nanos:09}", at.format(ID_STAMP_FORMAT))
}

fn month_of(id: &str) -> String {
    format!("{}-{}", &id[..4], &id[4..6])
}

fn instant_of(id: &str) -> DateTime<Utc> {
    let stamp = NaiveDateTime::parse_from_str(&id[..STAMP_LEN], ID_STAMP_FORMAT)
        .expect("a parsed id carries a real date and time");
    let nanos: i64 = id[STAMP_LEN + 1..]
        .parse()
        .expect("a parsed id ends in nine digits");

    stamp.and_utc() + TimeDelta::nanoseconds(nanos)
}

/// Define one of Idea Inbox's three identifier types.
///
/// All three are the same twenty-five characters under the same rules, and they
/// are three types rather than one because a capture id and an idea id are not
/// interchangeable: each becomes a path under a different directory, and a
/// function that accepted either would be a function that could read the wrong
/// tree. Writing that out three times would be three places for the parser to
/// drift apart, which is the one thing worse than a macro here.
///
/// Path construction is deliberately *not* generated. Captures and events are
/// filed under a `YYYY-MM` directory and threads are not, so each type spells
/// its own `to_path` and `from_relative_path` below, where the difference is
/// visible rather than hidden behind another macro argument.
macro_rules! stamp_id {
    (
        $(#[$doc:meta])*
        $name:ident, $kind:literal, $example:literal, $description:literal $(,)?
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
        #[schema(value_type = String, example = $example, description = $description)]
        pub struct $name(String);

        impl $name {
            /// Validate `raw` as an id of this kind.
            pub fn parse(raw: &str) -> Result<Self, IdError> {
                validate_id(raw, $kind)?;
                Ok(Self(raw.to_owned()))
            }

            /// Mint an id for a record created at `at`.
            ///
            /// `nudge` shifts the nanosecond half. Two records can be created
            /// within the same nanosecond, and a hand-written `created` almost
            /// never has sub-second precision at all, so without it the second
            /// one would name the first one's file. The store walks `nudge`
            /// upward until it finds a filename that is free.
            pub fn mint(at: DateTime<Utc>, nudge: u32) -> Self {
                Self(mint_id(at, nudge))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The compacted timestamp half, `20260820T141530`.
            pub fn stamp(&self) -> &str {
                &self.0[..STAMP_LEN]
            }

            /// The `YYYY-MM` this id is filed under.
            ///
            /// Taken from the id rather than from the record's `created`, so a
            /// path is a pure function of an id: correcting a hand-written
            /// timestamp never moves a file.
            pub fn month(&self) -> String {
                month_of(&self.0)
            }

            /// The instant this id names, nanoseconds included.
            pub fn instant(&self) -> DateTime<Utc> {
                instant_of(&self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        /// Validated on the way in, so an id in a frontmatter block is refused
        /// by the same rules as one in a URL.
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Self::parse(&raw).map_err(D::Error::custom)
            }
        }
    };
}

stamp_id!(
    /// A capture's identifier, and its filename.
    ///
    /// `20260820T141530-123456789`: the UTC instant it was recorded for,
    /// compacted, plus nanoseconds. It sorts chronologically as plain text, so
    /// a directory listing is the inbox in order and so is any `order by id`.
    CaptureId,
    "capture",
    "20260820T141530-123456789",
    "A capture's identifier: the UTC instant it was recorded for, compacted, plus \
     nanoseconds. `20260820T141530-123456789`.\n\n\
     Ids are generated by the server, never chosen by a caller, and they sort \
     chronologically as plain text.",
);

stamp_id!(
    /// An idea thread's identifier, and its filename.
    ///
    /// The same shape as a [`CaptureId`], and a different type: an idea is
    /// filed flat under `threads/` while a capture is filed by month, so the
    /// two cannot be swapped without reading the wrong directory.
    IdeaId,
    "idea",
    "20260820T142000-234567890",
    "An idea thread's identifier: the UTC instant it was created, compacted, plus \
     nanoseconds. `20260820T142000-234567890`.\n\n\
     Ids are generated by the server, never chosen by a caller, and they sort \
     chronologically as plain text.",
);

stamp_id!(
    /// A decision event's identifier, and its filename.
    ///
    /// **Event ids define fold order.** Current membership, rejections,
    /// retirement and promotion are all derived by walking events in id order,
    /// so an event's id is not merely a name: it is when the decision happened.
    /// That is why [`Event::from_markdown`] refuses a file whose `created`
    /// disagrees with its id.
    EventId,
    "event",
    "20260820T142030-345678901",
    "A decision event's identifier: the UTC instant the decision was taken, \
     compacted, plus nanoseconds. `20260820T142030-345678901`.\n\n\
     Event ids define the order decisions are folded in, so an event's `created` \
     may not disagree with its id.",
);

impl CaptureId {
    /// `<root>/<YYYY-MM>/<id>.md`.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        month_path(root, &self.0, &self.month())
    }

    /// Recover an id from a path relative to the captures directory.
    ///
    /// `None` for anything this module would not have written, so the walker
    /// skips strays rather than indexing captures it could never serve.
    pub fn from_relative_path(path: &Path) -> Option<Self> {
        let (month, stem) = month_parts(path)?;
        let id = Self::parse(stem).ok()?;

        // A file filed under the wrong month would be found by the walker and
        // then written back somewhere else, so it is not one of ours.
        (id.month() == month).then_some(id)
    }
}

impl IdeaId {
    /// `<root>/<id>.md`.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.md", self.0))
    }

    /// Recover an id from a path relative to the threads directory.
    pub fn from_relative_path(path: &Path) -> Option<Self> {
        Self::parse(flat_stem(path)?).ok()
    }
}

impl EventId {
    /// `<root>/<YYYY-MM>/<id>.md`.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        month_path(root, &self.0, &self.month())
    }

    /// Recover an id from a path relative to the events directory.
    pub fn from_relative_path(path: &Path) -> Option<Self> {
        let (month, stem) = month_parts(path)?;
        let id = Self::parse(stem).ok()?;
        (id.month() == month).then_some(id)
    }
}

fn month_path(root: &Path, id: &str, month: &str) -> PathBuf {
    root.join(month).join(format!("{id}.md"))
}

/// Split `<YYYY-MM>/<stem>.md` into its two halves.
fn month_parts(path: &Path) -> Option<(&str, &str)> {
    let segments = normal_segments(path)?;
    let [month, file_name] = segments.as_slice() else {
        return None;
    };
    Some((month, file_name.strip_suffix(".md")?))
}

/// The stem of `<stem>.md`, refusing anything nested.
fn flat_stem(path: &Path) -> Option<&str> {
    let segments = normal_segments(path)?;
    let [file_name] = segments.as_slice() else {
        return None;
    };
    file_name.strip_suffix(".md")
}

/// The path's components, or `None` if any of them is not a plain name.
fn normal_segments(path: &Path) -> Option<Vec<&str>> {
    let mut segments = Vec::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => segments.push(part.to_str()?),
            _ => return None,
        }
    }

    Some(segments)
}

/// Which of the three authored trees a record belongs to.
///
/// Carried by store errors so that one message, and later one error code, can
/// name what was not found without three near-identical variants of everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordKind {
    Capture,
    Idea,
    Event,
}

impl RecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Idea => "idea",
            Self::Event => "event",
        }
    }
}

impl fmt::Display for RecordKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Who a capture, an idea or an event belongs to.
///
/// Open is not "unknown". It is the wiki with no accounts, where there is one
/// user, nothing is refused, and an owner would be a name for the only person
/// there is. See `knowledge-base/accounts.md`.
///
/// Two records belong together only when their owners are **equal**, and that is
/// the whole of the rule. An open capture cannot be connected to `tim`'s idea,
/// which matters on a wiki that had no accounts when the capture was written and
/// has them now: adopting those captures means writing `owner:` into their
/// files, and there is deliberately no code path that guesses it for anybody.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Owner(Option<Username>);

impl Owner {
    /// The one user of a wiki with no accounts.
    pub fn open() -> Self {
        Self(None)
    }

    pub fn of(username: Username) -> Self {
        Self(Some(username))
    }

    pub fn is_open(&self) -> bool {
        self.0.is_none()
    }

    pub fn username(&self) -> Option<&Username> {
        self.0.as_ref()
    }
}

impl From<Option<Username>> for Owner {
    fn from(username: Option<Username>) -> Self {
        Self(username)
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(username) => formatter.write_str(username.as_str()),
            None => formatter.write_str("the open user"),
        }
    }
}

/// Why an authored file under `.rhizolog/ideas/` could not be read.
///
/// One type for all three formats, because they are three spellings of one
/// thing: a file that does not parse, reported by the store the same way
/// whichever tree it came from and skipped by reconciliation the same way.
#[derive(Debug, Error)]
pub enum RecordError {
    #[error(transparent)]
    Frontmatter(#[from] FrontmatterError),

    #[error("{record} files need a `{field}` in their frontmatter")]
    MissingField {
        record: RecordKind,
        field: &'static str,
    },

    #[error("an idea needs at least one id in `captures`: it is what the thread was started from")]
    NoSeeds,

    #[error("an idea may name at most {MAX_SEEDS} captures in `captures`, not {count}")]
    TooManySeeds { count: usize },

    #[error("a `{event}` event needs a `{field}`")]
    MissingReference {
        event: EventKind,
        field: &'static str,
    },

    #[error("a `{event}` event must not carry `{field}`")]
    UnexpectedReference {
        event: EventKind,
        field: &'static str,
    },

    #[error(
        "a `{event}` event names either an `idea` and a `capture` or a `capture` and an \
         `other_capture`, and this one does neither"
    )]
    CandidateShape { event: EventKind },

    #[error("a capture cannot be paired with itself")]
    SelfPair,

    #[error(
        "this event's id names {expected} and its `created` says {found}; an event's id is when \
         the decision happened, so the two cannot disagree"
    )]
    IdDisagreement { expected: String, found: String },
}

/// A capture's frontmatter, exactly as it appears in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureFrontmatter {
    /// When the text was captured.
    ///
    /// Reads a bare `2026-08-20` as midnight UTC, as a page's `created` does.
    /// Absent, it is the instant the id already names, which is exact rather
    /// than a guess: the id was minted from it.
    #[serde(
        default,
        deserialize_with = "frontmatter::timestamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub created: Option<DateTime<Utc>>,

    /// Absent on a wiki with no accounts.
    #[serde(default, skip_serializing_if = "Owner::is_open")]
    pub owner: Owner,
}

/// One captured thought, as parsed from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub id: CaptureId,
    pub created: DateTime<Utc>,
    pub owner: Owner,
    /// The text as it was supplied, without the frontmatter.
    pub body: String,
    /// The file's mtime. Not part of the file's contents.
    pub updated: DateTime<Utc>,
    /// The file's length in bytes, as it sits on disk.
    pub size: u64,
}

impl Capture {
    pub fn from_markdown(
        id: CaptureId,
        text: &str,
        updated: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        let size = text.len() as u64;
        let text = frontmatter::strip_bom(text);

        let (fields, body): (CaptureFrontmatter, &str) = match frontmatter::split(text)? {
            Some((yaml, body)) => (frontmatter::parse(yaml)?, body),
            None => (CaptureFrontmatter::default(), text),
        };

        Ok(Self {
            created: fields.created.unwrap_or_else(|| id.instant()),
            owner: fields.owner,
            id,
            body: body.to_owned(),
            updated,
            size,
        })
    }

    pub fn to_markdown(&self) -> String {
        let yaml = serde_yaml_ng::to_string(&CaptureFrontmatter {
            created: Some(self.created),
            owner: self.owner.clone(),
        })
        .expect("capture frontmatter is a timestamp and a name");

        frontmatter::compose(&yaml, &self.body)
    }

    /// Whether there is any text here at all.
    pub fn is_blank(&self) -> bool {
        self.body.trim().is_empty()
    }
}

/// An idea thread's frontmatter, exactly as it appears in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaFrontmatter {
    /// What the person called it. Never generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(
        default,
        deserialize_with = "frontmatter::timestamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub created: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Owner::is_open")]
    pub owner: Owner,

    /// The captures the thread was started from. Spelled `captures:` in the
    /// file and called seeds everywhere else, because that is what they are:
    /// the grouping somebody made, not the membership as it stands now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<CaptureId>,
}

/// One named thread, as parsed from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Idea {
    pub id: IdeaId,
    pub name: String,
    pub created: DateTime<Utc>,
    pub owner: Owner,
    /// The captures it was created from, deduplicated, in file order. Immutable
    /// through the API: later membership is a fold over events.
    pub seeds: Vec<CaptureId>,
    /// Working notes, without the frontmatter. Usually empty.
    pub note: String,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

impl Idea {
    pub fn from_markdown(
        id: IdeaId,
        text: &str,
        updated: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        let size = text.len() as u64;
        let text = frontmatter::strip_bom(text);

        let (fields, note): (IdeaFrontmatter, &str) = match frontmatter::split(text)? {
            Some((yaml, body)) => (frontmatter::parse(yaml)?, body),
            None => (IdeaFrontmatter::default(), text),
        };

        // A note can be empty and a timestamp can be recovered from the id, but
        // a name is the one thing here that nothing may invent: the promise is
        // that Rhizolog never generates one.
        let name = fields.name.ok_or(RecordError::MissingField {
            record: RecordKind::Idea,
            field: "name",
        })?;

        let seeds = dedupe(fields.captures);
        if seeds.is_empty() {
            return Err(RecordError::NoSeeds);
        }
        if seeds.len() > MAX_SEEDS {
            return Err(RecordError::TooManySeeds { count: seeds.len() });
        }

        Ok(Self {
            created: fields.created.unwrap_or_else(|| id.instant()),
            owner: fields.owner,
            id,
            name,
            seeds,
            note: note.to_owned(),
            updated,
            size,
        })
    }

    pub fn to_markdown(&self) -> String {
        let yaml = serde_yaml_ng::to_string(&IdeaFrontmatter {
            name: Some(self.name.clone()),
            created: Some(self.created),
            owner: self.owner.clone(),
            captures: self.seeds.clone(),
        })
        .expect("idea frontmatter is a name, a timestamp and a list of ids");

        frontmatter::compose(&yaml, &self.note)
    }
}

/// Keep the first mention of each capture and drop the rest.
fn dedupe(seeds: Vec<CaptureId>) -> Vec<CaptureId> {
    let mut seen = std::collections::HashSet::new();
    seeds
        .into_iter()
        .filter(|seed| seen.insert(seed.clone()))
        .collect()
}

/// What somebody did.
///
/// Twelve words, spelled in the file exactly as they are here. Adding one is an
/// authored-format change: a reader that has not heard of a kind refuses the
/// file, so a writer for a new kind may not ship before the reader for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    CaptureConnected,
    CaptureDisconnected,
    CandidateRejected,
    CandidateReconsidered,
    InterestAffirmed,
    CaptureArchived,
    CaptureRestored,
    CaptureDeleted,
    IdeaRetired,
    IdeaReopened,
    IdeaPromoted,
    RediscoveryDismissed,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CaptureConnected => "capture_connected",
            Self::CaptureDisconnected => "capture_disconnected",
            Self::CandidateRejected => "candidate_rejected",
            Self::CandidateReconsidered => "candidate_reconsidered",
            Self::InterestAffirmed => "interest_affirmed",
            Self::CaptureArchived => "capture_archived",
            Self::CaptureRestored => "capture_restored",
            Self::CaptureDeleted => "capture_deleted",
            Self::IdeaRetired => "idea_retired",
            Self::IdeaReopened => "idea_reopened",
            Self::IdeaPromoted => "idea_promoted",
            Self::RediscoveryDismissed => "rediscovery_dismissed",
        }
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What an event is about: the records it names, having been checked against
/// its kind.
///
/// Kept beside [`Event::kind`] rather than folded into it. The file's vocabulary
/// is the twelve words of [`EventKind`], and two of them accept either of two
/// reference shapes, so a single enum would have fourteen variants and no longer
/// match what is written down. This way the kind is the file's word for the
/// decision and the subject is the validated result of reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A capture and an idea: connected, disconnected, or suggested for it.
    IdeaCapture { idea: IdeaId, capture: CaptureId },

    /// Two captures suggested for each other, before either belongs to a
    /// thread. Held in lexical order, so the same pair has one identity
    /// whichever capture produced the suggestion.
    CapturePair {
        capture: CaptureId,
        other: CaptureId,
    },

    /// A whole idea: affirmed, retired, reopened, or dismissed from
    /// rediscovery.
    Idea { idea: IdeaId },

    /// A whole capture: archived, restored, or deleted.
    Capture { capture: CaptureId },

    /// The page an idea produced.
    Promotion { idea: IdeaId, page: Slug },
}

impl Subject {
    /// Two captures, in the order that makes the pair canonical.
    pub fn pair(capture: CaptureId, other: CaptureId) -> Self {
        if other < capture {
            Self::CapturePair {
                capture: other,
                other: capture,
            }
        } else {
            Self::CapturePair { capture, other }
        }
    }

    /// This subject, checked against the kind of event it would belong to.
    ///
    /// Runs the *reader's* validation over what the writer would put in the
    /// file, so there is exactly one place that decides which references a kind
    /// may carry. A subject that comes back from here is one the next startup
    /// can parse; a capture pair comes back in canonical order whichever way it
    /// went in.
    pub fn checked_for(self, kind: EventKind) -> Result<Self, RecordError> {
        Self::read(kind, &self.references())
    }

    pub fn idea(&self) -> Option<&IdeaId> {
        match self {
            Self::IdeaCapture { idea, .. } | Self::Idea { idea } | Self::Promotion { idea, .. } => {
                Some(idea)
            }
            Self::CapturePair { .. } | Self::Capture { .. } => None,
        }
    }

    pub fn capture(&self) -> Option<&CaptureId> {
        match self {
            Self::IdeaCapture { capture, .. }
            | Self::CapturePair { capture, .. }
            | Self::Capture { capture } => Some(capture),
            Self::Idea { .. } | Self::Promotion { .. } => None,
        }
    }

    pub fn other_capture(&self) -> Option<&CaptureId> {
        match self {
            Self::CapturePair { other, .. } => Some(other),
            _ => None,
        }
    }

    pub fn page(&self) -> Option<&Slug> {
        match self {
            Self::Promotion { page, .. } => Some(page),
            _ => None,
        }
    }

    fn references(&self) -> References {
        References {
            idea: self.idea().cloned(),
            capture: self.capture().cloned(),
            other_capture: self.other_capture().cloned(),
            page: self.page().cloned(),
        }
    }

    /// Read a subject from the references an event file carries.
    fn read(kind: EventKind, refs: &References) -> Result<Self, RecordError> {
        match kind {
            EventKind::CaptureConnected | EventKind::CaptureDisconnected => {
                let idea = refs.required_idea(kind)?;
                let capture = refs.required_capture(kind)?;
                refs.refuse(kind, &[Reference::OtherCapture, Reference::Page])?;
                Ok(Self::IdeaCapture { idea, capture })
            }

            // The one kind with two shapes, because a suggestion can point at a
            // thread that exists or at another loose capture, and connecting to
            // the second is how a thread comes to exist at all.
            EventKind::CandidateRejected | EventKind::CandidateReconsidered => {
                refs.refuse(kind, &[Reference::Page])?;

                match (&refs.idea, &refs.capture, &refs.other_capture) {
                    (Some(idea), Some(capture), None) => Ok(Self::IdeaCapture {
                        idea: idea.clone(),
                        capture: capture.clone(),
                    }),
                    (None, Some(capture), Some(other)) if capture == other => {
                        Err(RecordError::SelfPair)
                    }
                    (None, Some(capture), Some(other)) => {
                        Ok(Self::pair(capture.clone(), other.clone()))
                    }
                    _ => Err(RecordError::CandidateShape { event: kind }),
                }
            }

            EventKind::InterestAffirmed
            | EventKind::IdeaRetired
            | EventKind::IdeaReopened
            | EventKind::RediscoveryDismissed => {
                let idea = refs.required_idea(kind)?;
                refs.refuse(
                    kind,
                    &[Reference::Capture, Reference::OtherCapture, Reference::Page],
                )?;
                Ok(Self::Idea { idea })
            }

            EventKind::CaptureArchived | EventKind::CaptureRestored | EventKind::CaptureDeleted => {
                let capture = refs.required_capture(kind)?;
                refs.refuse(
                    kind,
                    &[Reference::Idea, Reference::OtherCapture, Reference::Page],
                )?;
                Ok(Self::Capture { capture })
            }

            EventKind::IdeaPromoted => {
                let idea = refs.required_idea(kind)?;
                let page = refs.page.clone().ok_or(RecordError::MissingReference {
                    event: kind,
                    field: Reference::Page.as_str(),
                })?;
                refs.refuse(kind, &[Reference::Capture, Reference::OtherCapture])?;
                Ok(Self::Promotion { idea, page })
            }
        }
    }
}

/// One of the four reference fields an event's frontmatter may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reference {
    Idea,
    Capture,
    OtherCapture,
    Page,
}

impl Reference {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idea => "idea",
            Self::Capture => "capture",
            Self::OtherCapture => "other_capture",
            Self::Page => "page",
        }
    }
}

/// The reference half of an event's frontmatter, on its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct References {
    idea: Option<IdeaId>,
    capture: Option<CaptureId>,
    other_capture: Option<CaptureId>,
    page: Option<Slug>,
}

impl References {
    fn is_present(&self, reference: Reference) -> bool {
        match reference {
            Reference::Idea => self.idea.is_some(),
            Reference::Capture => self.capture.is_some(),
            Reference::OtherCapture => self.other_capture.is_some(),
            Reference::Page => self.page.is_some(),
        }
    }

    fn required_idea(&self, event: EventKind) -> Result<IdeaId, RecordError> {
        self.idea.clone().ok_or(RecordError::MissingReference {
            event,
            field: Reference::Idea.as_str(),
        })
    }

    fn required_capture(&self, event: EventKind) -> Result<CaptureId, RecordError> {
        self.capture.clone().ok_or(RecordError::MissingReference {
            event,
            field: Reference::Capture.as_str(),
        })
    }

    /// Refuse a reference the fold will never look at.
    ///
    /// A stray `capture:` on a retirement is not harmless: it says something the
    /// file does not mean, and nothing downstream would ever contradict it.
    fn refuse(&self, event: EventKind, fields: &[Reference]) -> Result<(), RecordError> {
        for reference in fields {
            if self.is_present(*reference) {
                return Err(RecordError::UnexpectedReference {
                    event,
                    field: reference.as_str(),
                });
            }
        }
        Ok(())
    }
}

/// A decision event's frontmatter, exactly as it appears in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFrontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<EventKind>,

    #[serde(
        default,
        deserialize_with = "frontmatter::timestamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub created: Option<DateTime<Utc>>,

    /// Required on a wiki with accounts, absent on an open one.
    #[serde(default, skip_serializing_if = "Owner::is_open")]
    pub actor: Owner,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idea: Option<IdeaId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_capture: Option<CaptureId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Slug>,
}

impl EventFrontmatter {
    fn references(&self) -> References {
        References {
            idea: self.idea.clone(),
            capture: self.capture.clone(),
            other_capture: self.other_capture.clone(),
            page: self.page.clone(),
        }
    }
}

/// One decision, as parsed from its file.
///
/// **An event is its frontmatter.** Anything written after it is ignored: an
/// event records that something was done, and a place to write about it would
/// be a second home for the note that belongs on the idea. Nothing is lost by
/// ignoring it, because events are append-only and no API path ever rewrites
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: EventId,
    pub kind: EventKind,
    pub subject: Subject,
    pub created: DateTime<Utc>,
    pub actor: Owner,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

impl Event {
    pub fn from_markdown(
        id: EventId,
        text: &str,
        updated: DateTime<Utc>,
    ) -> Result<Self, RecordError> {
        let size = text.len() as u64;
        let text = frontmatter::strip_bom(text);

        let fields: EventFrontmatter = match frontmatter::split(text)? {
            Some((yaml, _)) => frontmatter::parse(yaml)?,
            None => EventFrontmatter::default(),
        };

        let kind = fields.kind.ok_or(RecordError::MissingField {
            record: RecordKind::Event,
            field: "kind",
        })?;
        let subject = Subject::read(kind, &fields.references())?;
        let created = created_agreeing_with(&id, fields.created)?;

        Ok(Self {
            id,
            kind,
            subject,
            created,
            actor: fields.actor,
            updated,
            size,
        })
    }

    pub fn to_markdown(&self) -> String {
        let refs = self.subject.references();
        let yaml = serde_yaml_ng::to_string(&EventFrontmatter {
            kind: Some(self.kind),
            created: Some(self.created),
            actor: self.actor.clone(),
            idea: refs.idea,
            capture: refs.capture,
            other_capture: refs.other_capture,
            page: refs.page,
        })
        .expect("event frontmatter is a word, a timestamp and a handful of ids");

        frontmatter::compose(&yaml, "")
    }
}

/// The instant an event happened, refusing a file that contradicts its own id.
///
/// Events are folded in id order, so an id is not merely a name: it is the
/// claim about when the decision was taken. A file whose `created` says
/// something else has two answers to one question, and picking either quietly
/// would put the event in an order its own frontmatter denies. An absent
/// `created` is not a contradiction and is filled in from the id, nanoseconds
/// included.
fn created_agreeing_with(
    id: &EventId,
    created: Option<DateTime<Utc>>,
) -> Result<DateTime<Utc>, RecordError> {
    let Some(created) = created else {
        return Ok(id.instant());
    };

    let found = created.format(ID_STAMP_FORMAT).to_string();
    if found != id.stamp() {
        return Err(RecordError::IdDisagreement {
            expected: id.stamp().to_owned(),
            found,
        });
    }

    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn capture_id(raw: &str) -> CaptureId {
        CaptureId::parse(raw).expect("valid capture id")
    }

    fn idea_id(raw: &str) -> IdeaId {
        IdeaId::parse(raw).expect("valid idea id")
    }

    fn event_id(raw: &str) -> EventId {
        EventId::parse(raw).expect("valid event id")
    }

    fn tim() -> Owner {
        Owner::of(Username::parse("tim").expect("valid username"))
    }

    #[test]
    fn ids_are_minted_from_the_instant_they_record() {
        let minted = CaptureId::mint(at("2026-08-20T14:15:30.123456789Z"), 0);

        assert_eq!(minted.as_str(), "20260820T141530-123456789");
        assert_eq!(minted.stamp(), "20260820T141530");
        assert_eq!(minted.month(), "2026-08");
        assert_eq!(minted.instant(), at("2026-08-20T14:15:30.123456789Z"));
    }

    /// A hand-written `created` is whole seconds, so two records made for the
    /// same second must not name the same file.
    #[test]
    fn the_nudge_separates_two_records_at_the_same_instant() {
        let created = at("2026-08-20T09:00:00Z");

        assert_eq!(
            EventId::mint(created, 0).as_str(),
            "20260820T090000-000000000"
        );
        assert_eq!(
            EventId::mint(created, 1).as_str(),
            "20260820T090000-000000001"
        );
    }

    /// The property the whole inbox leans on for ordering, and the one events
    /// lean on for fold order.
    #[test]
    fn ids_sort_chronologically_as_text() {
        let mut ids = [
            event_id("20260820T142030-345678901"),
            event_id("20250101T000000-000000000"),
            event_id("20260820T142030-345678902"),
        ];
        ids.sort();

        assert_eq!(
            ids.map(|id| id.as_str().to_owned()),
            [
                "20250101T000000-000000000",
                "20260820T142030-345678901",
                "20260820T142030-345678902",
            ]
        );
    }

    /// An id becomes a filesystem path, so the shape is the whole defence.
    #[test]
    fn rejects_anything_that_is_not_exactly_an_id() {
        for raw in [
            "",
            "20260820T141530",
            "20260820T141530-12345678",
            "20260820T141530-1234567890",
            "../../../../../../etc/passwd",
            "20260820X141530-123456789",
            "20260820T141530_123456789",
            "2026082aT141530-123456789",
            "20260820T141530-12345678a",
            "20260820T141530-12345678\u{e9}",
        ] {
            assert!(CaptureId::parse(raw).is_err(), "{raw:?} was accepted");
            assert!(IdeaId::parse(raw).is_err(), "{raw:?} was accepted");
            assert!(EventId::parse(raw).is_err(), "{raw:?} was accepted");
        }

        // Shaped correctly but not a real instant.
        assert!(matches!(
            CaptureId::parse("20261340T141530-123456789"),
            Err(IdError::Timestamp { .. })
        ));
        assert!(matches!(
            CaptureId::parse("20260820T256130-123456789"),
            Err(IdError::Timestamp { .. })
        ));
    }

    /// The message names which of the three trees the caller was addressing,
    /// because "id" on its own does not say what to go and look at.
    #[test]
    fn a_refusal_says_which_kind_of_id_it_wanted() {
        let error = IdeaId::parse("nope").expect_err("refused");
        assert!(error.to_string().starts_with("idea ids are"), "{error}");

        let error = EventId::parse("20260820X141530-123456789").expect_err("refused");
        assert!(
            error.to_string().starts_with("event ids may only"),
            "{error}"
        );
    }

    #[test]
    fn captures_and_events_are_filed_by_month_and_threads_are_not() {
        let root = Path::new("/wiki/.rhizolog/ideas");

        assert_eq!(
            capture_id("20260820T141530-123456789").to_path(&root.join(CAPTURES_DIR)),
            Path::new("/wiki/.rhizolog/ideas/captures/2026-08/20260820T141530-123456789.md")
        );
        assert_eq!(
            event_id("20260820T142030-345678901").to_path(&root.join(EVENTS_DIR)),
            Path::new("/wiki/.rhizolog/ideas/events/2026-08/20260820T142030-345678901.md")
        );
        assert_eq!(
            idea_id("20260820T142000-234567890").to_path(&root.join(THREADS_DIR)),
            Path::new("/wiki/.rhizolog/ideas/threads/20260820T142000-234567890.md")
        );
    }

    #[test]
    fn ids_round_trip_through_a_relative_path() {
        let capture = capture_id("20260820T141530-123456789");
        assert_eq!(
            CaptureId::from_relative_path(&capture.to_path(Path::new(""))),
            Some(capture)
        );

        let idea = idea_id("20260820T142000-234567890");
        assert_eq!(
            IdeaId::from_relative_path(&idea.to_path(Path::new(""))),
            Some(idea)
        );

        let event = event_id("20260820T142030-345678901");
        assert_eq!(
            EventId::from_relative_path(&event.to_path(Path::new(""))),
            Some(event)
        );
    }

    #[test]
    fn ignores_paths_that_are_not_records() {
        for path in [
            "2026-08/notes.md",
            "20260820T141530-123456789.md",
            "2026-08/20260820T141530-123456789.txt",
            "2026-08/sub/20260820T141530-123456789.md",
            // Filed under a month it does not belong to.
            "2026-07/20260820T141530-123456789.md",
        ] {
            assert_eq!(
                CaptureId::from_relative_path(Path::new(path)),
                None,
                "{path}"
            );
        }

        // A thread is flat, so a month directory is exactly as wrong here as a
        // missing one is for a capture.
        for path in [
            "notes.md",
            "2026-08/20260820T142000-234567890.md",
            "20260820T142000-234567890.txt",
        ] {
            assert_eq!(IdeaId::from_relative_path(Path::new(path)), None, "{path}");
        }
    }

    fn capture(text: &str) -> Capture {
        Capture::from_markdown(
            capture_id("20260820T141530-123456789"),
            text,
            at("2026-08-20T16:00:00Z"),
        )
        .expect("capture should parse")
    }

    #[test]
    fn parses_a_capture() {
        let parsed = capture(
            "---\ncreated: 2026-08-20T14:15:30Z\nowner: tim\n---\n\nMaybe dungeon quests should require finding particular seeds.\n",
        );

        assert_eq!(parsed.created, at("2026-08-20T14:15:30Z"));
        assert_eq!(parsed.owner, tim());
        assert_eq!(
            parsed.body,
            "\nMaybe dungeon quests should require finding particular seeds.\n"
        );
        assert!(!parsed.is_blank());
    }

    /// A capture is one text field and one action, so a file that is nothing but
    /// text is a capture. There is no field here worth refusing it over.
    #[test]
    fn a_capture_with_no_frontmatter_is_still_a_capture() {
        let parsed = capture("Just a thought.\n");

        assert_eq!(parsed.body, "Just a thought.\n");
        assert!(parsed.owner.is_open());
        // Recovered exactly from the id, which was minted from it.
        assert_eq!(parsed.created, at("2026-08-20T14:15:30.123456789Z"));
    }

    /// The same three invisible bytes that once hid a page's frontmatter.
    #[test]
    fn a_utf8_bom_does_not_hide_a_captures_frontmatter() {
        let parsed =
            capture("\u{feff}---\ncreated: 2026-08-20T14:15:30Z\nowner: tim\n---\n\nHi.\n");

        assert_eq!(parsed.owner, tim());
        assert_eq!(parsed.created, at("2026-08-20T14:15:30Z"));
    }

    /// Windows editors write CRLF, and this all runs on Windows.
    #[test]
    fn crlf_line_endings_parse() {
        let parsed =
            capture("---\r\ncreated: 2026-08-20T14:15:30Z\r\nowner: tim\r\n---\r\nHi.\r\n");

        assert_eq!(parsed.owner, tim());
        assert_eq!(parsed.body, "Hi.\r\n");
    }

    #[test]
    fn a_blank_capture_is_recognisable_as_one() {
        assert!(capture("---\ncreated: 2026-08-20T14:15:30Z\n---\n\n   \n").is_blank());
    }

    #[test]
    fn a_capture_round_trips_unchanged() {
        let text = "---\ncreated: 2026-08-20T14:15:30Z\nowner: tim\n---\n\nA thought.\n";
        assert_eq!(capture(text).to_markdown(), text);
    }

    fn idea(text: &str) -> Result<Idea, RecordError> {
        Idea::from_markdown(
            idea_id("20260820T142000-234567890"),
            text,
            at("2026-08-20T16:00:00Z"),
        )
    }

    #[test]
    fn parses_an_idea() {
        let parsed = idea(
            "---\nname: Dungeon seeds\ncreated: 2026-08-20T14:20:00Z\nowner: tim\ncaptures:\n- 20260820T141530-123456789\n---\n\nWorking notes.\n",
        )
        .expect("idea should parse");

        assert_eq!(parsed.name, "Dungeon seeds");
        assert_eq!(parsed.created, at("2026-08-20T14:20:00Z"));
        assert_eq!(parsed.owner, tim());
        assert_eq!(parsed.seeds, [capture_id("20260820T141530-123456789")]);
        assert_eq!(parsed.note, "\nWorking notes.\n");
    }

    /// The two things about an idea that nothing may invent: what somebody
    /// called it, and what it was started from.
    #[test]
    fn an_idea_without_a_name_or_a_seed_is_refused() {
        let error =
            idea("---\ncaptures:\n- 20260820T141530-123456789\n---\n").expect_err("refused");
        assert!(matches!(error, RecordError::MissingField { .. }), "{error}");

        let error = idea("---\nname: Dungeon seeds\n---\n").expect_err("refused");
        assert!(matches!(error, RecordError::NoSeeds), "{error}");

        let error = idea("Just a note.\n").expect_err("refused");
        assert!(matches!(error, RecordError::MissingField { .. }), "{error}");
    }

    #[test]
    fn a_repeated_seed_is_recorded_once() {
        let parsed = idea(
            "---\nname: Dungeon seeds\ncaptures:\n- 20260820T141530-123456789\n- 20260820T141530-123456789\n- 20260820T142000-000000000\n---\n",
        )
        .expect("idea should parse");

        assert_eq!(
            parsed.seeds,
            [
                capture_id("20260820T141530-123456789"),
                capture_id("20260820T142000-000000000"),
            ]
        );
    }

    #[test]
    fn an_idea_round_trips_unchanged() {
        let text = "---\nname: Dungeon seeds\ncreated: 2026-08-20T14:20:00Z\nowner: tim\ncaptures:\n- 20260820T141530-123456789\n---\n\nWorking notes.\n";
        assert_eq!(idea(text).expect("parse").to_markdown(), text);
    }

    fn event(text: &str) -> Result<Event, RecordError> {
        Event::from_markdown(
            event_id("20260820T142030-345678901"),
            text,
            at("2026-08-20T16:00:00Z"),
        )
    }

    #[test]
    fn parses_every_reference_shape() {
        let connected = event(
            "---\nkind: capture_connected\ncreated: 2026-08-20T14:20:30Z\nactor: tim\nidea: 20260820T142000-234567890\ncapture: 20260820T141530-123456789\n---\n",
        )
        .expect("connect should parse");
        assert_eq!(connected.kind, EventKind::CaptureConnected);
        assert_eq!(
            connected.subject,
            Subject::IdeaCapture {
                idea: idea_id("20260820T142000-234567890"),
                capture: capture_id("20260820T141530-123456789"),
            }
        );
        assert_eq!(connected.actor, tim());

        let retired = event("---\nkind: idea_retired\nidea: 20260820T142000-234567890\n---\n")
            .expect("retire should parse");
        assert_eq!(
            retired.subject,
            Subject::Idea {
                idea: idea_id("20260820T142000-234567890")
            }
        );

        let archived =
            event("---\nkind: capture_archived\ncapture: 20260820T141530-123456789\n---\n")
                .expect("archive should parse");
        assert_eq!(
            archived.subject,
            Subject::Capture {
                capture: capture_id("20260820T141530-123456789")
            }
        );

        let promoted = event(
            "---\nkind: idea_promoted\nidea: 20260820T142000-234567890\npage: notes/dungeon-seeds\n---\n",
        )
        .expect("promote should parse");
        assert_eq!(
            promoted.subject,
            Subject::Promotion {
                idea: idea_id("20260820T142000-234567890"),
                page: Slug::parse("notes/dungeon-seeds").expect("valid slug"),
            }
        );
    }

    /// A rejection points either at a thread or at another loose capture, and
    /// connecting to the second is how a thread comes to exist at all.
    #[test]
    fn a_candidate_rejection_takes_either_shape() {
        let against_idea = event(
            "---\nkind: candidate_rejected\nidea: 20260820T142000-234567890\ncapture: 20260820T141530-123456789\n---\n",
        )
        .expect("idea candidate should parse");
        assert!(matches!(against_idea.subject, Subject::IdeaCapture { .. }));

        let against_capture = event(
            "---\nkind: candidate_rejected\ncapture: 20260820T141530-123456789\nother_capture: 20260820T142000-000000000\n---\n",
        )
        .expect("capture pair should parse");
        assert!(matches!(
            against_capture.subject,
            Subject::CapturePair { .. }
        ));

        let neither =
            event("---\nkind: candidate_rejected\ncapture: 20260820T141530-123456789\n---\n")
                .expect_err("refused");
        assert!(
            matches!(neither, RecordError::CandidateShape { .. }),
            "{neither}"
        );
    }

    /// One pair, one identity, whichever capture produced the suggestion.
    #[test]
    fn a_capture_pair_is_held_in_lexical_order() {
        let first = capture_id("20260820T141530-123456789");
        let second = capture_id("20260820T142000-000000000");

        let forwards = Subject::pair(first.clone(), second.clone());
        let backwards = Subject::pair(second.clone(), first.clone());

        assert_eq!(forwards, backwards);
        assert_eq!(
            forwards,
            Subject::CapturePair {
                capture: first,
                other: second
            }
        );
    }

    #[test]
    fn a_capture_cannot_be_paired_with_itself() {
        let error = event(
            "---\nkind: candidate_rejected\ncapture: 20260820T141530-123456789\nother_capture: 20260820T141530-123456789\n---\n",
        )
        .expect_err("refused");

        assert!(matches!(error, RecordError::SelfPair), "{error}");
    }

    /// A reference the fold will never look at says something the file does not
    /// mean, and nothing downstream would ever contradict it.
    #[test]
    fn a_reference_the_kind_does_not_take_is_refused() {
        let error = event(
            "---\nkind: idea_retired\nidea: 20260820T142000-234567890\ncapture: 20260820T141530-123456789\n---\n",
        )
        .expect_err("refused");
        assert!(
            matches!(
                error,
                RecordError::UnexpectedReference {
                    field: "capture",
                    ..
                }
            ),
            "{error}"
        );

        let error = event(
            "---\nkind: capture_connected\nidea: 20260820T142000-234567890\ncapture: 20260820T141530-123456789\npage: notes/anything\n---\n",
        )
        .expect_err("refused");
        assert!(
            matches!(
                error,
                RecordError::UnexpectedReference { field: "page", .. }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_reference_the_kind_needs_is_named_when_it_is_missing() {
        let error =
            event("---\nkind: capture_connected\ncapture: 20260820T141530-123456789\n---\n")
                .expect_err("refused");
        assert!(
            matches!(error, RecordError::MissingReference { field: "idea", .. }),
            "{error}"
        );

        let error = event("---\nkind: idea_promoted\nidea: 20260820T142000-234567890\n---\n")
            .expect_err("refused");
        assert!(
            matches!(error, RecordError::MissingReference { field: "page", .. }),
            "{error}"
        );
    }

    #[test]
    fn an_event_without_a_kind_is_refused() {
        let error = event("---\nidea: 20260820T142000-234567890\n---\n").expect_err("refused");
        assert!(
            matches!(
                error,
                RecordError::MissingField {
                    record: RecordKind::Event,
                    field: "kind"
                }
            ),
            "{error}"
        );
    }

    /// An id is when the decision happened, so a file that contradicts its own
    /// id has two answers to the question the fold order is built on.
    #[test]
    fn an_event_may_not_disagree_with_its_own_id() {
        let error = event(
            "---\nkind: idea_retired\ncreated: 2026-08-20T14:20:31Z\nidea: 20260820T142000-234567890\n---\n",
        )
        .expect_err("refused");

        assert!(
            matches!(error, RecordError::IdDisagreement { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("20260820T142030"), "{error}");
    }

    /// Absent is not a contradiction, and the id carries the nanoseconds too.
    #[test]
    fn an_event_with_no_created_takes_the_instant_from_its_id() {
        let parsed = event("---\nkind: idea_retired\nidea: 20260820T142000-234567890\n---\n")
            .expect("parse");

        assert_eq!(parsed.created, at("2026-08-20T14:20:30.345678901Z"));
    }

    #[test]
    fn an_event_round_trips_unchanged() {
        let text = "---\nkind: capture_connected\ncreated: 2026-08-20T14:20:30Z\nactor: tim\nidea: 20260820T142000-234567890\ncapture: 20260820T141530-123456789\n---\n";
        assert_eq!(event(text).expect("parse").to_markdown(), text);
    }

    /// An event is its frontmatter. Nothing writes a body, so nothing is lost
    /// by refusing to keep one.
    #[test]
    fn an_event_writes_no_body() {
        let text =
            "---\nkind: idea_retired\nidea: 20260820T142000-234567890\n---\n\nWhy I did it.\n";
        let parsed = event(text).expect("parse");

        assert_eq!(
            parsed.to_markdown(),
            "---\nkind: idea_retired\ncreated: 2026-08-20T14:20:30.345678901Z\nidea: 20260820T142000-234567890\n---\n"
        );
    }

    /// One validator for what a kind may name, run over what a writer would
    /// write, so a subject that passes is one the next startup can read back.
    #[test]
    fn checking_a_subject_against_a_kind_uses_the_readers_rules() {
        let idea = idea_id("20260820T142000-234567890");
        let capture = capture_id("20260820T141530-123456789");

        assert!(
            Subject::Idea { idea: idea.clone() }
                .checked_for(EventKind::IdeaRetired)
                .is_ok()
        );

        let error = Subject::Idea { idea: idea.clone() }
            .checked_for(EventKind::CaptureConnected)
            .expect_err("refused");
        assert!(
            matches!(
                error,
                RecordError::MissingReference {
                    field: "capture",
                    ..
                }
            ),
            "{error}"
        );

        let error = Subject::IdeaCapture { idea, capture }
            .checked_for(EventKind::CaptureArchived)
            .expect_err("refused");
        assert!(
            matches!(
                error,
                RecordError::UnexpectedReference { field: "idea", .. }
            ),
            "{error}"
        );
    }

    #[test]
    fn an_owner_prints_as_a_name_or_as_the_one_open_user() {
        assert_eq!(tim().to_string(), "tim");
        assert_eq!(Owner::open().to_string(), "the open user");
        assert!(Owner::open().is_open());
        assert!(!tim().is_open());
        assert_ne!(tim(), Owner::open());
    }
}
