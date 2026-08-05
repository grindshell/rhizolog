//! Reconciling the index with what is actually on disk.
//!
//! This runs at startup and behind `POST /api/reindex`, and it is the seam
//! where "files are the source of truth" is actually enforced. Everything the
//! index believes is checked against the wiki directory and corrected.
//!
//! A page is considered unchanged when its mtime and size both match what was
//! recorded. That misses an edit that preserves both, which needs a tool that
//! restores mtime and a replacement of exactly equal length — rare enough that
//! paying for a hash of every file on every startup is the worse trade, and
//! [`rebuild`] fixes it when it does happen.

use chrono::Utc;

use crate::index::{Index, IndexError};
use crate::store::Store;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Pages found on disk.
    pub scanned: usize,
    /// Pages read and written to the index because they were new or changed.
    pub indexed: usize,
    /// Pages already indexed with a matching mtime and size.
    pub unchanged: usize,
    /// Pages dropped from the index because they are no longer on disk.
    pub removed: usize,
    /// Pages on disk that could not be read, and so are not in the index.
    pub failed: usize,
}

impl SyncReport {
    pub fn changed_anything(&self) -> bool {
        self.indexed > 0 || self.removed > 0
    }
}

/// Bring the index in line with the wiki directory.
pub async fn sync(store: &Store, index: &Index) -> Result<SyncReport, IndexError> {
    let walker = store.clone();
    let entries = tokio::task::spawn_blocking(move || walker.walk())
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?;

    // Entries are removed as they are accounted for; whatever remains at the
    // end is indexed but no longer on disk.
    let mut stale = index.stamps().await?;

    let mut report = SyncReport {
        scanned: entries.len(),
        ..SyncReport::default()
    };

    for entry in entries {
        let previous = stale.remove(&entry.slug);

        if previous.is_some_and(|stamp| stamp.updated == entry.updated && stamp.size == entry.size)
        {
            report.unchanged += 1;
            continue;
        }

        match store.read(&entry.slug).await {
            Ok(page) => {
                index.upsert(&page).await?;
                report.indexed += 1;
            }
            Err(error) => {
                // Drop it rather than leaving a stale row behind: a page that
                // cannot be read cannot be served, and a search hit that 404s
                // is worse than no hit at all. Its stamp is gone too, so a
                // transient failure simply reindexes on the next scan.
                tracing::warn!(slug = %entry.slug, %error, "could not index page");
                index.remove(&entry.slug).await?;
                report.failed += 1;
            }
        }
    }

    for slug in stale.keys() {
        index.remove(slug).await?;
        report.removed += 1;
    }

    index.set_last_sync(Utc::now()).await?;
    Ok(report)
}

/// Throw the index away and build it again from the wiki.
///
/// The escape hatch for anything incremental syncing can miss. It is cheap
/// precisely because the index holds nothing that is not already on disk.
pub async fn rebuild(store: &Store, index: &Index) -> Result<SyncReport, IndexError> {
    index.clear().await?;
    sync(store, index).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::index::Stamp;
    use crate::page::Frontmatter;
    use crate::slug::Slug;
    use std::collections::HashMap;

    async fn fixture() -> (TempDir, Store, Index) {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let index = Index::open(None).await.expect("open index");
        (directory, store, index)
    }

    fn slug(raw: &str) -> Slug {
        Slug::parse(raw).expect("valid slug")
    }

    async fn write(store: &Store, raw: &str, body: &str) {
        store
            .write(&slug(raw), Frontmatter::default(), body)
            .await
            .expect("write page");
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
        let (_directory, store, index) = fixture().await;
        for n in 0..3 {
            write(&store, &format!("page-{n}"), "A page about rhizomes.\n").await;
        }

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(report.scanned, 3);
        assert_eq!(report.indexed, 3);
        assert_eq!(report.unchanged, 0);
        assert_eq!(index.count().await.unwrap(), 3);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 3);
    }

    #[tokio::test]
    async fn a_second_scan_reindexes_nothing() {
        let (_directory, store, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        sync(&store, &index).await.unwrap();

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(report.unchanged, 1);
        assert_eq!(report.indexed, 0);
        assert!(!report.changed_anything());
    }

    #[tokio::test]
    async fn picks_up_a_page_edited_outside_the_api() {
        let (directory, store, index) = fixture().await;
        write(&store, "page-0", "Original body.\n").await;
        sync(&store, &index).await.unwrap();

        // Edited behind the server's back, as an external editor would.
        tokio::fs::write(
            directory.path().join("page-0.md"),
            "Replaced body about rhizomes.\n",
        )
        .await
        .expect("external edit");

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(report.indexed, 1);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 1);
        assert_eq!(index.search("Original", 10, 0).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn drops_a_page_deleted_outside_the_api() {
        let (directory, store, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        write(&store, "page-1", "Body.\n").await;
        sync(&store, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("external delete");

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(report.removed, 1);
        assert_eq!(index.count().await.unwrap(), 1);
        assert!(!index.stamps().await.unwrap().contains_key(&slug("page-0")));
    }

    /// One unreadable file must not stop the wiki from indexing.
    #[tokio::test]
    async fn a_malformed_page_is_skipped_not_fatal() {
        let (directory, store, index) = fixture().await;
        write(&store, "good", "A page about rhizomes.\n").await;
        tokio::fs::write(
            directory.path().join("broken.md"),
            "---\ntitle: never closed\n\nBody.\n",
        )
        .await
        .expect("write broken page");

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(report.scanned, 2);
        assert_eq!(report.indexed, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(index.count().await.unwrap(), 1);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 1);
    }

    /// A page that becomes malformed leaves the index rather than lingering as
    /// a search hit that cannot be fetched.
    #[tokio::test]
    async fn a_page_that_breaks_is_dropped_from_the_index() {
        let (directory, store, index) = fixture().await;
        write(&store, "page-0", "A page about rhizomes.\n").await;
        sync(&store, &index).await.unwrap();
        assert_eq!(index.count().await.unwrap(), 1);

        tokio::fs::write(
            directory.path().join("page-0.md"),
            "---\ntitle: never closed\n\nrhizomes\n",
        )
        .await
        .expect("break the page");

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(report.failed, 1);
        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.search("rhizomes", 10, 0).await.unwrap().total, 0);
    }

    /// The invariant the whole storage design rests on: whatever incremental
    /// syncing produces must be what a from-scratch rebuild produces.
    #[tokio::test]
    async fn incremental_syncing_matches_a_full_rebuild() {
        let (directory, store, index) = fixture().await;

        // A history of edits, arriving through both the API and the filesystem.
        write(&store, "page-0", "The first page.\n").await;
        write(&store, "notes/page-1", "The second page.\n").await;
        sync(&store, &index).await.unwrap();

        write(&store, "notes/deep/page-2", "The third page.\n").await;
        store.delete(&slug("page-0")).await.expect("delete");
        sync(&store, &index).await.unwrap();

        tokio::fs::write(
            directory.path().join("notes/page-1.md"),
            "---\ntitle: Edited\ntags: [theory]\n---\n\nThe second page, rewritten.\n",
        )
        .await
        .expect("external edit");
        let incremental = sync(&store, &index).await.unwrap();
        assert!(incremental.changed_anything());

        let after_incremental = snapshot(&index).await;

        let report = rebuild(&store, &index).await.unwrap();
        let after_rebuild = snapshot(&index).await;

        assert_eq!(report.indexed, 2, "rebuild should reindex every page");
        assert_eq!(report.unchanged, 0);
        assert_eq!(
            after_incremental, after_rebuild,
            "incremental sync diverged from a full rebuild"
        );
    }

    #[tokio::test]
    async fn rebuilding_clears_rows_for_pages_that_are_gone() {
        let (directory, store, index) = fixture().await;
        write(&store, "page-0", "Body.\n").await;
        sync(&store, &index).await.unwrap();

        tokio::fs::remove_file(directory.path().join("page-0.md"))
            .await
            .expect("remove");
        rebuild(&store, &index).await.unwrap();

        assert_eq!(index.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn syncing_records_when_it_last_ran() {
        let (_directory, store, index) = fixture().await;
        assert_eq!(index.last_sync().await.unwrap(), None);

        sync(&store, &index).await.unwrap();

        assert!(index.last_sync().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn an_empty_wiki_syncs_cleanly() {
        let (_directory, store, index) = fixture().await;

        let report = sync(&store, &index).await.unwrap();

        assert_eq!(
            report,
            SyncReport {
                scanned: 0,
                ..SyncReport::default()
            }
        );
        assert_eq!(index.count().await.unwrap(), 0);
    }
}
