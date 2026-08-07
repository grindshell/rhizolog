//! Watching the wiki directory for edits made outside the API.
//!
//! Files are the source of truth, which means an editor, a `git checkout`, or
//! an agent writing markdown directly are all first-class ways to change the
//! wiki. The startup scan catches whatever happened while the server was down;
//! this catches what happens while it is up. It covers the time log for the
//! same reason it covers the pages: both are files, so both can be edited
//! behind the server's back.
//!
//! ## Why there is no echo suppression
//!
//! The API's own writes trigger events here too. Nothing tries to filter them
//! out, because reindexing is idempotent and driven entirely by what is on disk
//! *now*: an event says "something happened to this slug", and the response is
//! to go and look. Re-reading a page the API just wrote produces the same index
//! rows it already wrote. Suppression logic would be a source of bugs guarding
//! against a harmless duplicate read.
//!
//! ## Why events are debounced
//!
//! Editors save by writing a temporary file and renaming it over the original —
//! Rhizolog's own [`crate::store`] does the same — so a single save can arrive
//! as several events. Windows is especially chatty here. The debouncer collapses
//! a burst into one batch.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};
use tokio::task::JoinHandle;

use crate::index::{Index, sync::sync};
use crate::slug::Slug;
use crate::store::{INTERNAL_DIR, Store};
use crate::times::{TIMES_DIR, TimeId, TimeStore};

/// How long to wait for a burst of events to settle.
///
/// Long enough to collapse a temp-write-and-rename into one batch, short enough
/// that a save feels like it took effect immediately.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// What a batch of filesystem events asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reindex {
    /// Re-read these specific files.
    Targets {
        pages: BTreeSet<Slug>,
        times: BTreeSet<TimeId>,
    },
    /// Something happened that cannot be attributed to individual files — a
    /// directory was renamed or deleted, say. Rescan everything.
    Everything,
}

/// Start watching `store`'s directory.
///
/// Failure is reported, not fatal: a wiki on a filesystem that cannot be
/// watched should still be served, just without live pickup of external edits.
/// That is the `None` case, and it is why the caller gets an `Option` rather
/// than a handle it can rely on.
///
/// The returned handle exists so a shutdown can cancel the watcher and wait for
/// it to let go of the index; see [`crate::server::Server::shutdown`]. Dropping
/// it detaches the watcher, which is what a process about to exit wants.
pub fn spawn(store: Store, times: TimeStore, index: Index) -> Option<JoinHandle<()>> {
    let root = store.root().to_path_buf();
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();

    let debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        // Runs on the watcher's own thread. An unbounded send never blocks, so
        // a slow reindex cannot stall the notify backend and make it drop
        // events. A failed send means the receiver is gone, i.e. shutdown.
        let _ = events.send(result);
    });

    let mut debouncer = match debouncer {
        Ok(debouncer) => debouncer,
        Err(error) => {
            tracing::warn!(%error, "could not start the file watcher; external edits will only be picked up on restart");
            return None;
        }
    };

    if let Err(error) = debouncer.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(%error, path = %crate::store::display_path(&root), "could not watch the wiki directory; external edits will only be picked up on restart");
        return None;
    }

    Some(tokio::spawn(async move {
        // The debouncer stops watching when dropped, so the task owns it for as
        // long as it runs even though it never touches it again.
        let _debouncer = debouncer;

        tracing::info!(path = %crate::store::display_path(&root), "watching the wiki for external edits");

        while let Some(result) = receiver.recv().await {
            let paths = match result {
                Ok(events) => events
                    .into_iter()
                    .flat_map(|event| event.paths.clone())
                    .collect::<Vec<PathBuf>>(),
                Err(errors) => {
                    for error in errors {
                        tracing::warn!(%error, "file watcher error");
                    }
                    continue;
                }
            };

            let Some(plan) = plan(&root, paths.iter().map(PathBuf::as_path)) else {
                continue;
            };

            apply(&store, &times, &index, plan).await;
        }

        tracing::debug!("file watcher stopped");
    }))
}

/// What one changed path turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// Not ours: the index database, a temporary file, `.git`.
    Ignore,
    Page(Slug),
    Time(TimeId),
    /// Something whose effects cannot be enumerated from the event alone.
    Rescan,
}

/// Decide what a batch of changed paths requires.
///
/// Returns `None` when nothing in the batch concerns us — which covers the
/// index's own database and the temporary files [`crate::store`] writes
/// through, both of which live inside the wiki directory and would otherwise
/// have this chasing its own tail.
pub fn plan<'a>(root: &Path, paths: impl Iterator<Item = &'a Path>) -> Option<Reindex> {
    let mut pages = BTreeSet::new();
    let mut times = BTreeSet::new();
    let mut rescan = false;

    for path in paths {
        let Ok(relative) = path.strip_prefix(root) else {
            // Outside the wiki entirely; not ours to care about.
            continue;
        };

        match classify(relative) {
            Change::Ignore => {}
            Change::Page(slug) => {
                pages.insert(slug);
            }
            Change::Time(id) => {
                times.insert(id);
            }
            Change::Rescan => rescan = true,
        }
    }

    if rescan {
        Some(Reindex::Everything)
    } else if pages.is_empty() && times.is_empty() {
        None
    } else {
        Some(Reindex::Targets { pages, times })
    }
}

/// Work out what a path relative to the wiki root is.
fn classify(relative: &Path) -> Change {
    let Some(segments) = segments(relative) else {
        // A path we cannot read as text is a path we cannot address.
        return Change::Rescan;
    };

    // The temporary files an atomic write goes through, in either tree.
    if segments
        .last()
        .is_some_and(|name| name.starts_with('.') && name.ends_with(".tmp"))
    {
        return Change::Ignore;
    }

    if segments.first() == Some(&INTERNAL_DIR) {
        return classify_internal(&segments[1..]);
    }

    // `.git`, and anything else hidden by convention. Slug validation rejects
    // dot-segments too, so these could never be addressed as pages anyway.
    if segments.iter().any(|name| name.starts_with('.')) {
        return Change::Ignore;
    }

    match Slug::from_relative_path(relative) {
        Some(slug) => Change::Page(slug),
        // A directory, or a file that is not a page. A directory rename or
        // delete can take many pages with it and arrives as a single event
        // naming only the directory, so the safe reading is that we no longer
        // know what changed. A rescan is cheap when nothing did — it compares
        // mtimes and reads nothing.
        None => Change::Rescan,
    }
}

/// Work out what a path inside `.rhizolog/` is.
///
/// Almost everything here is the server's own business — the database and its
/// write-ahead log, which is what the blanket "ignore hidden paths" rule used
/// to be for. The exception is the time log, which is authored data that
/// happens to live in the same directory, and which therefore has to be watched
/// exactly as the pages are.
fn classify_internal(rest: &[&str]) -> Change {
    if rest.first() != Some(&TIMES_DIR) {
        return Change::Ignore;
    }

    match rest.len() {
        // The times directory itself was created, moved or removed.
        1 => Change::Rescan,
        3 => match TimeId::from_relative_path(Path::new(rest[1]).join(rest[2]).as_path()) {
            Some(id) => Change::Time(id),
            None => Change::Rescan,
        },
        // A month directory, or something nested deeper than an entry can be.
        _ => Change::Rescan,
    }
}

/// A relative path as text segments, or `None` if it holds anything that is not
/// a plain name.
fn segments(relative: &Path) -> Option<Vec<&str>> {
    relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect()
}

async fn apply(store: &Store, times: &TimeStore, index: &Index, plan: Reindex) {
    match plan {
        Reindex::Everything => match sync(store, times, index).await {
            Ok(report) if report.changed_anything() => {
                tracing::info!(
                    pages = report.pages.indexed,
                    times = report.times.indexed,
                    removed = report.pages.removed + report.times.removed,
                    "picked up external changes"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not rescan the wiki"),
        },
        Reindex::Targets {
            pages,
            times: changed,
        } => {
            for slug in pages {
                if let Err(error) = reindex_page(store, index, &slug).await {
                    tracing::warn!(%slug, %error, "could not reindex a changed page");
                }
            }
            for id in changed {
                if let Err(error) = reindex_time(times, index, &id).await {
                    tracing::warn!(%id, %error, "could not reindex a changed time entry");
                }
            }
        }
    }
}

/// Bring one page's index entry in line with what is on disk.
///
/// Deliberately ignores which kind of event arrived. Created, modified, moved,
/// deleted — the answer is the same: read the file, and index whatever is
/// there, or drop it if there is nothing. That is what makes this idempotent,
/// and idempotence is what makes the API's own write echoes harmless.
async fn reindex_page(
    store: &Store,
    index: &Index,
    slug: &Slug,
) -> Result<(), crate::index::IndexError> {
    match store.read(slug).await {
        Ok(page) => {
            index.upsert(&page).await?;
            tracing::debug!(%slug, "reindexed after an external edit");
        }
        Err(error) => {
            // Gone, or no longer readable. Either way it cannot be served, so
            // it should not be findable. A page that becomes readable again
            // comes back on the next event or the next scan.
            tracing::debug!(%slug, %error, "dropping a page that could not be read");
            index.remove(slug).await?;
        }
    }
    Ok(())
}

/// The same, for one time entry.
async fn reindex_time(
    times: &TimeStore,
    index: &Index,
    id: &TimeId,
) -> Result<(), crate::index::IndexError> {
    match times.read(id).await {
        Ok(entry) => {
            index.upsert_time(&entry).await?;
            tracing::debug!(%id, "reindexed a time entry after an external edit");
        }
        Err(error) => {
            tracing::debug!(%id, %error, "dropping a time entry that could not be read");
            index.remove_time(id).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/wiki")
    }

    fn planned(paths: &[&str]) -> Option<Reindex> {
        let root = root();
        let paths: Vec<PathBuf> = paths.iter().map(|path| root.join(path)).collect();
        plan(&root, paths.iter().map(PathBuf::as_path))
    }

    fn slugs(raw: &[&str]) -> Reindex {
        Reindex::Targets {
            pages: raw.iter().map(|s| Slug::parse(s).unwrap()).collect(),
            times: BTreeSet::new(),
        }
    }

    fn ids(raw: &[&str]) -> Reindex {
        Reindex::Targets {
            pages: BTreeSet::new(),
            times: raw.iter().map(|s| TimeId::parse(s).unwrap()).collect(),
        }
    }

    #[test]
    fn a_changed_page_reindexes_just_that_page() {
        assert_eq!(
            planned(&["notes/rhizome.md"]),
            Some(slugs(&["notes/rhizome"]))
        );
    }

    #[test]
    fn a_burst_collapses_into_one_batch() {
        assert_eq!(
            planned(&["a.md", "notes/b.md", "a.md"]),
            Some(slugs(&["a", "notes/b"])),
            "the same page touched twice should be reindexed once"
        );
    }

    /// The index database lives inside the wiki. Without this the server would
    /// watch itself write and reindex forever.
    #[test]
    fn the_servers_own_files_are_ignored() {
        assert_eq!(planned(&[".rhizolog/index.db"]), None);
        assert_eq!(planned(&[".rhizolog/index.db-wal"]), None);
        // The endpoint file, which this server writes about itself on startup.
        assert_eq!(planned(&[".rhizolog/server.json"]), None);
        // Temporary files from an atomic write, in either tree.
        assert_eq!(planned(&[".notes.md.tmp"]), None);
        assert_eq!(planned(&["notes/.rhizome.md.tmp"]), None);
        assert_eq!(
            planned(&[".rhizolog/times/2026-08/.20260806T090000-000000000.md.tmp"]),
            None
        );
        // And anything else hidden, like a git checkout touching .git.
        assert_eq!(planned(&[".git/index"]), None);
    }

    /// The time log shares a directory with the database, and the two must not
    /// share a fate: one is ours to ignore, the other is authored data.
    #[test]
    fn a_changed_time_entry_reindexes_just_that_entry() {
        assert_eq!(
            planned(&[".rhizolog/times/2026-08/20260806T090000-000000000.md"]),
            Some(ids(&["20260806T090000-000000000"]))
        );
    }

    #[test]
    fn pages_and_times_can_change_in_the_same_batch() {
        assert_eq!(
            planned(&[
                "notes/rhizome.md",
                ".rhizolog/times/2026-08/20260806T090000-000000000.md",
            ]),
            Some(Reindex::Targets {
                pages: [Slug::parse("notes/rhizome").unwrap()].into(),
                times: [TimeId::parse("20260806T090000-000000000").unwrap()].into(),
            })
        );
    }

    #[test]
    fn a_stray_file_in_the_time_log_forces_a_rescan_rather_than_being_guessed_at() {
        assert_eq!(
            planned(&[".rhizolog/times/2026-08/notes.md"]),
            Some(Reindex::Everything)
        );
        assert_eq!(
            planned(&[".rhizolog/times/2026-08"]),
            Some(Reindex::Everything)
        );
        assert_eq!(planned(&[".rhizolog/times"]), Some(Reindex::Everything));
    }

    #[test]
    fn a_hidden_path_does_not_drag_a_real_page_into_a_rescan() {
        assert_eq!(
            planned(&[".rhizolog/index.db", "notes/rhizome.md"]),
            Some(slugs(&["notes/rhizome"])),
            "the ignored path should not have forced a full rescan"
        );
    }

    /// A directory event names only the directory, so its pages cannot be
    /// enumerated from the event alone.
    #[test]
    fn a_directory_change_forces_a_rescan() {
        assert_eq!(planned(&["notes"]), Some(Reindex::Everything));
        assert_eq!(planned(&["notes/rust"]), Some(Reindex::Everything));
        // Even alongside identifiable pages: the directory may have taken
        // others with it.
        assert_eq!(planned(&["notes", "a.md"]), Some(Reindex::Everything));
    }

    #[test]
    fn a_non_page_file_forces_a_rescan_rather_than_being_guessed_at() {
        assert_eq!(planned(&["notes.txt"]), Some(Reindex::Everything));
    }

    #[test]
    fn paths_outside_the_wiki_are_ignored() {
        let paths = [PathBuf::from("/elsewhere/notes.md")];
        assert_eq!(plan(&root(), paths.iter().map(PathBuf::as_path)), None);
    }

    #[test]
    fn an_empty_batch_asks_for_nothing() {
        assert_eq!(planned(&[]), None);
    }
}
