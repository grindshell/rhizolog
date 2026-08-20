//! Idea Inbox over the assembled router.
//!
//! Two wikis are driven here and the difference between them is the point. One
//! has no accounts and is the single open user; the other has two accounts, and
//! what each of them can see of the other is most of what this file is about.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use rhizolog::{AppState, Assets, IdeaService, IdeaStore, Index, Store, TimeStore, UserStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct App {
    _directory: TempDir,
    router: Router,
    /// A bearer token, on a wiki with accounts.
    token: Option<String>,
}

struct Res {
    status: StatusCode,
    body: Value,
}

impl Res {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("<no code>")
    }
}

impl App {
    /// A wiki with no accounts: every request is the one open user.
    async fn open() -> Self {
        let directory = TempDir::new().expect("temp dir");
        let router = router_over(directory.path()).await;
        Self {
            _directory: directory,
            router,
            token: None,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        self.send_as(self.token.as_deref(), method, path, body)
            .await
    }

    async fn send_as(
        &self,
        token: Option<&str>,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Res {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }

        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        Res { status, body }
    }

    async fn get(&self, path: &str) -> Res {
        self.send(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Option<Value>) -> Res {
        self.send(Method::POST, path, body).await
    }

    async fn capture(&self, text: &str) -> String {
        let res = self
            .post("/api/captures", Some(json!({ "text": text })))
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        res.body["id"].as_str().expect("id").to_owned()
    }

    async fn start_idea(&self, name: &str, captures: &[&str]) -> String {
        let res = self
            .post(
                "/api/ideas",
                Some(json!({ "name": name, "captures": captures })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        res.body["id"].as_str().expect("id").to_owned()
    }
}

async fn router_over(root: &std::path::Path) -> Router {
    let store = Store::open(root).await.expect("open store");
    let times = TimeStore::open(root).await.expect("open time log");
    let ideas = IdeaStore::open(root).await.expect("open ideas");
    let users = UserStore::open(root).await.expect("open users");
    let index = Index::open(None).await.expect("open index");

    rhizolog::router(AppState {
        store,
        times,
        ideas: IdeaService::new(ideas),
        users,
        index,
        usage: rhizolog::UsageTally::new(),
        assets: Assets::None,
        secure_cookies: false,
        anonymous_read: false,
    })
}

#[tokio::test]
async fn captures_text_and_reads_it_back() {
    let app = App::open().await;

    let res = app
        .post(
            "/api/captures",
            Some(json!({ "text": "Dungeon quests should require seeds.\n" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["text"], "Dungeon quests should require seeds.\n");
    assert_eq!(res.body["archived"], false);

    let id = res.body["id"].as_str().expect("id");
    let read = app.get(&format!("/api/captures/{id}")).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["text"], "Dungeon quests should require seeds.\n");

    let list = app.get("/api/captures").await;
    assert_eq!(list.body["total"], 1);
    assert_eq!(list.body["captures"][0]["id"], id);
}

/// The one thing a capture is refused over, and nothing reaches the disk.
#[tokio::test]
async fn a_capture_with_no_text_is_refused() {
    let app = App::open().await;

    let res = app
        .post("/api/captures", Some(json!({ "text": "   " })))
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "capture_empty");
    assert_eq!(app.get("/api/captures").await.body["total"], 0);
}

#[tokio::test]
async fn the_inbox_can_be_searched_and_narrowed_without_being_reordered() {
    let app = App::open().await;
    app.capture("Dungeon seeds decide the loot.\n").await;
    app.capture("Something else entirely.\n").await;
    let newest = app.capture("Seeds again, and dungeon corridors.\n").await;

    let all = app.get("/api/captures").await;
    assert_eq!(all.body["total"], 3);
    // Newest first, and searching keeps that order rather than ranking.
    assert_eq!(all.body["captures"][0]["id"], newest);

    let found = app.get("/api/captures?q=dungeon").await;
    assert_eq!(found.body["total"], 2);
    assert_eq!(found.body["captures"][0]["id"], newest);

    // An empty search is no search, not no results.
    assert_eq!(app.get("/api/captures?q=").await.body["total"], 3);
}

#[tokio::test]
async fn archiving_takes_a_capture_out_of_the_inbox_and_restoring_puts_it_back() {
    let app = App::open().await;
    let id = app.capture("A thought.\n").await;

    let archived = app.post(&format!("/api/captures/{id}/archive"), None).await;
    assert_eq!(archived.status, StatusCode::OK);
    assert_eq!(archived.body["archived"], true);
    assert_eq!(
        app.get("/api/captures?archived=false").await.body["total"],
        0
    );
    assert_eq!(
        app.get("/api/captures?archived=true").await.body["total"],
        1
    );
    // Absent means both: an archived capture has not stopped existing.
    assert_eq!(app.get("/api/captures").await.body["total"], 1);

    // Archiving again writes no second decision.
    app.post(&format!("/api/captures/{id}/archive"), None).await;

    let restored = app.post(&format!("/api/captures/{id}/restore"), None).await;
    assert_eq!(restored.body["archived"], false);
    assert_eq!(
        app.get("/api/captures?archived=false").await.body["total"],
        1
    );
}

#[tokio::test]
async fn an_idea_holds_its_seeds_and_can_be_renamed() {
    let app = App::open().await;
    let first = app.capture("Dungeon seeds.\n").await;
    let second = app.capture("Seeded loot tables.\n").await;

    let idea = app.start_idea("Dungeon seeds", &[&first, &second]).await;

    let read = app.get(&format!("/api/ideas/{idea}")).await;
    assert_eq!(read.body["name"], "Dungeon seeds");
    assert_eq!(read.body["captures"].as_array().unwrap().len(), 2);
    assert_eq!(read.body["retired"], false);
    assert_eq!(read.body["needs_repair"], false);

    let renamed = app
        .send(
            Method::PATCH,
            &format!("/api/ideas/{idea}"),
            Some(json!({ "name": "Seeded dungeons" })),
        )
        .await;
    assert_eq!(renamed.body["name"], "Seeded dungeons");
    assert_eq!(renamed.body["captures"].as_array().unwrap().len(), 2);

    let list = app.get("/api/ideas").await;
    assert_eq!(list.body["total"], 1);
    assert_eq!(list.body["ideas"][0]["captures"], 2);
}

#[tokio::test]
async fn an_idea_needs_a_name_and_a_capture_that_exists() {
    let app = App::open().await;
    let capture = app.capture("A thought.\n").await;

    let unnamed = app
        .post(
            "/api/ideas",
            Some(json!({ "name": "  ", "captures": [&capture] })),
        )
        .await;
    assert_eq!(unnamed.status, StatusCode::BAD_REQUEST);
    assert_eq!(unnamed.code(), "idea_name_empty");

    let seedless = app
        .post(
            "/api/ideas",
            Some(json!({ "name": "Nothing", "captures": [] })),
        )
        .await;
    assert_eq!(seedless.code(), "idea_no_seeds");

    let missing = app
        .post(
            "/api/ideas",
            Some(json!({ "name": "Nothing", "captures": ["20200101T000000-000000000"] })),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.code(), "capture_not_found");

    assert_eq!(app.get("/api/ideas").await.body["total"], 0);
}

/// The state being asked for is in the URL, so repeating yourself changes
/// nothing and writes no second decision.
#[tokio::test]
async fn connecting_and_disconnecting_are_idempotent() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let loose = app.capture("Seeded loot tables.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;

    let path = format!("/api/ideas/{idea}/captures/{loose}");
    for _ in 0..2 {
        let connected = app.send(Method::PUT, &path, None).await;
        assert_eq!(connected.status, StatusCode::OK);
        assert_eq!(connected.body["captures"].as_array().unwrap().len(), 2);
    }

    for _ in 0..2 {
        let disconnected = app.send(Method::DELETE, &path, None).await;
        assert_eq!(disconnected.status, StatusCode::OK);
        assert_eq!(disconnected.body["captures"].as_array().unwrap().len(), 1);
    }
}

/// An idea with nothing connected has no authored evidence to derive anything
/// from, so the last capture cannot simply be taken away.
#[tokio::test]
async fn an_idea_cannot_be_emptied() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;

    let res = app
        .send(
            Method::DELETE,
            &format!("/api/ideas/{idea}/captures/{seed}"),
            None,
        )
        .await;

    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.code(), "idea_would_be_empty");
    assert_eq!(
        app.get(&format!("/api/ideas/{idea}")).await.body["captures"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_candidate_can_be_rejected_and_reconsidered() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let other = app.capture("Unrelated.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;
    let path = format!("/api/ideas/{idea}/rejections/{other}");

    let rejected = app.send(Method::PUT, &path, None).await;
    assert_eq!(rejected.body["rejected"][0], other);

    let reconsidered = app.send(Method::DELETE, &path, None).await;
    assert_eq!(reconsidered.body["rejected"].as_array().unwrap().len(), 0);
}

/// A pair is one decision whichever capture produced the suggestion.
#[tokio::test]
async fn a_capture_pair_rejection_has_one_identity() {
    let app = App::open().await;
    let first = app.capture("Dungeon seeds.\n").await;
    let second = app.capture("Seeded loot.\n").await;

    let rejected = app
        .send(
            Method::PUT,
            &format!("/api/captures/{first}/rejections/{second}"),
            None,
        )
        .await;
    assert_eq!(rejected.status, StatusCode::NO_CONTENT);

    // The same pair from the other side, reconsidered.
    let reconsidered = app
        .send(
            Method::DELETE,
            &format!("/api/captures/{second}/rejections/{first}"),
            None,
        )
        .await;
    assert_eq!(reconsidered.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn retiring_and_reopening_are_reversible_and_refuse_to_repeat() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;

    let retired = app.post(&format!("/api/ideas/{idea}/retire"), None).await;
    assert_eq!(retired.body["retired"], true);

    let again = app.post(&format!("/api/ideas/{idea}/retire"), None).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.code(), "idea_already_retired");

    let reopened = app.post(&format!("/api/ideas/{idea}/reopen"), None).await;
    assert_eq!(reopened.body["retired"], false);

    let not_retired = app.post(&format!("/api/ideas/{idea}/reopen"), None).await;
    assert_eq!(not_retired.code(), "idea_not_retired");
}

/// Affirming is an act rather than a state, so it means something every time.
#[tokio::test]
async fn affirming_moves_the_last_signal_every_time() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;

    let first = app.post(&format!("/api/ideas/{idea}/affirm"), None).await;
    assert_eq!(first.status, StatusCode::OK);
    let after_first = first.body["last_signal"].clone();

    let second = app.post(&format!("/api/ideas/{idea}/affirm"), None).await;
    assert!(second.body["last_signal"].as_str() >= after_first.as_str());

    // Dismissing is its own thing and does not retire anything.
    let dismissed = app.post(&format!("/api/ideas/{idea}/dismiss"), None).await;
    assert_eq!(dismissed.body["retired"], false);
}

/// Deleting a capture an idea has nothing else to stand on is refused, and the
/// refusal names the thread so a caller can say which one.
#[tokio::test]
async fn a_capture_an_idea_depends_on_cannot_be_deleted() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let spare = app.capture("Seeded loot.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;

    let refused = app
        .send(Method::DELETE, &format!("/api/captures/{seed}"), None)
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.code(), "capture_required_by_idea");
    assert_eq!(refused.body["error"]["details"]["name"], "Dungeon seeds");

    // Connect another and the deletion becomes possible.
    app.send(
        Method::PUT,
        &format!("/api/ideas/{idea}/captures/{spare}"),
        None,
    )
    .await;
    let deleted = app
        .send(Method::DELETE, &format!("/api/captures/{seed}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.body["ideas"][0]["name"], "Dungeon seeds");
    assert_eq!(deleted.body["ideas"][0]["needs_repair"], false);

    assert_eq!(
        app.get(&format!("/api/captures/{seed}")).await.status,
        StatusCode::NOT_FOUND
    );
    let idea_now = app.get(&format!("/api/ideas/{idea}")).await;
    assert_eq!(idea_now.body["captures"].as_array().unwrap().len(), 1);
    // The membership the deleted capture had is named rather than forgotten.
    assert_eq!(idea_now.body["missing"][0], seed);
}

/// The gate on phase I2: everything an API client does has to survive the
/// derived half being thrown away and rebuilt from the files.
#[tokio::test]
async fn the_whole_loop_survives_a_rebuild() {
    let app = App::open().await;
    let first = app.capture("Dungeon seeds.\n").await;
    let second = app.capture("Seeded loot tables.\n").await;
    let third = app.capture("Unrelated.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&first]).await;

    app.send(
        Method::PUT,
        &format!("/api/ideas/{idea}/captures/{second}"),
        None,
    )
    .await;
    app.send(
        Method::PUT,
        &format!("/api/ideas/{idea}/rejections/{third}"),
        None,
    )
    .await;
    app.post(&format!("/api/captures/{second}/archive"), None)
        .await;
    app.post(&format!("/api/ideas/{idea}/retire"), None).await;
    app.post(&format!("/api/ideas/{idea}/reopen"), None).await;

    let before = app.get(&format!("/api/ideas/{idea}")).await.body;
    let captures_before = app.get("/api/captures").await.body;

    let rebuilt = app.post("/api/reindex", None).await;
    assert_eq!(rebuilt.status, StatusCode::OK);
    assert_eq!(rebuilt.body["captures"]["indexed"], 3);
    assert_eq!(rebuilt.body["ideas"]["indexed"], 1);

    assert_eq!(app.get(&format!("/api/ideas/{idea}")).await.body, before);
    assert_eq!(app.get("/api/captures").await.body, captures_before);
}

// -------------------------------------------------- wikis that have accounts

/// A wiki with two accounts, and a session token for each.
async fn with_two_accounts() -> (App, String, String) {
    let directory = TempDir::new().expect("temp dir");
    let app = App {
        router: router_over(directory.path()).await,
        _directory: directory,
        token: None,
    };

    // The first account is the bootstrap and needs nobody's permission. Every
    // account after it does, which is the door closing behind the first.
    let tim = sign_up(&app, None, "tim").await;
    let alice = sign_up(&app, Some(&tim), "alice").await;

    (app, tim, alice)
}

async fn sign_up(app: &App, as_owner: Option<&str>, name: &str) -> String {
    let created = app
        .send_as(
            as_owner,
            Method::POST,
            "/api/users",
            Some(json!({ "username": name, "password": "correct horse battery" })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);

    let session = app
        .post(
            "/api/auth/login",
            Some(json!({ "username": name, "password": "correct horse battery" })),
        )
        .await;
    assert_eq!(session.status, StatusCode::OK, "{:?}", session.body);

    session.body["token"].as_str().expect("token").to_owned()
}

/// The disclosure rule, end to end: somebody else's capture is missing, not
/// forbidden, and the response says nothing that would tell the two apart.
#[tokio::test]
async fn one_account_sees_nothing_of_another() {
    let (app, tim, alice) = with_two_accounts().await;

    let created = app
        .send_as(
            Some(&tim),
            Method::POST,
            "/api/captures",
            Some(json!({ "text": "Dungeon seeds.\n" })),
        )
        .await;
    let id = created.body["id"].as_str().expect("id").to_owned();

    let hers = app
        .send_as(
            Some(&alice),
            Method::GET,
            &format!("/api/captures/{id}"),
            None,
        )
        .await;
    assert_eq!(hers.status, StatusCode::NOT_FOUND);
    assert_eq!(hers.code(), "capture_not_found");

    // Word for word what an id that never existed gets.
    let never = app
        .send_as(
            Some(&alice),
            Method::GET,
            "/api/captures/20200101T000000-000000000",
            None,
        )
        .await;
    assert_eq!(never.code(), hers.code());
    assert_eq!(never.status, hers.status);

    assert_eq!(
        app.send_as(Some(&alice), Method::GET, "/api/captures", None)
            .await
            .body["total"],
        0
    );
    assert_eq!(
        app.send_as(Some(&tim), Method::GET, "/api/captures", None)
            .await
            .body["total"],
        1
    );
}

/// Not one idea route is reachable without an account, and that has to hold
/// however the instance is configured.
#[tokio::test]
async fn no_idea_route_answers_an_anonymous_caller() {
    let (app, _tim, _alice) = with_two_accounts().await;

    for (method, path) in [
        (Method::GET, "/api/captures"),
        (Method::POST, "/api/captures"),
        (Method::GET, "/api/captures/20200101T000000-000000000"),
        (Method::GET, "/api/ideas"),
        (Method::POST, "/api/ideas"),
        (Method::GET, "/api/ideas/20200101T000000-000000000"),
        (Method::POST, "/api/ideas/20200101T000000-000000000/retire"),
    ] {
        let res = app.send_as(None, method.clone(), path, None).await;
        assert_eq!(
            res.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} answered a stranger"
        );
    }
}

/// The day adoption exists for: a wiki full of captures gains its first
/// account, and the person who wrote them keeps them.
#[tokio::test]
async fn the_first_account_inherits_the_open_users_inbox() {
    let directory = TempDir::new().expect("temp dir");
    let app = App {
        router: router_over(directory.path()).await,
        _directory: directory,
        token: None,
    };

    let capture = app.capture("Dungeon seeds.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&capture]).await;
    app.post(&format!("/api/ideas/{idea}/affirm"), None).await;

    let created = app
        .post(
            "/api/users",
            Some(json!({ "username": "tim", "password": "correct horse battery" })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);

    let session = app
        .post(
            "/api/auth/login",
            Some(json!({ "username": "tim", "password": "correct horse battery" })),
        )
        .await;
    let token = session.body["token"].as_str().expect("token").to_owned();

    // No restart, no reindex: the inbox is there on the very next request.
    let inbox = app
        .send_as(Some(&token), Method::GET, "/api/captures", None)
        .await;
    assert_eq!(inbox.body["total"], 1, "{:?}", inbox.body);
    assert_eq!(inbox.body["captures"][0]["id"], capture);

    let read = app
        .send_as(
            Some(&token),
            Method::GET,
            &format!("/api/ideas/{idea}"),
            None,
        )
        .await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["name"], "Dungeon seeds");
    assert_eq!(read.body["captures"].as_array().unwrap().len(), 1);
}
