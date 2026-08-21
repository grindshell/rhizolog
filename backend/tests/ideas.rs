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

    fn root(&self) -> &std::path::Path {
        self._directory.path()
    }

    async fn get(&self, path: &str) -> Res {
        self.send(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Option<Value>) -> Res {
        self.send(Method::POST, path, body).await
    }

    async fn put(&self, path: &str, body: Option<Value>) -> Res {
        self.send(Method::PUT, path, body).await
    }

    /// How many decision events are on disk.
    ///
    /// What proves an idempotent route is idempotent: a repeated request has to
    /// leave the authored tree exactly as it found it, and only the files can
    /// say so. A response that looks the same would look the same either way.
    fn events(&self) -> usize {
        let mut count = 0;
        let mut pending = vec![self.root().join(".rhizolog/ideas/events")];

        while let Some(directory) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|suffix| suffix == "md") {
                    count += 1;
                }
            }
        }

        count
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

/// Rediscovery decides what to offer today, and it needs to know what it was
/// told to stop offering. A dismissal is the one decision nothing folds into
/// anything, so the listing carries when it happened and the caller applies the
/// thirty days.
#[tokio::test]
async fn a_dismissal_is_visible_to_whoever_chooses_what_to_resurface() {
    let app = App::open().await;
    let seed = app.capture("Dungeon seeds.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&seed]).await;

    let before = app.get("/api/ideas").await;
    assert_eq!(before.body["ideas"][0]["dismissed"], Value::Null);

    app.post(&format!("/api/ideas/{idea}/dismiss"), None).await;

    let after = app.get("/api/ideas").await;
    let first = after.body["ideas"][0]["dismissed"]
        .as_str()
        .expect("a dismissal on the listing")
        .to_owned();

    // And on the idea itself, where the detail view says why no card appeared.
    let detail = app.get(&format!("/api/ideas/{idea}")).await;
    assert_eq!(detail.body["dismissed"], first);

    // Dismissing again moves it, because the thirty days start over.
    app.post(&format!("/api/ideas/{idea}/dismiss"), None).await;
    let again = app.get("/api/ideas").await;
    assert!(again.body["ideas"][0]["dismissed"].as_str() >= Some(first.as_str()));

    // It is not a lifecycle input. Looking away from a thought does not change
    // what the thought is worth.
    assert_eq!(
        again.body["ideas"][0]["momentum"],
        before.body["ideas"][0]["momentum"]
    );
    assert_eq!(
        again.body["ideas"][0]["state"],
        before.body["ideas"][0]["state"]
    );
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

    let before = timeless(app.get(&format!("/api/ideas/{idea}")).await.body);
    let captures_before = app.get("/api/captures").await.body;

    let rebuilt = app.post("/api/reindex", None).await;
    assert_eq!(rebuilt.status, StatusCode::OK);
    assert_eq!(rebuilt.body["captures"]["indexed"], 3);
    assert_eq!(rebuilt.body["ideas"]["indexed"], 1);

    // `state` and `momentum` are inside this comparison, so the rebuild has to
    // reproduce the lifecycle answer and not merely the folded rows.
    assert_eq!(
        timeless(app.get(&format!("/api/ideas/{idea}")).await.body),
        before
    );
    assert_eq!(app.get("/api/captures").await.body, captures_before);
}

/// Drop `computed_at`, the one field in an idea that is a function of when you
/// asked rather than of what is on disk.
///
/// Everything else has to come back identical after a rebuild, this one cannot,
/// and that is the whole point of not storing it.
fn timeless(mut body: Value) -> Value {
    if let Some(object) = body.as_object_mut() {
        object.remove("computed_at");
    }
    body
}

// --------------------------------------------- candidates and the lifecycle

/// One thought written three times, which is what the whole feature exists to
/// notice.
async fn a_recurring_thought() -> (App, String, String, String) {
    let app = App::open().await;
    let first = app.capture("Dungeon seeds should decide the loot.\n").await;
    let second = app
        .capture("Dungeon seeds should decide the layout.\n")
        .await;
    let third = app
        .capture("Dungeon seeds should decide the rooms.\n")
        .await;
    (app, first, second, third)
}

fn number(value: &Value) -> f64 {
    value.as_f64().expect("a number")
}

/// Six decimal places, which is what the API rounds to.
fn near(found: f64, expected: f64) {
    assert!(
        (found - expected).abs() < 2e-6,
        "expected {expected}, found {found}"
    );
}

/// The gate on phase I3, for candidates: every number in the response can be
/// checked against the other numbers in the response.
#[tokio::test]
async fn a_candidate_carries_the_arithmetic_that_produced_it() {
    let (app, _first, _second, third) = a_recurring_thought().await;

    let res = app.get(&format!("/api/captures/{third}/candidates")).await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);

    assert_eq!(res.body["analyzer"], "tfidf/v1");
    assert_eq!(res.body["capture"], third);
    assert_eq!(
        res.body["corpus"], 3,
        "the owner's captures, and only those"
    );
    // Six unigrams and the five bigrams between them.
    assert_eq!(res.body["terms"], 11);
    near(number(&res.body["threshold"]), 0.35);

    let candidates = res.body["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 2, "{:#?}", res.body);

    for candidate in candidates {
        assert_eq!(candidate["kind"], "capture");
        assert!(candidate["capture"]["text"].is_string());
        assert!(candidate["idea"].is_null());

        let similarity = number(&candidate["similarity"]);
        assert!(similarity >= 0.35, "{similarity} is below the threshold");
        assert!(similarity <= 1.0);

        let signals = candidate["signals"].as_array().expect("signals");
        assert!(!signals.is_empty());
        assert!(signals.len() <= 5);

        // Each contribution is the product of the two weights beside it, the
        // signals are ordered by it, and they add up to what the response says
        // they add up to.
        let mut running = 0.0;
        let mut previous = f64::INFINITY;
        for signal in signals {
            let contribution = number(&signal["contribution"]);
            near(
                contribution,
                number(&signal["capture_weight"]) * number(&signal["target_weight"]),
            );
            assert!(contribution <= previous, "{signals:#?} is out of order");
            assert!(!signal["term"].as_str().expect("a term").is_empty());
            assert!(signal["documents"].as_u64().expect("a count") >= 1);
            previous = contribution;
            running += contribution;
        }

        near(number(&candidate["explained"]), running);
        assert!(
            number(&candidate["explained"]) <= similarity + 2e-6,
            "five signals cannot explain more than the whole score"
        );
    }
}

/// Every signal is a term appearing literally in both captures, because the
/// receipt says "these words are why" and a stem is not a word anybody wrote.
#[tokio::test]
async fn every_signal_is_text_that_appears_in_both_captures() {
    let (app, _first, _second, third) = a_recurring_thought().await;

    let res = app.get(&format!("/api/captures/{third}/candidates")).await;
    let asking = app.get(&format!("/api/captures/{third}")).await.body["text"]
        .as_str()
        .expect("text")
        .to_lowercase();

    for candidate in res.body["candidates"].as_array().expect("candidates") {
        let target = candidate["capture"]["text"]
            .as_str()
            .expect("text")
            .to_lowercase();
        for signal in candidate["signals"].as_array().expect("signals") {
            let term = signal["term"].as_str().expect("a term");
            assert!(asking.contains(term), "{term:?} is not in the capture");
            assert!(target.contains(term), "{term:?} is not in the target");
        }
    }
}

/// Nothing to match on is a different answer from nothing matched, and the
/// response says which it is.
#[tokio::test]
async fn a_capture_with_no_terms_says_so_rather_than_failing() {
    let app = App::open().await;
    app.capture("Dungeon seeds should decide the loot.\n").await;
    let punctuation = app.capture("...\n").await;

    let res = app
        .get(&format!("/api/captures/{punctuation}/candidates"))
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["terms"], 0);
    assert_eq!(res.body["candidates"].as_array().expect("empty").len(), 0);
    // And it still counts toward the corpus, because it is still a capture.
    assert_eq!(res.body["corpus"], 2);
}

/// Once a thread exists it is the suggestion, and its members stop being
/// suggested on their own: connecting to one of them would be proposing a
/// grouping that already exists.
#[tokio::test]
async fn a_thread_takes_over_from_the_captures_it_holds() {
    let (app, first, second, third) = a_recurring_thought().await;
    let idea = app.start_idea("Dungeon seeds", &[&first, &second]).await;

    let res = app.get(&format!("/api/captures/{third}/candidates")).await;
    let candidates = res.body["candidates"].as_array().expect("candidates");

    assert_eq!(candidates.len(), 1, "{:#?}", res.body);
    assert_eq!(candidates[0]["kind"], "idea");
    assert_eq!(candidates[0]["idea"]["id"], idea);
    assert_eq!(candidates[0]["idea"]["name"], "Dungeon seeds");
    assert_eq!(candidates[0]["idea"]["captures"], 2);
    assert!(candidates[0]["capture"].is_null());
}

/// A retired thread is not suggested, and its captures go back to being loose.
#[tokio::test]
async fn retiring_a_thread_frees_the_captures_it_held() {
    let (app, first, second, third) = a_recurring_thought().await;
    let idea = app.start_idea("Dungeon seeds", &[&first, &second]).await;
    app.post(&format!("/api/ideas/{idea}/retire"), None).await;

    let res = app.get(&format!("/api/captures/{third}/candidates")).await;
    let candidates = res.body["candidates"].as_array().expect("candidates");

    assert_eq!(candidates.len(), 2);
    for candidate in candidates {
        assert_eq!(candidate["kind"], "capture");
    }
}

/// Saying no has to stick, including across the derived half being thrown away,
/// and reconsidering has to undo it.
#[tokio::test]
async fn a_rejected_candidate_stays_rejected_across_a_rebuild() {
    let (app, first, second, third) = a_recurring_thought().await;
    let idea = app.start_idea("Dungeon seeds", &[&first, &second]).await;
    let candidates = format!("/api/captures/{third}/candidates");

    assert_eq!(
        app.get(&candidates).await.body["candidates"][0]["kind"],
        "idea"
    );

    app.send(
        Method::PUT,
        &format!("/api/ideas/{idea}/rejections/{third}"),
        None,
    )
    .await;
    assert_eq!(
        app.get(&candidates).await.body["candidates"]
            .as_array()
            .expect("empty")
            .len(),
        0
    );

    app.post("/api/reindex", None).await;
    assert_eq!(
        app.get(&candidates).await.body["candidates"]
            .as_array()
            .expect("still empty")
            .len(),
        0,
        "the rejection did not survive the rebuild"
    );

    app.send(
        Method::DELETE,
        &format!("/api/ideas/{idea}/rejections/{third}"),
        None,
    )
    .await;
    assert_eq!(
        app.get(&candidates).await.body["candidates"][0]["kind"],
        "idea"
    );
}

/// Two loose captures turned down are one decision, whichever way round it was
/// recorded.
#[tokio::test]
async fn a_rejected_pair_is_not_suggested_from_either_side() {
    let (app, first, second, third) = a_recurring_thought().await;

    for other in [&first, &second] {
        app.send(
            Method::PUT,
            &format!("/api/captures/{other}/rejections/{third}"),
            None,
        )
        .await;
    }

    assert_eq!(
        app.get(&format!("/api/captures/{third}/candidates"))
            .await
            .body["candidates"]
            .as_array()
            .expect("empty")
            .len(),
        0
    );
    // Recorded against `third` from the other side, and suppressed here too.
    assert_eq!(
        app.get(&format!("/api/captures/{first}/candidates"))
            .await
            .body["candidates"]
            .as_array()
            .expect("one left")
            .len(),
        1
    );
}

/// The gate on phase I3, for the lifecycle: the momentum can be recomputed from
/// the receipt without reading any of the code that produced it.
#[tokio::test]
async fn a_receipt_reconstructs_its_own_momentum() {
    let (app, first, second, third) = a_recurring_thought().await;
    let idea = app
        .start_idea("Dungeon seeds", &[&first, &second, &third])
        .await;

    let res = app.get(&format!("/api/ideas/{idea}/receipt")).await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);

    assert_eq!(res.body["ruleset"], "idea-momentum/v1");
    assert_eq!(res.body["idea"], idea);
    assert_eq!(res.body["name"], "Dungeon seeds");
    assert_eq!(res.body["integrity"], "sound");
    assert_eq!(res.body["state"], "active");
    assert_eq!(res.body["missing"].as_array().expect("none").len(), 0);
    assert!(res.body["boundaries"]["recent_14"].is_string());
    assert!(res.body["boundaries"]["recent_30"].is_string());
    assert!(res.body["boundaries"]["dormant"].is_string());

    // The counted flags on the evidence add up to the counts in the components.
    let captures = res.body["captures"].as_array().expect("captures");
    assert_eq!(captures.len(), 3);
    let within_14 = captures
        .iter()
        .filter(|capture| capture["within_14_days"] == true)
        .count();
    let within_30 = captures
        .iter()
        .filter(|capture| capture["within_30_days"] == true)
        .count();

    let components = &res.body["components"];
    assert_eq!(components["total"], captures.len());
    assert_eq!(components["recent_14"], within_14);
    assert_eq!(components["recent_30"], within_30);

    // And the components add up to the score, by the rule the ruleset states.
    let total = components["total"].as_u64().expect("total");
    let base = total.min(4);
    let recency = if within_14 >= 3 {
        2
    } else if within_30 >= 1 {
        1
    } else {
        0
    };
    let affirmation = u64::from(
        res.body["affirmations"]
            .as_array()
            .expect("affirmations")
            .iter()
            .any(|event| event["within_30_days"] == true),
    );
    assert_eq!(components["base"], base);
    assert_eq!(components["recency"], recency);
    assert_eq!(components["affirmation"], affirmation);
    assert_eq!(
        components["momentum"],
        (base + recency + affirmation).min(10)
    );
    assert_eq!(res.body["momentum"], components["momentum"]);

    // Two sentences from fixed templates, naming the state and the arithmetic.
    let explanation = res.body["explanation"].as_array().expect("explanation");
    assert_eq!(explanation.len(), 2);
    assert!(
        explanation[0]
            .as_str()
            .expect("prose")
            .starts_with("Active:"),
        "{explanation:#?}"
    );
    assert!(
        explanation[1]
            .as_str()
            .expect("prose")
            .starts_with("Momentum 5 ="),
        "{explanation:#?}"
    );
}

/// Affirming is worth a point, and the event that earned it is named.
#[tokio::test]
async fn an_affirmation_shows_up_in_the_receipt_that_counted_it() {
    let (app, first, second, _third) = a_recurring_thought().await;
    let idea = app.start_idea("Dungeon seeds", &[&first, &second]).await;

    let before = app.get(&format!("/api/ideas/{idea}/receipt")).await;
    assert_eq!(before.body["components"]["affirmation"], 0);
    assert_eq!(before.body["momentum"], 3);

    app.post(&format!("/api/ideas/{idea}/affirm"), None).await;

    let after = app.get(&format!("/api/ideas/{idea}/receipt")).await;
    assert_eq!(after.body["components"]["affirmation"], 1);
    assert_eq!(after.body["momentum"], 4);
    assert_eq!(after.body["state"], "active");

    let affirmations = after.body["affirmations"].as_array().expect("affirmations");
    assert_eq!(affirmations.len(), 1);
    assert_eq!(affirmations[0]["kind"], "interest_affirmed");
    assert_eq!(affirmations[0]["within_30_days"], true);
}

/// Nothing about the state is stored, so the same files answer differently at a
/// different moment. That is what makes dormancy need no scheduler.
#[tokio::test]
async fn a_receipt_answers_for_a_moment_you_choose() {
    let (app, first, second, _third) = a_recurring_thought().await;
    let idea = app.start_idea("Dungeon seeds", &[&first, &second]).await;

    let now = app.get(&format!("/api/ideas/{idea}/receipt")).await;
    assert_eq!(now.body["state"], "recurring");
    assert_eq!(now.body["components"]["momentum"], 3);

    let later = app
        .get(&format!(
            "/api/ideas/{idea}/receipt?at=2030-01-01T00:00:00Z"
        ))
        .await;
    assert_eq!(later.status, StatusCode::OK);
    assert_eq!(later.body["state"], "dormant");
    assert_eq!(later.body["computed_at"], "2030-01-01T00:00:00Z");
    // The captures are the same ones; only which windows they fall in moved.
    assert_eq!(later.body["components"]["total"], 2);
    assert_eq!(later.body["components"]["recent_30"], 0);
    assert_eq!(later.body["components"]["momentum"], 2);
    assert!(
        later.body["explanation"][0]
            .as_str()
            .expect("prose")
            .starts_with("Dormant:"),
        "{:#?}",
        later.body["explanation"]
    );
}

/// A capture deleted from under an idea leaves it with nothing to stand on. The
/// answer is to say so, not to derive a state from the absence.
#[tokio::test]
async fn an_idea_that_lost_its_evidence_gets_no_state_and_no_score() {
    let app = App::open().await;
    let only = app.capture("Dungeon seeds should decide the loot.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&only]).await;

    // The API refuses to take an idea's last capture away, so this is the
    // external deletion the plan describes: somebody's editor, or a sync.
    let month = format!("{}-{}", &only[..4], &only[4..6]);
    let path = app
        .root()
        .join(".rhizolog/ideas/captures")
        .join(month)
        .join(format!("{only}.md"));
    std::fs::remove_file(&path).expect("remove the capture file");
    app.post("/api/reindex", None).await;

    let res = app.get(&format!("/api/ideas/{idea}/receipt")).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["integrity"], "evidence_missing");
    assert!(res.body["state"].is_null());
    assert!(res.body["momentum"].is_null());
    assert!(res.body["components"].is_null());
    assert_eq!(res.body["missing"][0], only);
    assert!(
        res.body["explanation"][0]
            .as_str()
            .expect("prose")
            .contains("no authored evidence"),
        "{:#?}",
        res.body["explanation"]
    );

    // The listing agrees, and groups it where the dashboard puts Needs repair.
    let repair = app.get("/api/ideas?integrity=evidence_missing").await;
    assert_eq!(repair.body["total"], 1);
    assert_eq!(repair.body["ideas"][0]["id"], idea);
    assert_eq!(repair.body["ideas"][0]["needs_repair"], true);
    assert!(repair.body["ideas"][0]["state"].is_null());
    assert_eq!(app.get("/api/ideas?integrity=sound").await.body["total"], 0);
}

/// The listing carries the same answers the receipts do, and can be narrowed by
/// them.
#[tokio::test]
async fn the_ideas_listing_can_be_filtered_by_state() {
    let (app, first, second, third) = a_recurring_thought().await;
    let busy = app
        .start_idea("Dungeon seeds", &[&first, &second, &third])
        .await;
    let lone = app.capture("Compiler passes run in order.\n").await;
    let quiet = app.start_idea("Compiler passes", &[&lone]).await;
    app.post(&format!("/api/ideas/{quiet}/retire"), None).await;

    let all = app.get("/api/ideas").await;
    assert_eq!(all.status, StatusCode::OK);
    assert_eq!(all.body["total"], 2);
    assert_eq!(all.body["ruleset"], "idea-momentum/v1");
    assert!(all.body["at"].is_string());

    let active = app.get("/api/ideas?state=active").await;
    assert_eq!(active.body["total"], 1);
    assert_eq!(active.body["ideas"][0]["id"], busy);
    assert_eq!(active.body["ideas"][0]["state"], "active");
    assert_eq!(active.body["ideas"][0]["momentum"], 5);
    assert_eq!(active.body["ideas"][0]["integrity"], "sound");

    let retired = app.get("/api/ideas?state=retired").await;
    assert_eq!(retired.body["total"], 1);
    assert_eq!(retired.body["ideas"][0]["id"], quiet);

    assert_eq!(app.get("/api/ideas?state=dormant").await.body["total"], 0);

    // And the same listing at a moment where nothing has happened for years.
    // The retired one stays retired: that is a decision rather than an
    // inference, and no amount of time passing overturns it.
    let later = app
        .get("/api/ideas?at=2030-01-01T00:00:00Z&state=dormant")
        .await;
    assert_eq!(later.body["total"], 1);
    assert_eq!(later.body["ideas"][0]["id"], busy);
    assert_eq!(later.body["at"], "2030-01-01T00:00:00Z");
    assert_eq!(
        app.get("/api/ideas?at=2030-01-01T00:00:00Z&state=retired")
            .await
            .body["total"],
        1
    );
}

/// A filter nobody could have meant names the ones that would have worked, so a
/// caller can fix itself from the response.
#[tokio::test]
async fn an_unknown_state_names_the_states_there_are() {
    let app = App::open().await;

    let res = app.get("/api/ideas?state=dormantish").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid_parameter");
    assert_eq!(res.body["error"]["details"]["parameter"], "state");
    assert_eq!(res.body["error"]["details"]["value"], "dormantish");
    let allowed = res.body["error"]["details"]["allowed"]
        .as_array()
        .expect("allowed");
    assert_eq!(allowed.len(), 5);
    assert!(allowed.contains(&json!("dormant")));

    let integrity = app.get("/api/ideas?integrity=broken").await;
    assert_eq!(integrity.status, StatusCode::BAD_REQUEST);
    assert_eq!(integrity.body["error"]["details"]["parameter"], "integrity");
}

/// A `total` that counted ideas the filter excluded would make paging through a
/// filtered view miss some of them.
#[tokio::test]
async fn a_filtered_total_counts_only_what_matched() {
    let (app, first, second, third) = a_recurring_thought().await;
    for (name, seed) in [("One", &first), ("Two", &second), ("Three", &third)] {
        let idea = app.start_idea(name, &[seed.as_str()]).await;
        if name == "Three" {
            app.post(&format!("/api/ideas/{idea}/retire"), None).await;
        }
    }

    let res = app.get("/api/ideas?state=new&limit=1").await;

    assert_eq!(res.body["total"], 2, "{:#?}", res.body);
    assert_eq!(res.body["ideas"].as_array().expect("one page").len(), 1);
    assert_eq!(res.body["limit"], 1);

    let second_page = app.get("/api/ideas?state=new&limit=1&offset=1").await;
    assert_eq!(second_page.body["total"], 2);
    assert_ne!(
        second_page.body["ideas"][0]["id"],
        res.body["ideas"][0]["id"]
    );
}

// ---------------------------------------------------------------- promotion

/// The draft is the idea's own material in the order it was thought, and
/// nothing else. Every capture is in it: promoting is not a way of losing one,
/// and nothing is summarised, rewritten or annotated on the way out.
#[tokio::test]
async fn a_draft_is_every_capture_the_idea_holds_oldest_first() {
    let app = App::open().await;
    let first = app.capture("Seeds should decide the loot.\n").await;
    let second = app.capture("And the corridors.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&first]).await;
    app.put(&format!("/api/ideas/{idea}/captures/{second}"), None)
        .await;
    app.send(
        Method::PATCH,
        &format!("/api/ideas/{idea}"),
        Some(json!({ "note": "Worth writing up.\n" })),
    )
    .await;

    let res = app.get(&format!("/api/ideas/{idea}/draft")).await;

    assert_eq!(res.status, StatusCode::OK, "{:?}", res.body);
    assert_eq!(res.body["idea"], idea);
    assert_eq!(res.body["title"], "Dungeon seeds");
    assert_eq!(
        res.body["markdown"],
        "# Dungeon seeds\n\nWorth writing up.\n\nSeeds should decide the loot.\n\nAnd the \
         corridors.\n"
    );

    // Provenance is in the response rather than in the page, so the person
    // promoting is not handed prose of ours to delete.
    let sources = res.body["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["id"], first);
    assert_eq!(sources[1]["id"], second);
    assert!(sources[0]["created"].is_string());
    assert_eq!(sources[0]["archived"], false);
    assert!(res.body["missing"].as_array().expect("missing").is_empty());
    assert!(res.body["promoted_to"].is_null());

    // Reading a draft writes nothing. It is a suggestion about a page that does
    // not exist yet, and there is nothing about it to record.
    assert_eq!(app.events(), 1, "the connect above, and nothing else");
}

/// An archived capture is still in the draft. Archiving means processed, not
/// "this thought never happened", and a promotion that quietly dropped the
/// older half of a thread would be the wrong reading of both.
#[tokio::test]
async fn a_draft_keeps_the_archived_captures_and_names_the_ones_that_are_gone() {
    let app = App::open().await;
    let older = app.capture("The first version of the thought.\n").await;
    let newer = app.capture("The second version.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&older, &newer]).await;
    app.post(&format!("/api/captures/{older}/archive"), None)
        .await;

    let res = app.get(&format!("/api/ideas/{idea}/draft")).await;
    assert!(
        res.body["markdown"]
            .as_str()
            .expect("markdown")
            .contains("The first version of the thought."),
        "{:?}",
        res.body["markdown"]
    );
    assert_eq!(res.body["sources"][0]["archived"], true);

    // Delete the newer one out from under the thread, and the draft says what it
    // is short of rather than coming back quietly shorter.
    app.send(Method::DELETE, &format!("/api/captures/{newer}"), None)
        .await;

    let after = app.get(&format!("/api/ideas/{idea}/draft")).await;
    assert_eq!(after.body["sources"].as_array().expect("sources").len(), 1);
    assert_eq!(after.body["missing"][0], newer);
    assert!(
        !after.body["markdown"]
            .as_str()
            .expect("markdown")
            .contains("The second version."),
    );
}

/// Recording the association is the third step and the only one that is safe to
/// repeat, which is what makes the whole two-write sequence recoverable: if this
/// fails after the page was created, the caller sends it again.
#[tokio::test]
async fn recording_a_promotion_repeats_without_writing_a_second_decision() {
    let app = App::open().await;
    let capture = app.capture("Seeds should decide the loot.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&capture]).await;
    for slug in ["notes/dungeon-seeds", "notes/seeded-dungeons"] {
        let page = app
            .post(
                "/api/pages",
                Some(json!({ "slug": slug, "content": "# Dungeon seeds\n" })),
            )
            .await;
        assert_eq!(page.status, StatusCode::CREATED, "{:?}", page.body);
    }
    let promotion = format!("/api/ideas/{idea}/promotion");

    let promoted = app
        .put(&promotion, Some(json!({ "page": "notes/dungeon-seeds" })))
        .await;
    assert_eq!(promoted.status, StatusCode::OK, "{:?}", promoted.body);
    assert_eq!(promoted.body["promoted_to"], "notes/dungeon-seeds");
    assert_eq!(app.events(), 1);

    // The same slug again is the same fact, so there is nothing to write down.
    let again = app
        .put(&promotion, Some(json!({ "page": "notes/dungeon-seeds" })))
        .await;
    assert_eq!(again.body["promoted_to"], "notes/dungeon-seeds");
    assert_eq!(app.events(), 1, "a repeat wrote a second event");

    // A different slug is a different decision. It becomes the current answer
    // and the earlier one stays in the log, because both of them happened.
    let moved = app
        .put(&promotion, Some(json!({ "page": "notes/seeded-dungeons" })))
        .await;
    assert_eq!(moved.body["promoted_to"], "notes/seeded-dungeons");
    assert_eq!(app.events(), 2);

    // Nothing was consumed. The thread still holds what the page was made from,
    // so the sources of a promoted page can still be read.
    assert_eq!(
        moved.body["captures"].as_array().expect("captures").len(),
        1
    );
    assert_eq!(moved.body["captures"][0]["id"], capture);

    let listed = app.get("/api/ideas").await;
    assert_eq!(
        listed.body["ideas"][0]["promoted_to"],
        "notes/seeded-dungeons"
    );
    let draft = app.get(&format!("/api/ideas/{idea}/draft")).await;
    assert_eq!(draft.body["promoted_to"], "notes/seeded-dungeons");
}

/// The promotion endpoint creates nothing. A slug with no page behind it is
/// refused, and the idea is left exactly as it was.
#[tokio::test]
async fn a_promotion_refuses_a_page_that_does_not_exist() {
    let app = App::open().await;
    let capture = app.capture("Seeds should decide the loot.\n").await;
    let idea = app.start_idea("Dungeon seeds", &[&capture]).await;

    let res = app
        .put(
            &format!("/api/ideas/{idea}/promotion"),
            Some(json!({ "page": "notes/dungeon-seeds" })),
        )
        .await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "idea_promotion_page_not_found");
    assert_eq!(res.body["error"]["details"]["slug"], "notes/dungeon-seeds");
    assert_eq!(app.events(), 0);
    assert!(app.get(&format!("/api/ideas/{idea}")).await.body["promoted_to"].is_null());
    assert_eq!(app.get("/api/pages").await.body["total"], 0);
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

/// The reason the corpus is one owner's and never the wiki's.
///
/// Two accounts write the same words. Neither is suggested to the other, and
/// neither one's `N` or document frequency moves when the other writes, which
/// would otherwise make a similarity score a channel out of somebody's inbox.
#[tokio::test]
async fn one_account_never_sees_another_owners_candidates() {
    let (app, tim, alice) = with_two_accounts().await;

    async fn capture_as(app: &App, token: &str, text: &str) -> String {
        let res = app
            .send_as(
                Some(token),
                Method::POST,
                "/api/captures",
                Some(json!({ "text": text })),
            )
            .await;
        assert_eq!(res.status, StatusCode::CREATED, "{:?}", res.body);
        res.body["id"].as_str().expect("id").to_owned()
    }

    capture_as(&app, &tim, "Dungeon seeds should decide the loot.\n").await;
    let asking = capture_as(&app, &tim, "Dungeon seeds should decide the rooms.\n").await;

    let alone = app
        .send_as(
            Some(&tim),
            Method::GET,
            &format!("/api/captures/{asking}/candidates"),
            None,
        )
        .await;
    assert_eq!(alone.body["corpus"], 2);
    let signals = alone.body["candidates"][0]["signals"].clone();
    let similarity = alone.body["candidates"][0]["similarity"].clone();

    // Alice writes the same thing three times. Nothing about tim's answer moves.
    for text in [
        "Dungeon seeds should decide the loot.\n",
        "Dungeon seeds should decide the layout.\n",
        "Dungeon seeds should decide the rooms.\n",
    ] {
        capture_as(&app, &alice, text).await;
    }

    let after = app
        .send_as(
            Some(&tim),
            Method::GET,
            &format!("/api/captures/{asking}/candidates"),
            None,
        )
        .await;
    assert_eq!(
        after.body["corpus"], 2,
        "alice's captures joined the corpus"
    );
    assert_eq!(after.body["candidates"].as_array().expect("one").len(), 1);
    assert_eq!(after.body["candidates"][0]["similarity"], similarity);
    assert_eq!(after.body["candidates"][0]["signals"], signals);

    // And alice cannot ask about tim's capture at all.
    let hers = app
        .send_as(
            Some(&alice),
            Method::GET,
            &format!("/api/captures/{asking}/candidates"),
            None,
        )
        .await;
    assert_eq!(hers.status, StatusCode::NOT_FOUND);
    assert_eq!(hers.code(), "capture_not_found");
}

/// A page that is not there and a page that is not yours are one answer, word
/// for word.
///
/// Promotion takes a slug somebody typed, so an error that told those apart
/// would answer questions about another account's wiki for the price of guessing
/// one. It is the same rule that sends a private page's read to `404`.
#[tokio::test]
async fn a_promotion_refuses_a_missing_page_and_an_unreadable_one_identically() {
    let (app, tim, alice) = with_two_accounts().await;

    let capture = app
        .send_as(
            Some(&tim),
            Method::POST,
            "/api/captures",
            Some(json!({ "text": "Seeds should decide the loot.\n" })),
        )
        .await;
    let capture = capture.body["id"].as_str().expect("id").to_owned();
    let idea = app
        .send_as(
            Some(&tim),
            Method::POST,
            "/api/ideas",
            Some(json!({ "name": "Dungeon seeds", "captures": [capture] })),
        )
        .await;
    let idea = idea.body["id"].as_str().expect("id").to_owned();
    let promotion = format!("/api/ideas/{idea}/promotion");
    let hers = json!({ "page": "notes/hers" });

    let missing = app
        .send_as(Some(&tim), Method::PUT, &promotion, Some(hers.clone()))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.code(), "idea_promotion_page_not_found");

    // Alice writes a private page at exactly that slug. Tim's answer does not
    // move, which is the whole point: he cannot tell that anything appeared.
    let created = app
        .send_as(
            Some(&alice),
            Method::POST,
            "/api/pages",
            Some(json!({
                "slug": "notes/hers",
                "content": "Hers.\n",
                "visibility": "private",
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);

    let refused = app
        .send_as(Some(&tim), Method::PUT, &promotion, Some(hers))
        .await;
    assert_eq!(refused.status, missing.status);
    assert_eq!(refused.body, missing.body);

    // His own private page is one he can read, so it can be recorded. The rule
    // is "a page you can read", not "a page everybody can".
    app.send_as(
        Some(&tim),
        Method::POST,
        "/api/pages",
        Some(json!({
            "slug": "notes/his",
            "content": "Mine.\n",
            "visibility": "private",
        })),
    )
    .await;
    let recorded = app
        .send_as(
            Some(&tim),
            Method::PUT,
            &promotion,
            Some(json!({ "page": "notes/his" })),
        )
        .await;
    assert_eq!(recorded.status, StatusCode::OK, "{:?}", recorded.body);
    assert_eq!(recorded.body["promoted_to"], "notes/his");

    // And alice cannot read the draft of a thread that is not hers.
    let draft = app
        .send_as(
            Some(&alice),
            Method::GET,
            &format!("/api/ideas/{idea}/draft"),
            None,
        )
        .await;
    assert_eq!(draft.status, StatusCode::NOT_FOUND);
    assert_eq!(draft.code(), "idea_not_found");
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
        (
            Method::GET,
            "/api/captures/20200101T000000-000000000/candidates",
        ),
        (Method::GET, "/api/ideas/20200101T000000-000000000/receipt"),
        (Method::GET, "/api/ideas/20200101T000000-000000000/draft"),
        (
            Method::PUT,
            "/api/ideas/20200101T000000-000000000/promotion",
        ),
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
