//! Reconciling the index with what is actually on disk.
//!
//! This runs at startup and behind `POST /api/reindex`, and it is the seam
//! where "files are the source of truth" is actually enforced. Everything the
//! index believes is checked against the wiki directory and corrected.
//!
//! There are five trees to check and they are all checked the same way: the
//! pages, the time log under `.rhizolog/times/`, and Idea Inbox's captures,
//! threads and decision events under `.rhizolog/ideas/`. Every one of them is
//! files, every one is authoritative, and every one is compared by mtime and
//! size against what was recorded. That misses an edit that preserves both,
//! which needs a tool that restores mtime and a replacement of exactly equal
//! length. That is rare enough that paying for a hash of every file on every
//! startup is the worse trade, and [`rebuild`] fixes it when it does happen.
//!
//! Idea Inbox's three trees are counted separately rather than added together.
//! A scan that says "412 idea files" when a thread has gone missing is a scan
//! nobody can act on, and the three are read by different code with different
//! ways of being malformed.

use chrono::Utc;

use crate::ideas::IdeaStore;
use crate::index::{Index, IndexError};
use crate::store::Store;
use crate::times::TimeStore;

/// What one scan of one tree found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    /// Files found on disk.
    pub scanned: usize,
    /// Files read and written to the index because they were new or changed.
    pub indexed: usize,
    /// Files already indexed with a matching mtime and size.
    pub unchanged: usize,
    /// Rows dropped from the index because the file is no longer on disk.
    pub removed: usize,
    /// Files on disk that could not be read, and so are not in the index.
    pub failed: usize,
}

impl SyncCounts {
    pub fn changed_anything(&self) -> bool {
        self.indexed > 0 || self.removed > 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub pages: SyncCounts,
    pub times: SyncCounts,
    pub captures: SyncCounts,
    pub ideas: SyncCounts,
    pub events: SyncCounts,
}

impl SyncReport {
    pub fn changed_anything(&self) -> bool {
        self.every_tree().iter().any(SyncCounts::changed_anything)
    }

    /// Every tree's counts, for a caller that wants to total something across
    /// all of them without naming five fields and forgetting the sixth.
    pub fn every_tree(&self) -> [SyncCounts; 5] {
        [
            self.pages,
            self.times,
            self.captures,
            self.ideas,
            self.events,
        ]
    }

    /// Files that could not be read, across every tree.
    pub fn failed(&self) -> usize {
        self.every_tree().iter().map(|counts| counts.failed).sum()
    }

    /// Rows dropped because their file is gone, across every tree.
    pub fn removed(&self) -> usize {
        self.every_tree().iter().map(|counts| counts.removed).sum()
    }
}

/// Bring the index in line with the wiki directory, the time log and Idea Inbox.
pub async fn sync(
    store: &Store,
    times: &TimeStore,
    ideas: &IdeaStore,
    index: &Index,
) -> Result<SyncReport, IndexError> {
    // Captures, then threads, then events, which is the order that folds each
    // idea the fewest times. Correctness does not depend on it: every write
    // recomputes what it bears on, so an event read before the thread it names
    // folds again when that thread arrives. See [`crate::index::ideas`].
    let report = SyncReport {
        pages: sync_pages(store, index).await?,
        times: sync_times(times, index).await?,
        captures: sync_captures(ideas, index).await?,
        ideas: sync_idea_threads(ideas, index).await?,
        events: sync_idea_events(ideas, index).await?,
    };

    index.set_last_sync(Utc::now()).await?;
    Ok(report)
}

async fn sync_pages(store: &Store, index: &Index) -> Result<SyncCounts, IndexError> {
    let walker = store.clone();
    let entries = tokio::task::spawn_blocking(move || walker.walk())
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?;

    // Entries are removed as they are accounted for; whatever remains at the
    // end is indexed but no longer on disk.
    let mut stale = index.stamps().await?;

    let mut counts = SyncCounts {
        scanned: entries.len(),
        ..SyncCounts::default()
    };

    for entry in entries {
        let previous = stale.remove(&entry.slug);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            counts.unchanged += 1;
            continue;
        }

        match store.read(&entry.slug).await {
            Ok(page) => {
                index.upsert(&page).await?;
                counts.indexed += 1;
            }
            Err(error) => {
                // Drop it rather than leaving a stale row behind: a page that
                // cannot be read cannot be served, and a search hit that 404s
                // is worse than no hit at all. Its stamp is gone too, so a
                // transient failure simply reindexes on the next scan.
                tracing::warn!(slug = %entry.slug, %error, "could not index page");
                index.remove(&entry.slug).await?;
                counts.failed += 1;
            }
        }
    }

    for slug in stale.keys() {
        index.remove(slug).await?;
        counts.removed += 1;
    }

    Ok(counts)
}

async fn sync_times(times: &TimeStore, index: &Index) -> Result<SyncCounts, IndexError> {
    let walker = times.clone();
    let entries = tokio::task::spawn_blocking(move || walker.walk())
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?;

    let mut stale = index.time_stamps().await?;

    let mut counts = SyncCounts {
        scanned: entries.len(),
        ..SyncCounts::default()
    };

    for entry in entries {
        let previous = stale.remove(&entry.id);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            counts.unchanged += 1;
            continue;
        }

        match times.read(&entry.id).await {
            Ok(recorded) => {
                index.upsert_time(&recorded).await?;
                counts.indexed += 1;
            }
            Err(error) => {
                tracing::warn!(id = %entry.id, %error, "could not index time entry");
                index.remove_time(&entry.id).await?;
                counts.failed += 1;
            }
        }
    }

    for id in stale.keys() {
        index.remove_time(id).await?;
        counts.removed += 1;
    }

    Ok(counts)
}

async fn sync_captures(ideas: &IdeaStore, index: &Index) -> Result<SyncCounts, IndexError> {
    let walker = ideas.clone();
    let entries = tokio::task::spawn_blocking(move || walker.walk_captures())
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?;

    let mut stale = index.capture_stamps().await?;

    let mut counts = SyncCounts {
        scanned: entries.len(),
        ..SyncCounts::default()
    };

    for entry in entries {
        let previous = stale.remove(&entry.id);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            counts.unchanged += 1;
            continue;
        }

        match ideas.read_capture(&entry.id).await {
            Ok(capture) => {
                index.upsert_capture(&capture).await?;
                counts.indexed += 1;
            }
            Err(error) => {
                tracing::warn!(id = %entry.id, %error, "could not index capture");
                index.remove_capture(&entry.id).await?;
                counts.failed += 1;
            }
        }
    }

    for id in stale.keys() {
        index.remove_capture(id).await?;
        counts.removed += 1;
    }

    Ok(counts)
}

async fn sync_idea_threads(ideas: &IdeaStore, index: &Index) -> Result<SyncCounts, IndexError> {
    let walker = ideas.clone();
    let entries = tokio::task::spawn_blocking(move || walker.walk_ideas())
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?;

    let mut stale = index.idea_stamps().await?;

    let mut counts = SyncCounts {
        scanned: entries.len(),
        ..SyncCounts::default()
    };

    for entry in entries {
        let previous = stale.remove(&entry.id);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            counts.unchanged += 1;
            continue;
        }

        match ideas.read_idea(&entry.id).await {
            Ok(idea) => {
                index.upsert_idea(&idea).await?;
                counts.indexed += 1;
            }
            Err(error) => {
                tracing::warn!(id = %entry.id, %error, "could not index idea thread");
                index.remove_idea(&entry.id).await?;
                counts.failed += 1;
            }
        }
    }

    for id in stale.keys() {
        index.remove_idea(id).await?;
        counts.removed += 1;
    }

    Ok(counts)
}

async fn sync_idea_events(ideas: &IdeaStore, index: &Index) -> Result<SyncCounts, IndexError> {
    let walker = ideas.clone();
    let entries = tokio::task::spawn_blocking(move || walker.walk_events())
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?;

    let mut stale = index.idea_event_stamps().await?;

    let mut counts = SyncCounts {
        scanned: entries.len(),
        ..SyncCounts::default()
    };

    for entry in entries {
        let previous = stale.remove(&entry.id);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            counts.unchanged += 1;
            continue;
        }

        match ideas.read_event(&entry.id).await {
            Ok(event) => {
                index.upsert_idea_event(&event).await?;
                counts.indexed += 1;
            }
            Err(error) => {
                // A malformed event is dropped rather than left behind, exactly
                // as a malformed page is. It is a decision nobody can read, and
                // folding half of one would put an idea in a state its own
                // history does not support.
                tracing::warn!(id = %entry.id, %error, "could not index decision event");
                index.remove_idea_event(&entry.id).await?;
                counts.failed += 1;
            }
        }
    }

    for id in stale.keys() {
        index.remove_idea_event(id).await?;
        counts.removed += 1;
    }

    Ok(counts)
}

/// Throw the index away and build it again from disk.
///
/// The escape hatch for anything incremental syncing can miss. It is cheap
/// precisely because the index holds nothing that is not already in a file.
pub async fn rebuild(
    store: &Store,
    times: &TimeStore,
    ideas: &IdeaStore,
    index: &Index,
) -> Result<SyncReport, IndexError> {
    index.clear().await?;
    sync(store, times, ideas, index).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{DateTime, Utc};
    use tempfile::TempDir;

    use crate::ideas::{
        CaptureDraft, CaptureId, EventDraft, EventKind, IdeaDraft, IdeaId, Owner, Subject,
    };
    use crate::index::Audience;
    use crate::index::{CaptureRecord, IdeaState, Stamp, TimeListOptions};
    use crate::page::Frontmatter;
    /// Every test in this file runs against a wiki with no accounts, where
    /// there is nobody to keep a page from and visibility does not apply.
    /// What happens when it does is `tests/visibility.rs`, which is a whole
    /// file rather than a case here for exactly that reason.
    const EVERYONE: Audience = Audience::Everything;
    use crate::slug::Slug;
    use crate::times::store::TimeDraft;
    use std::collections::HashMap;

    async fn fixture() -> (TempDir, Store, TimeStore, IdeaStore, Index) {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let times = TimeStore::open(directory.path())
            .await
            .expect("open time log");
        let ideas = IdeaStore::open(directory.path())
            .await
            .expect("open idea inbox");
        let index = Index::open(None).await.expect("open index");
        (directory, store, times, ideas, index)
    }

    fn slug(raw: &str) -> Slug {
        Slug::parse(raw).expect("valid slug")
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    async fn write(store: &Store, raw: &str, body: &str) {
        store
            .write(&slug(raw), Frontmatter::default(), body)
            .await
            .expect("write page");
    }

    fn draft(name: &str, start: &str) -> TimeDraft {
        TimeDraft {
            name: name.to_owned(),
            start: at(start),
            end: None,
            pages: Vec::new(),
            note: String::new(),
        }
    }

    /// Everything the index knows, in a form two indexes can be compared by.
    async fn snapshot(index: &Index) -> (Vec<(String, String)>, HashMap<Slug, Stamp>) {
        let mut hits: Vec<(String, String)> = index
            .search("page", 100, 0, &EVERYONE)
            .await
            .expect("search")
            .hits
            .into_iter()
            .map(|hit| (hit.slug.to_string(), hit.title))
            .collect();
        hits.sort();

        (hits, index.stamps().await.expect("stamps"))
    }

    #[tokio::test]
    async fn indexes_everything_on_a_first_scan() {
        let (_directory, store, times, ideas, index) = fixture().await;
        for n in 0..3 {
            write(&store, &format!("page-{n}"), "A page about rhizomes.\n").await;
        }

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 3);
        assert_eq!(report.pages.indexed, 3);
        assert_eq!(report.pages.unchanged, 0);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 3);
        assert_eq!(
            index
                .search("rhizomes", 10, 0, &EVERYONE)
                .await
                .unwrap()
                .total,
            3
        );
    }

    #[tokio::test]
    async fn a_second_scan_reindexes_nothing() {
        let (_directory, store, times, ideas, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &ideas, &index).await.unwrap();

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.unchanged, 1);
        assert_eq!(report.times.unchanged, 1);
        assert_eq!(report.pages.indexed, 0);
        assert_eq!(report.times.indexed, 0);
        assert!(!report.changed_anything());
    }

    #[tokio::test]
    async fn picks_up_a_page_edited_outside_the_api() {
        let (directory, store, times, ideas, index) = fixture().await;
        write(&store, "page-0", "Original body.\n").await;
        sync(&store, &times, &ideas, &index).await.unwrap();

        // Edited behind the server's back, as an external editor would.
        tokio::fs::write(
            directory.path().join("page-0.md"),
            "Replaced body about rhizomes.\n",
        )
        .await
        .expect("external edit");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.indexed, 1);
        assert_eq!(
            index
                .search("rhizomes", 10, 0, &EVERYONE)
                .await
                .unwrap()
                .total,
            1
        );
        assert_eq!(
            index
                .search("Original", 10, 0, &EVERYONE)
                .await
                .unwrap()
                .total,
            0
        );
    }

    #[tokio::test]
    async fn drops_a_page_deleted_outside_the_api() {
        let (directory, store, times, ideas, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        write(&store, "page-1", "Body.\n").await;
        sync(&store, &times, &ideas, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("external delete");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.removed, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 1);
        assert!(!index.stamps().await.unwrap().contains_key(&slug("page-0")));
    }

    /// The time log is files too, so hand-editing it has to work the same way.
    #[tokio::test]
    async fn picks_up_a_time_entry_written_by_hand() {
        let (_directory, store, times, ideas, index) = fixture().await;
        let month = times.root().join("2026-08");
        tokio::fs::create_dir_all(&month).await.expect("month");
        tokio::fs::write(
            month.join("20260806T090000-000000000.md"),
            "---\nname: Deep work\nstart: 2026-08-06T09:00:00Z\nend: 2026-08-06T11:00:00Z\n---\n",
        )
        .await
        .expect("hand-written entry");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.times.scanned, 1);
        assert_eq!(report.times.indexed, 1);
        let groups = index.time_groups(at("2026-08-06T20:00:00Z")).await.unwrap();
        assert_eq!(groups[0].name, "Deep work");
        assert_eq!(groups[0].seconds, 2 * 3600);
    }

    #[tokio::test]
    async fn drops_a_time_entry_deleted_outside_the_api() {
        let (_directory, store, times, ideas, index) = fixture().await;
        let written = times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &ideas, &index).await.unwrap();
        assert_eq!(index.count_times().await.unwrap(), 1);

        tokio::fs::remove_file(written.id.to_path(times.root()))
            .await
            .expect("external delete");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.times.removed, 1);
        assert_eq!(index.count_times().await.unwrap(), 0);
    }

    /// One unreadable file must not stop the wiki from indexing.
    #[tokio::test]
    async fn a_malformed_page_is_skipped_not_fatal() {
        let (directory, store, times, ideas, index) = fixture().await;
        write(&store, "good", "A page about rhizomes.\n").await;
        tokio::fs::write(
            directory.path().join("broken.md"),
            "---\ntitle: never closed\n\nBody.\n",
        )
        .await
        .expect("write broken page");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 2);
        assert_eq!(report.pages.indexed, 1);
        assert_eq!(report.pages.failed, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 1);
        assert_eq!(
            index
                .search("rhizomes", 10, 0, &EVERYONE)
                .await
                .unwrap()
                .total,
            1
        );
    }

    #[tokio::test]
    async fn a_malformed_time_entry_is_skipped_not_fatal() {
        let (_directory, store, times, ideas, index) = fixture().await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        let month = times.root().join("2026-08");
        tokio::fs::write(
            month.join("20260806T100000-000000000.md"),
            "---\nname: no start at all\n---\n",
        )
        .await
        .expect("write broken entry");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.times.scanned, 2);
        assert_eq!(report.times.indexed, 1);
        assert_eq!(report.times.failed, 1);
        assert_eq!(index.count_times().await.unwrap(), 1);
    }

    /// A page that becomes malformed leaves the index rather than lingering as
    /// a search hit that cannot be fetched.
    #[tokio::test]
    async fn a_page_that_breaks_is_dropped_from_the_index() {
        let (directory, store, times, ideas, index) = fixture().await;
        write(&store, "page-0", "A page about rhizomes.\n").await;
        sync(&store, &times, &ideas, &index).await.unwrap();
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 1);

        tokio::fs::write(
            directory.path().join("page-0.md"),
            "---\ntitle: never closed\n\nrhizomes\n",
        )
        .await
        .expect("break the page");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.failed, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
        assert_eq!(
            index
                .search("rhizomes", 10, 0, &EVERYONE)
                .await
                .unwrap()
                .total,
            0
        );
    }

    /// The invariant the whole storage design rests on: whatever incremental
    /// syncing produces must be what a from-scratch rebuild produces.
    #[tokio::test]
    async fn incremental_syncing_matches_a_full_rebuild() {
        let (directory, store, times, ideas, index) = fixture().await;

        // A history of edits, arriving through both the API and the filesystem.
        write(&store, "page-0", "The first page.\n").await;
        write(&store, "notes/page-1", "The second page.\n").await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &ideas, &index).await.unwrap();

        write(&store, "notes/deep/page-2", "The third page.\n").await;
        store.delete(&slug("page-0")).await.expect("delete");
        sync(&store, &times, &ideas, &index).await.unwrap();

        tokio::fs::write(
            directory.path().join("notes/page-1.md"),
            "---\ntitle: Edited\ntags: [theory]\n---\n\nThe second page, rewritten.\n",
        )
        .await
        .expect("external edit");
        let incremental = sync(&store, &times, &ideas, &index).await.unwrap();
        assert!(incremental.changed_anything());

        let after_incremental = snapshot(&index).await;
        let times_after_incremental = index.time_stamps().await.unwrap();

        let report = rebuild(&store, &times, &ideas, &index).await.unwrap();
        let after_rebuild = snapshot(&index).await;

        assert_eq!(report.pages.indexed, 2, "rebuild should reindex every page");
        assert_eq!(report.pages.unchanged, 0);
        assert_eq!(report.times.indexed, 1);
        assert_eq!(
            after_incremental, after_rebuild,
            "incremental sync diverged from a full rebuild"
        );
        assert_eq!(
            times_after_incremental,
            index.time_stamps().await.unwrap(),
            "the time log diverged from a full rebuild"
        );
    }

    #[tokio::test]
    async fn rebuilding_clears_rows_for_files_that_are_gone() {
        let (directory, store, times, ideas, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        let entry = times
            .create(TimeDraft {
                note: "Chased the poll loop.\n".to_owned(),
                ..draft("Deep work", "2026-08-06T09:00:00Z")
            })
            .await
            .unwrap();
        sync(&store, &times, &ideas, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("remove page");
        tokio::fs::remove_file(entry.id.to_path(times.root()))
            .await
            .expect("remove entry");
        rebuild(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
        assert_eq!(index.count_times().await.unwrap(), 0);

        // A file that is gone is walked by nothing, so only `Index::clear`
        // can take its searchable text with it. The full-text tables are the
        // easy ones to forget there — they hold their own copy of the row.
        assert_eq!(
            index.search("body", 10, 0, &EVERYONE).await.unwrap().total,
            0
        );
        let searched = index
            .list_times(
                TimeListOptions {
                    query: Some("poll".to_owned()),
                    ..TimeListOptions::default()
                },
                Utc::now(),
                &EVERYONE,
            )
            .await
            .unwrap();
        assert_eq!(searched.total, 0, "the deleted entry is still searchable");
    }

    fn capture_draft(created: &str, body: &str) -> CaptureDraft {
        CaptureDraft {
            created: at(created),
            owner: Owner::open(),
            body: body.to_owned(),
        }
    }

    async fn record(ideas: &IdeaStore, kind: EventKind, subject: Subject, at_: &str) {
        ideas
            .append_event(EventDraft::new(kind, subject, at(at_), Owner::open()).expect("draft"))
            .await
            .expect("append event");
    }

    /// Everything the index believes about Idea Inbox, in a form two indexes can
    /// be compared by.
    #[allow(clippy::type_complexity)]
    async fn idea_snapshot(
        index: &Index,
    ) -> (
        Vec<IdeaState>,
        Vec<CaptureRecord>,
        Vec<(CaptureId, CaptureId)>,
    ) {
        let owner = Owner::open();

        let mut ideas: Vec<_> = index
            .idea_stamps()
            .await
            .expect("stamps")
            .into_keys()
            .collect();
        ideas.sort();
        let mut states = Vec::new();
        for id in ideas {
            states.push(index.idea_state(&owner, &id).await.expect("state"));
        }

        let mut captures: Vec<_> = index
            .capture_stamps()
            .await
            .expect("stamps")
            .into_keys()
            .collect();
        captures.sort();
        let mut records = Vec::new();
        for id in captures {
            records.push(index.capture(&owner, &id).await.expect("capture"));
        }

        (
            states.into_iter().flatten().collect(),
            records.into_iter().flatten().collect(),
            index
                .rejected_capture_pairs(&owner)
                .await
                .expect("rejected pairs"),
        )
    }

    /// Idea Inbox is three more trees of files, so a capture or a thread typed
    /// straight into an editor is as real as one made through the API.
    #[tokio::test]
    async fn picks_up_idea_files_written_by_hand() {
        let (_directory, store, times, ideas, index) = fixture().await;
        let capture = "20260820T141530-000000000";
        let thread = IdeaId::parse("20260820T142000-000000000").expect("valid idea id");

        let month = ideas.captures_root().join("2026-08");
        tokio::fs::create_dir_all(&month).await.expect("month");
        tokio::fs::write(
            month.join(format!("{capture}.md")),
            "---\ncreated: 2026-08-20T14:15:30Z\n---\n\nDungeon seeds.\n",
        )
        .await
        .expect("write capture");

        tokio::fs::create_dir_all(ideas.threads_root())
            .await
            .expect("threads root");
        tokio::fs::write(
            ideas.threads_root().join(format!("{thread}.md")),
            format!("---\nname: Dungeon seeds\ncaptures:\n- {capture}\n---\n"),
        )
        .await
        .expect("write thread");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.captures.scanned, 1);
        assert_eq!(report.captures.indexed, 1);
        assert_eq!(report.ideas.indexed, 1);
        assert_eq!(report.events.scanned, 0);

        let state = index
            .idea_state(&Owner::open(), &thread)
            .await
            .unwrap()
            .expect("indexed");
        assert_eq!(state.name, "Dungeon seeds");
        assert_eq!(state.members.len(), 1);
    }

    /// Idea Inbox lives inside `.rhizolog/`, which the page walker skips, so a
    /// capture must never also turn up as a page.
    #[tokio::test]
    async fn idea_files_are_not_pages() {
        let (_directory, store, times, ideas, index) = fixture().await;
        ideas
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .unwrap();

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 0);
        assert_eq!(report.captures.scanned, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
        assert_eq!(index.count_captures(&Owner::open()).await.unwrap(), 1);
    }

    /// One unreadable decision must not stop the rest of the inbox indexing,
    /// and it must not be folded half-way into a state its own history does not
    /// support.
    #[tokio::test]
    async fn a_malformed_idea_file_is_skipped_not_fatal() {
        let (_directory, store, times, ideas, index) = fixture().await;
        let capture = ideas
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .unwrap();
        ideas
            .create_idea(IdeaDraft {
                name: "Dungeon seeds".to_owned(),
                created: at("2026-08-20T14:20:00Z"),
                owner: Owner::open(),
                seeds: vec![capture.id.clone()],
                note: String::new(),
            })
            .await
            .unwrap();

        // An event naming a kind nobody has heard of, written by hand.
        let month = ideas.events_root().join("2026-08");
        tokio::fs::create_dir_all(&month).await.expect("month");
        tokio::fs::write(
            month.join("20260820T142100-000000000.md"),
            "---\nkind: idea_forgotten\nidea: 20260820T142000-000000000\n---\n",
        )
        .await
        .expect("write broken event");

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.events.scanned, 1);
        assert_eq!(report.events.indexed, 0);
        assert_eq!(report.events.failed, 1);
        assert_eq!(report.ideas.indexed, 1);
        assert_eq!(index.count_idea_events(&Owner::open()).await.unwrap(), 0);
    }

    /// The gate on phase I1, and the invariant the whole storage design rests
    /// on: whatever folding decisions incrementally produces has to be what
    /// deleting `index.db` and starting again produces.
    #[tokio::test]
    async fn a_rebuild_reproduces_the_folded_idea_state() {
        let (_directory, store, times, ideas, index) = fixture().await;

        let first = ideas
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "Dungeon seeds.\n"))
            .await
            .unwrap();
        let second = ideas
            .create_capture(capture_draft("2026-08-20T14:16:00Z", "Seeded loot.\n"))
            .await
            .unwrap();
        let third = ideas
            .create_capture(capture_draft("2026-08-20T14:17:00Z", "Unrelated.\n"))
            .await
            .unwrap();
        let thread = ideas
            .create_idea(IdeaDraft {
                name: "Dungeon seeds".to_owned(),
                created: at("2026-08-20T14:20:00Z"),
                owner: Owner::open(),
                seeds: vec![first.id.clone()],
                note: String::new(),
            })
            .await
            .unwrap();
        sync(&store, &times, &ideas, &index).await.unwrap();

        // Connect, reject, archive.
        record(
            &ideas,
            EventKind::CaptureConnected,
            Subject::IdeaCapture {
                idea: thread.id.clone(),
                capture: second.id.clone(),
            },
            "2026-08-20T14:21:00Z",
        )
        .await;
        record(
            &ideas,
            EventKind::CandidateRejected,
            Subject::IdeaCapture {
                idea: thread.id.clone(),
                capture: third.id.clone(),
            },
            "2026-08-20T14:22:00Z",
        )
        .await;
        record(
            &ideas,
            EventKind::CaptureArchived,
            Subject::Capture {
                capture: second.id.clone(),
            },
            "2026-08-20T14:23:00Z",
        )
        .await;
        sync(&store, &times, &ideas, &index).await.unwrap();

        // An inverse event, a pair rejection, and an edit to a capture's text.
        record(
            &ideas,
            EventKind::CaptureDisconnected,
            Subject::IdeaCapture {
                idea: thread.id.clone(),
                capture: second.id.clone(),
            },
            "2026-08-20T14:24:00Z",
        )
        .await;
        record(
            &ideas,
            EventKind::CandidateRejected,
            Subject::pair(second.id.clone(), third.id.clone()),
            "2026-08-20T14:25:00Z",
        )
        .await;
        ideas
            .patch_capture(&first.id, "Dungeon seeds, rewritten.\n")
            .await
            .unwrap();
        sync(&store, &times, &ideas, &index).await.unwrap();

        // Retire, reopen, promote, and lose a capture entirely.
        record(
            &ideas,
            EventKind::IdeaRetired,
            Subject::Idea {
                idea: thread.id.clone(),
            },
            "2026-08-20T14:26:00Z",
        )
        .await;
        record(
            &ideas,
            EventKind::IdeaReopened,
            Subject::Idea {
                idea: thread.id.clone(),
            },
            "2026-08-20T14:27:00Z",
        )
        .await;
        record(
            &ideas,
            EventKind::IdeaPromoted,
            Subject::Promotion {
                idea: thread.id.clone(),
                page: slug("notes/dungeon-seeds"),
            },
            "2026-08-20T14:28:00Z",
        )
        .await;
        ideas.delete_capture(&third.id).await.unwrap();
        let incremental = sync(&store, &times, &ideas, &index).await.unwrap();
        assert!(incremental.changed_anything());

        let after_incremental = idea_snapshot(&index).await;

        // The state is not merely self-consistent, it is the state the history
        // describes. Asserting this before the rebuild is what stops the
        // comparison below from passing on two identically wrong answers.
        let state = &after_incremental.0[0];
        assert_eq!(state.members, std::slice::from_ref(&first.id));
        assert!(!state.retired, "the reopen should have undone the retire");
        assert_eq!(
            state.promoted_to.as_ref().map(Slug::as_str),
            Some("notes/dungeon-seeds")
        );
        assert!(
            state.rejected.is_empty(),
            "the rejected capture was deleted, so it can no longer be suggested"
        );
        assert!(after_incremental.2.is_empty(), "so can the pair it was in");
        assert_eq!(after_incremental.1.len(), 2);
        assert_eq!(after_incremental.1[0].body, "Dungeon seeds, rewritten.\n");
        assert!(after_incremental.1[1].archived);

        let report = rebuild(&store, &times, &ideas, &index).await.unwrap();
        assert_eq!(report.captures.indexed, 2, "a rebuild reads every capture");
        assert_eq!(report.ideas.indexed, 1);
        assert_eq!(report.events.indexed, 8);

        assert_eq!(
            after_incremental,
            idea_snapshot(&index).await,
            "folding incrementally diverged from a full rebuild"
        );
    }

    #[tokio::test]
    async fn syncing_records_when_it_last_ran() {
        let (_directory, store, times, ideas, index) = fixture().await;
        assert_eq!(index.last_sync().await.unwrap(), None);

        sync(&store, &times, &ideas, &index).await.unwrap();

        assert!(index.last_sync().await.unwrap().is_some());
    }

    /// The time log lives inside `.rhizolog/`, which the page walker skips —
    /// so an entry must never also turn up as a page.
    #[tokio::test]
    async fn time_entries_are_not_pages() {
        let (_directory, store, times, ideas, index) = fixture().await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 0);
        assert_eq!(report.times.scanned, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_empty_wiki_syncs_cleanly() {
        let (_directory, store, times, ideas, index) = fixture().await;

        let report = sync(&store, &times, &ideas, &index).await.unwrap();

        assert_eq!(report, SyncReport::default());
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
        assert_eq!(index.count_times().await.unwrap(), 0);
    }
}
