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
use crate::words::{self, By, Held, WordLog};

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

/// What reading the word log found.
///
/// Deliberately **not** [`SyncCounts`], and the difference is the whole reason
/// this type exists. The five trees below are compared file by file, so
/// `indexed`, `unchanged` and `removed` each mean something about them. The word
/// log is read and replaced wholesale on every run, so all three would be zero
/// for reasons that say nothing, and a report shaped like the others would be
/// inviting somebody to read those zeroes as news.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WordSync {
    /// Observations folded into `page_words`.
    pub observations: usize,
    /// Lines that would not parse. Each costs itself and nothing else: a
    /// truncated last line after a hard power-off should not lose a year.
    pub skipped: usize,
    /// Whether the log could be read at all.
    ///
    /// False is a wiki being served with an empty series rather than a wiki
    /// refusing to start, which is the same stance a directory that cannot be
    /// watched gets.
    pub read: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub pages: SyncCounts,
    pub times: SyncCounts,
    pub captures: SyncCounts,
    pub ideas: SyncCounts,
    pub events: SyncCounts,
    /// The word log, which is read rather than reconciled. See [`WordSync`].
    pub words: WordSync,
}

impl SyncReport {
    /// Whether the wiki on disk turned out to differ from what was indexed.
    ///
    /// The word log is not consulted, and that is not an omission. It is
    /// replaced on every run, so it would answer "yes" every time a wiki had any
    /// history at all, and the question this is asked is whether anything
    /// *changed*.
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
    words: &WordLog,
    index: &Index,
) -> Result<SyncReport, IndexError> {
    // **Before the pages**, and the order is load-bearing rather than tidy. The
    // page scan asks the index what each page's last recorded total was, to tell
    // a first sighting from an edit. On a database that has just been deleted
    // the answer is nothing at all until the log has been read back in, and
    // every page in the wiki would be reported as written today.
    let read = sync_words(words, index).await?;

    // Captures, then threads, then events, which is the order that folds each
    // idea the fewest times. Correctness does not depend on it: every write
    // recomputes what it bears on, so an event read before the thread it names
    // folds again when that thread arrives. See [`crate::index::ideas`].
    let report = SyncReport {
        pages: sync_pages(store, words, index).await?,
        times: sync_times(times, index).await?,
        captures: sync_captures(ideas, index).await?,
        ideas: sync_idea_threads(ideas, index).await?,
        events: sync_idea_events(ideas, index).await?,
        words: read,
    };

    index.set_last_sync(Utc::now()).await?;
    Ok(report)
}

/// Read the word log back into `page_words`.
///
/// Wholesale, every time, rather than compared file by file like the five trees
/// below. The log is a few hundred kilobytes a year and a partial rebuild has
/// states a whole one cannot get into, so the cheap and obviously correct
/// operation is the right one. It is also what makes deleting the database cost
/// nothing but a read.
///
/// Not an error if the log is unreadable: a wiki whose word history cannot be
/// loaded should still be served, with the series empty, exactly as a wiki that
/// cannot be watched is still served. It is loud about it.
///
/// What comes back is what the log **held when it was read**, which is before
/// the page scan below has had any chance to append to it. That is the right
/// figure for a report about what reconciling found: three pages written while
/// the server was down are three lines the scan writes, not three lines it
/// discovered.
///
/// The log is held from the read to the end of the rebuild, since an
/// observation recorded in between would otherwise be dropped from the table or
/// folded into it twice; the lock on [`WordLog`] says how.
pub(crate) async fn sync_words(words: &WordLog, index: &Index) -> Result<WordSync, IndexError> {
    let mut held = words.hold().await;
    fold_words(&mut held, index).await
}

/// [`sync_words`], unless the log is exactly what this process last read or
/// wrote, in which case `None` and nothing is read.
///
/// For the watcher, which runs it before every batch of pages for the reason the
/// scan reads the log first: see `crate::watcher`, "The word log is read, not
/// watched". The API's own writes echo into the watcher, so without the check
/// every save would pay for reading the whole log.
pub(crate) async fn refresh_words(
    words: &WordLog,
    index: &Index,
) -> Result<Option<WordSync>, IndexError> {
    let mut held = words.hold().await;

    if held.unchanged().await {
        return Ok(None);
    }

    fold_words(&mut held, index).await.map(Some)
}

async fn fold_words(held: &mut Held<'_>, index: &Index) -> Result<WordSync, IndexError> {
    let (observations, skipped) = match held.read().await {
        Ok(read) => read,
        Err(error) => {
            tracing::error!(%error, "could not read the word log; the series will be empty");
            return Ok(WordSync::default());
        }
    };

    if skipped > 0 {
        tracing::warn!(skipped, "skipped word log lines that would not parse");
    }

    index.rebuild_words(&observations).await?;
    tracing::debug!(observations = observations.len(), "loaded the word log");

    Ok(WordSync {
        observations: observations.len(),
        skipped,
        read: true,
    })
}

async fn sync_pages(
    store: &Store,
    words: &WordLog,
    index: &Index,
) -> Result<SyncCounts, IndexError> {
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

    // One instant for the whole scan. A scan of twenty thousand pages that
    // stamped each observation with its own `now` would spread one restart
    // across several minutes of the chart.
    let now = Utc::now();

    for entry in entries {
        let previous = stale.remove(&entry.slug);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            counts.unchanged += 1;
            continue;
        }

        match store.read(&entry.slug).await {
            Ok(page) => {
                let change = index.upsert(&page).await?;
                // A page nobody touched produces nothing here, which is what
                // makes a rebuild free: the file's count and the log's last
                // total agree, so there is nothing to record. The same goes for
                // an index older than the log, whose body `weigh` does not
                // mistake for the one the log last saw.
                words::observe(words, index, &page.slug, &By::scan(), now, change).await;
                counts.indexed += 1;
            }
            Err(error) => {
                // Drop it rather than leaving a stale row behind: a page that
                // cannot be read cannot be served, and a search hit that 404s
                // is worse than no hit at all. Its stamp is gone too, so a
                // transient failure simply reindexes on the next scan.
                //
                // No word log entry: the file is still there and is still
                // somebody's writing. Only a page that has actually gone gets a
                // `deleted` marker, which is the loop below.
                tracing::warn!(slug = %entry.slug, %error, "could not index page");
                index.remove(&entry.slug).await?;
                counts.failed += 1;
            }
        }
    }

    for slug in stale.keys() {
        // Gone from disk while nothing was watching. The marker is what stops a
        // page later written at the same slug from continuing this one's series.
        index.remove(slug).await?;
        words::deleted(words, index, slug, &By::scan(), now).await;
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
    words: &WordLog,
    index: &Index,
) -> Result<SyncReport, IndexError> {
    index.clear().await?;
    sync(store, times, ideas, words, index).await
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

    async fn fixture() -> (TempDir, Store, TimeStore, IdeaStore, WordLog, Index) {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let times = TimeStore::open(directory.path())
            .await
            .expect("open time log");
        let ideas = IdeaStore::open(directory.path())
            .await
            .expect("open idea inbox");
        let words = WordLog::open(directory.path())
            .await
            .expect("open word log");
        let index = Index::open(None).await.expect("open index");
        (directory, store, times, ideas, words, index)
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
        let (_directory, store, times, ideas, words, index) = fixture().await;
        for n in 0..3 {
            write(&store, &format!("page-{n}"), "A page about rhizomes.\n").await;
        }

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let (_directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.pages.unchanged, 1);
        assert_eq!(report.times.unchanged, 1);
        assert_eq!(report.pages.indexed, 0);
        assert_eq!(report.times.indexed, 0);
        assert!(!report.changed_anything());
    }

    #[tokio::test]
    async fn picks_up_a_page_edited_outside_the_api() {
        let (directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "page-0", "Original body.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        // Edited behind the server's back, as an external editor would.
        tokio::fs::write(
            directory.path().join("page-0.md"),
            "Replaced body about rhizomes.\n",
        )
        .await
        .expect("external edit");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let (directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        write(&store, "page-1", "Body.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("external delete");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.pages.removed, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 1);
        assert!(!index.stamps().await.unwrap().contains_key(&slug("page-0")));
    }

    /// The time log is files too, so hand-editing it has to work the same way.
    #[tokio::test]
    async fn picks_up_a_time_entry_written_by_hand() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        let month = times.root().join("2026-08");
        tokio::fs::create_dir_all(&month).await.expect("month");
        tokio::fs::write(
            month.join("20260806T090000-000000000.md"),
            "---\nname: Deep work\nstart: 2026-08-06T09:00:00Z\nend: 2026-08-06T11:00:00Z\n---\n",
        )
        .await
        .expect("hand-written entry");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.times.scanned, 1);
        assert_eq!(report.times.indexed, 1);
        let groups = index.time_groups(at("2026-08-06T20:00:00Z")).await.unwrap();
        assert_eq!(groups[0].name, "Deep work");
        assert_eq!(groups[0].seconds, 2 * 3600);
    }

    #[tokio::test]
    async fn drops_a_time_entry_deleted_outside_the_api() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        let written = times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &ideas, &words, &index).await.unwrap();
        assert_eq!(index.count_times().await.unwrap(), 1);

        tokio::fs::remove_file(written.id.to_path(times.root()))
            .await
            .expect("external delete");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.times.removed, 1);
        assert_eq!(index.count_times().await.unwrap(), 0);
    }

    /// One unreadable file must not stop the wiki from indexing.
    #[tokio::test]
    async fn a_malformed_page_is_skipped_not_fatal() {
        let (directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "good", "A page about rhizomes.\n").await;
        tokio::fs::write(
            directory.path().join("broken.md"),
            "---\ntitle: never closed\n\nBody.\n",
        )
        .await
        .expect("write broken page");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let (_directory, store, times, ideas, words, index) = fixture().await;
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

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.times.scanned, 2);
        assert_eq!(report.times.indexed, 1);
        assert_eq!(report.times.failed, 1);
        assert_eq!(index.count_times().await.unwrap(), 1);
    }

    /// A page that becomes malformed leaves the index rather than lingering as
    /// a search hit that cannot be fetched.
    #[tokio::test]
    async fn a_page_that_breaks_is_dropped_from_the_index() {
        let (directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "page-0", "A page about rhizomes.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 1);

        tokio::fs::write(
            directory.path().join("page-0.md"),
            "---\ntitle: never closed\n\nrhizomes\n",
        )
        .await
        .expect("break the page");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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

    // -------------------------------------------------------------- word log

    /// A wiki that existed before the server did is not a wiki written today.
    #[tokio::test]
    async fn a_first_scan_baselines_rather_than_claiming_the_wiki_was_written() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "a", "One two three four.\n").await;
        write(&store, "b", "Five six.\n").await;

        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        let (log, skipped) = words.read().await.unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(log.len(), 2);

        for observation in &log {
            assert_eq!(observation.kind, crate::words::Kind::Baseline);
            assert_eq!(observation.actor, crate::words::ACTOR_SCAN);
            assert_eq!((observation.added, observation.removed), (0, 0));
        }
        assert_eq!(log[0].total, 4);
        assert_eq!(log[1].total, 2);
    }

    /// A second scan over a wiki nobody touched writes nothing at all.
    #[tokio::test]
    async fn a_scan_that_found_nothing_new_records_nothing() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "a", "One two three.\n").await;

        sync(&store, &times, &ideas, &words, &index).await.unwrap();
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(words.read().await.unwrap().0.len(), 1);
    }

    /// **The property that made this a log on disk rather than rows in the
    /// database.** Every document in this project tells the reader that deleting
    /// the index costs one scan, and a writing history is unreconstructable, so
    /// the two claims have to be able to coexist.
    #[tokio::test]
    async fn deleting_the_index_reproduces_the_whole_series() {
        let (_directory, store, times, ideas, words, index) = fixture().await;

        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();
        write(&store, "a", "One two three four five.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        let before = index.count_words_observed().await.unwrap();
        assert_eq!(before, 2, "a baseline and an edit");
        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(5));

        // Throw the whole database away, which is what the architecture says is
        // safe to do.
        let fresh = Index::open(None).await.expect("a new index");
        sync(&store, &times, &ideas, &words, &fresh).await.unwrap();

        assert_eq!(fresh.count_words_observed().await.unwrap(), before);
        assert_eq!(fresh.last_word_total(&slug("a")).await.unwrap(), Some(5));
        assert_eq!(
            words.read().await.unwrap().0.len(),
            before,
            "and the scan added nothing: every page's count still matches the log"
        );
    }

    /// The one case a lost index really does cost something. The previous body
    /// went with `pages_fts`, so the difference between the two totals is all
    /// there is, and it is recorded as a net rather than dressed up as a churn.
    #[tokio::test]
    async fn a_change_made_while_the_index_was_gone_is_a_net() {
        let (_directory, store, times, ideas, words, index) = fixture().await;

        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        // The database is deleted, and only then does the file change.
        let fresh = Index::open(None).await.expect("a new index");
        write(&store, "a", "One two three four five.\n").await;
        sync(&store, &times, &ideas, &words, &fresh).await.unwrap();

        let (log, _) = words.read().await.unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[1].kind, crate::words::Kind::Net);
        assert_eq!((log[1].added, log[1].removed), (2, 0));
        assert_eq!(log[1].total, 5);
    }

    /// A page at five words whose edit from three is already in the log, and an
    /// index that still holds the three: two machines sharing a wiki through
    /// git, seen from the one that did not make the edit.
    async fn an_index_one_edit_behind() -> (TempDir, Store, TimeStore, IdeaStore, WordLog, Index) {
        let (directory, store, times, ideas, words, index) = fixture().await;

        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();
        let elsewhere = Index::open(None).await.expect("a second index");
        sync(&store, &times, &ideas, &words, &elsewhere)
            .await
            .unwrap();

        write(&store, "a", "One two three four five.\n").await;
        sync(&store, &times, &ideas, &words, &elsewhere)
            .await
            .unwrap();
        assert_eq!(
            words.read().await.unwrap().0.len(),
            2,
            "a baseline, and the edit as the other machine saw it"
        );

        (directory, store, times, ideas, words, index)
    }

    /// An index older than the log is not a previous body. Diffing against it
    /// records the edit a second time, which is what starting a server against
    /// `example-wiki/` did with an index left over from an older checkout.
    #[tokio::test]
    async fn a_stale_index_does_not_record_what_the_log_already_holds() {
        let (_directory, store, times, ideas, words, index) = an_index_one_edit_behind().await;

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(
            report.pages.indexed, 1,
            "the page did change, as far as this index knew"
        );
        assert_eq!(
            words.read().await.unwrap().0.len(),
            2,
            "and the edit is in the log once, not twice"
        );
        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(5));
    }

    /// The same stale index, and a page that has moved on from the log as well.
    /// What the log has not seen is recorded, as the net it is.
    #[tokio::test]
    async fn a_stale_index_records_what_the_log_has_not_seen_as_a_net() {
        let (_directory, store, times, ideas, words, index) = an_index_one_edit_behind().await;

        write(&store, "a", "One two three four five six seven.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        let (log, _) = words.read().await.unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[2].kind, crate::words::Kind::Net);
        assert_eq!((log[2].added, log[2].removed), (2, 0));
        assert_eq!(log[2].total, 7);
    }

    /// A line the log failed to take. The page and the index moved on and the
    /// log did not, which is the same disagreement the other way round, and it
    /// comes back at the next write as a net covering both edits rather than as
    /// a churn that leaves the series short.
    #[tokio::test]
    async fn a_line_the_log_lost_comes_back_at_the_next_write_as_a_net() {
        let (_directory, store, times, ideas, words, index) = fixture().await;

        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        // Indexed and never written down, which is what `words::record` leaves
        // behind when appending to the log fails.
        write(&store, "a", "One two three four five.\n").await;
        let page = store.read(&slug("a")).await.expect("read");
        index.upsert(&page).await.expect("index without recording");

        write(&store, "a", "One two three four five six.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        let (log, _) = words.read().await.unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[1].kind, crate::words::Kind::Net);
        assert_eq!((log[1].added, log[1].removed), (3, 0));
        assert_eq!(log[1].total, 6);
    }

    /// A rebuild waits for the log to be let go. A line and its row are written
    /// under the same hold, so a rebuild that did not wait could land between
    /// them and drop the row, or fold it in twice.
    #[tokio::test]
    async fn a_rebuild_waits_while_the_log_is_held() {
        let (_directory, _store, _times, _ideas, words, index) = fixture().await;
        let held = words.hold().await;

        let rebuild = tokio::spawn({
            let (words, index) = (words.clone(), index.clone());
            async move { sync_words(&words, &index).await }
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !rebuild.is_finished(),
            "the rebuild went ahead while the log was held"
        );

        drop(held);
        rebuild.await.expect("task").expect("rebuild");
    }

    /// The watcher rereads the log only when somebody other than this process
    /// has written to it, which is what keeps the reread off every save.
    #[tokio::test]
    async fn the_log_is_reread_only_when_somebody_else_wrote_to_it() {
        let (directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert!(
            refresh_words(&words, &index).await.unwrap().is_none(),
            "the scan's own baseline is not news"
        );

        // Another writer over the same directory, which is what git or a second
        // machine looks like from here.
        let elsewhere = WordLog::open(directory.path())
            .await
            .expect("another writer");
        elsewhere
            .append(&crate::words::Observation {
                at: Utc::now(),
                slug: slug("b"),
                actor: "file".to_owned(),
                account: None,
                kind: crate::words::Kind::Baseline,
                added: 0,
                removed: 0,
                total: 3,
                from: None,
            })
            .await
            .expect("append elsewhere");

        let reread = refresh_words(&words, &index).await.unwrap();
        assert_eq!(reread.map(|read| read.observations), Some(2));
        assert!(
            refresh_words(&words, &index).await.unwrap().is_none(),
            "and once read, it is not news twice"
        );
    }

    /// A page gone from disk while nothing was watching closes its series, so
    /// the next page written at that slug is a new page rather than an edit.
    #[tokio::test]
    async fn a_page_that_vanished_while_the_server_was_down_is_marked_deleted() {
        let (_directory, store, times, ideas, words, index) = fixture().await;

        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        store.delete(&slug("a")).await.expect("delete");
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        let (log, _) = words.read().await.unwrap();
        assert_eq!(log[1].kind, crate::words::Kind::Deleted);
        assert_eq!(log[1].actor, crate::words::ACTOR_SCAN);
        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), None);
    }

    /// A file that will not parse is still somebody's writing. Marking it
    /// deleted would make the next successful save look like a brand new page.
    #[tokio::test]
    async fn a_page_that_stopped_parsing_does_not_close_its_series() {
        let (directory, store, times, ideas, words, index) = fixture().await;

        write(&store, "a", "One two three.\n").await;
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        tokio::fs::write(
            directory.path().join("a.md"),
            "---\ntags: not a list\n---\n",
        )
        .await
        .expect("break the page");
        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.pages.failed, 1);
        assert_eq!(words.read().await.unwrap().0.len(), 1);
        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(3));
    }

    /// A log line nobody can read costs itself and nothing else, and the scan
    /// says so rather than failing.
    #[tokio::test]
    async fn a_broken_log_line_does_not_stop_a_scan() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        tokio::fs::write(words.root().join("2026-08.log"), "not a line at all\n")
            .await
            .expect("write a broken log");

        write(&store, "a", "One two three.\n").await;
        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.pages.indexed, 1);
        assert_eq!(index.count_words_observed().await.unwrap(), 1);
    }

    /// The invariant the whole storage design rests on: whatever incremental
    /// syncing produces must be what a from-scratch rebuild produces.
    #[tokio::test]
    async fn incremental_syncing_matches_a_full_rebuild() {
        let (directory, store, times, ideas, words, index) = fixture().await;

        // A history of edits, arriving through both the API and the filesystem.
        write(&store, "page-0", "The first page.\n").await;
        write(&store, "notes/page-1", "The second page.\n").await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        write(&store, "notes/deep/page-2", "The third page.\n").await;
        store.delete(&slug("page-0")).await.expect("delete");
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        tokio::fs::write(
            directory.path().join("notes/page-1.md"),
            "---\ntitle: Edited\ntags: [theory]\n---\n\nThe second page, rewritten.\n",
        )
        .await
        .expect("external edit");
        let incremental = sync(&store, &times, &ideas, &words, &index).await.unwrap();
        assert!(incremental.changed_anything());

        let after_incremental = snapshot(&index).await;
        let times_after_incremental = index.time_stamps().await.unwrap();

        let report = rebuild(&store, &times, &ideas, &words, &index)
            .await
            .unwrap();
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
        let (directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        let entry = times
            .create(TimeDraft {
                note: "Chased the poll loop.\n".to_owned(),
                ..draft("Deep work", "2026-08-06T09:00:00Z")
            })
            .await
            .unwrap();
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("remove page");
        tokio::fs::remove_file(entry.id.to_path(times.root()))
            .await
            .expect("remove entry");
        rebuild(&store, &times, &ideas, &words, &index)
            .await
            .unwrap();

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
        let (_directory, store, times, ideas, words, index) = fixture().await;
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

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let (_directory, store, times, ideas, words, index) = fixture().await;
        ideas
            .create_capture(capture_draft("2026-08-20T14:15:30Z", "A thought.\n"))
            .await
            .unwrap();

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let (_directory, store, times, ideas, words, index) = fixture().await;
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

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let (_directory, store, times, ideas, words, index) = fixture().await;

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
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        sync(&store, &times, &ideas, &words, &index).await.unwrap();

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
        let incremental = sync(&store, &times, &ideas, &words, &index).await.unwrap();
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

        let report = rebuild(&store, &times, &ideas, &words, &index)
            .await
            .unwrap();
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
        let (_directory, store, times, ideas, words, index) = fixture().await;
        assert_eq!(index.last_sync().await.unwrap(), None);

        sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert!(index.last_sync().await.unwrap().is_some());
    }

    /// The time log lives inside `.rhizolog/`, which the page walker skips —
    /// so an entry must never also turn up as a page.
    #[tokio::test]
    async fn time_entries_are_not_pages() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 0);
        assert_eq!(report.times.scanned, 1);
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_empty_wiki_syncs_cleanly() {
        let (_directory, store, times, ideas, words, index) = fixture().await;

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(
            report,
            SyncReport {
                // An empty log that was read is not the same as a log that could
                // not be read, which is the one thing this field is for.
                words: WordSync {
                    observations: 0,
                    skipped: 0,
                    read: true,
                },
                ..SyncReport::default()
            }
        );
        assert!(!report.changed_anything());
        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
        assert_eq!(index.count_times().await.unwrap(), 0);
    }

    /// The word log is read and replaced rather than reconciled, so the report
    /// says what it held rather than pretending to five fields that would be
    /// zero for reasons that mean nothing.
    #[tokio::test]
    async fn the_report_says_what_the_word_log_held_when_it_was_read() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        write(&store, "a", "One two three.\n").await;

        // The first scan reads an empty log and then writes a baseline into it.
        let first = sync(&store, &times, &ideas, &words, &index).await.unwrap();
        assert_eq!(first.words.observations, 0);
        assert!(first.words.read);

        // The second reads the line the first one wrote.
        let second = sync(&store, &times, &ideas, &words, &index).await.unwrap();
        assert_eq!(second.words.observations, 1);
        assert_eq!(second.words.skipped, 0);

        // A rebuild is not a change to the wiki, and must not be reported as one.
        assert!(!second.changed_anything());
    }

    #[tokio::test]
    async fn a_line_that_will_not_parse_is_counted_in_the_report() {
        let (_directory, store, times, ideas, words, index) = fixture().await;
        tokio::fs::write(
            words.root().join("2026-08.log"),
            "not a line at all\nnor this one\n",
        )
        .await
        .expect("write a broken log");

        let report = sync(&store, &times, &ideas, &words, &index).await.unwrap();

        assert_eq!(report.words.skipped, 2);
        assert_eq!(report.words.observations, 0);
        assert!(report.words.read);
    }
}
