//! The Idea Inbox trees on disk.
//!
//! Three directories under `<wiki>/.rhizolog/ideas/`, and the same
//! temporary-file-and-rename dance every other authored file in Rhizolog goes
//! through, so a reader or the watcher never sees half a record:
//!
//! ```text
//! captures/2026-08/20260820T141530-123456789.md
//! threads/20260820T142000-234567890.md
//! events/2026-08/20260820T142030-345678901.md
//! ```
//!
//! Captures and events are bucketed by the month in their id, because both
//! arrive in volume and a heavy year should not become one directory of
//! thousands of files. Threads are not, because they are named by hand one at a
//! time and there are never many.
//!
//! ## What this layer does not do
//!
//! It reads and writes files. It does not know who is asking, whether a capture
//! exists before an event names it, or what an idea currently holds. Those are
//! [`super::service::IdeaService`]'s job and the derived index's, and keeping
//! them out of here is what lets startup reconciliation walk every record on a
//! wiki with accounts without having to pretend to be somebody.
//!
//! There is deliberately no way to rewrite or remove an event. Decisions are
//! append-only, and reversing one writes its inverse. A capture can be deleted
//! because the API offers that; an idea is retired rather than deleted, which is
//! an event like any other.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use thiserror::Error;
use walkdir::WalkDir;

use crate::ideas::{
    CAPTURES_DIR, Capture, CaptureId, EVENTS_DIR, Event, EventId, EventKind, IDEAS_DIR, Idea,
    IdeaId, Owner, RecordError, RecordKind, Subject, THREADS_DIR,
};
use crate::store::{INTERNAL_DIR, display_path, modified_at, write_atomically};

/// How many ids to try before giving up on finding a free one.
///
/// Only reached when many records share an instant to the nanosecond, which in
/// practice means several written for the same whole second by hand.
const MINT_ATTEMPTS: u32 = 1_000;

#[derive(Debug, Error)]
pub enum IdeaStoreError {
    #[error("no {kind} {id}")]
    NotFound { kind: RecordKind, id: String },

    #[error("{kind} {id} is not valid UTF-8")]
    NotUtf8 { kind: RecordKind, id: String },

    #[error("{kind} {id} resolves outside the {kind} directory")]
    EscapesRoot { kind: RecordKind, id: String },

    #[error("could not parse {kind} {id}: {source}")]
    Malformed {
        kind: RecordKind,
        id: String,
        #[source]
        source: RecordError,
    },

    #[error("could not mint an id for a {kind} created at {at}")]
    NoFreeId { kind: RecordKind, at: DateTime<Utc> },

    #[error(transparent)]
    Io(#[from] io::Error),
}

impl IdeaStoreError {
    /// A stable, machine-readable identifier for what went wrong.
    ///
    /// Same contract as [`crate::slug::SlugError::code`]: a caller branches on
    /// this and never on the prose. The record kind is folded into the code
    /// rather than reported beside it, because "not found" means something
    /// different to a client depending on which of the three it was.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { kind, .. } => match kind {
                RecordKind::Capture => "capture_not_found",
                RecordKind::Idea => "idea_not_found",
                RecordKind::Event => "idea_event_not_found",
            },
            Self::NotUtf8 { kind, .. } => match kind {
                RecordKind::Capture => "capture_not_utf8",
                RecordKind::Idea => "idea_not_utf8",
                RecordKind::Event => "idea_event_not_utf8",
            },
            Self::EscapesRoot { kind, .. } => match kind {
                RecordKind::Capture => "capture_escapes_root",
                RecordKind::Idea => "idea_escapes_root",
                RecordKind::Event => "idea_event_escapes_root",
            },
            Self::Malformed { kind, .. } => match kind {
                RecordKind::Capture => "capture_malformed",
                RecordKind::Idea => "idea_malformed",
                RecordKind::Event => "idea_event_malformed",
            },
            Self::NoFreeId { .. } => "idea_id_exhausted",
            Self::Io(_) => "io_error",
        }
    }

    /// The id the failure is about, for the error response's `details`.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::NotFound { id, .. }
            | Self::NotUtf8 { id, .. }
            | Self::EscapesRoot { id, .. }
            | Self::Malformed { id, .. } => Some(id),
            Self::NoFreeId { .. } | Self::Io(_) => None,
        }
    }

    fn not_found(kind: RecordKind, id: impl fmt::Display) -> Self {
        Self::NotFound {
            kind,
            id: id.to_string(),
        }
    }

    fn not_utf8(kind: RecordKind, id: impl fmt::Display) -> Self {
        Self::NotUtf8 {
            kind,
            id: id.to_string(),
        }
    }

    fn malformed(kind: RecordKind, id: impl fmt::Display, source: RecordError) -> Self {
        Self::Malformed {
            kind,
            id: id.to_string(),
            source,
        }
    }
}

/// One record found while walking a tree, without its contents.
///
/// The indexer compares `updated` and `size` against what it already has to
/// decide whether a record needs re-reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaWalkEntry<Id> {
    pub id: Id,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

/// What a caller wants a capture to say.
///
/// Everything is required, because a write replaces the file wholesale;
/// "leave this alone" lives in [`IdeaStore::patch_capture`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDraft {
    pub created: DateTime<Utc>,
    pub owner: Owner,
    pub body: String,
}

/// What a caller wants an idea's file to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaDraft {
    pub name: String,
    pub created: DateTime<Utc>,
    pub owner: Owner,
    pub seeds: Vec<CaptureId>,
    pub note: String,
}

/// What a caller wants an event to record.
///
/// Built through [`EventDraft::new`], which runs the reader's own rules over the
/// references the draft would write. A draft that exists is one the next startup
/// can parse, which is the whole of the "never ship a writer for a format the
/// reader refuses" rule expressed in a constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDraft {
    kind: EventKind,
    subject: Subject,
    created: DateTime<Utc>,
    actor: Owner,
}

impl EventDraft {
    pub fn new(
        kind: EventKind,
        subject: Subject,
        created: DateTime<Utc>,
        actor: Owner,
    ) -> Result<Self, RecordError> {
        Ok(Self {
            kind,
            // Comes back canonical: a capture pair is put in lexical order here
            // rather than at every call site that might build one.
            subject: subject.checked_for(kind)?,
            created,
            actor,
        })
    }

    pub fn kind(&self) -> EventKind {
        self.kind
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn created(&self) -> DateTime<Utc> {
        self.created
    }

    pub fn actor(&self) -> &Owner {
        &self.actor
    }
}

/// The three authored trees of Idea Inbox.
#[derive(Debug, Clone)]
pub struct IdeaStore {
    root: PathBuf,
    captures: PathBuf,
    threads: PathBuf,
    events: PathBuf,
}

impl IdeaStore {
    /// Point at Idea Inbox for the wiki at `wiki_root`, without creating it.
    ///
    /// **Nothing is written here**, unlike [`crate::times::TimeStore::open`] and
    /// the accounts store, and the reason is worth keeping. `server::start`
    /// opens the stores and then watches the wiki directory, so a store that
    /// creates directories on open is the server writing into the tree it is
    /// about to watch. Windows reports those creations *after* the watch is
    /// established, which lands a spurious full rescan in the first debounce
    /// window; that rescan indexes files whose own create events are still
    /// pending, and a file created and deleted inside one window correctly
    /// collapses to no event at all. The result is an index row for a file that
    /// is gone, until the next scan. Creating four directories at startup is not
    /// worth that.
    ///
    /// So the trees appear when something is first written to them, which also
    /// means a wiki that has never captured a thought has no `ideas/` directory
    /// to explain, back up or wonder about.
    ///
    /// The wiki root is canonicalised so the containment check in [`Self::resolve`]
    /// compares like with like; on Windows that means both sides carry the
    /// `\\?\` verbatim prefix. The tree paths are then plain joins onto it,
    /// which is the same path canonicalising would produce unless one of
    /// `.rhizolog`, `ideas`, `captures`, `threads` or `events` is itself a
    /// symlink. One that is gets refused rather than followed, exactly as a
    /// symlinked month directory does.
    pub async fn open(wiki_root: impl AsRef<Path>) -> Result<Self, IdeaStoreError> {
        let root = tokio::fs::canonicalize(wiki_root.as_ref())
            .await?
            .join(INTERNAL_DIR)
            .join(IDEAS_DIR);

        Ok(Self {
            captures: root.join(CAPTURES_DIR),
            threads: root.join(THREADS_DIR),
            events: root.join(EVENTS_DIR),
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn root_display(&self) -> String {
        display_path(&self.root)
    }

    pub fn captures_root(&self) -> &Path {
        &self.captures
    }

    pub fn threads_root(&self) -> &Path {
        &self.threads
    }

    pub fn events_root(&self) -> &Path {
        &self.events
    }

    fn tree(&self, kind: RecordKind) -> &Path {
        match kind {
            RecordKind::Capture => &self.captures,
            RecordKind::Idea => &self.threads,
            RecordKind::Event => &self.events,
        }
    }

    /// The path an id names, having confirmed it does not leave its tree.
    ///
    /// An id cannot traverse on its own: that is settled by its parser, which
    /// accepts twenty-five characters of digits and two separators. A symlinked
    /// month directory could point anywhere, though, and reading through one
    /// would be an arbitrary file read.
    async fn resolve(
        &self,
        kind: RecordKind,
        id: impl fmt::Display,
        path: PathBuf,
    ) -> Result<PathBuf, IdeaStoreError> {
        match tokio::fs::canonicalize(&path).await {
            Ok(canonical) if !canonical.starts_with(self.tree(kind)) => {
                Err(IdeaStoreError::EscapesRoot {
                    kind,
                    id: id.to_string(),
                })
            }
            Ok(_) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
            Err(error) => Err(error.into()),
        }
    }

    // Captures.

    pub async fn exists_capture(&self, id: &CaptureId) -> Result<bool, IdeaStoreError> {
        let path = self.capture_path(id).await?;
        Ok(tokio::fs::try_exists(&path).await?)
    }

    pub async fn read_capture(&self, id: &CaptureId) -> Result<Capture, IdeaStoreError> {
        let path = self.capture_path(id).await?;

        match slurp(&path).await? {
            Slurped::Missing => Err(IdeaStoreError::not_found(RecordKind::Capture, id)),
            Slurped::NotUtf8 => Err(IdeaStoreError::not_utf8(RecordKind::Capture, id)),
            Slurped::Text { text, updated } => Capture::from_markdown(id.clone(), &text, updated)
                .map_err(|source| IdeaStoreError::malformed(RecordKind::Capture, id, source)),
        }
    }

    /// Write a capture at `id`, replacing whatever was there.
    pub async fn write_capture(
        &self,
        id: &CaptureId,
        draft: CaptureDraft,
    ) -> Result<Capture, IdeaStoreError> {
        let path = self.capture_path(id).await?;

        let capture = Capture {
            id: id.clone(),
            created: draft.created,
            owner: draft.owner,
            body: draft.body,
            // Both replaced below with what the filesystem actually recorded.
            updated: Utc::now(),
            size: 0,
        };
        let (updated, size) = put(&path, &capture.to_markdown()).await?;

        Ok(Capture {
            updated,
            size,
            ..capture
        })
    }

    /// Record a new capture, minting an id for it.
    pub async fn create_capture(&self, draft: CaptureDraft) -> Result<Capture, IdeaStoreError> {
        for nudge in 0..MINT_ATTEMPTS {
            let id = CaptureId::mint(draft.created, nudge);
            if !self.exists_capture(&id).await? {
                return self.write_capture(&id, draft).await;
            }
        }

        Err(IdeaStoreError::NoFreeId {
            kind: RecordKind::Capture,
            at: draft.created,
        })
    }

    /// Correct a capture's text, leaving everything else exactly as it was.
    ///
    /// `created` and `owner` are carried over rather than accepted, which is
    /// what makes them immutable through the API rather than merely undocumented
    /// as changeable. Git remains the history of such edits, as it is for pages.
    pub async fn patch_capture(
        &self,
        id: &CaptureId,
        body: &str,
    ) -> Result<Capture, IdeaStoreError> {
        let existing = self.read_capture(id).await?;

        self.write_capture(
            id,
            CaptureDraft {
                created: existing.created,
                owner: existing.owner,
                body: body.to_owned(),
            },
        )
        .await
    }

    pub async fn delete_capture(&self, id: &CaptureId) -> Result<(), IdeaStoreError> {
        let path = self.capture_path(id).await?;
        remove(&path, RecordKind::Capture, id).await?;
        prune_month(&path, &self.captures).await;
        Ok(())
    }

    /// Every capture in the inbox.
    ///
    /// Blocking: callers on an async task should wrap this in `spawn_blocking`.
    /// It exists for the startup scan.
    pub fn walk_captures(&self) -> Vec<IdeaWalkEntry<CaptureId>> {
        walk(&self.captures, CaptureId::from_relative_path, "captures")
    }

    async fn capture_path(&self, id: &CaptureId) -> Result<PathBuf, IdeaStoreError> {
        self.resolve(RecordKind::Capture, id, id.to_path(&self.captures))
            .await
    }

    // Idea threads.

    pub async fn exists_idea(&self, id: &IdeaId) -> Result<bool, IdeaStoreError> {
        let path = self.idea_path(id).await?;
        Ok(tokio::fs::try_exists(&path).await?)
    }

    pub async fn read_idea(&self, id: &IdeaId) -> Result<Idea, IdeaStoreError> {
        let path = self.idea_path(id).await?;

        match slurp(&path).await? {
            Slurped::Missing => Err(IdeaStoreError::not_found(RecordKind::Idea, id)),
            Slurped::NotUtf8 => Err(IdeaStoreError::not_utf8(RecordKind::Idea, id)),
            Slurped::Text { text, updated } => Idea::from_markdown(id.clone(), &text, updated)
                .map_err(|source| IdeaStoreError::malformed(RecordKind::Idea, id, source)),
        }
    }

    pub async fn write_idea(&self, id: &IdeaId, draft: IdeaDraft) -> Result<Idea, IdeaStoreError> {
        let path = self.idea_path(id).await?;

        let idea = Idea {
            id: id.clone(),
            name: draft.name,
            created: draft.created,
            owner: draft.owner,
            seeds: draft.seeds,
            note: draft.note,
            updated: Utc::now(),
            size: 0,
        };
        let (updated, size) = put(&path, &idea.to_markdown()).await?;

        Ok(Idea {
            updated,
            size,
            ..idea
        })
    }

    /// Start a new thread, minting an id for it.
    ///
    /// One authored-file write, seeds included, so an idea either exists with
    /// the grouping somebody made or does not exist at all.
    pub async fn create_idea(&self, draft: IdeaDraft) -> Result<Idea, IdeaStoreError> {
        for nudge in 0..MINT_ATTEMPTS {
            let id = IdeaId::mint(draft.created, nudge);
            if !self.exists_idea(&id).await? {
                return self.write_idea(&id, draft).await;
            }
        }

        Err(IdeaStoreError::NoFreeId {
            kind: RecordKind::Idea,
            at: draft.created,
        })
    }

    /// Rename a thread or edit its note, leaving its seeds and origin alone.
    ///
    /// `None` means "leave this one as it is", which is what makes this a patch
    /// rather than a write with two fields the caller has to remember to repeat.
    pub async fn patch_idea(
        &self,
        id: &IdeaId,
        name: Option<&str>,
        note: Option<&str>,
    ) -> Result<Idea, IdeaStoreError> {
        let existing = self.read_idea(id).await?;

        self.write_idea(
            id,
            IdeaDraft {
                name: name.map_or(existing.name, str::to_owned),
                created: existing.created,
                owner: existing.owner,
                seeds: existing.seeds,
                note: note.map_or(existing.note, str::to_owned),
            },
        )
        .await
    }

    /// Every idea thread.
    ///
    /// Blocking, for the same reason [`IdeaStore::walk_captures`] is.
    pub fn walk_ideas(&self) -> Vec<IdeaWalkEntry<IdeaId>> {
        walk(&self.threads, IdeaId::from_relative_path, "idea threads")
    }

    async fn idea_path(&self, id: &IdeaId) -> Result<PathBuf, IdeaStoreError> {
        self.resolve(RecordKind::Idea, id, id.to_path(&self.threads))
            .await
    }

    // Decision events.

    pub async fn exists_event(&self, id: &EventId) -> Result<bool, IdeaStoreError> {
        let path = self.event_path(id).await?;
        Ok(tokio::fs::try_exists(&path).await?)
    }

    pub async fn read_event(&self, id: &EventId) -> Result<Event, IdeaStoreError> {
        let path = self.event_path(id).await?;

        match slurp(&path).await? {
            Slurped::Missing => Err(IdeaStoreError::not_found(RecordKind::Event, id)),
            Slurped::NotUtf8 => Err(IdeaStoreError::not_utf8(RecordKind::Event, id)),
            Slurped::Text { text, updated } => Event::from_markdown(id.clone(), &text, updated)
                .map_err(|source| IdeaStoreError::malformed(RecordKind::Event, id, source)),
        }
    }

    /// Append a decision, minting an id for it.
    ///
    /// The only way to write an event. There is no rewrite and no delete: ids
    /// define fold order, so editing one would reorder history rather than
    /// correct it, and the inverse event is how a decision is taken back.
    pub async fn append_event(&self, draft: EventDraft) -> Result<Event, IdeaStoreError> {
        for nudge in 0..MINT_ATTEMPTS {
            let id = EventId::mint(draft.created, nudge);
            if self.exists_event(&id).await? {
                continue;
            }

            let path = self.event_path(&id).await?;
            let event = Event {
                // Minted from `created` and then agreeing with it by
                // construction, which is the invariant `Event::from_markdown`
                // refuses a file over.
                created: id.instant(),
                id,
                kind: draft.kind,
                subject: draft.subject,
                actor: draft.actor,
                updated: Utc::now(),
                size: 0,
            };
            let (updated, size) = put(&path, &event.to_markdown()).await?;

            return Ok(Event {
                updated,
                size,
                ..event
            });
        }

        Err(IdeaStoreError::NoFreeId {
            kind: RecordKind::Event,
            at: draft.created,
        })
    }

    /// Write `actor` into an event that has none.
    ///
    /// The one operation in this module that rewrites a decision, and it exists
    /// for exactly one caller: [`crate::ideas::adoption`], which stamps an owner
    /// onto the records a wiki accumulated before it had accounts. It changes
    /// who a decision is attributed to and never what was decided or when, so
    /// the fold is untouched and the id, which *is* the fold order, does not
    /// move.
    ///
    /// An event that already names an actor is left alone and reported `false`,
    /// so this can never reattribute one person's decision to another.
    pub(crate) async fn adopt_event(
        &self,
        id: &EventId,
        actor: &Owner,
    ) -> Result<Option<Event>, IdeaStoreError> {
        let existing = self.read_event(id).await?;
        if !existing.actor.is_open() {
            return Ok(None);
        }

        let path = self.event_path(id).await?;
        let event = Event {
            actor: actor.clone(),
            ..existing
        };
        let (updated, size) = put(&path, &event.to_markdown()).await?;

        Ok(Some(Event {
            updated,
            size,
            ..event
        }))
    }

    /// Every decision event, in no particular order.
    ///
    /// Blocking, for the same reason [`IdeaStore::walk_captures`] is. Sorting
    /// by id is the fold's job and it has to sort across months anyway.
    pub fn walk_events(&self) -> Vec<IdeaWalkEntry<EventId>> {
        walk(&self.events, EventId::from_relative_path, "decision events")
    }

    async fn event_path(&self, id: &EventId) -> Result<PathBuf, IdeaStoreError> {
        self.resolve(RecordKind::Event, id, id.to_path(&self.events))
            .await
    }
}

/// The three outcomes of reading an authored file.
enum Slurped {
    Text {
        text: String,
        updated: DateTime<Utc>,
    },
    Missing,
    NotUtf8,
}

async fn slurp(path: &Path) -> io::Result<Slurped> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(error) => {
            return match error.kind() {
                io::ErrorKind::NotFound => Ok(Slurped::Missing),
                io::ErrorKind::InvalidData => Ok(Slurped::NotUtf8),
                _ => Err(error),
            };
        }
    };

    let updated = modified_at(&tokio::fs::metadata(path).await?)?;
    Ok(Slurped::Text { text, updated })
}

/// Write `text` atomically, creating the month directory if it is not there,
/// and report what the filesystem recorded.
async fn put(path: &Path, text: &str) -> io::Result<(DateTime<Utc>, u64)> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    write_atomically(path, text.as_bytes()).await?;

    let metadata = tokio::fs::metadata(path).await?;
    Ok((modified_at(&metadata)?, metadata.len()))
}

async fn remove(
    path: &Path,
    kind: RecordKind,
    id: impl fmt::Display,
) -> Result<(), IdeaStoreError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(IdeaStoreError::not_found(kind, id))
        }
        Err(error) => Err(error.into()),
    }
}

/// Remove a month directory that a delete has emptied.
///
/// Best effort, and only the one level: the tree's own root stays.
async fn prune_month(path: &Path, tree: &Path) {
    if let Some(month) = path.parent()
        && month.starts_with(tree)
        && month != tree
    {
        let _ = tokio::fs::remove_dir(month).await;
    }
}

/// Walk one tree, keeping only the files this module would have written.
///
/// A tree that is not there is empty rather than an error. Nothing creates these
/// directories until something is written to them, so on a wiki that has never
/// captured a thought this is the ordinary case, and warning about it would be
/// noise on every scan.
fn walk<Id>(root: &Path, recover: fn(&Path) -> Option<Id>, what: &str) -> Vec<IdeaWalkEntry<Id>> {
    if !root.is_dir() {
        return Vec::new();
    }

    let mut entries = Vec::new();

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            // The `.<name>.md.tmp` files an atomic write goes through.
            entry.depth() == 0
                || !entry
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
        });

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(%error, %what, "skipping unreadable entry while walking");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(id) = recover(relative) else {
            continue;
        };

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(%error, %what, path = %entry.path().display(), "skipping unreadable record");
                continue;
            }
        };
        let Ok(updated) = modified_at(&metadata) else {
            continue;
        };

        entries.push(IdeaWalkEntry {
            id,
            updated,
            size: metadata.len(),
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::slug::Slug;
    use crate::users::Username;

    async fn store() -> (TempDir, IdeaStore) {
        let directory = TempDir::new().expect("temp dir");
        let store = IdeaStore::open(directory.path()).await.expect("open");
        (directory, store)
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn tim() -> Owner {
        Owner::of(Username::parse("tim").expect("valid username"))
    }

    fn capture_draft(created: &str, body: &str) -> CaptureDraft {
        CaptureDraft {
            created: at(created),
            owner: tim(),
            body: body.to_owned(),
        }
    }

    fn idea_draft(created: &str, name: &str, seeds: Vec<CaptureId>) -> IdeaDraft {
        IdeaDraft {
            name: name.to_owned(),
            created: at(created),
            owner: tim(),
            seeds,
            note: String::new(),
        }
    }

    #[tokio::test]
    async fn writes_and_reads_a_capture() {
        let (_directory, store) = store().await;

        let written = store
            .create_capture(capture_draft(
                "2026-08-20T14:15:30Z",
                "Dungeon quests should require seeds.\n",
            ))
            .await
            .expect("create");

        let read = store.read_capture(&written.id).await.expect("read");
        assert_eq!(read.body, "Dungeon quests should require seeds.\n");
        assert_eq!(read.owner, tim());
        assert_eq!(read.created, at("2026-08-20T14:15:30Z"));
        assert_eq!(read.size, written.size);
    }

    #[tokio::test]
    async fn captures_and_events_are_grouped_by_month_and_threads_are_flat() {
        let (_directory, store) = store().await;

        let capture = store
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .expect("create capture");
        let idea = store
            .create_idea(idea_draft(
                "2026-08-20T14:20:00Z",
                "Dungeon seeds",
                vec![capture.id.clone()],
            ))
            .await
            .expect("create idea");
        let event = store
            .append_event(
                EventDraft::new(
                    EventKind::InterestAffirmed,
                    Subject::Idea {
                        idea: idea.id.clone(),
                    },
                    at("2026-08-20T14:20:30Z"),
                    tim(),
                )
                .expect("draft"),
            )
            .await
            .expect("append");

        assert!(
            store
                .captures_root()
                .join("2026-08")
                .join(format!("{}.md", capture.id))
                .is_file()
        );
        assert!(
            store
                .threads_root()
                .join(format!("{}.md", idea.id))
                .is_file()
        );
        assert!(
            store
                .events_root()
                .join("2026-08")
                .join(format!("{}.md", event.id))
                .is_file()
        );
    }

    /// Two records written for the same whole second are two records.
    #[tokio::test]
    async fn records_at_the_same_instant_get_distinct_ids() {
        let (_directory, store) = store().await;

        let first = store
            .create_capture(capture_draft("2026-08-20T09:00:00Z", "First.\n"))
            .await
            .expect("first");
        let second = store
            .create_capture(capture_draft("2026-08-20T09:00:00Z", "Second.\n"))
            .await
            .expect("second");

        assert_ne!(first.id, second.id);
        assert_eq!(
            store.read_capture(&first.id).await.unwrap().body,
            "First.\n"
        );
        assert_eq!(
            store.read_capture(&second.id).await.unwrap().body,
            "Second.\n"
        );
    }

    #[tokio::test]
    async fn reading_a_missing_record_reports_not_found() {
        let (_directory, store) = store().await;

        let error = store
            .read_capture(&CaptureId::parse("20260820T141530-123456789").unwrap())
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                IdeaStoreError::NotFound {
                    kind: RecordKind::Capture,
                    ..
                }
            ),
            "{error}"
        );

        let error = store
            .read_idea(&IdeaId::parse("20260820T142000-234567890").unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().starts_with("no idea"), "{error}");
    }

    /// `created` and `owner` are the two fields the API may not change, and
    /// carrying them over is what makes that true rather than merely stated.
    #[tokio::test]
    async fn patching_a_capture_changes_only_its_text() {
        let (_directory, store) = store().await;
        let written = store
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "First draft.\n"))
            .await
            .expect("create");

        let patched = store
            .patch_capture(&written.id, "Second draft.\n")
            .await
            .expect("patch");

        assert_eq!(patched.body, "Second draft.\n");
        assert_eq!(patched.created, written.created);
        assert_eq!(patched.owner, written.owner);
        assert_eq!(patched.id, written.id);
    }

    #[tokio::test]
    async fn patching_an_idea_leaves_its_seeds_and_origin_alone() {
        let (_directory, store) = store().await;
        let capture = store
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .expect("create capture");
        let written = store
            .create_idea(IdeaDraft {
                note: "Notes.\n".to_owned(),
                ..idea_draft(
                    "2026-08-20T14:20:00Z",
                    "Dungeon seeds",
                    vec![capture.id.clone()],
                )
            })
            .await
            .expect("create idea");

        let renamed = store
            .patch_idea(&written.id, Some("Seeded dungeons"), None)
            .await
            .expect("patch");

        assert_eq!(renamed.name, "Seeded dungeons");
        assert_eq!(renamed.note, "Notes.\n");
        assert_eq!(renamed.seeds, [capture.id]);
        assert_eq!(renamed.created, written.created);
    }

    #[tokio::test]
    async fn deleting_a_capture_prunes_the_month_it_emptied() {
        let (_directory, store) = store().await;
        let written = store
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .expect("create");
        let month = store.captures_root().join("2026-08");

        store.delete_capture(&written.id).await.expect("delete");

        assert!(!month.exists(), "empty month directory not pruned");
        assert!(
            store.captures_root().exists(),
            "the captures root must survive pruning"
        );
        assert!(matches!(
            store.delete_capture(&written.id).await.unwrap_err(),
            IdeaStoreError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn an_event_round_trips_through_its_file() {
        let (_directory, store) = store().await;
        let idea = IdeaId::parse("20260820T142000-234567890").unwrap();
        let capture = CaptureId::parse("20260820T141530-123456789").unwrap();

        let written = store
            .append_event(
                EventDraft::new(
                    EventKind::CaptureConnected,
                    Subject::IdeaCapture {
                        idea: idea.clone(),
                        capture: capture.clone(),
                    },
                    at("2026-08-20T14:20:30Z"),
                    tim(),
                )
                .expect("draft"),
            )
            .await
            .expect("append");

        let read = store.read_event(&written.id).await.expect("read");
        assert_eq!(read, written);
        assert_eq!(read.kind, EventKind::CaptureConnected);
        assert_eq!(read.subject, Subject::IdeaCapture { idea, capture });
        assert_eq!(read.actor, tim());
    }

    /// An id is when the decision happened, and an appended event's `created`
    /// has to be exactly that or the file it wrote would not read back.
    #[tokio::test]
    async fn an_appended_events_created_is_the_instant_its_id_names() {
        let (_directory, store) = store().await;

        let first = store
            .append_event(
                EventDraft::new(
                    EventKind::IdeaRetired,
                    Subject::Idea {
                        idea: IdeaId::parse("20260820T142000-234567890").unwrap(),
                    },
                    at("2026-08-20T09:00:00Z"),
                    Owner::open(),
                )
                .expect("draft"),
            )
            .await
            .expect("first");
        let second = store
            .append_event(
                EventDraft::new(
                    EventKind::IdeaReopened,
                    Subject::Idea {
                        idea: IdeaId::parse("20260820T142000-234567890").unwrap(),
                    },
                    at("2026-08-20T09:00:00Z"),
                    Owner::open(),
                )
                .expect("draft"),
            )
            .await
            .expect("second");

        // The nudge moved the second id, so its `created` has to move with it.
        assert_ne!(first.id, second.id);
        assert_eq!(first.created, first.id.instant());
        assert_eq!(second.created, second.id.instant());
        assert_eq!(store.read_event(&second.id).await.expect("read"), second);
    }

    /// A draft is checked when it is built, so a store write cannot be the
    /// thing that produces a file the next startup refuses.
    #[test]
    fn a_draft_that_does_not_match_its_kind_cannot_be_built() {
        let error = EventDraft::new(
            EventKind::IdeaPromoted,
            Subject::Idea {
                idea: IdeaId::parse("20260820T142000-234567890").unwrap(),
            },
            at("2026-08-20T14:20:30Z"),
            Owner::open(),
        )
        .expect_err("refused");

        assert!(
            matches!(error, RecordError::MissingReference { field: "page", .. }),
            "{error}"
        );
    }

    #[test]
    fn a_draft_puts_a_capture_pair_in_canonical_order() {
        let first = CaptureId::parse("20260820T141530-123456789").unwrap();
        let second = CaptureId::parse("20260820T142000-000000000").unwrap();

        let draft = EventDraft::new(
            EventKind::CandidateRejected,
            Subject::CapturePair {
                capture: second.clone(),
                other: first.clone(),
            },
            at("2026-08-20T14:20:30Z"),
            Owner::open(),
        )
        .expect("draft");

        assert_eq!(
            draft.subject(),
            &Subject::CapturePair {
                capture: first,
                other: second
            }
        );
    }

    #[tokio::test]
    async fn walking_finds_every_record_and_skips_strays() {
        let (_directory, store) = store().await;

        for created in [
            "2026-07-30T09:00:00Z",
            "2026-08-20T14:15:30Z",
            "2026-08-21T14:15:30Z",
        ] {
            store
                .create_capture(capture_draft(created, "A thought.\n"))
                .await
                .expect("create capture");
        }
        store
            .create_idea(idea_draft(
                "2026-08-20T14:20:00Z",
                "Dungeon seeds",
                vec![CaptureId::parse("20260820T141530-123456789").unwrap()],
            ))
            .await
            .expect("create idea");

        tokio::fs::write(store.captures_root().join("2026-08").join("notes.md"), "no")
            .await
            .expect("stray capture");
        tokio::fs::write(store.threads_root().join("notes.txt"), "no")
            .await
            .expect("stray thread");

        let mut captures: Vec<String> = store
            .walk_captures()
            .into_iter()
            .map(|entry| entry.id.to_string())
            .collect();
        captures.sort();

        assert_eq!(captures.len(), 3, "got {captures:?}");
        assert!(captures[0].starts_with("20260730T090000"));
        assert_eq!(store.walk_ideas().len(), 1);
        assert_eq!(store.walk_events().len(), 0);
    }

    /// The temporary file a write goes through must never be walked.
    #[tokio::test]
    async fn atomic_writes_leave_nothing_visible_behind() {
        let (_directory, store) = store().await;
        store
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .expect("create");

        let names: Vec<String> = std::fs::read_dir(store.captures_root().join("2026-08"))
            .expect("read dir")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names.len(), 1, "got {names:?}");
    }

    #[tokio::test]
    async fn a_malformed_record_is_reported_against_its_id() {
        let (_directory, store) = store().await;
        let id = IdeaId::parse("20260820T142000-234567890").unwrap();

        tokio::fs::create_dir_all(store.threads_root())
            .await
            .expect("threads root");
        tokio::fs::write(
            id.to_path(store.threads_root()),
            "---\nname: No seeds\n---\n",
        )
        .await
        .expect("write by hand");

        let error = store.read_idea(&id).await.unwrap_err();
        assert!(
            matches!(
                error,
                IdeaStoreError::Malformed {
                    kind: RecordKind::Idea,
                    ..
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains(id.as_str()), "{error}");
    }

    /// Files are the truth here as much as anywhere else: a capture dropped in
    /// by hand is a capture, with nothing to convert and no server to restart.
    #[tokio::test]
    async fn a_record_written_by_hand_is_read_back() {
        let (_directory, store) = store().await;
        let id = CaptureId::parse("20260820T141530-123456789").unwrap();

        tokio::fs::create_dir_all(store.captures_root().join("2026-08"))
            .await
            .expect("month");
        tokio::fs::write(id.to_path(store.captures_root()), "A thought, typed in.\n")
            .await
            .expect("write by hand");

        let read = store.read_capture(&id).await.expect("read");
        assert_eq!(read.body, "A thought, typed in.\n");
        assert!(read.owner.is_open());
        assert_eq!(store.walk_captures().len(), 1);
    }

    /// The gate on phase I0: everything written through the drafts is still
    /// there, and still says the same thing, after the store is opened again.
    #[tokio::test]
    async fn a_reopened_store_reads_back_what_the_drafts_wrote() {
        let directory = TempDir::new().expect("temp dir");

        let (capture, idea, event) = {
            let store = IdeaStore::open(directory.path()).await.expect("open");

            let capture = store
                .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
                .await
                .expect("create capture");
            let idea = store
                .create_idea(IdeaDraft {
                    note: "Notes.\n".to_owned(),
                    ..idea_draft(
                        "2026-08-20T14:20:00Z",
                        "Dungeon seeds",
                        vec![capture.id.clone()],
                    )
                })
                .await
                .expect("create idea");
            let event = store
                .append_event(
                    EventDraft::new(
                        EventKind::IdeaPromoted,
                        Subject::Promotion {
                            idea: idea.id.clone(),
                            page: Slug::parse("notes/dungeon-seeds").expect("valid slug"),
                        },
                        at("2026-08-20T14:20:30Z"),
                        tim(),
                    )
                    .expect("draft"),
                )
                .await
                .expect("append");

            (capture, idea, event)
        };

        let reopened = IdeaStore::open(directory.path()).await.expect("reopen");

        assert_eq!(reopened.read_capture(&capture.id).await.unwrap(), capture);
        assert_eq!(reopened.read_idea(&idea.id).await.unwrap(), idea);
        assert_eq!(reopened.read_event(&event.id).await.unwrap(), event);
    }

    #[tokio::test]
    async fn the_displayed_root_is_free_of_verbatim_prefixes() {
        let (_directory, store) = store().await;
        assert!(!store.root_display().starts_with(r"\\?\"));
    }

    /// Opening the store writes nothing, and walking a tree that is not there is
    /// empty rather than an error.
    ///
    /// The server opens its stores and then watches the wiki directory, so a
    /// store that created directories on open would be the server writing into
    /// the tree it is about to watch. That cost a spurious full rescan in the
    /// first debounce window, and a rescan racing a pending create event leaves
    /// an index row for a file that has already been deleted. See
    /// [`IdeaStore::open`].
    #[tokio::test]
    async fn opening_creates_nothing() {
        let (directory, store) = store().await;

        assert!(
            !directory.path().join(INTERNAL_DIR).join(IDEAS_DIR).exists(),
            "opening the store wrote into the wiki"
        );
        assert!(store.walk_captures().is_empty());
        assert!(store.walk_ideas().is_empty());
        assert!(store.walk_events().is_empty());

        // And the first write brings the tree it needs into being.
        store
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .expect("create");
        assert!(store.captures_root().is_dir());
        assert!(!store.threads_root().exists());
    }
}
