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

use crate::ideas::{
    CAPTURES_DIR, CaptureId, EVENTS_DIR, EventId, IDEAS_DIR, IdeaId, IdeaStore, THREADS_DIR,
};
use crate::index::{Index, sync::sync};
use crate::slug::Slug;
use chrono::Utc;

use crate::store::{INTERNAL_DIR, Store, StoreError};
use crate::times::{TIMES_DIR, TimeId, TimeStore};
use crate::words::{self, By, WordLog};

/// How long to wait for a burst of events to settle.
///
/// Long enough to collapse a temp-write-and-rename into one batch, short enough
/// that a save feels like it took effect immediately.
const DEBOUNCE: Duration = Duration::from_millis(500);

/// The individual files a batch of events named.
///
/// A struct rather than five fields on the enum variant below, so that a caller
/// building one can name the tree it cares about and default the rest. There are
/// five authored trees now and there is no reason to think that is the end of
/// it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Targets {
    pub pages: BTreeSet<Slug>,
    pub times: BTreeSet<TimeId>,
    pub captures: BTreeSet<CaptureId>,
    pub ideas: BTreeSet<IdeaId>,
    pub events: BTreeSet<EventId>,
}

impl Targets {
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
            && self.times.is_empty()
            && self.captures.is_empty()
            && self.ideas.is_empty()
            && self.events.is_empty()
    }
}

/// What a batch of filesystem events asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reindex {
    /// Re-read these specific files.
    Targets(Targets),
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
pub fn spawn(
    store: Store,
    times: TimeStore,
    ideas: IdeaStore,
    words: WordLog,
    index: Index,
) -> Option<JoinHandle<()>> {
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

            apply(&store, &times, &ideas, &words, &index, plan).await;
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
    Capture(CaptureId),
    Idea(IdeaId),
    Event(EventId),
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
    let mut targets = Targets::default();
    let mut rescan = false;

    for path in paths {
        let Ok(relative) = path.strip_prefix(root) else {
            // Outside the wiki entirely; not ours to care about.
            continue;
        };

        match classify(relative) {
            Change::Ignore => {}
            Change::Page(slug) => {
                targets.pages.insert(slug);
            }
            Change::Time(id) => {
                targets.times.insert(id);
            }
            Change::Capture(id) => {
                targets.captures.insert(id);
            }
            Change::Idea(id) => {
                targets.ideas.insert(id);
            }
            Change::Event(id) => {
                targets.events.insert(id);
            }
            Change::Rescan => rescan = true,
        }
    }

    if rescan {
        Some(Reindex::Everything)
    } else if targets.is_empty() {
        None
    } else {
        Some(Reindex::Targets(targets))
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
/// Most of what lives here is the server's own business — the database and its
/// write-ahead log, which is what the blanket "ignore hidden paths" rule used to
/// be for. The exceptions are the time log and Idea Inbox, which are authored
/// data that happen to share the directory, and which therefore have to be
/// watched exactly as the pages are. Accounts stay ignored: they are read from
/// disk on every request and there is no index over them to keep in step.
fn classify_internal(rest: &[&str]) -> Change {
    match rest {
        [TIMES_DIR, tail @ ..] => classify_time(tail),
        [IDEAS_DIR, tail @ ..] => classify_idea(tail),
        _ => Change::Ignore,
    }
}

fn classify_time(rest: &[&str]) -> Change {
    match rest {
        [month, file] => month_filed(TimeId::from_relative_path, month, file)
            .map_or(Change::Rescan, Change::Time),
        // The times directory itself, a month directory on its own, or
        // something nested deeper than an entry can be.
        _ => Change::Rescan,
    }
}

/// Work out which of Idea Inbox's three trees a path is in, and which record.
///
/// Anything that is not exactly one of the three shapes forces a rescan rather
/// than being guessed at, which covers a tree directory being created or moved
/// and a stray file dropped in by hand.
fn classify_idea(rest: &[&str]) -> Change {
    match rest {
        [CAPTURES_DIR, month, file] => month_filed(CaptureId::from_relative_path, month, file)
            .map_or(Change::Rescan, Change::Capture),
        [THREADS_DIR, file] => {
            IdeaId::from_relative_path(Path::new(file)).map_or(Change::Rescan, Change::Idea)
        }
        [EVENTS_DIR, month, file] => month_filed(EventId::from_relative_path, month, file)
            .map_or(Change::Rescan, Change::Event),
        _ => Change::Rescan,
    }
}

/// Recover an id from the `<YYYY-MM>/<id>.md` shape the two bucketed trees use.
fn month_filed<Id>(recover: fn(&Path) -> Option<Id>, month: &str, file: &str) -> Option<Id> {
    recover(&Path::new(month).join(file))
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

async fn apply(
    store: &Store,
    times: &TimeStore,
    ideas: &IdeaStore,
    words: &WordLog,
    index: &Index,
    plan: Reindex,
) {
    match plan {
        Reindex::Everything => match sync(store, times, ideas, words, index).await {
            Ok(report) if report.changed_anything() => {
                tracing::info!(
                    pages = report.pages.indexed,
                    times = report.times.indexed,
                    captures = report.captures.indexed,
                    ideas = report.ideas.indexed,
                    events = report.events.indexed,
                    removed = report.removed(),
                    "picked up external changes"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not rescan the wiki"),
        },
        Reindex::Targets(targets) => {
            for slug in targets.pages {
                if let Err(error) = reindex_page(store, words, index, &slug).await {
                    tracing::warn!(%slug, %error, "could not reindex a changed page");
                }
            }
            for id in targets.times {
                if let Err(error) = reindex_time(times, index, &id).await {
                    tracing::warn!(%id, %error, "could not reindex a changed time entry");
                }
            }
            for id in targets.captures {
                if let Err(error) = reindex_capture(ideas, index, &id).await {
                    tracing::warn!(%id, %error, "could not reindex a changed capture");
                }
            }
            for id in targets.ideas {
                if let Err(error) = reindex_idea(ideas, index, &id).await {
                    tracing::warn!(%id, %error, "could not reindex a changed idea thread");
                }
            }
            for id in targets.events {
                if let Err(error) = reindex_event(ideas, index, &id).await {
                    tracing::warn!(%id, %error, "could not reindex a changed decision event");
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
    words: &WordLog,
    index: &Index,
    slug: &Slug,
) -> Result<(), crate::index::IndexError> {
    match store.read(slug).await {
        Ok(page) => {
            let change = index.upsert(&page).await?;
            // `file`, which is the writer in their own editor. It is the one
            // actor nobody can claim over HTTP, because it is the one the server
            // works out for itself.
            words::observe(
                words,
                index,
                slug,
                &By::file(),
                Utc::now(),
                // A page appearing under a running server is somebody writing
                // one, not a wiki that was already there. Only the startup scan
                // baselines.
                change.as_written(),
            )
            .await;
            tracing::debug!(%slug, "reindexed after an external edit");
        }
        Err(error) => {
            // Gone, or no longer readable. Either way it cannot be served, so
            // it should not be findable. A page that becomes readable again
            // comes back on the next event or the next scan.
            tracing::debug!(%slug, %error, "dropping a page that could not be read");
            index.remove(slug).await?;

            // Only a page that has actually **gone** closes its series. A page
            // that has merely stopped parsing is still somebody's writing, and
            // marking it deleted would make the next successful save look like a
            // brand new page.
            if matches!(error, StoreError::NotFound { .. }) {
                words::deleted(words, index, slug, &By::file(), Utc::now()).await;
            }
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

/// The same, for one capture.
///
/// The index recomputes every idea this capture bears on, so a capture appearing
/// or disappearing under the server's feet moves the threads that name it too.
async fn reindex_capture(
    ideas: &IdeaStore,
    index: &Index,
    id: &CaptureId,
) -> Result<(), crate::index::IndexError> {
    match ideas.read_capture(id).await {
        Ok(capture) => {
            index.upsert_capture(&capture).await?;
            tracing::debug!(%id, "reindexed a capture after an external edit");
        }
        Err(error) => {
            tracing::debug!(%id, %error, "dropping a capture that could not be read");
            index.remove_capture(id).await?;
        }
    }
    Ok(())
}

/// The same, for one idea thread.
async fn reindex_idea(
    ideas: &IdeaStore,
    index: &Index,
    id: &IdeaId,
) -> Result<(), crate::index::IndexError> {
    match ideas.read_idea(id).await {
        Ok(idea) => {
            index.upsert_idea(&idea).await?;
            tracing::debug!(%id, "reindexed an idea thread after an external edit");
        }
        Err(error) => {
            tracing::debug!(%id, %error, "dropping an idea thread that could not be read");
            index.remove_idea(id).await?;
        }
    }
    Ok(())
}

/// The same, for one decision event.
///
/// Editing an event file by hand is not how decisions are meant to be reversed,
/// and it is a thing a person with the disk can do. Reindexing it refolds the
/// idea and captures it names, so the state comes back in line with whatever the
/// files now say.
async fn reindex_event(
    ideas: &IdeaStore,
    index: &Index,
    id: &EventId,
) -> Result<(), crate::index::IndexError> {
    match ideas.read_event(id).await {
        Ok(event) => {
            index.upsert_idea_event(&event).await?;
            tracing::debug!(%id, "reindexed a decision event after an external edit");
        }
        Err(error) => {
            tracing::debug!(%id, %error, "dropping a decision event that could not be read");
            index.remove_idea_event(id).await?;
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
        Reindex::Targets(Targets {
            pages: raw.iter().map(|s| Slug::parse(s).unwrap()).collect(),
            ..Targets::default()
        })
    }

    fn ids(raw: &[&str]) -> Reindex {
        Reindex::Targets(Targets {
            times: raw.iter().map(|s| TimeId::parse(s).unwrap()).collect(),
            ..Targets::default()
        })
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
            Some(Reindex::Targets(Targets {
                pages: [Slug::parse("notes/rhizome").unwrap()].into(),
                times: [TimeId::parse("20260806T090000-000000000").unwrap()].into(),
                ..Targets::default()
            }))
        );
    }

    /// Idea Inbox is three trees under one directory, each with its own shape,
    /// and a path in one of them must never be read as a record in another.
    #[test]
    fn each_idea_tree_reindexes_just_the_record_that_changed() {
        assert_eq!(
            planned(&[".rhizolog/ideas/captures/2026-08/20260820T141530-123456789.md"]),
            Some(Reindex::Targets(Targets {
                captures: [CaptureId::parse("20260820T141530-123456789").unwrap()].into(),
                ..Targets::default()
            }))
        );
        assert_eq!(
            planned(&[".rhizolog/ideas/threads/20260820T142000-234567890.md"]),
            Some(Reindex::Targets(Targets {
                ideas: [IdeaId::parse("20260820T142000-234567890").unwrap()].into(),
                ..Targets::default()
            }))
        );
        assert_eq!(
            planned(&[".rhizolog/ideas/events/2026-08/20260820T142030-345678901.md"]),
            Some(Reindex::Targets(Targets {
                events: [EventId::parse("20260820T142030-345678901").unwrap()].into(),
                ..Targets::default()
            }))
        );
    }

    /// A thread is filed flat and a capture is filed by month, so the wrong
    /// shape in the right tree is not a record: it is something nobody can
    /// attribute, and the safe reading is to go and look at everything.
    #[test]
    fn the_wrong_shape_in_an_idea_tree_forces_a_rescan() {
        for path in [
            // A thread nested under a month, and a capture that is not.
            ".rhizolog/ideas/threads/2026-08/20260820T142000-234567890.md",
            ".rhizolog/ideas/captures/20260820T141530-123456789.md",
            // Filed under a month it does not belong to.
            ".rhizolog/ideas/captures/2026-07/20260820T141530-123456789.md",
            // A stray file, and the directories themselves.
            ".rhizolog/ideas/captures/2026-08/notes.md",
            ".rhizolog/ideas/captures/2026-08",
            ".rhizolog/ideas/captures",
            ".rhizolog/ideas/threads",
            ".rhizolog/ideas",
            // A tree nobody has heard of.
            ".rhizolog/ideas/drafts/20260820T142000-234567890.md",
        ] {
            assert_eq!(planned(&[path]), Some(Reindex::Everything), "for {path}");
        }
    }

    /// Accounts share the directory and are deliberately not watched: they are
    /// read from disk on every request, so there is no index to keep in step.
    #[test]
    fn accounts_are_still_ignored() {
        assert_eq!(planned(&[".rhizolog/users/tim.md"]), None);
        assert_eq!(planned(&[".rhizolog/users"]), None);
    }

    #[test]
    fn the_temporary_files_of_an_idea_write_are_ignored() {
        assert_eq!(
            planned(&[".rhizolog/ideas/captures/2026-08/.20260820T141530-123456789.md.tmp"]),
            None
        );
        assert_eq!(
            planned(&[".rhizolog/ideas/threads/.20260820T142000-234567890.md.tmp"]),
            None
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
