//! Watching the wiki directory for edits made outside the API.
//!
//! Files are the source of truth, which means an editor, a `git checkout`, or
//! an agent writing markdown directly are all first-class ways to change the
//! wiki. The startup scan catches whatever happened while the server was down;
//! this catches what happens while it is up.
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

use crate::index::{Index, sync::sync};
use crate::slug::Slug;
use crate::store::Store;

/// How long to wait for a burst of events to settle.
///
/// Long enough to collapse a temp-write-and-rename into one batch, short enough
/// that a save feels like it took effect immediately.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// What a batch of filesystem events asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reindex {
    /// Re-read these specific pages.
    Pages(BTreeSet<Slug>),
    /// Something happened that cannot be attributed to individual pages — a
    /// directory was renamed or deleted, say. Rescan the wiki.
    Everything,
}

/// Start watching `store`'s directory.
///
/// Failure is reported, not fatal: a wiki on a filesystem that cannot be
/// watched should still be served, just without live pickup of external edits.
pub fn spawn(store: Store, index: Index) {
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
            return;
        }
    };

    if let Err(error) = debouncer.watch(&root, RecursiveMode::Recursive) {
        tracing::warn!(%error, path = %crate::store::display_path(&root), "could not watch the wiki directory; external edits will only be picked up on restart");
        return;
    }

    tokio::spawn(async move {
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

            apply(&store, &index, plan).await;
        }

        tracing::debug!("file watcher stopped");
    });
}

/// Decide what a batch of changed paths requires.
///
/// Returns `None` when nothing in the batch concerns us — which covers the
/// index's own database and the temporary files [`crate::store`] writes
/// through, both of which live inside the wiki directory and would otherwise
/// have this chasing its own tail.
pub fn plan<'a>(root: &Path, paths: impl Iterator<Item = &'a Path>) -> Option<Reindex> {
    let mut pages = BTreeSet::new();
    let mut rescan = false;

    for path in paths {
        let Ok(relative) = path.strip_prefix(root) else {
            // Outside the wiki entirely; not ours to care about.
            continue;
        };
        if is_hidden(relative) {
            continue;
        }

        match Slug::from_relative_path(relative) {
            Some(slug) => {
                pages.insert(slug);
            }
            None => {
                // A directory, or a file that is not a page. A directory rename
                // or delete can take many pages with it and arrives as a single
                // event naming only the directory, so the safe reading is that
                // we no longer know what changed. A rescan is cheap when
                // nothing did — it compares mtimes and reads nothing.
                rescan = true;
            }
        }
    }

    if rescan {
        Some(Reindex::Everything)
    } else if pages.is_empty() {
        None
    } else {
        Some(Reindex::Pages(pages))
    }
}

async fn apply(store: &Store, index: &Index, plan: Reindex) {
    match plan {
        Reindex::Everything => match sync(store, index).await {
            Ok(report) if report.changed_anything() => {
                tracing::info!(
                    indexed = report.indexed,
                    removed = report.removed,
                    "picked up external changes"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not rescan the wiki"),
        },
        Reindex::Pages(slugs) => {
            for slug in slugs {
                if let Err(error) = reindex_page(store, index, &slug).await {
                    tracing::warn!(%slug, %error, "could not reindex a changed page");
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

/// Whether any component of a relative path is a dot-entry.
///
/// Catches `.rhizolog/index.db` and the `.page.md.tmp` files atomic writes go
/// through, which are the two ways the server's own activity shows up here.
fn is_hidden(relative: &Path) -> bool {
    relative.components().any(|component| {
        matches!(component, Component::Normal(name)
            if name.to_str().is_some_and(|name| name.starts_with('.')))
    })
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
        Reindex::Pages(raw.iter().map(|s| Slug::parse(s).unwrap()).collect())
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
        // Temporary files from an atomic write.
        assert_eq!(planned(&[".notes.md.tmp"]), None);
        assert_eq!(planned(&["notes/.rhizome.md.tmp"]), None);
        // And anything else hidden, like a git checkout touching .git.
        assert_eq!(planned(&[".git/index"]), None);
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
