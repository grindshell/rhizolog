//! The time log on disk.
//!
//! Time entries are files, for the same reason pages are: a developer should be
//! able to `grep` last month, fix a typo in an editor, and commit the lot to
//! git. The SQLite index over them is derived and disposable, exactly as it is
//! for pages — delete `.rhizolog/index.db` and the log is untouched.
//!
//! The tree is `<wiki>/.rhizolog/times/<YYYY-MM>/<id>.md`. The month directory
//! keeps a heavy year from becoming one directory of four thousand files, and
//! it is derived from the id rather than from the entry's `start`, so a path is
//! a pure function of an id and editing a start time never moves a file.
//!
//! Writes go through the same temporary-file-and-rename dance as pages, so a
//! reader or the watcher never sees half an entry.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use thiserror::Error;
use walkdir::WalkDir;

use crate::slug::Slug;
use crate::store::{INTERNAL_DIR, modified_at, write_atomically};
use crate::times::{TIMES_DIR, TimeEntry, TimeError, TimeId};

/// How many ids to try before giving up on finding a free one.
///
/// Only reached when many entries share an instant to the nanosecond, which in
/// practice means many manual entries logged for the same whole second.
const MINT_ATTEMPTS: u32 = 1_000;

#[derive(Debug, Error)]
pub enum TimeStoreError {
    #[error("no time entry {id}")]
    NotFound { id: TimeId },

    #[error("{id} is not valid UTF-8")]
    NotUtf8 { id: TimeId },

    #[error("{id} resolves outside the times directory")]
    EscapesRoot { id: TimeId },

    #[error("could not parse {id}: {source}")]
    Malformed {
        id: TimeId,
        #[source]
        source: TimeError,
    },

    #[error("could not mint an id for an entry starting at {start}")]
    NoFreeId { start: DateTime<Utc> },

    #[error(transparent)]
    Io(#[from] io::Error),
}

/// One entry found while walking the log, without its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeWalkEntry {
    pub id: TimeId,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

/// What a caller wants an entry to say. Everything is required, because a write
/// replaces the file wholesale; the API layer is where "leave this alone" lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeDraft {
    pub name: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub pages: Vec<Slug>,
    pub note: String,
}

/// A directory of time entries.
#[derive(Debug, Clone)]
pub struct TimeStore {
    root: PathBuf,
}

impl TimeStore {
    /// Open (creating if necessary) the time log for the wiki at `wiki_root`.
    pub async fn open(wiki_root: impl AsRef<Path>) -> Result<Self, TimeStoreError> {
        let root = wiki_root.as_ref().join(INTERNAL_DIR).join(TIMES_DIR);
        tokio::fs::create_dir_all(&root).await?;
        // Canonicalised so the containment check in `resolve` compares like
        // with like — on Windows that means both sides carry `\\?\`.
        let root = tokio::fs::canonicalize(&root).await?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn root_display(&self) -> String {
        crate::store::display_path(&self.root)
    }

    /// The path an id names, having confirmed it does not leave the log.
    ///
    /// An id cannot traverse on its own — that is settled by [`TimeId::parse`],
    /// which accepts twenty-five characters of digits and two separators — but
    /// a symlinked month directory could point anywhere.
    async fn resolve(&self, id: &TimeId) -> Result<PathBuf, TimeStoreError> {
        let path = id.to_path(&self.root);

        match tokio::fs::canonicalize(&path).await {
            Ok(canonical) if !canonical.starts_with(&self.root) => {
                Err(TimeStoreError::EscapesRoot { id: id.clone() })
            }
            Ok(_) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn exists(&self, id: &TimeId) -> Result<bool, TimeStoreError> {
        let path = self.resolve(id).await?;
        Ok(tokio::fs::try_exists(&path).await?)
    }

    pub async fn read(&self, id: &TimeId) -> Result<TimeEntry, TimeStoreError> {
        let path = self.resolve(id).await?;

        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(error) => {
                return Err(match error.kind() {
                    io::ErrorKind::NotFound => TimeStoreError::NotFound { id: id.clone() },
                    io::ErrorKind::InvalidData => TimeStoreError::NotUtf8 { id: id.clone() },
                    _ => error.into(),
                });
            }
        };

        let updated = modified_at(&tokio::fs::metadata(&path).await?)?;

        TimeEntry::from_markdown(id.clone(), &text, updated).map_err(|source| {
            TimeStoreError::Malformed {
                id: id.clone(),
                source,
            }
        })
    }

    /// Write an entry at `id`, replacing whatever was there.
    pub async fn write(&self, id: &TimeId, draft: TimeDraft) -> Result<TimeEntry, TimeStoreError> {
        let path = self.resolve(id).await?;

        let entry = TimeEntry {
            id: id.clone(),
            name: draft.name,
            start: draft.start,
            end: draft.end,
            pages: draft.pages,
            note: draft.note,
            // Both replaced below with what the filesystem actually recorded.
            updated: Utc::now(),
            size: 0,
        };

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        write_atomically(&path, entry.to_markdown().as_bytes()).await?;

        let metadata = tokio::fs::metadata(&path).await?;
        Ok(TimeEntry {
            updated: modified_at(&metadata)?,
            size: metadata.len(),
            ..entry
        })
    }

    /// Record a new entry, minting an id for it.
    ///
    /// The id comes from the entry's `start`, so the log sorts and files itself
    /// by when the time was spent rather than by when it was typed in. A manual
    /// entry usually has whole-second precision, so several can land on the
    /// same instant; the nudge walks the nanosecond half upward until the file
    /// is free.
    pub async fn create(&self, draft: TimeDraft) -> Result<TimeEntry, TimeStoreError> {
        for nudge in 0..MINT_ATTEMPTS {
            let id = TimeId::mint(draft.start, nudge);
            if !self.exists(&id).await? {
                return self.write(&id, draft).await;
            }
        }

        Err(TimeStoreError::NoFreeId { start: draft.start })
    }

    pub async fn delete(&self, id: &TimeId) -> Result<(), TimeStoreError> {
        let path = self.resolve(id).await?;

        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(TimeStoreError::NotFound { id: id.clone() });
            }
            Err(error) => return Err(error.into()),
        }

        // A month that has been emptied is noise in a directory listing. Only
        // the one level: the times root itself stays.
        if let Some(month) = path.parent()
            && month.starts_with(&self.root)
            && month != self.root
        {
            let _ = tokio::fs::remove_dir(month).await;
        }

        Ok(())
    }

    /// Every entry in the log.
    ///
    /// Blocking: callers on an async task should wrap this in `spawn_blocking`.
    /// It exists for the startup scan.
    pub fn walk(&self) -> Vec<TimeWalkEntry> {
        let mut entries = Vec::new();

        let walker = WalkDir::new(&self.root)
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
                    tracing::warn!(%error, "skipping unreadable entry while walking the time log");
                    continue;
                }
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let Ok(relative) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            let Some(id) = TimeId::from_relative_path(relative) else {
                continue;
            };

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    tracing::warn!(%error, path = %entry.path().display(), "skipping unreadable time entry");
                    continue;
                }
            };
            let Ok(updated) = modified_at(&metadata) else {
                continue;
            };

            entries.push(TimeWalkEntry {
                id,
                updated,
                size: metadata.len(),
            });
        }

        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    async fn store() -> (TempDir, TimeStore) {
        let directory = TempDir::new().expect("temp dir");
        let store = TimeStore::open(directory.path()).await.expect("open");
        (directory, store)
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn draft(name: &str, start: &str, end: Option<&str>) -> TimeDraft {
        TimeDraft {
            name: name.to_owned(),
            start: at(start),
            end: end.map(at),
            pages: Vec::new(),
            note: String::new(),
        }
    }

    #[tokio::test]
    async fn writes_and_reads_an_entry() {
        let (_directory, store) = store().await;

        let written = store
            .create(TimeDraft {
                pages: vec![Slug::parse("notes/rust/async").unwrap()],
                note: "Chased a lifetime error.\n".to_owned(),
                ..draft(
                    "Deep work",
                    "2026-08-06T14:25:30Z",
                    Some("2026-08-06T15:40:00Z"),
                )
            })
            .await
            .expect("create");

        let read = store.read(&written.id).await.expect("read");
        assert_eq!(read.name, "Deep work");
        assert_eq!(read.pages[0].as_str(), "notes/rust/async");
        assert_eq!(read.note, "Chased a lifetime error.\n");
        assert_eq!(read.seconds(at("2026-08-06T20:00:00Z")), 74 * 60 + 30);
    }

    #[tokio::test]
    async fn files_are_grouped_by_month() {
        let (directory, store) = store().await;
        let written = store
            .create(draft("Deep work", "2026-08-06T14:25:30Z", None))
            .await
            .expect("create");

        let expected = directory
            .path()
            .join(INTERNAL_DIR)
            .join(TIMES_DIR)
            .join("2026-08")
            .join(format!("{}.md", written.id));
        assert!(expected.is_file(), "not at {}", expected.display());
    }

    /// Two manual entries logged for the same whole second are two entries.
    #[tokio::test]
    async fn entries_at_the_same_instant_get_distinct_ids() {
        let (_directory, store) = store().await;

        let first = store
            .create(draft("Standup", "2026-08-06T09:00:00Z", None))
            .await
            .expect("first");
        let second = store
            .create(draft("Review", "2026-08-06T09:00:00Z", None))
            .await
            .expect("second");

        assert_ne!(first.id, second.id);
        assert_eq!(store.read(&first.id).await.unwrap().name, "Standup");
        assert_eq!(store.read(&second.id).await.unwrap().name, "Review");
    }

    #[tokio::test]
    async fn reading_a_missing_entry_reports_not_found() {
        let (_directory, store) = store().await;
        let id = TimeId::parse("20260806T142530-123456789").unwrap();

        assert!(matches!(
            store.read(&id).await.unwrap_err(),
            TimeStoreError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn deleting_prunes_the_month_it_emptied() {
        let (directory, store) = store().await;
        let written = store
            .create(draft("Deep work", "2026-08-06T14:25:30Z", None))
            .await
            .expect("create");
        let month = directory
            .path()
            .join(INTERNAL_DIR)
            .join(TIMES_DIR)
            .join("2026-08");

        store.delete(&written.id).await.expect("delete");

        assert!(!month.exists(), "empty month directory not pruned");
        assert!(store.root().exists(), "the log root must survive pruning");
        assert!(matches!(
            store.delete(&written.id).await.unwrap_err(),
            TimeStoreError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn walking_finds_every_entry_and_skips_strays() {
        let (_directory, store) = store().await;
        for start in [
            "2026-07-30T09:00:00Z",
            "2026-08-06T14:25:30Z",
            "2026-08-07T14:25:30Z",
        ] {
            store
                .create(draft("Deep work", start, None))
                .await
                .expect("create");
        }
        tokio::fs::write(
            store.root().join("2026-08").join("notes.md"),
            "not an entry",
        )
        .await
        .expect("write stray");

        let mut found: Vec<String> = store
            .walk()
            .into_iter()
            .map(|entry| entry.id.to_string())
            .collect();
        found.sort();

        assert_eq!(found.len(), 3, "got {found:?}");
        assert!(found[0].starts_with("20260730T090000"));
    }

    /// The temporary file a write goes through must never be walked.
    #[tokio::test]
    async fn atomic_writes_leave_nothing_visible_behind() {
        let (_directory, store) = store().await;
        store
            .create(draft("Deep work", "2026-08-06T14:25:30Z", None))
            .await
            .expect("create");

        let names: Vec<String> = std::fs::read_dir(store.root().join("2026-08"))
            .expect("read dir")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names.len(), 1, "got {names:?}");
    }

    #[tokio::test]
    async fn a_malformed_entry_is_reported_against_its_id() {
        let (_directory, store) = store().await;
        let id = TimeId::parse("20260806T142530-123456789").unwrap();
        tokio::fs::create_dir_all(store.root().join("2026-08"))
            .await
            .expect("create month");
        tokio::fs::write(id.to_path(store.root()), "---\nname: No start\n---\n")
            .await
            .expect("write");

        assert!(matches!(
            store.read(&id).await.unwrap_err(),
            TimeStoreError::Malformed { .. }
        ));
    }
}
