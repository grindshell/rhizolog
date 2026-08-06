//! Reconciling the index with what is actually on disk.
//!
//! This runs at startup and behind `POST /api/reindex`, and it is the seam
//! where "files are the source of truth" is actually enforced. Everything the
//! index believes is checked against the wiki directory and corrected.
//!
//! There are two trees to check and they are checked the same way: the pages,
//! and the time log under `.rhizolog/times/`. Both are files, both are
//! authoritative, and both are compared by mtime and size against what was
//! recorded. That misses an edit that preserves both, which needs a tool that
//! restores mtime and a replacement of exactly equal length — rare enough that
//! paying for a hash of every file on every startup is the worse trade, and
//! [`rebuild`] fixes it when it does happen.

use chrono::Utc;

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
}

impl SyncReport {
    pub fn changed_anything(&self) -> bool {
        self.pages.changed_anything() || self.times.changed_anything()
    }
}

/// Bring the index in line with the wiki directory and the time log.
pub async fn sync(
    store: &Store,
    times: &TimeStore,
    index: &Index,
) -> Result<SyncReport, IndexError> {
    let report = SyncReport {
        pages: sync_pages(store, index).await?,
        times: sync_times(times, index).await?,
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

/// Throw the index away and build it again from disk.
///
/// The escape hatch for anything incremental syncing can miss. It is cheap
/// precisely because the index holds nothing that is not already in a file.
pub async fn rebuild(
    store: &Store,
    times: &TimeStore,
    index: &Index,
) -> Result<SyncReport, IndexError> {
    index.clear().await?;
    sync(store, times, index).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{DateTime, Utc};
    use tempfile::TempDir;

    use crate::index::{Stamp, TimeListOptions};
    use crate::page::Frontmatter;
    use crate::slug::Slug;
    use crate::times::store::TimeDraft;
    use std::collections::HashMap;

    async fn fixture() -> (TempDir, Store, TimeStore, Index) {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let times = TimeStore::open(directory.path())
            .await
            .expect("open time log");
        let index = Index::open(None).await.expect("open index");
        (directory, store, times, index)
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
            .search("page", 100, 0)
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
        let (_directory, store, times, index) = fixture().await;
        for n in 0..3 {
            write(&store, &format!("page-{n}"), "A page about rhizomes.\n").await;
        }

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 3);
        assert_eq!(report.pages.indexed, 3);
        assert_eq!(report.pages.unchanged, 0);
        assert_eq!(index.count().await.unwrap(), 3);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 3);
    }

    #[tokio::test]
    async fn a_second_scan_reindexes_nothing() {
        let (_directory, store, times, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &index).await.unwrap();

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.unchanged, 1);
        assert_eq!(report.times.unchanged, 1);
        assert_eq!(report.pages.indexed, 0);
        assert_eq!(report.times.indexed, 0);
        assert!(!report.changed_anything());
    }

    #[tokio::test]
    async fn picks_up_a_page_edited_outside_the_api() {
        let (directory, store, times, index) = fixture().await;
        write(&store, "page-0", "Original body.\n").await;
        sync(&store, &times, &index).await.unwrap();

        // Edited behind the server's back, as an external editor would.
        tokio::fs::write(
            directory.path().join("page-0.md"),
            "Replaced body about rhizomes.\n",
        )
        .await
        .expect("external edit");

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.indexed, 1);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 1);
        assert_eq!(index.search("Original", 10, 0).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn drops_a_page_deleted_outside_the_api() {
        let (directory, store, times, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        write(&store, "page-1", "Body.\n").await;
        sync(&store, &times, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("external delete");

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.removed, 1);
        assert_eq!(index.count().await.unwrap(), 1);
        assert!(!index.stamps().await.unwrap().contains_key(&slug("page-0")));
    }

    /// The time log is files too, so hand-editing it has to work the same way.
    #[tokio::test]
    async fn picks_up_a_time_entry_written_by_hand() {
        let (_directory, store, times, index) = fixture().await;
        let month = times.root().join("2026-08");
        tokio::fs::create_dir_all(&month).await.expect("month");
        tokio::fs::write(
            month.join("20260806T090000-000000000.md"),
            "---\nname: Deep work\nstart: 2026-08-06T09:00:00Z\nend: 2026-08-06T11:00:00Z\n---\n",
        )
        .await
        .expect("hand-written entry");

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.times.scanned, 1);
        assert_eq!(report.times.indexed, 1);
        let groups = index.time_groups(at("2026-08-06T20:00:00Z")).await.unwrap();
        assert_eq!(groups[0].name, "Deep work");
        assert_eq!(groups[0].seconds, 2 * 3600);
    }

    #[tokio::test]
    async fn drops_a_time_entry_deleted_outside_the_api() {
        let (_directory, store, times, index) = fixture().await;
        let written = times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &index).await.unwrap();
        assert_eq!(index.count_times().await.unwrap(), 1);

        tokio::fs::remove_file(written.id.to_path(times.root()))
            .await
            .expect("external delete");

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.times.removed, 1);
        assert_eq!(index.count_times().await.unwrap(), 0);
    }

    /// One unreadable file must not stop the wiki from indexing.
    #[tokio::test]
    async fn a_malformed_page_is_skipped_not_fatal() {
        let (directory, store, times, index) = fixture().await;
        write(&store, "good", "A page about rhizomes.\n").await;
        tokio::fs::write(
            directory.path().join("broken.md"),
            "---\ntitle: never closed\n\nBody.\n",
        )
        .await
        .expect("write broken page");

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 2);
        assert_eq!(report.pages.indexed, 1);
        assert_eq!(report.pages.failed, 1);
        assert_eq!(index.count().await.unwrap(), 1);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 1);
    }

    #[tokio::test]
    async fn a_malformed_time_entry_is_skipped_not_fatal() {
        let (_directory, store, times, index) = fixture().await;
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

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.times.scanned, 2);
        assert_eq!(report.times.indexed, 1);
        assert_eq!(report.times.failed, 1);
        assert_eq!(index.count_times().await.unwrap(), 1);
    }

    /// A page that becomes malformed leaves the index rather than lingering as
    /// a search hit that cannot be fetched.
    #[tokio::test]
    async fn a_page_that_breaks_is_dropped_from_the_index() {
        let (directory, store, times, index) = fixture().await;
        write(&store, "page-0", "A page about rhizomes.\n").await;
        sync(&store, &times, &index).await.unwrap();
        assert_eq!(index.count().await.unwrap(), 1);

        tokio::fs::write(
            directory.path().join("page-0.md"),
            "---\ntitle: never closed\n\nrhizomes\n",
        )
        .await
        .expect("break the page");

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.failed, 1);
        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 0);
    }

    /// The invariant the whole storage design rests on: whatever incremental
    /// syncing produces must be what a from-scratch rebuild produces.
    #[tokio::test]
    async fn incremental_syncing_matches_a_full_rebuild() {
        let (directory, store, times, index) = fixture().await;

        // A history of edits, arriving through both the API and the filesystem.
        write(&store, "page-0", "The first page.\n").await;
        write(&store, "notes/page-1", "The second page.\n").await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();
        sync(&store, &times, &index).await.unwrap();

        write(&store, "notes/deep/page-2", "The third page.\n").await;
        store.delete(&slug("page-0")).await.expect("delete");
        sync(&store, &times, &index).await.unwrap();

        tokio::fs::write(
            directory.path().join("notes/page-1.md"),
            "---\ntitle: Edited\ntags: [theory]\n---\n\nThe second page, rewritten.\n",
        )
        .await
        .expect("external edit");
        let incremental = sync(&store, &times, &index).await.unwrap();
        assert!(incremental.changed_anything());

        let after_incremental = snapshot(&index).await;
        let times_after_incremental = index.time_stamps().await.unwrap();

        let report = rebuild(&store, &times, &index).await.unwrap();
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
        let (directory, store, times, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        let entry = times
            .create(TimeDraft {
                note: "Chased the poll loop.\n".to_owned(),
                ..draft("Deep work", "2026-08-06T09:00:00Z")
            })
            .await
            .unwrap();
        sync(&store, &times, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("remove page");
        tokio::fs::remove_file(entry.id.to_path(times.root()))
            .await
            .expect("remove entry");
        rebuild(&store, &times, &index).await.unwrap();

        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.count_times().await.unwrap(), 0);

        // A file that is gone is walked by nothing, so only `Index::clear`
        // can take its searchable text with it. The full-text tables are the
        // easy ones to forget there — they hold their own copy of the row.
        assert_eq!(index.search("body", 10, 0).await.unwrap().total, 0);
        let searched = index
            .list_times(
                TimeListOptions {
                    query: Some("poll".to_owned()),
                    ..TimeListOptions::default()
                },
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(searched.total, 0, "the deleted entry is still searchable");
    }

    #[tokio::test]
    async fn syncing_records_when_it_last_ran() {
        let (_directory, store, times, index) = fixture().await;
        assert_eq!(index.last_sync().await.unwrap(), None);

        sync(&store, &times, &index).await.unwrap();

        assert!(index.last_sync().await.unwrap().is_some());
    }

    /// The time log lives inside `.rhizolog/`, which the page walker skips —
    /// so an entry must never also turn up as a page.
    #[tokio::test]
    async fn time_entries_are_not_pages() {
        let (_directory, store, times, index) = fixture().await;
        times
            .create(draft("Deep work", "2026-08-06T09:00:00Z"))
            .await
            .unwrap();

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report.pages.scanned, 0);
        assert_eq!(report.times.scanned, 1);
        assert_eq!(index.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_empty_wiki_syncs_cleanly() {
        let (_directory, store, times, index) = fixture().await;

        let report = sync(&store, &times, &index).await.unwrap();

        assert_eq!(report, SyncReport::default());
        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.count_times().await.unwrap(), 0);
    }
}
