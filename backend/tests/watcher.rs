//! The file watcher, against real filesystem events.
//!
//! `watcher::plan` is unit-tested for the logic; these exercise the plumbing
//! underneath it — notify's backend, the debouncer, and the channel into the
//! async reindexer. That is inherently timing-dependent, so each test polls up
//! to [`PATIENCE`] rather than sleeping a fixed amount and hoping.

use std::future::Future;
use std::time::{Duration, Instant};

use rhizolog::{IdeaId, IdeaStore, Index, Owner, Store, TimeStore, watcher};
use tempfile::TempDir;

/// The wikis in this file have no accounts, so every page is visible. What
/// happens when they do is `tests/visibility.rs`.
const EVERYONE: rhizolog::index::Audience = rhizolog::index::Audience::Everything;

/// Generous on purpose: the debounce window is 500ms, and a loaded machine can
/// take a while to deliver events. A test that fails here should mean the
/// watcher is broken, not that the machine was busy.
const PATIENCE: Duration = Duration::from_secs(15);

/// Time for the watcher thread to register before the test changes anything.
const STARTUP: Duration = Duration::from_millis(500);

async fn watched() -> (TempDir, Store, Index) {
    let (directory, store, _times, _ideas, index) = watched_with_stores().await;
    (directory, store, index)
}

async fn watched_with_times() -> (TempDir, Store, TimeStore, Index) {
    let (directory, store, times, _ideas, index) = watched_with_stores().await;
    (directory, store, times, index)
}

async fn watched_with_ideas() -> (TempDir, IdeaStore, Index) {
    let (directory, _store, _times, ideas, index) = watched_with_stores().await;
    (directory, ideas, index)
}

async fn watched_with_stores() -> (TempDir, Store, TimeStore, IdeaStore, Index) {
    let directory = TempDir::new().expect("temp dir");
    let store = Store::open(directory.path()).await.expect("open store");
    let times = TimeStore::open(directory.path())
        .await
        .expect("open time log");
    let ideas = IdeaStore::open(directory.path())
        .await
        .expect("open idea inbox");
    let index = Index::open(None).await.expect("open index");

    watcher::spawn(store.clone(), times.clone(), ideas.clone(), index.clone());
    tokio::time::sleep(STARTUP).await;

    (directory, store, times, ideas, index)
}

/// Poll `condition` until it holds or [`PATIENCE`] runs out.
async fn eventually<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + PATIENCE;

    while Instant::now() < deadline {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("timed out after {PATIENCE:?} waiting for {what}");
}

#[tokio::test]
async fn picks_up_a_page_created_outside_the_api() {
    let (directory, _store, index) = watched().await;

    tokio::fs::write(
        directory.path().join("external.md"),
        "---\ntitle: External\ntags: [handwritten]\n---\n\nWritten in someone's editor.\n",
    )
    .await
    .expect("write page");

    eventually("the new page to be indexed", || async {
        index.count(&EVERYONE).await.unwrap_or(0) == 1
    })
    .await;

    let hits = index
        .search("editor", 10, 0, &EVERYONE)
        .await
        .expect("search");
    assert_eq!(hits.total, 1);
    assert_eq!(hits.hits[0].slug.as_str(), "external");
    assert_eq!(hits.hits[0].title, "External");
}

/// The time log is files too, and it lives inside `.rhizolog/` — the one
/// directory the watcher used to ignore wholesale. A hand-written entry has to
/// arrive the same way a hand-written page does.
#[tokio::test]
async fn picks_up_a_time_entry_written_outside_the_api() {
    let (_directory, _store, times, index) = watched_with_times().await;
    let month = times.root().join("2026-08");
    tokio::fs::create_dir_all(&month)
        .await
        .expect("month directory");
    let path = month.join("20260806T090000-000000000.md");

    tokio::fs::write(
        &path,
        "---\nname: By hand\nstart: 2026-08-06T09:00:00Z\nend: 2026-08-06T11:00:00Z\n---\n",
    )
    .await
    .expect("write entry");

    eventually("the new entry to be indexed", || async {
        index.count_times().await.unwrap_or(0) == 1
    })
    .await;

    tokio::fs::remove_file(&path).await.expect("remove entry");

    eventually("the removal to be picked up", || async {
        index.count_times().await.unwrap_or(1) == 0
    })
    .await;
}

/// Idea Inbox is three more trees inside `.rhizolog/`, and the folded state has
/// to follow a file appearing or vanishing there exactly as an index row follows
/// a page. The event is the interesting one: it is not a record anybody reads
/// directly, it is a decision, so removing its file has to take the decision
/// back out of the state it produced.
#[tokio::test]
async fn picks_up_idea_files_written_outside_the_api() {
    let (_directory, ideas, index) = watched_with_ideas().await;
    let capture = "20260820T141530-000000000";
    let thread = IdeaId::parse("20260820T142000-000000000").expect("valid idea id");

    let captures_month = ideas.captures_root().join("2026-08");
    tokio::fs::create_dir_all(&captures_month)
        .await
        .expect("captures month");
    tokio::fs::write(
        captures_month.join(format!("{capture}.md")),
        "---\ncreated: 2026-08-20T14:15:30Z\n---\n\nDungeon seeds.\n",
    )
    .await
    .expect("write capture");
    tokio::fs::create_dir_all(ideas.threads_root())
        .await
        .expect("threads root");
    tokio::fs::write(
        ideas.threads_root().join(format!("{thread}.md")),
        format!(
            "---\nname: Dungeon seeds\ncreated: 2026-08-20T14:20:00Z\ncaptures:\n- {capture}\n---\n"
        ),
    )
    .await
    .expect("write thread");

    eventually("the thread and its seed to be indexed", || async {
        matches!(
            index.idea_state(&Owner::open(), &thread).await,
            Ok(Some(state)) if state.members.len() == 1
        )
    })
    .await;

    let events_month = ideas.events_root().join("2026-08");
    tokio::fs::create_dir_all(&events_month)
        .await
        .expect("events month");
    let retirement = events_month.join("20260820T142100-000000000.md");
    tokio::fs::write(
        &retirement,
        format!("---\nkind: idea_retired\ncreated: 2026-08-20T14:21:00Z\nidea: {thread}\n---\n"),
    )
    .await
    .expect("write event");

    eventually("the retirement to be folded in", || async {
        matches!(
            index.idea_state(&Owner::open(), &thread).await,
            Ok(Some(state)) if state.retired
        )
    })
    .await;

    tokio::fs::remove_file(&retirement)
        .await
        .expect("remove event");

    eventually("the retirement to be folded back out", || async {
        matches!(
            index.idea_state(&Owner::open(), &thread).await,
            Ok(Some(state)) if !state.retired
        )
    })
    .await;
}

/// The database sits beside the time log, and the two must not share a fate:
/// the server writes to it constantly, and reacting to that would be a loop.
#[tokio::test]
async fn the_index_database_beside_the_time_log_is_still_ignored() {
    let (directory, _store, _times, index) = watched_with_times().await;
    let internal = directory.path().join(".rhizolog");
    tokio::fs::create_dir_all(&internal)
        .await
        .expect("internal directory");

    tokio::fs::write(internal.join("index.db"), "not a real database")
        .await
        .expect("write database");
    tokio::fs::write(directory.path().join("page.md"), "Body about rhizomes.\n")
        .await
        .expect("write page");

    eventually("the page to be indexed", || async {
        index
            .search("rhizomes", 10, 0, &EVERYONE)
            .await
            .unwrap()
            .total
            == 1
    })
    .await;
    assert_eq!(
        index.count_times().await.unwrap(),
        0,
        "the database was mistaken for a time entry"
    );
}

#[tokio::test]
async fn picks_up_an_edit_made_outside_the_api() {
    let (directory, _store, index) = watched().await;
    let path = directory.path().join("page.md");

    tokio::fs::write(&path, "Original wording.\n")
        .await
        .expect("write page");
    eventually("the page to be indexed", || async {
        index
            .search("Original", 10, 0, &EVERYONE)
            .await
            .unwrap()
            .total
            == 1
    })
    .await;

    tokio::fs::write(&path, "Replaced wording.\n")
        .await
        .expect("edit page");

    eventually("the edit to be picked up", || async {
        index
            .search("Replaced", 10, 0, &EVERYONE)
            .await
            .unwrap()
            .total
            == 1
    })
    .await;
    assert_eq!(
        index
            .search("Original", 10, 0, &EVERYONE)
            .await
            .unwrap()
            .total,
        0,
        "the old text is still searchable"
    );
}

#[tokio::test]
async fn picks_up_a_deletion_made_outside_the_api() {
    let (directory, _store, index) = watched().await;
    let path = directory.path().join("page.md");

    tokio::fs::write(&path, "Body.\n")
        .await
        .expect("write page");
    eventually("the page to be indexed", || async {
        index.count(&EVERYONE).await.unwrap_or(0) == 1
    })
    .await;

    tokio::fs::remove_file(&path).await.expect("delete page");

    eventually("the deletion to be picked up", || async {
        index.count(&EVERYONE).await.unwrap_or(1) == 0
    })
    .await;
}

/// A directory event names only the directory, so the pages inside it have to
/// be found by rescanning.
#[tokio::test]
async fn picks_up_a_directory_removed_outside_the_api() {
    let (directory, _store, index) = watched().await;
    let nested = directory.path().join("notes");

    tokio::fs::create_dir_all(&nested)
        .await
        .expect("create dir");
    tokio::fs::write(nested.join("a.md"), "First.\n")
        .await
        .expect("write a");
    tokio::fs::write(nested.join("b.md"), "Second.\n")
        .await
        .expect("write b");
    eventually("both pages to be indexed", || async {
        index.count(&EVERYONE).await.unwrap_or(0) == 2
    })
    .await;

    tokio::fs::remove_dir_all(&nested)
        .await
        .expect("remove directory");

    eventually("both pages to be dropped", || async {
        index.count(&EVERYONE).await.unwrap_or(2) == 0
    })
    .await;
}

/// Writes through the API trigger events too. Nothing suppresses them, because
/// reindexing is idempotent — but that has to actually hold, not just be
/// asserted in a comment.
#[tokio::test]
async fn the_apis_own_writes_do_not_corrupt_the_index() {
    let (_directory, store, index) = watched().await;
    let slug = rhizolog::Slug::parse("notes/page").expect("valid slug");

    let page = store
        .write(
            &slug,
            rhizolog::Frontmatter {
                title: Some("Written by the API".to_owned()),
                tags: vec!["api".to_owned()],
                created: None,
                ..rhizolog::Frontmatter::default()
            },
            "Body about rhizomes.\n",
        )
        .await
        .expect("write page");
    index.upsert(&page).await.expect("index page");

    // Let the watcher see its own echo and act on it.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert_eq!(
        index.count(&EVERYONE).await.unwrap(),
        1,
        "the echo duplicated a page"
    );
    let hits = index.search("rhizomes", 10, 0, &EVERYONE).await.unwrap();
    assert_eq!(hits.total, 1);
    assert_eq!(hits.hits[0].title, "Written by the API");
    assert_eq!(hits.hits[0].tags, ["api"]);
}
