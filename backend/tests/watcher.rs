//! The file watcher, against real filesystem events.
//!
//! `watcher::plan` is unit-tested for the logic; these exercise the plumbing
//! underneath it — notify's backend, the debouncer, and the channel into the
//! async reindexer. That is inherently timing-dependent, so each test polls up
//! to [`PATIENCE`] rather than sleeping a fixed amount and hoping.

use std::future::Future;
use std::time::{Duration, Instant};

use rhizowiki::{Index, Store, watcher};
use tempfile::TempDir;

/// Generous on purpose: the debounce window is 500ms, and a loaded machine can
/// take a while to deliver events. A test that fails here should mean the
/// watcher is broken, not that the machine was busy.
const PATIENCE: Duration = Duration::from_secs(15);

/// Time for the watcher thread to register before the test changes anything.
const STARTUP: Duration = Duration::from_millis(500);

async fn watched() -> (TempDir, Store, Index) {
    let directory = TempDir::new().expect("temp dir");
    let store = Store::open(directory.path()).await.expect("open store");
    let index = Index::open(None).await.expect("open index");

    watcher::spawn(store.clone(), index.clone());
    tokio::time::sleep(STARTUP).await;

    (directory, store, index)
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
        index.count().await.unwrap_or(0) == 1
    })
    .await;

    let hits = index.search("editor", 10, 0).await.expect("search");
    assert_eq!(hits.total, 1);
    assert_eq!(hits.hits[0].slug.as_str(), "external");
    assert_eq!(hits.hits[0].title, "External");
}

#[tokio::test]
async fn picks_up_an_edit_made_outside_the_api() {
    let (directory, _store, index) = watched().await;
    let path = directory.path().join("page.md");

    tokio::fs::write(&path, "Original wording.\n")
        .await
        .expect("write page");
    eventually("the page to be indexed", || async {
        index.search("Original", 10, 0).await.unwrap().total == 1
    })
    .await;

    tokio::fs::write(&path, "Replaced wording.\n")
        .await
        .expect("edit page");

    eventually("the edit to be picked up", || async {
        index.search("Replaced", 10, 0).await.unwrap().total == 1
    })
    .await;
    assert_eq!(
        index.search("Original", 10, 0).await.unwrap().total,
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
        index.count().await.unwrap_or(0) == 1
    })
    .await;

    tokio::fs::remove_file(&path).await.expect("delete page");

    eventually("the deletion to be picked up", || async {
        index.count().await.unwrap_or(1) == 0
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
        index.count().await.unwrap_or(0) == 2
    })
    .await;

    tokio::fs::remove_dir_all(&nested)
        .await
        .expect("remove directory");

    eventually("both pages to be dropped", || async {
        index.count().await.unwrap_or(2) == 0
    })
    .await;
}

/// Writes through the API trigger events too. Nothing suppresses them, because
/// reindexing is idempotent — but that has to actually hold, not just be
/// asserted in a comment.
#[tokio::test]
async fn the_apis_own_writes_do_not_corrupt_the_index() {
    let (_directory, store, index) = watched().await;
    let slug = rhizowiki::Slug::parse("notes/page").expect("valid slug");

    let page = store
        .write(
            &slug,
            rhizowiki::Frontmatter {
                title: Some("Written by the API".to_owned()),
                tags: vec!["api".to_owned()],
                created: None,
            },
            "Body about rhizomes.\n",
        )
        .await
        .expect("write page");
    index.upsert(&page).await.expect("index page");

    // Let the watcher see its own echo and act on it.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert_eq!(
        index.count().await.unwrap(),
        1,
        "the echo duplicated a page"
    );
    let hits = index.search("rhizomes", 10, 0).await.unwrap();
    assert_eq!(hits.total, 1);
    assert_eq!(hits.hits[0].title, "Written by the API");
    assert_eq!(hits.hits[0].tags, ["api"]);
}
