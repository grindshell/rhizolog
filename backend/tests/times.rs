//! Time tracking, end to end over the assembled router.
//!
//! Driven the same way `tests/api.rs` drives the page API: in-process with
//! `tower::ServiceExt::oneshot` against a throwaway wiki. The time log lives
//! inside that wiki, under `.rhizolog/times/`, so it goes away with it.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use rhizolog::{AppState, Assets, Index, Store, TimeStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct App {
    directory: TempDir,
    router: Router,
}

struct Res {
    status: StatusCode,
    body: Value,
    location: Option<String>,
}

impl Res {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("<no code>")
    }
}

impl App {
    async fn new() -> Self {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let times = TimeStore::open(directory.path())
            .await
            .expect("open time log");
        let users = rhizolog::UserStore::open(directory.path())
            .await
            .expect("open users");
        let index = Index::open(None).await.expect("open index");

        Self {
            router: rhizolog::router(AppState {
                store,
                times,
                users,
                index,
                usage: rhizolog::UsageTally::new(),
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read: false,
            }),
            directory,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        let builder = Request::builder().method(method).uri(path);

        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router response");

        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");

        Res {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            location,
        }
    }

    async fn get(&self, path: &str) -> Res {
        self.send(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Res {
        self.send(Method::POST, path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: Value) -> Res {
        self.send(Method::PATCH, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> Res {
        self.send(Method::DELETE, path, None).await
    }

    /// Record an entry, asserting it worked, and hand back its id.
    async fn track(&self, body: Value) -> String {
        let res = self.post("/api/times", body).await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "track failed: {:?}",
            res.body
        );
        res.body["id"].as_str().expect("an id").to_owned()
    }

    async fn seed_page(&self, slug: &str, title: &str) {
        let res = self
            .post("/api/pages", json!({ "slug": slug, "title": title }))
            .await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "seed failed: {:?}",
            res.body
        );
    }
}

// ------------------------------------------------------------ manual entries

#[tokio::test]
async fn records_a_finished_entry() {
    let app = App::new().await;

    let res = app
        .post(
            "/api/times",
            json!({
                "name": "Deep work",
                "start": "2026-08-06T14:00:00Z",
                "end": "2026-08-06T15:30:00Z",
                "note": "Chased a lifetime error.\n",
            }),
        )
        .await;

    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["name"], "Deep work");
    assert_eq!(res.body["running"], false);
    assert_eq!(res.body["seconds"], 90 * 60);
    assert_eq!(res.body["note"], "Chased a lifetime error.\n");
    // The id is minted from the start, so the log sorts by when the time was
    // spent rather than by when it was typed in.
    assert!(
        res.body["id"]
            .as_str()
            .expect("an id")
            .starts_with("20260806T140000-"),
        "got {:?}",
        res.body["id"]
    );
    assert_eq!(
        res.location.as_deref(),
        Some(format!("/api/times/{}", res.body["id"].as_str().unwrap()).as_str())
    );
}

/// The entry lands on disk as markdown, because the log is files.
#[tokio::test]
async fn an_entry_is_a_file_in_the_wiki() {
    let app = App::new().await;
    let id = app
        .track(json!({
            "name": "Deep work",
            "start": "2026-08-06T14:00:00Z",
            "end": "2026-08-06T15:00:00Z",
            "pages": ["notes/rust/async"],
            "note": "A note.\n",
        }))
        .await;

    let path = app
        .directory
        .path()
        .join(".rhizolog")
        .join("times")
        .join("2026-08")
        .join(format!("{id}.md"));
    let text = std::fs::read_to_string(&path).expect("entry on disk");

    assert!(text.starts_with("---\n"), "got {text:?}");
    assert!(text.contains("name: Deep work"));
    assert!(text.contains("- notes/rust/async"));
    assert!(text.ends_with("A note.\n"));
}

/// A time entry must never turn up as a page: it lives under `.rhizolog/`,
/// which the page walker skips, and its slug would be unwritable anyway.
#[tokio::test]
async fn entries_do_not_appear_in_the_page_listing() {
    let app = App::new().await;
    app.track(json!({ "name": "Deep work" })).await;

    assert_eq!(app.get("/api/pages").await.body["total"], 0);
    assert_eq!(app.get("/api/search?q=Deep").await.body["total"], 0);
    assert_eq!(app.get("/api/health").await.body["pages"], 0);
    assert_eq!(app.get("/api/health").await.body["times"], 1);
}

#[tokio::test]
async fn a_range_that_runs_backwards_is_refused() {
    let app = App::new().await;

    let res = app
        .post(
            "/api/times",
            json!({
                "name": "Backwards",
                "start": "2026-08-06T15:00:00Z",
                "end": "2026-08-06T14:00:00Z",
            }),
        )
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "time_range_inverted");
}

#[tokio::test]
async fn a_nameless_entry_is_refused_because_a_name_is_the_group() {
    let app = App::new().await;

    let res = app.post("/api/times", json!({ "name": "   " })).await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid_request_body");
}

// -------------------------------------------------------------------- timers

#[tokio::test]
async fn a_timer_starts_running_and_stops_on_request() {
    let app = App::new().await;

    let started = app.post("/api/times", json!({ "name": "Deep work" })).await;
    assert_eq!(started.status, StatusCode::CREATED);
    assert_eq!(started.body["running"], true);
    assert!(started.body["end"].is_null());

    let id = started.body["id"].as_str().expect("an id");
    let stopped = app.post(&format!("/api/times/{id}/stop"), json!({})).await;

    assert_eq!(stopped.status, StatusCode::OK);
    assert_eq!(stopped.body["running"], false);
    assert!(!stopped.body["end"].is_null());
}

/// A second stop usually means a second tab got there first, and a caller that
/// could not tell would show the wrong duration.
#[tokio::test]
async fn stopping_a_stopped_timer_is_a_conflict() {
    let app = App::new().await;
    let id = app.track(json!({ "name": "Deep work" })).await;
    app.post(&format!("/api/times/{id}/stop"), json!({})).await;

    let again = app.post(&format!("/api/times/{id}/stop"), json!({})).await;

    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.code(), "time_not_running");
}

/// The whole point of allowing several: attention is not exclusive.
#[tokio::test]
async fn several_timers_run_at_once_and_may_overlap() {
    let app = App::new().await;
    for name in ["Pairing", "Listening", "Waiting on CI"] {
        app.track(json!({ "name": name, "start": "2026-08-06T14:00:00Z" }))
            .await;
    }

    let running = app.get("/api/times?running=true").await;

    assert_eq!(running.body["total"], 3);
    assert_eq!(app.get("/api/health").await.body["running_timers"], 3);
    // Identical starts are three entries, not one clobbering the others.
    let ids: Vec<&str> = running.body["times"]
        .as_array()
        .expect("times")
        .iter()
        .map(|entry| entry["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "ids collided: {ids:?}"
    );
}

// ------------------------------------------------------------------- editing

#[tokio::test]
async fn patching_leaves_the_fields_it_was_not_given() {
    let app = App::new().await;
    let id = app
        .track(json!({
            "name": "Deep work",
            "start": "2026-08-06T14:00:00Z",
            "end": "2026-08-06T15:00:00Z",
            "note": "Original.\n",
        }))
        .await;

    let patched = app
        .patch(
            &format!("/api/times/{id}"),
            json!({ "name": "Shallow work" }),
        )
        .await;

    assert_eq!(patched.status, StatusCode::OK);
    assert_eq!(patched.body["name"], "Shallow work");
    assert_eq!(patched.body["note"], "Original.\n");
    assert_eq!(patched.body["seconds"], 3600);
    assert_eq!(patched.body["id"], id, "the id must survive an edit");
}

/// `null` clears the end, which is how a timer is set running again.
#[tokio::test]
async fn clearing_the_end_restarts_the_timer() {
    let app = App::new().await;
    let id = app
        .track(json!({
            "name": "Deep work",
            "start": "2026-08-06T14:00:00Z",
            "end": "2026-08-06T15:00:00Z",
        }))
        .await;

    let patched = app
        .patch(&format!("/api/times/{id}"), json!({ "end": null }))
        .await;

    assert_eq!(patched.body["running"], true);
    assert!(patched.body["end"].is_null());
}

/// Moving the start does not move the id. An id names the entry; it is not a
/// claim about its contents, and a caller holding one must keep it working.
#[tokio::test]
async fn editing_the_start_keeps_the_id_and_the_file() {
    let app = App::new().await;
    let id = app
        .track(json!({ "name": "Deep work", "start": "2026-08-06T14:00:00Z" }))
        .await;

    let patched = app
        .patch(
            &format!("/api/times/{id}"),
            json!({ "start": "2026-08-06T09:00:00Z" }),
        )
        .await;

    assert_eq!(patched.body["id"], id);
    assert_eq!(patched.body["start"], "2026-08-06T09:00:00Z");
    assert_eq!(
        app.get(&format!("/api/times/{id}")).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn deleting_an_entry_removes_it_everywhere() {
    let app = App::new().await;
    let id = app.track(json!({ "name": "Deep work" })).await;

    assert_eq!(
        app.delete(&format!("/api/times/{id}")).await.status,
        StatusCode::NO_CONTENT
    );

    assert_eq!(
        app.get(&format!("/api/times/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.get("/api/times").await.body["total"], 0);
    assert!(
        app.get("/api/time-groups").await.body["groups"]
            .as_array()
            .expect("groups")
            .is_empty()
    );
    assert_eq!(
        app.delete(&format!("/api/times/{id}")).await.code(),
        "time_not_found"
    );
}

/// An id becomes a filesystem path, so a rejection is the whole defence.
#[tokio::test]
async fn a_malformed_id_is_refused_before_it_reaches_the_disk() {
    let app = App::new().await;

    for id in ["nonsense", "20260806T142530", "..%2F..%2Fetc%2Fpasswd"] {
        let res = app.get(&format!("/api/times/{id}")).await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{id} was accepted");
        assert_eq!(res.code(), "invalid_time_id");
    }
}

// ----------------------------------------------------------------- listing

#[tokio::test]
async fn lists_newest_first_and_filters_by_group() {
    let app = App::new().await;
    for (name, start) in [
        ("Deep work", "2026-08-06T09:00:00Z"),
        ("Email", "2026-08-06T11:00:00Z"),
        ("Deep work", "2026-08-06T14:00:00Z"),
    ] {
        app.track(json!({ "name": name, "start": start, "end": "2026-08-06T15:00:00Z" }))
            .await;
    }

    let all = app.get("/api/times").await;
    assert_eq!(all.body["total"], 3);
    assert_eq!(all.body["times"][0]["start"], "2026-08-06T14:00:00Z");

    let deep = app.get("/api/times?name=Deep%20work").await;
    assert_eq!(deep.body["total"], 2);

    let ascending = app.get("/api/times?order=asc").await;
    assert_eq!(ascending.body["times"][0]["start"], "2026-08-06T09:00:00Z");

    let nonsense = app.get("/api/times?sort=colour").await;
    assert_eq!(nonsense.status, StatusCode::BAD_REQUEST);
    assert_eq!(nonsense.code(), "invalid_parameter");
}

/// Seed a log with something written on it, for the search tests below.
async fn seed_notes(app: &App) {
    app.track(json!({
        "name": "Deep work",
        "start": "2026-08-06T09:00:00Z",
        "end": "2026-08-06T11:00:00Z",
        "pages": ["notes/rust/async"],
        "note": "Chased down a lifetime error in the poll loop.\n",
    }))
    .await;
    app.track(json!({
        "name": "Deep work",
        "start": "2026-08-05T09:00:00Z",
        "end": "2026-08-05T10:00:00Z",
        "note": "Wrote the poll loop up in the knowledge base.\n",
    }))
    .await;
    app.track(json!({
        "name": "Email",
        "start": "2026-08-06T13:00:00Z",
        "end": "2026-08-06T13:30:00Z",
        "note": "Inbox, mostly recruiters.\n",
    }))
    .await;
}

#[tokio::test]
async fn the_log_is_searchable_by_note_and_by_name() {
    let app = App::new().await;
    seed_notes(&app).await;

    let notes = app.get("/api/times?q=lifetime").await;
    assert_eq!(notes.body["total"], 1);
    assert_eq!(notes.body["times"][0]["name"], "Deep work");
    assert_eq!(
        notes.body["times"][0]["snippet"],
        "Chased down a <mark>lifetime</mark> error in the poll loop.\n"
    );

    // The name is searchable too, which is what makes this worth having on a
    // log where most entries carry no note.
    assert_eq!(app.get("/api/times?q=email").await.body["total"], 1);

    // Terms are ANDed, and a term nobody wrote finds nothing.
    assert_eq!(app.get("/api/times?q=poll%20loop").await.body["total"], 2);
    assert_eq!(
        app.get("/api/times?q=poll%20recruiters").await.body["total"],
        0
    );
    assert_eq!(app.get("/api/times?q=kubernetes").await.body["total"], 0);
}

/// The reason `q` is a filter on the listing rather than its own endpoint.
#[tokio::test]
async fn a_search_narrows_the_log_alongside_every_other_filter() {
    let app = App::new().await;
    seed_notes(&app).await;

    assert_eq!(app.get("/api/times?q=loop").await.body["total"], 2);
    assert_eq!(
        app.get("/api/times?q=loop&page=notes/rust/async")
            .await
            .body["total"],
        1
    );
    assert_eq!(
        app.get("/api/times?q=loop&name=Email").await.body["total"],
        0
    );
    assert_eq!(
        app.get("/api/times?q=loop&from=2026-08-06T00:00:00Z")
            .await
            .body["total"],
        1
    );

    // And it still reads newest first, rather than by relevance.
    let both = app.get("/api/times?q=loop").await;
    assert_eq!(both.body["times"][0]["start"], "2026-08-06T09:00:00Z");
}

/// The snippet says *why* an entry matched. When the name is the reason, the
/// name is already on screen and an excerpt of the note explains nothing.
#[tokio::test]
async fn a_snippet_is_present_only_when_the_note_is_what_matched() {
    let app = App::new().await;
    seed_notes(&app).await;

    let by_name = app.get("/api/times?q=email").await;
    assert_eq!(by_name.body["times"][0]["snippet"], Value::Null);

    let unsearched = app.get("/api/times").await;
    assert_eq!(unsearched.body["times"][0]["snippet"], Value::Null);
}

/// Notes are searched here and nowhere else: a time entry is not a page, and
/// `/api/search` would have to flatten one into the other to carry both.
#[tokio::test]
async fn time_notes_are_not_in_the_page_search() {
    let app = App::new().await;
    app.seed_page("notes/rust/async", "Async in Rust").await;
    seed_notes(&app).await;

    assert_eq!(app.get("/api/search?q=lifetime").await.body["total"], 0);
    assert_eq!(app.get("/api/times?q=lifetime").await.body["total"], 1);
}

/// Editing an entry has to replace what is searchable, not add to it — the
/// full-text table holds its own copy of the row and has no upsert.
#[tokio::test]
async fn rewriting_an_entry_replaces_what_is_searchable() {
    let app = App::new().await;
    let id = app
        .track(json!({
            "name": "Deep work",
            "start": "2026-08-06T09:00:00Z",
            "note": "Chased the poll loop.\n",
        }))
        .await;

    let res = app
        .patch(
            &format!("/api/times/{id}"),
            json!({ "name": "Email", "note": "Answered recruiters instead.\n" }),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);

    assert_eq!(app.get("/api/times?q=recruiters").await.body["total"], 1);
    assert_eq!(app.get("/api/times?q=poll").await.body["total"], 0);
    assert_eq!(app.get("/api/times?q=deep").await.body["total"], 0);

    app.delete(&format!("/api/times/{id}")).await;
    assert_eq!(app.get("/api/times?q=recruiters").await.body["total"], 0);
}

/// A search box sends whatever was typed into it, and none of it may be read
/// as FTS5 syntax.
#[tokio::test]
async fn punctuation_in_a_search_never_errors() {
    let app = App::new().await;
    seed_notes(&app).await;

    for query in [
        "%22",
        "*",
        "(",
        "AND",
        "OR",
        "NEAR",
        "%5E",
        "-",
        "a%20OR%20b",
        "%22unclosed",
        "",
    ] {
        let res = app.get(&format!("/api/times?q={query}")).await;
        assert_eq!(
            res.status,
            StatusCode::OK,
            "q={query:?} failed: {:?}",
            res.body
        );
    }

    // A query with no terms in it matches nothing, rather than quietly
    // becoming no filter and handing back the whole log.
    assert_eq!(app.get("/api/times?q=").await.body["total"], 0);
    assert_eq!(app.get("/api/times").await.body["total"], 3);
}

/// A window admits what overlaps it, not only what starts inside it.
#[tokio::test]
async fn a_window_admits_a_session_that_began_before_it() {
    let app = App::new().await;
    app.track(json!({
        "name": "Overnight",
        "start": "2026-08-05T22:00:00Z",
        "end": "2026-08-06T02:00:00Z",
    }))
    .await;
    app.track(json!({
        "name": "Yesterday",
        "start": "2026-08-05T09:00:00Z",
        "end": "2026-08-05T10:00:00Z",
    }))
    .await;

    let today = app
        .get("/api/times?from=2026-08-06T00:00:00Z&to=2026-08-07T00:00:00Z")
        .await;

    assert_eq!(today.body["total"], 1);
    assert_eq!(today.body["times"][0]["name"], "Overnight");
}

#[tokio::test]
async fn groups_are_the_names_and_nothing_else() {
    let app = App::new().await;
    for (name, start, end) in [
        ("Deep work", "2026-08-06T09:00:00Z", "2026-08-06T11:00:00Z"),
        ("Deep work", "2026-08-06T14:00:00Z", "2026-08-06T15:00:00Z"),
        ("Email", "2026-08-06T11:00:00Z", "2026-08-06T11:30:00Z"),
    ] {
        app.track(json!({ "name": name, "start": start, "end": end }))
            .await;
    }

    let res = app.get("/api/time-groups").await;

    assert_eq!(res.body["groups"][0]["name"], "Deep work");
    assert_eq!(res.body["groups"][0]["entries"], 2);
    assert_eq!(res.body["groups"][0]["seconds"], 3 * 3600);
    assert_eq!(res.body["groups"][1]["name"], "Email");
    assert_eq!(res.body["totals"]["groups"], 2);
    assert_eq!(res.body["totals"]["entries"], 3);
}

/// Names are not normalised, exactly as tags are not.
#[tokio::test]
async fn two_spellings_are_two_groups() {
    let app = App::new().await;
    app.track(json!({ "name": "Deep work" })).await;
    app.track(json!({ "name": "deep work", "start": "2026-08-06T09:00:00Z" }))
        .await;

    assert_eq!(
        app.get("/api/time-groups").await.body["totals"]["groups"],
        2
    );
}

// ------------------------------------------------------- the link to a page

/// The whole reason a time link is not an ordinary link: a page collects
/// hundreds of them, and they must not drown its backlinks.
#[tokio::test]
async fn time_attached_to_a_page_is_summarised_not_listed_as_backlinks() {
    let app = App::new().await;
    app.seed_page("notes/rust/async", "Async in Rust").await;
    for hour in 9..12 {
        app.track(json!({
            "name": "Deep work",
            "start": format!("2026-08-06T{hour:02}:00:00Z"),
            "end": format!("2026-08-06T{:02}:00:00Z", hour + 1),
            "pages": ["notes/rust/async"],
        }))
        .await;
    }

    let links = app.get("/api/links/notes/rust/async").await;

    assert_eq!(links.status, StatusCode::OK);
    assert_eq!(links.body["times"]["entries"], 3);
    assert_eq!(links.body["times"]["seconds"], 3 * 3600);
    assert_eq!(links.body["times"]["groups"], 1);
    assert_eq!(links.body["times"]["recent"][0]["name"], "Deep work");
    // Not one of them is a backlink, and none of them counts as a link.
    assert!(
        links.body["inbound"]
            .as_array()
            .expect("inbound")
            .is_empty(),
        "time entries leaked into the link graph"
    );
    let stats = app.get("/api/stats").await;
    assert_eq!(stats.body["links"]["internal"], 0);
    assert_eq!(
        stats.body["orphan_count"], 1,
        "tracked time must not stop a page being an orphan"
    );
}

/// Attachment is a query, not a cache — the same property the link graph has.
#[tokio::test]
async fn time_can_be_tracked_against_a_page_that_does_not_exist_yet() {
    let app = App::new().await;
    let id = app
        .track(json!({ "name": "Deep work", "pages": ["notes/later"] }))
        .await;

    let before = app.get(&format!("/api/times/{id}")).await;
    assert_eq!(before.body["pages"][0]["exists"], false);
    assert!(before.body["pages"][0]["title"].is_null());

    // Only the page is written. Nothing touches the entry.
    app.seed_page("notes/later", "Later").await;

    let after = app.get(&format!("/api/times/{id}")).await;
    assert_eq!(after.body["pages"][0]["exists"], true);
    assert_eq!(after.body["pages"][0]["title"], "Later");
}

#[tokio::test]
async fn entries_can_be_listed_by_the_page_they_were_spent_on() {
    let app = App::new().await;
    app.track(json!({ "name": "Deep work", "pages": ["notes/a"] }))
        .await;
    app.track(json!({
        "name": "Email",
        "start": "2026-08-06T09:00:00Z",
        "pages": ["notes/b"],
    }))
    .await;

    let res = app.get("/api/times?page=notes/a").await;

    assert_eq!(res.body["total"], 1);
    assert_eq!(res.body["times"][0]["name"], "Deep work");
}

/// A note is markdown like anything else, wikilinks included.
#[tokio::test]
async fn a_note_renders_on_request() {
    let app = App::new().await;
    let id = app
        .track(json!({
            "name": "Deep work",
            "note": "Chased [[notes/rust/async]] down.\n",
        }))
        .await;

    let res = app.get(&format!("/api/times/{id}?render=true")).await;

    assert!(
        res.body["html"]
            .as_str()
            .expect("html")
            .contains(r#"href="/pages/notes/rust/async""#),
        "got {:?}",
        res.body["html"]
    );
}

// -------------------------------------------------------------- statistics

#[tokio::test]
async fn statistics_bucket_the_day_in_the_callers_offset() {
    let app = App::new().await;
    // 08:00-10:00 UTC is 01:00-03:00 in UTC-7.
    app.track(json!({
        "name": "Deep work",
        "start": "2026-08-06T08:00:00Z",
        "end": "2026-08-06T10:00:00Z",
    }))
    .await;

    let res = app
        .get("/api/time-stats?offset=-420&at=2026-08-06T20:00:00Z")
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["offset_minutes"], -420);
    assert_eq!(res.body["all_time"]["seconds"], 2 * 3600);

    let day = &res.body["periods"][0];
    assert_eq!(day["period"], "day");
    assert_eq!(day["seconds"], 2 * 3600);
    assert_eq!(day["names"][0]["name"], "Deep work");
    assert_eq!(day["buckets"].as_array().expect("buckets").len(), 24);
    assert_eq!(day["buckets"][1]["seconds"], 3600, "local 01:00");
    assert_eq!(day["buckets"][2]["seconds"], 3600, "local 02:00");

    let periods: Vec<&str> = res.body["periods"]
        .as_array()
        .expect("periods")
        .iter()
        .map(|period| period["period"].as_str().expect("a name"))
        .collect();
    assert_eq!(periods, ["day", "week", "month", "year"]);
}

/// The reason the buckets are computed in Rust: an overnight session belongs to
/// both days, and to every hour it touched.
#[tokio::test]
async fn an_overnight_session_is_split_across_days_and_hours() {
    let app = App::new().await;
    app.track(json!({
        "name": "Night",
        "start": "2026-08-06T22:30:00Z",
        "end": "2026-08-07T01:00:00Z",
    }))
    .await;

    let res = app.get("/api/time-stats?at=2026-08-07T12:00:00Z").await;

    let day = &res.body["periods"][0];
    assert_eq!(day["seconds"], 3600, "only the 7th's share");
    assert_eq!(
        res.body["periods"][1]["seconds"],
        150 * 60,
        "the whole week"
    );

    let lit: Vec<(u64, u64, u64)> = res.body["heatmap"]["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .filter(|cell| cell["seconds"].as_u64() != Some(0))
        .map(|cell| {
            (
                cell["weekday"].as_u64().expect("weekday"),
                cell["hour"].as_u64().expect("hour"),
                cell["seconds"].as_u64().expect("seconds"),
            )
        })
        .collect();
    // 2026-08-06 is a Thursday, weekday 3 with Monday at 0.
    assert_eq!(lit, [(3, 22, 1800), (3, 23, 3600), (4, 0, 3600)]);
}

#[tokio::test]
async fn statistics_rank_the_pages_the_time_went_to() {
    let app = App::new().await;
    app.seed_page("notes/a", "A").await;
    app.track(json!({
        "name": "Deep work",
        "start": "2026-08-06T08:00:00Z",
        "end": "2026-08-06T10:00:00Z",
        "pages": ["notes/a"],
    }))
    .await;

    let res = app.get("/api/time-stats?at=2026-08-06T20:00:00Z").await;

    let day = &res.body["periods"][0];
    assert_eq!(day["pages"][0]["slug"], "notes/a");
    assert_eq!(day["pages"][0]["title"], "A");
    assert_eq!(day["pages"][0]["seconds"], 2 * 3600);
}

#[tokio::test]
async fn an_empty_log_still_answers_with_a_full_grid() {
    let app = App::new().await;

    let res = app.get("/api/time-stats").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["all_time"]["entries"], 0);
    assert!(res.body["all_time"]["first_start"].is_null());
    assert_eq!(
        res.body["heatmap"]["cells"]
            .as_array()
            .expect("cells")
            .len(),
        168
    );
    for period in res.body["periods"].as_array().expect("periods") {
        assert_eq!(period["seconds"], 0);
    }
}

/// A wild offset is clamped rather than answered with a panic or a 500.
#[tokio::test]
async fn a_nonsense_offset_is_clamped() {
    let app = App::new().await;

    let res = app.get("/api/time-stats?offset=999999").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["offset_minutes"], 24 * 60);
}

// ------------------------------------------------------------------- the log

/// The log is derived into the index and rebuilt from disk, exactly as pages
/// are — and a rebuild must not lose a single entry.
#[tokio::test]
async fn reindexing_rebuilds_the_log_from_its_files() {
    let app = App::new().await;
    for hour in 9..12 {
        app.track(json!({
            "name": "Deep work",
            "start": format!("2026-08-06T{hour:02}:00:00Z"),
            "end": format!("2026-08-06T{:02}:00:00Z", hour + 1),
        }))
        .await;
    }

    let res = app.post("/api/reindex", json!({})).await;

    assert_eq!(res.body["times"]["scanned"], 3);
    assert_eq!(res.body["times"]["indexed"], 3);
    assert_eq!(res.body["times"]["failed"], 0);
    assert_eq!(app.get("/api/times").await.body["total"], 3);
    assert_eq!(
        app.get("/api/time-groups").await.body["groups"][0]["seconds"],
        3 * 3600
    );
}

/// An entry written into the log by hand is as real as one made through the
/// API. This is the same claim `files are the source of truth` makes for pages.
#[tokio::test]
async fn an_entry_written_by_hand_is_picked_up_by_a_reindex() {
    let app = App::new().await;
    let month = app
        .directory
        .path()
        .join(".rhizolog")
        .join("times")
        .join("2026-08");
    std::fs::create_dir_all(&month).expect("month directory");
    std::fs::write(
        month.join("20260806T090000-000000000.md"),
        "---\nname: By hand\nstart: 2026-08-06T09:00:00Z\nend: 2026-08-06T10:00:00Z\n---\n\nTyped straight into the file.\n",
    )
    .expect("hand-written entry");

    app.post("/api/reindex", json!({})).await;

    let res = app.get("/api/times/20260806T090000-000000000").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], "By hand");
    assert_eq!(res.body["seconds"], 3600);
    assert_eq!(res.body["note"], "\nTyped straight into the file.\n");
}
