//! End-to-end tests over the assembled router.
//!
//! The app is driven in-process with `tower::ServiceExt::oneshot` against a
//! throwaway wiki directory: no ports to allocate, no server task to tear down,
//! and no chance of two tests colliding on the same wiki.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, Days, SecondsFormat, Utc};
use rhizolog::{
    AppState, Assets, IdeaService, IdeaStore, Index, Store, TimeStore, UserStore, WordLog,
};
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
        // In-memory index: these tests are about the HTTP surface, not
        // persistence, which `index::sync` covers.
        let index = Index::open(None).await.expect("open index");
        // No accounts, so the wiki is open and every request below is the single
        // user — which is what keeps this file testing the API rather than the
        // authentication in front of it. `signed_in` is the other case.
        let users = UserStore::open(directory.path()).await.expect("open users");
        let ideas = IdeaStore::open(directory.path()).await.expect("open ideas");
        let words = WordLog::open(directory.path())
            .await
            .expect("open word log");
        Self {
            router: rhizolog::router(AppState {
                store,
                times,
                ideas: IdeaService::new(ideas),
                users,
                words,
                index,
                usage: rhizolog::UsageTally::new(),
                // API-only: the SPA fallback is covered in tests/frontend.rs.
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read: false,
            }),
            directory,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        self.dispatch(method, path, body, None).await
    }

    /// The same, labelled with an `X-Rhizolog-Actor` header.
    async fn send_as(&self, method: Method, path: &str, body: Option<Value>, actor: &str) -> Res {
        self.dispatch(method, path, body, Some(actor)).await
    }

    async fn dispatch(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        actor: Option<&str>,
    ) -> Res {
        let mut builder = Request::builder().method(method).uri(path);

        if let Some(actor) = actor {
            builder = builder.header("x-rhizolog-actor", actor);
        }

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

    async fn put(&self, path: &str, body: Value) -> Res {
        self.send(Method::PUT, path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: Value) -> Res {
        self.send(Method::PATCH, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> Res {
        self.send(Method::DELETE, path, None).await
    }

    /// Every line of the word log, split into its fields.
    ///
    /// Read off disk rather than out of `/api/word-stats`, deliberately: the log
    /// is the authored copy and the table is a reading of it, so a test that
    /// asked the API would be checking the reading against itself.
    ///
    /// The fields are `at`, `slug`, `actor`, `account`, `kind`, `added`,
    /// `removed`, `total`, `from`.
    fn log(&self) -> Vec<Vec<String>> {
        let directory = self.directory.path().join(".rhizolog").join("words");
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return Vec::new();
        };

        let mut months: Vec<std::path::PathBuf> =
            entries.flatten().map(|entry| entry.path()).collect();
        months.sort();

        months
            .iter()
            .filter_map(|month| std::fs::read_to_string(month).ok())
            .flat_map(|text| {
                text.lines()
                    .map(|line| line.split('\t').map(str::to_owned).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Write `.rhizolog/prose.toml` into this wiki.
    ///
    /// Straight to disk, because there is no API that writes it: the rules are
    /// authored configuration, and a second way to write them would be a second
    /// place for them to be wrong.
    fn rules(&self, toml: &str) {
        let internal = self.directory.path().join(".rhizolog");
        std::fs::create_dir_all(&internal).expect("internal directory");
        std::fs::write(internal.join("prose.toml"), toml).expect("write the rules");
    }

    /// Create a page, asserting it worked.
    async fn seed(&self, slug: &str, body: Value) {
        let mut payload = json!({ "slug": slug });
        for (key, value) in body.as_object().expect("object body") {
            payload[key] = value.clone();
        }
        let res = self.post("/api/pages", payload).await;
        assert_eq!(
            res.status,
            StatusCode::CREATED,
            "seed failed: {:?}",
            res.body
        );
    }
}

// ------------------------------------------------------------------- meta

#[tokio::test]
async fn health_reports_the_wiki_it_is_serving() {
    let app = App::new().await;

    let res = app.get("/api/health").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["status"], "ok");
    assert_eq!(res.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(res.body["pages"], 0);
    assert!(
        !res.body["wiki_root"].as_str().unwrap().starts_with(r"\\?\"),
        "verbatim prefix leaked into the API"
    );
}

#[tokio::test]
async fn unknown_routes_are_404() {
    let app = App::new().await;
    assert_eq!(app.get("/api/nonsense").await.status, StatusCode::NOT_FOUND);
}

/// Usage counts are the one thing in the index that is *not* derived from the
/// markdown beside it, so a rebuild cannot restore them. They live in the
/// durable half of the schema, and shutdown flushes the in-memory tally into it
/// — this is that round trip, minus the signal handler.
#[tokio::test]
async fn usage_counts_survive_a_restart() {
    let wiki = TempDir::new().expect("wiki dir");
    // Beside the wiki rather than inside it, so nothing here depends on how the
    // page walker treats a stray file.
    let state_dir = TempDir::new().expect("state dir");
    let database = state_dir.path().join("index.db");

    let health_calls = |body: &Value| -> u64 {
        body["api_usage"]
            .as_array()
            .expect("api_usage")
            .iter()
            .find(|entry| entry["route"] == "/api/health")
            .map(|entry| entry["count"].as_u64().expect("count"))
            .unwrap_or(0)
    };

    // First run.
    {
        let store = Store::open(wiki.path()).await.expect("open store");
        let times = TimeStore::open(wiki.path()).await.expect("open time log");
        let users = UserStore::open(wiki.path()).await.expect("open users");
        let ideas = IdeaStore::open(wiki.path()).await.expect("open ideas");
        let words = WordLog::open(wiki.path()).await.expect("open word log");
        let index = Index::open(Some(&database)).await.expect("open index");
        let usage = rhizolog::UsageTally::new();
        let app = App {
            router: rhizolog::router(AppState {
                store,
                times,
                ideas: IdeaService::new(ideas),
                users,
                words,
                index: index.clone(),
                usage: usage.clone(),
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read: false,
            }),
            directory: wiki,
        };

        for _ in 0..3 {
            assert_eq!(app.get("/api/health").await.status, StatusCode::OK);
        }
        // Counted before anything is written: the tally is read back live.
        assert_eq!(health_calls(&app.get("/api/stats").await.body), 3);

        // What shutdown does.
        rhizolog::api::graph::flush_usage(&index, &usage).await;
    }

    // Second run, same database, a tally that has never seen a request.
    let wiki = TempDir::new().expect("wiki dir");
    let store = Store::open(wiki.path()).await.expect("open store");
    let times = TimeStore::open(wiki.path()).await.expect("open time log");
    let users = UserStore::open(wiki.path()).await.expect("open users");
    let ideas = IdeaStore::open(wiki.path()).await.expect("open ideas");
    let words = WordLog::open(wiki.path()).await.expect("open word log");
    let index = Index::open(Some(&database)).await.expect("reopen index");
    let app = App {
        router: rhizolog::router(AppState {
            store,
            times,
            ideas: IdeaService::new(ideas),
            users,
            words,
            index,
            usage: rhizolog::UsageTally::new(),
            assets: Assets::None,
            secure_cookies: false,
            anonymous_read: false,
        }),
        directory: wiki,
    };

    assert_eq!(
        health_calls(&app.get("/api/stats").await.body),
        3,
        "usage counts did not survive the restart"
    );
}

// -------------------------------------------------------------- openapi

#[tokio::test]
async fn the_openapi_document_is_served() {
    let app = App::new().await;

    let res = app.get("/api-docs/openapi.json").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["info"]["title"], "Rhizolog");
    assert!(res.body["paths"]["/api/health"]["get"].is_object());
}

/// Every `$ref` in the document must point at something that is in it.
///
/// This is not a style check. `pnpm gen:api` refuses a spec with a dangling
/// reference outright, so one unresolvable `$ref` means the frontend's types
/// cannot be regenerated at all, and the failure is silent from the Rust side,
/// where `cargo test` and the served document are both perfectly happy.
///
/// The way to write one by accident is specific and worth naming: a `ToSchema`
/// enum used only as a query parameter through `IntoParams`. utoipa emits a
/// reference to it and registers no component, because nothing in a request body
/// or a response ever named it. `/api/compile`'s `format` was exactly that, and
/// it got as far as a working endpoint and a served spec before anything noticed.
#[tokio::test]
async fn every_reference_in_the_spec_resolves() {
    let app = App::new().await;
    let spec = app.get("/api-docs/openapi.json").await.body;

    fn refs(value: &Value, found: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                for (key, child) in fields {
                    if key == "$ref"
                        && let Some(target) = child.as_str()
                    {
                        found.push(target.to_owned());
                    }
                    refs(child, found);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| refs(item, found)),
            _ => {}
        }
    }

    let mut found = Vec::new();
    refs(&spec, &mut found);
    assert!(
        !found.is_empty(),
        "a spec with no references at all is suspect"
    );

    for reference in found {
        let pointer = reference
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("only local references are expected: {reference}"));
        assert!(
            spec.pointer(pointer).is_some(),
            "{reference} resolves to nothing, so `pnpm gen:api` will refuse the whole document"
        );
    }
}

/// Every route must appear in the spec with a description and responses. If
/// this fails, agents reading the document will not know the endpoint exists or
/// what it returns.
#[tokio::test]
async fn every_api_route_is_documented() {
    let app = App::new().await;

    let spec = app.get("/api-docs/openapi.json").await.body;
    let paths = spec["paths"].as_object().expect("spec has paths");

    assert!(!paths.is_empty(), "the spec documents no routes at all");
    for (path, operations) in paths {
        assert!(path.starts_with("/api/"), "undocumented path shape: {path}");
        for (method, operation) in operations.as_object().expect("operations") {
            assert!(
                operation["responses"]
                    .as_object()
                    .is_some_and(|responses| !responses.is_empty()),
                "{method} {path} documents no responses"
            );
            assert!(
                operation["description"].is_string() || operation["summary"].is_string(),
                "{method} {path} has no description"
            );
        }
    }
}

/// Operation ids are global to the document, but utoipa takes each one from its
/// handler's function name — which is only unique within a Rust module. Two
/// modules that both call a handler `list` publish two operations with one id,
/// and a client generated from that document silently keeps one of them.
///
/// This is the check that catches it, because nothing else does: the spec still
/// validates, both routes still work, and only the generated client is wrong.
#[tokio::test]
async fn operation_ids_are_unique_across_the_document() {
    let app = App::new().await;

    let spec = app.get("/api-docs/openapi.json").await.body;
    let paths = spec["paths"].as_object().expect("spec has paths");

    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (path, operations) in paths {
        for (method, operation) in operations.as_object().expect("operations") {
            let id = operation["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path} has no operationId"))
                .to_owned();
            let here = format!("{} {path}", method.to_uppercase());
            if let Some(previous) = seen.insert(id.clone(), here.clone()) {
                panic!("operationId {id:?} is used by both {previous} and {here}");
            }
        }
    }
}

/// Doc comments are written for people reading the source. Some of them talk
/// about Rust types and link to other items, and a rustdoc link on the wire is a
/// dead reference — the reader has no crate to resolve it against. Where that
/// happens the schema has to carry an explicit `description` instead.
#[tokio::test]
async fn schema_descriptions_do_not_leak_rustdoc_links() {
    let app = App::new().await;

    let spec = app.get("/api-docs/openapi.json").await.body;
    let schemas = spec["components"]["schemas"]
        .as_object()
        .expect("spec has schemas");

    for (name, schema) in schemas {
        let rendered = schema.to_string();
        assert!(
            !rendered.contains("[`"),
            "the {name} schema publishes a rustdoc link: {rendered}"
        );
    }
}

/// For a tool-using agent the examples are most of what the document teaches, so
/// a field that carries none is a field it has to guess at.
#[tokio::test]
async fn the_page_schemas_carry_examples() {
    let app = App::new().await;

    let spec = app.get("/api-docs/openapi.json").await.body;
    let schemas = &spec["components"]["schemas"];

    for name in ["PageView", "PageSummary", "CreatePage", "SearchHitView"] {
        let properties = schemas[name]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} has no properties"));

        for (field, property) in properties {
            // A `$ref` takes its example from the schema it points at, and
            // booleans and timestamps are self-describing.
            if property.get("$ref").is_some()
                || property.get("oneOf").is_some()
                || property["type"] == "boolean"
                || property.get("format").is_some()
            {
                continue;
            }
            assert!(
                property.get("example").is_some(),
                "{name}.{field} has no example"
            );
        }
    }
}

/// `{*slug}` is an axum routing spelling. It must not reach the published
/// document, where it would generate a parameter literally named `*slug`.
#[tokio::test]
async fn the_spec_does_not_leak_axum_wildcard_syntax() {
    let app = App::new().await;

    let spec = app.get("/api-docs/openapi.json").await.body;
    let paths = spec["paths"].as_object().expect("spec has paths");

    for path in paths.keys() {
        assert!(
            !path.contains("{*"),
            "wildcard syntax leaked into spec: {path}"
        );
    }
    assert!(
        paths.contains_key("/api/pages/{slug}"),
        "expected the normalised page path, got {:?}",
        paths.keys().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------- create

#[tokio::test]
async fn creates_a_page_and_points_at_it() {
    let app = App::new().await;

    let res = app
        .post(
            "/api/pages",
            json!({
                "slug": "notes/rhizome",
                "title": "Rhizome",
                "tags": ["theory", "deleuze"],
                "content": "Knowledge branches off chaotically.\n"
            }),
        )
        .await;

    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["slug"], "notes/rhizome");
    assert_eq!(res.body["title"], "Rhizome");
    assert_eq!(res.body["tags"], json!(["theory", "deleuze"]));
    assert_eq!(res.body["content"], "Knowledge branches off chaotically.\n");
    assert_eq!(res.location.as_deref(), Some("/api/pages/notes/rhizome"));
    // Not asked for, so not sent.
    assert!(res.body.get("html").is_none());
}

#[tokio::test]
async fn creating_over_an_existing_page_is_a_conflict() {
    let app = App::new().await;
    app.seed("notes/rhizome", json!({ "content": "First.\n" }))
        .await;

    let res = app
        .post(
            "/api/pages",
            json!({ "slug": "notes/rhizome", "content": "Second.\n" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.code(), "page_already_exists");
    // The original survived.
    assert_eq!(
        app.get("/api/pages/notes/rhizome").await.body["content"],
        "First.\n"
    );
}

/// A bad slug in a *body* must fail the same way as one in a URL. This is the
/// asymmetry to watch: body slugs are rejected by serde during extraction, so
/// without a custom extractor they would come back in axum's rejection format
/// instead of the envelope every other error uses.
#[tokio::test]
async fn a_slug_that_would_escape_the_wiki_is_refused_in_the_envelope() {
    let app = App::new().await;

    let res = app
        .post("/api/pages", json!({ "slug": "../../etc/passwd" }))
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid_request_body");
    assert!(
        res.body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("relative path component")),
        "the reason was lost: {:?}",
        res.body
    );
}

#[tokio::test]
async fn malformed_json_is_refused_in_the_envelope() {
    let app = App::new().await;

    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/pages")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .expect("build request");
    let response = app.router.clone().oneshot(request).await.expect("response");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("error body is JSON");

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request_body");
}

// ------------------------------------------------------------------ read

/// The reason the page routes are a wildcard at all.
#[tokio::test]
async fn reads_a_deeply_nested_slug() {
    let app = App::new().await;
    app.seed(
        "notes/rust/async/pinning",
        json!({ "content": "Pinned.\n" }),
    )
    .await;

    let res = app.get("/api/pages/notes/rust/async/pinning").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["slug"], "notes/rust/async/pinning");
    assert_eq!(res.body["content"], "Pinned.\n");
}

/// Reading a page and writing it straight back must not change what it means.
///
/// A title the server derived from the body would otherwise freeze into the
/// frontmatter on the first save, and stop tracking the heading it came from —
/// a trap for any read-modify-write client, the dashboard's editor included.
#[tokio::test]
async fn a_derived_title_says_so_and_survives_a_round_trip() {
    let app = App::new().await;
    app.seed(
        "notes/rhizome",
        json!({ "content": "# Rhizome\n\nBody.\n" }),
    )
    .await;

    let read = app.get("/api/pages/notes/rhizome").await;
    assert_eq!(read.body["title"], "Rhizome");
    assert_eq!(read.body["title_derived"], true);

    // Written back with a null title, it stays derived and follows the heading.
    let put = app
        .put(
            "/api/pages/notes/rhizome",
            json!({ "title": null, "content": "# Rhizomes\n\nBody.\n" }),
        )
        .await;
    assert_eq!(put.body["title"], "Rhizomes");
    assert_eq!(put.body["title_derived"], true);

    // Given one, it is stored, and the heading no longer decides the title.
    let put = app
        .put(
            "/api/pages/notes/rhizome",
            json!({ "title": "Pinned", "content": "# Something else\n\nBody.\n" }),
        )
        .await;
    assert_eq!(put.body["title"], "Pinned");
    assert_eq!(put.body["title_derived"], false);
}

#[tokio::test]
async fn an_explicit_title_is_not_reported_as_derived() {
    let app = App::new().await;
    app.seed(
        "notes/rhizome",
        json!({ "title": "Rhizome", "content": "# Other heading\n" }),
    )
    .await;

    let res = app.get("/api/pages/notes/rhizome").await;

    assert_eq!(res.body["title"], "Rhizome");
    assert_eq!(res.body["title_derived"], false);
}

#[tokio::test]
async fn renders_html_only_when_asked() {
    let app = App::new().await;
    app.seed(
        "notes/rhizome",
        json!({ "content": "# Heading\n\nBody *text*.\n" }),
    )
    .await;

    let plain = app.get("/api/pages/notes/rhizome").await;
    assert!(plain.body.get("html").is_none(), "html sent unrequested");

    let rendered = app.get("/api/pages/notes/rhizome?render=true").await;
    let html = rendered.body["html"].as_str().expect("html field");
    assert!(html.contains("<h1>Heading</h1>"));
    assert!(html.contains("<em>text</em>"));
    // The markdown source is still there; rendering adds, never replaces.
    assert_eq!(rendered.body["content"], "# Heading\n\nBody *text*.\n");
}

/// Rendered wikilinks have to be usable as links. A relative `href` would
/// resolve against whatever page the reader is on, so it would break, and break
/// differently depending on how deeply nested that page was.
#[tokio::test]
async fn rendered_links_point_at_browsable_urls() {
    let app = App::new().await;
    app.seed(
        "notes/rust/async",
        json!({ "content": "See [[notes/rhizome]] and [pinning](pinning.md).\n" }),
    )
    .await;

    let res = app.get("/api/pages/notes/rust/async?render=true").await;
    let html = res.body["html"].as_str().expect("html field");

    assert!(
        html.contains(r#"href="/pages/notes/rhizome""#),
        "got {html}"
    );
    // Resolved against the page's own directory, not the wiki root.
    assert!(
        html.contains(r#"href="/pages/notes/rust/pinning""#),
        "got {html}"
    );
}

#[tokio::test]
async fn renders_markdown_that_has_not_been_saved() {
    let app = App::new().await;

    let res = app
        .post(
            "/api/render",
            json!({ "content": "# Draft\n\nSee [[notes/rhizome]].\n" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK);
    let html = res.body["html"].as_str().expect("html field");
    assert!(html.contains("<h1>Draft</h1>"), "got {html}");
    assert!(
        html.contains(r#"href="/pages/notes/rhizome""#),
        "got {html}"
    );

    // Nothing was stored: this is a pure function over the body.
    assert_eq!(app.get("/api/pages").await.body["total"], 0);
}

/// The slug is what tells a relative link where it is being written from.
#[tokio::test]
async fn rendering_a_draft_resolves_relative_links_against_its_slug() {
    let app = App::new().await;
    let content = "See [traits](traits.md).\n";

    let rooted = app.post("/api/render", json!({ "content": content })).await;
    assert!(
        rooted.body["html"]
            .as_str()
            .expect("html")
            .contains(r#"href="/pages/traits""#),
        "got {:?}",
        rooted.body["html"]
    );

    let nested = app
        .post(
            "/api/render",
            json!({ "content": content, "slug": "notes/rust/async" }),
        )
        .await;
    assert!(
        nested.body["html"]
            .as_str()
            .expect("html")
            .contains(r#"href="/pages/notes/rust/traits""#),
        "got {:?}",
        nested.body["html"]
    );
}

/// A preview is rendered from whatever an editor has typed, so it is exactly
/// the path by which markup would reach the dashboard.
#[tokio::test]
async fn rendering_a_draft_does_not_pass_raw_html_through() {
    let app = App::new().await;

    let res = app
        .post(
            "/api/render",
            json!({ "content": "<script>alert(1)</script>\n\nAnd <img src=x onerror=alert(1)>.\n" }),
        )
        .await;

    let html = res.body["html"].as_str().expect("html field");
    assert!(!html.contains("<script"), "got {html}");
    assert!(!html.contains("<img"), "got {html}");
}

#[tokio::test]
async fn rendering_rejects_a_bad_slug_in_the_envelope() {
    let app = App::new().await;

    let res = app
        .post(
            "/api/render",
            json!({ "content": "x", "slug": "../escape" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid_request_body");
}

#[tokio::test]
async fn a_missing_page_names_the_slug_it_looked_for() {
    let app = App::new().await;

    let res = app.get("/api/pages/notes/asnyc").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "page_not_found");
    assert_eq!(res.body["error"]["details"]["slug"], "notes/asnyc");
}

/// A caller that builds a bad slug should be able to fix it from the response.
#[tokio::test]
async fn a_traversal_attempt_in_the_url_is_rejected_by_rule() {
    let app = App::new().await;

    let res = app.get("/api/pages/notes/../../etc/passwd").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "slug_relative_segment");
    assert_eq!(
        res.body["error"]["details"]["rule"],
        "slug_relative_segment"
    );
    assert!(res.body["error"]["details"]["reason"].is_string());
}

// --------------------------------------------------------------- replace

#[tokio::test]
async fn put_creates_then_replaces() {
    let app = App::new().await;

    let created = app
        .put("/api/pages/notes/rhizome", json!({ "content": "First.\n" }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let created_at = created.body["created"].clone();

    let replaced = app
        .put(
            "/api/pages/notes/rhizome",
            json!({ "title": "Renamed", "content": "Second.\n" }),
        )
        .await;

    assert_eq!(replaced.status, StatusCode::OK);
    assert_eq!(replaced.body["content"], "Second.\n");
    assert_eq!(replaced.body["title"], "Renamed");
    // A replace is not a re-creation.
    assert_eq!(
        replaced.body["created"], created_at,
        "PUT reset the creation time"
    );
}

#[tokio::test]
async fn put_clears_fields_it_omits() {
    let app = App::new().await;
    app.seed(
        "notes/rhizome",
        json!({ "title": "Rhizome", "tags": ["theory"], "content": "Body.\n" }),
    )
    .await;

    let res = app
        .put("/api/pages/notes/rhizome", json!({ "content": "Body.\n" }))
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["tags"], json!([]), "PUT should be a full replace");
}

// ----------------------------------------------------------------- patch

#[tokio::test]
async fn patch_leaves_omitted_fields_alone() {
    let app = App::new().await;
    app.seed(
        "notes/rhizome",
        json!({ "title": "Rhizome", "tags": ["theory"], "content": "Body.\n" }),
    )
    .await;

    let res = app
        .patch(
            "/api/pages/notes/rhizome",
            json!({ "content": "Rewritten.\n" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["content"], "Rewritten.\n");
    assert_eq!(res.body["title"], "Rhizome", "title was not preserved");
    assert_eq!(
        res.body["tags"],
        json!(["theory"]),
        "tags were not preserved"
    );
}

/// The reason PATCH needs to tell `null` from absent.
#[tokio::test]
async fn patching_a_null_title_falls_back_to_the_heading() {
    let app = App::new().await;
    app.seed(
        "notes/rhizome",
        json!({ "title": "Explicit", "content": "# From heading\n" }),
    )
    .await;

    let res = app
        .patch("/api/pages/notes/rhizome", json!({ "title": null }))
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["title"], "From heading");
}

#[tokio::test]
async fn patching_a_missing_page_is_a_404() {
    let app = App::new().await;

    let res = app
        .patch("/api/pages/nothing/here", json!({ "content": "x" }))
        .await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "page_not_found");
}

// ---------------------------------------------------------------- delete

#[tokio::test]
async fn deletes_a_page() {
    let app = App::new().await;
    app.seed("notes/rhizome", json!({ "content": "Body.\n" }))
        .await;

    let deleted = app.delete("/api/pages/notes/rhizome").await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    assert_eq!(
        app.get("/api/pages/notes/rhizome").await.status,
        StatusCode::NOT_FOUND
    );
}

/// A deletion that did nothing is worth knowing about.
#[tokio::test]
async fn deleting_a_missing_page_is_a_404() {
    let app = App::new().await;

    let res = app.delete("/api/pages/nothing/here").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "page_not_found");
}

// ------------------------------------------------------------------ move

#[tokio::test]
async fn moves_a_page_and_leaves_nothing_behind() {
    let app = App::new().await;
    app.seed(
        "notes/old",
        json!({ "title": "Kept", "content": "Body.\n" }),
    )
    .await;

    let res = app
        .post(
            "/api/move",
            json!({ "from": "notes/old", "to": "archive/new" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["slug"], "archive/new");
    assert_eq!(res.body["title"], "Kept");

    assert_eq!(
        app.get("/api/pages/notes/old").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/api/pages/archive/new").await.status,
        StatusCode::OK
    );

    // The index followed the move: the old slug is gone from listings.
    let listing = app.get("/api/pages").await;
    assert_eq!(listing.body["total"], 1);
    assert_eq!(listing.body["pages"][0]["slug"], "archive/new");
}

#[tokio::test]
async fn moving_onto_an_existing_page_is_a_conflict() {
    let app = App::new().await;
    app.seed("notes/old", json!({ "content": "Source.\n" }))
        .await;
    app.seed("notes/taken", json!({ "content": "Destination.\n" }))
        .await;

    let res = app
        .post(
            "/api/move",
            json!({ "from": "notes/old", "to": "notes/taken" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.code(), "page_already_exists");
    // Neither side moved.
    assert_eq!(app.get("/api/pages/notes/old").await.status, StatusCode::OK);
    assert_eq!(
        app.get("/api/pages/notes/taken").await.body["content"],
        "Destination.\n"
    );
}

// ------------------------------------------------------------------ list

#[tokio::test]
async fn lists_pages_without_their_bodies() {
    let app = App::new().await;
    app.seed("a", json!({ "title": "Alpha", "content": "Long body.\n" }))
        .await;
    app.seed("b", json!({ "title": "Beta", "content": "Long body.\n" }))
        .await;

    let res = app.get("/api/pages").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["total"], 2);
    assert_eq!(res.body["limit"], 50);
    let first = &res.body["pages"][0];
    assert_eq!(first["slug"], "a");
    assert!(first.get("content").is_none(), "listing included bodies");
}

#[tokio::test]
async fn lists_filtered_by_tag_and_sorted() {
    let app = App::new().await;
    app.seed("a", json!({ "title": "Alpha", "tags": ["theory"] }))
        .await;
    app.seed("b", json!({ "title": "Beta", "tags": ["practice"] }))
        .await;

    let tagged = app.get("/api/pages?tag=theory").await;
    assert_eq!(tagged.body["total"], 1);
    assert_eq!(tagged.body["pages"][0]["slug"], "a");

    let descending = app.get("/api/pages?sort=title&order=desc").await;
    assert_eq!(descending.body["pages"][0]["title"], "Beta");
}

#[tokio::test]
async fn lists_filtered_by_slug_path() {
    let app = App::new().await;
    for slug in [
        "notes/rust",
        "notes/rust/async",
        "notes/rustlings",
        "code/rust/traits",
    ] {
        app.seed(slug, json!({ "content": "Body.\n" })).await;
    }

    // Hierarchical: everything at or under one path, and nothing that merely
    // starts with the same characters.
    let under = app.get("/api/pages?prefix=notes/rust").await;
    assert_eq!(under.status, StatusCode::OK);
    assert_eq!(under.body["total"], 2);
    assert_eq!(under.body["pages"][0]["slug"], "notes/rust");
    assert_eq!(under.body["pages"][1]["slug"], "notes/rust/async");

    // Flat, the way a tag is: every `rust` directory, wherever it sits.
    let anywhere = app.get("/api/pages?segment=rust").await;
    assert_eq!(anywhere.body["total"], 2);
    assert_eq!(anywhere.body["pages"][0]["slug"], "code/rust/traits");
    assert_eq!(anywhere.body["pages"][1]["slug"], "notes/rust/async");

    // The filters intersect rather than widening each other.
    let both = app.get("/api/pages?segment=rust&prefix=code").await;
    assert_eq!(both.body["total"], 1);
    assert_eq!(both.body["pages"][0]["slug"], "code/rust/traits");

    // A path nobody uses is an empty listing, not an error: these are filters,
    // not lookups, and there is no such thing as a missing directory.
    let nowhere = app.get("/api/pages?prefix=nonsense").await;
    assert_eq!(nowhere.status, StatusCode::OK);
    assert_eq!(nowhere.body["total"], 0);
}

#[tokio::test]
async fn listing_can_be_narrowed_to_named_fields() {
    let app = App::new().await;
    app.seed("a", json!({ "title": "Alpha", "tags": ["theory"] }))
        .await;

    let res = app.get("/api/pages?fields=slug,tags").await;

    assert_eq!(res.status, StatusCode::OK);
    let page = &res.body["pages"][0];
    assert_eq!(page["slug"], "a");
    assert_eq!(page["tags"], json!(["theory"]));
    assert!(page.get("title").is_none());
    assert!(page.get("size").is_none());
    // The envelope keeps its own fields.
    assert_eq!(res.body["total"], 1);
}

#[tokio::test]
async fn an_unknown_field_is_refused_with_the_valid_ones() {
    let app = App::new().await;

    let res = app.get("/api/pages?fields=slug,body").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "unknown_fields");
    assert_eq!(res.body["error"]["details"]["unknown"], json!(["body"]));
    let valid = res.body["error"]["details"]["valid"]
        .as_array()
        .expect("valid list");
    assert!(valid.contains(&json!("slug")));
    assert!(valid.contains(&json!("title")));
}

#[tokio::test]
async fn an_unknown_sort_key_is_refused_with_the_valid_ones() {
    let app = App::new().await;

    let res = app.get("/api/pages?sort=size").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid_parameter");
    assert_eq!(res.body["error"]["details"]["parameter"], "sort");
    assert_eq!(res.body["error"]["details"]["value"], "size");
    assert!(
        res.body["error"]["details"]["allowed"]
            .as_array()
            .unwrap()
            .contains(&json!("title"))
    );
}

// ----------------------------------------------------------------- words

/// The whole point of putting the count in the index: a manuscript's length is
/// a prefix filter and one field, rather than reading every file to add them up.
#[tokio::test]
async fn a_prefix_total_is_every_page_under_it() {
    let app = App::new().await;
    app.seed("book/one", json!({ "content": "one two three\n" }))
        .await;
    app.seed("book/two", json!({ "content": "four five\n" }))
        .await;
    app.seed("elsewhere", json!({ "content": "not part of the book\n" }))
        .await;

    let book = app.get("/api/pages?prefix=book").await;
    assert_eq!(book.body["total"], 2);
    assert_eq!(book.body["words"], 5, "three words plus two");

    let everything = app.get("/api/pages").await;
    assert_eq!(everything.body["words"], 10);
}

/// The total is the filtered set, not the page of results. Summing what came
/// back would make a book's length depend on how the caller paginated it, which
/// is the kind of number that looks right until somebody scrolls.
#[tokio::test]
async fn a_prefix_total_does_not_move_when_the_limit_does() {
    let app = App::new().await;
    for slug in ["book/one", "book/two", "book/three"] {
        app.seed(slug, json!({ "content": "two words\n" })).await;
    }

    let all = app.get("/api/pages?prefix=book").await;
    let one = app.get("/api/pages?prefix=book&limit=1").await;

    assert_eq!(all.body["words"], 6);
    assert_eq!(one.body["words"], 6, "the sum followed the limit");
    assert_eq!(one.body["pages"].as_array().unwrap().len(), 1);
}

/// `size` is the file and `words` is the prose, and a page that is mostly a code
/// fence is the case where they disagree loudly. Sorting on one is not sorting
/// on the other.
#[tokio::test]
async fn words_are_prose_and_size_is_bytes() {
    let app = App::new().await;
    app.seed(
        "sample",
        json!({ "content": "Two words.\n\n```rust\nfn main() { a lot of text in here }\n```\n" }),
    )
    .await;
    app.seed("prose", json!({ "content": "one two three four five\n" }))
        .await;

    let by_words = app.get("/api/pages?sort=words&order=desc").await;
    assert_eq!(by_words.body["pages"][0]["slug"], "prose");
    assert_eq!(by_words.body["pages"][0]["words"], 5);
    assert_eq!(by_words.body["pages"][1]["words"], 2);

    // The code-heavy page is the larger file and the smaller count.
    let sample = app.get("/api/pages/sample").await;
    let prose = app.get("/api/pages/prose").await;
    assert!(sample.body["size"].as_u64().unwrap() > prose.body["size"].as_u64().unwrap());
    assert!(sample.body["words"].as_u64().unwrap() < prose.body["words"].as_u64().unwrap());
}

/// A rebuild recomputes every count from the files. If it did not agree with the
/// incremental path, the number on screen would depend on when it was last
/// written rather than on what the page says.
#[tokio::test]
async fn a_rebuild_produces_the_same_counts() {
    let app = App::new().await;
    app.seed("book/one", json!({ "content": "one two three\n" }))
        .await;
    app.seed("book/two", json!({ "content": "four five\n" }))
        .await;

    let before = app.get("/api/pages?prefix=book").await;
    assert_eq!(
        app.post("/api/reindex", json!({})).await.status,
        StatusCode::OK
    );
    let after = app.get("/api/pages?prefix=book").await;

    assert_eq!(before.body["words"], after.body["words"]);
    assert_eq!(before.body["pages"], after.body["pages"]);
}

// -------------------------------------------------------------- contents

/// Absent means an ordinary page and `[]` means a manuscript with no chapters
/// yet. Collapsing the two would make "start a book" unexpressible.
#[tokio::test]
async fn an_absent_contents_and_an_empty_one_are_different() {
    let app = App::new().await;
    app.seed("ordinary", json!({ "content": "Body.\n" })).await;
    app.seed("started", json!({ "content": "Body.\n", "contents": [] }))
        .await;

    let ordinary = app.get("/api/pages/ordinary").await;
    let started = app.get("/api/pages/started").await;

    assert!(
        ordinary.body.get("contents").is_none(),
        "an ordinary page claimed to assemble something"
    );
    assert_eq!(started.body["contents"], json!([]));
}

/// A `PUT` replaces every field, so a client that does not send the list back
/// unmakes the manuscript. That is the documented behaviour rather than a bug,
/// and it is worth a test precisely because it is the trap `owner` already set.
#[tokio::test]
async fn a_put_that_omits_contents_clears_it() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "Body.\n", "contents": ["book/one"], "target": 90000 }),
    )
    .await;

    let kept = app
        .put(
            "/api/pages/book",
            json!({ "content": "Body.\n", "contents": ["book/one"], "target": 90000 }),
        )
        .await;
    assert_eq!(kept.body["contents"], json!(["book/one"]));
    assert_eq!(kept.body["target"], 90000);

    let dropped = app
        .put("/api/pages/book", json!({ "content": "Body.\n" }))
        .await;
    assert!(dropped.body.get("contents").is_none());
    assert!(dropped.body.get("target").is_none());
}

/// `PATCH` has three answers where `PUT` has two, and all three are different
/// requests: leave it, empty it, and make the page ordinary again.
#[tokio::test]
async fn patch_tells_an_omitted_contents_from_a_null_and_an_empty_one() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "Body.\n", "contents": ["book/one"] }),
    )
    .await;

    let untouched = app
        .patch("/api/pages/book", json!({ "title": "Book" }))
        .await;
    assert_eq!(untouched.body["contents"], json!(["book/one"]));

    let emptied = app
        .patch("/api/pages/book", json!({ "contents": [] }))
        .await;
    assert_eq!(emptied.body["contents"], json!([]));

    let cleared = app
        .patch("/api/pages/book", json!({ "contents": null }))
        .await;
    assert!(cleared.body.get("contents").is_none());
}

/// An entry nobody can resolve is one bad chapter, never a bad page. Parsing
/// these into slugs on the way in would make a typo cost the title, the tags and
/// every listing the page appears in.
#[tokio::test]
async fn a_contents_entry_that_is_not_a_slug_leaves_the_page_alone() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({
            "title": "The Long Way Round",
            "tags": ["manuscript"],
            "content": "Body.\n",
            "contents": ["book/one", "../etc/passwd", "", "not a slug at all"],
        }),
    )
    .await;

    let listing = app.get("/api/pages?tag=manuscript").await;
    assert_eq!(
        listing.body["total"], 1,
        "the page fell out of its own listing"
    );
    assert_eq!(listing.body["pages"][0]["title"], "The Long Way Round");

    let read = app.get("/api/pages/book").await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["title"], "The Long Way Round");
    assert_eq!(
        read.body["contents"],
        json!(["book/one", "../etc/passwd", "", "not a slug at all"]),
        "entries are kept as written for compile to judge"
    );
}

// -------------------------------------------------------------- drafting

/// The vocabulary is not fixed. A writer whose process has `with-beta-readers`
/// in it should not have to argue with a schema, so an unknown stage round-trips
/// as typed, appears in a listing, and can be filtered and sorted on.
#[tokio::test]
async fn a_stage_nobody_has_heard_of_survives_and_appears_in_a_listing() {
    let app = App::new().await;
    app.seed(
        "book/one/the-ferry",
        json!({ "content": "Prose.\n", "stage": "with-beta-readers" }),
    )
    .await;
    app.seed(
        "book/one/opening",
        json!({ "content": "Prose.\n", "stage": "Drafted" }),
    )
    .await;
    app.seed("book/two", json!({ "content": "Prose.\n" })).await;

    let read = app.get("/api/pages/book/one/the-ferry").await;
    assert_eq!(read.body["stage"], "with-beta-readers");

    let listing = app.get("/api/pages?prefix=book").await;
    let stages: Vec<Option<&str>> = listing.body["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|page| page["stage"].as_str())
        .collect();
    assert_eq!(
        stages,
        [Some("Drafted"), Some("with-beta-readers"), None],
        "a stage did not reach the listing as written"
    );

    // Filtering folds case, because two spellings of one stage are one stage.
    let drafted = app.get("/api/pages?stage=drafted").await;
    assert_eq!(drafted.body["total"], 1);
    assert_eq!(drafted.body["pages"][0]["slug"], "book/one/opening");
    assert_eq!(app.get("/api/pages?stage=DRAFTED").await.body["total"], 1);

    // A stage nobody uses is a question with an empty answer, not a mistake.
    let unused = app.get("/api/pages?stage=final").await;
    assert_eq!(unused.status, StatusCode::OK);
    assert_eq!(unused.body["total"], 0);

    // Sorted, with the pages that say nothing first.
    let sorted = app.get("/api/pages?prefix=book&sort=stage").await;
    let slugs: Vec<&str> = sorted.body["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|page| page["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        slugs,
        ["book/two", "book/one/opening", "book/one/the-ferry"]
    );
}

/// The API is unforgiving where a hand-written file is not: a stage that is not
/// a string never reaches the file, so nothing has to be lenient about it later.
#[tokio::test]
async fn a_stage_that_is_not_a_string_is_refused() {
    let app = App::new().await;

    let refused = app
        .post(
            "/api/pages",
            json!({ "slug": "book", "content": "B.\n", "stage": 3 }),
        )
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
}

/// A synopsis is prose somebody wrote about their own chapter. Every byte of it
/// is theirs, including the blank line that makes it two paragraphs and the
/// trailing space that a naive YAML emitter would eat.
#[tokio::test]
async fn a_synopsis_comes_back_byte_for_byte() {
    let app = App::new().await;
    let card = "He misses the crossing: and decides not to mind.\n\n\
                First time the narrator chooses to be late. ";

    app.seed(
        "book/one/the-ferry",
        json!({ "content": "Prose.\n", "synopsis": card }),
    )
    .await;

    let read = app.get("/api/pages/book/one/the-ferry").await;
    assert_eq!(read.body["synopsis"], card);

    // And off disk, which is where it actually has to survive.
    let written = std::fs::read_to_string(
        app.directory
            .path()
            .join("book")
            .join("one")
            .join("the-ferry.md"),
    )
    .expect("the page is on disk");
    let reread = app.get("/api/pages/book/one/the-ferry").await;
    assert_eq!(reread.body["synopsis"], card, "{written}");

    assert_eq!(
        app.get("/api/pages?prefix=book").await.body["pages"][0]["synopsis"],
        json!(card),
        "the listing disagreed with the page"
    );
}

/// Nothing fills a synopsis in. A page whose body opens with a perfectly good
/// sentence still has no synopsis, because a synopsis is a claim about what the
/// chapter does and nobody has made one.
#[tokio::test]
async fn a_synopsis_is_never_derived_from_the_body() {
    let app = App::new().await;
    app.seed(
        "book/one/the-ferry",
        json!({ "content": "# The Ferry\n\nHe misses the crossing. It is the first time.\n" }),
    )
    .await;

    let read = app.get("/api/pages/book/one/the-ferry").await;
    assert_eq!(
        read.body["title"], "The Ferry",
        "the title still falls back"
    );
    assert!(
        read.body.get("synopsis").is_none(),
        "a synopsis was invented: {:?}",
        read.body["synopsis"]
    );
}

/// Default true, and `true` writes nothing: the ordinary page's file must not
/// grow a line saying it is ordinary.
#[tokio::test]
async fn compile_false_round_trips_and_an_absent_one_is_not_written() {
    let app = App::new().await;
    app.seed("book/one/opening", json!({ "content": "Prose.\n" }))
        .await;
    app.seed(
        "book/one/cut-scene",
        json!({ "content": "Prose.\n", "compile": false }),
    )
    .await;

    let ordinary = app.get("/api/pages/book/one/opening").await;
    assert_eq!(ordinary.body["compile"], json!(true));

    let cut = app.get("/api/pages/book/one/cut-scene").await;
    assert_eq!(cut.body["compile"], json!(false));

    let file = |name: &str| {
        std::fs::read_to_string(
            app.directory
                .path()
                .join("book")
                .join("one")
                .join(format!("{name}.md")),
        )
        .expect("the page is on disk")
    };

    assert!(
        !file("opening").contains("compile"),
        "an ordinary page grew a compile line:\n{}",
        file("opening")
    );
    assert!(
        file("cut-scene").contains("compile: false"),
        "{}",
        file("cut-scene")
    );

    // Sending `true` back is the same as not saying it, so a page put back in
    // the book loses the line rather than gaining `compile: true`.
    app.patch("/api/pages/book/one/cut-scene", json!({ "compile": true }))
        .await;
    assert!(
        !file("cut-scene").contains("compile"),
        "{}",
        file("cut-scene")
    );
}

/// The rule that has been written down twice and broken once. A `PUT` replaces
/// every field, so an editor that does not send all four back unmakes them, and
/// nothing can infer any of them from anything else.
#[tokio::test]
async fn a_put_that_omits_the_drafting_fields_clears_all_four() {
    let app = App::new().await;
    let full = json!({
        "content": "Prose.\n",
        "synopsis": "He misses the crossing.",
        "stage": "drafted",
        "target": 3000,
        "compile": false,
    });
    app.seed("book/one/the-ferry", full.clone()).await;

    let kept = app.put("/api/pages/book/one/the-ferry", full).await;
    assert_eq!(kept.body["synopsis"], "He misses the crossing.");
    assert_eq!(kept.body["stage"], "drafted");
    assert_eq!(kept.body["target"], 3000);
    assert_eq!(kept.body["compile"], json!(false));

    let dropped = app
        .put(
            "/api/pages/book/one/the-ferry",
            json!({ "content": "Prose.\n" }),
        )
        .await;
    assert!(dropped.body.get("synopsis").is_none());
    assert!(dropped.body.get("stage").is_none());
    assert!(dropped.body.get("target").is_none());
    assert_eq!(
        dropped.body["compile"],
        json!(true),
        "an omitted compile flag is the default rather than a cleared field"
    );
}

/// `PATCH` leaves what it does not mention alone, and `null` is how each of the
/// four is cleared without touching the others.
#[tokio::test]
async fn patch_tells_an_omitted_drafting_field_from_a_null_one() {
    let app = App::new().await;
    app.seed(
        "book/one/the-ferry",
        json!({
            "content": "Prose.\n",
            "synopsis": "He misses the crossing.",
            "stage": "drafted",
            "compile": false,
        }),
    )
    .await;

    let untouched = app
        .patch(
            "/api/pages/book/one/the-ferry",
            json!({ "title": "The Ferry" }),
        )
        .await;
    assert_eq!(untouched.body["synopsis"], "He misses the crossing.");
    assert_eq!(untouched.body["stage"], "drafted");
    assert_eq!(untouched.body["compile"], json!(false));

    let restaged = app
        .patch(
            "/api/pages/book/one/the-ferry",
            json!({ "stage": "revised" }),
        )
        .await;
    assert_eq!(restaged.body["stage"], "revised");
    assert_eq!(
        restaged.body["synopsis"], "He misses the crossing.",
        "changing the stage moved the synopsis"
    );

    let cleared = app
        .patch(
            "/api/pages/book/one/the-ferry",
            json!({ "synopsis": null, "stage": null, "compile": null }),
        )
        .await;
    assert!(cleared.body.get("synopsis").is_none());
    assert!(cleared.body.get("stage").is_none());
    assert_eq!(cleared.body["compile"], json!(true));
}

/// The columns are derived, so the incremental path and the rebuild have to
/// produce the same rows. If they did not, what a listing said would depend on
/// when the page was last written rather than on what it says.
#[tokio::test]
async fn a_rebuild_produces_the_same_synopses_and_stages() {
    let app = App::new().await;
    app.seed(
        "book/one/opening",
        json!({ "content": "Prose.\n", "synopsis": "They leave.", "stage": "Revised" }),
    )
    .await;
    app.seed(
        "book/one/the-ferry",
        json!({ "content": "Prose.\n", "stage": "with-beta-readers" }),
    )
    .await;
    app.seed("book/two", json!({ "content": "Prose.\n" })).await;

    let before = app.get("/api/pages?prefix=book&sort=stage").await;
    assert_eq!(
        app.post("/api/reindex", json!({})).await.status,
        StatusCode::OK
    );
    let after = app.get("/api/pages?prefix=book&sort=stage").await;

    assert_eq!(before.body["pages"], after.body["pages"]);
    assert_eq!(after.body["pages"][2]["synopsis"], json!(null));
    assert_eq!(
        app.get("/api/pages?stage=REVISED").await.body["total"],
        1,
        "the filter stopped working after a rebuild"
    );
}

/// A due date is a full timestamp over the API and normalises like `created`.
#[tokio::test]
async fn a_target_and_a_due_date_round_trip() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "Body.\n", "target": 90000, "due": "2027-03-01T00:00:00Z" }),
    )
    .await;

    let read = app.get("/api/pages/book").await;
    assert_eq!(read.body["target"], 90000);
    assert_eq!(read.body["due"], "2027-03-01T00:00:00Z");
}

// --------------------------------------------------------------- compile

/// Seed a small book: a root, a part, and two chapters under it.
async fn seed_book(app: &App) {
    app.seed(
        "book",
        json!({ "content": "# The Long Way Round\n\nA note.\n", "contents": ["book/one"], "target": 90000 }),
    )
    .await;
    app.seed(
        "book/one",
        json!({ "content": "# Part One\n\n> An epigraph.\n", "contents": ["book/one/opening", "book/one/the-ferry"] }),
    )
    .await;
    app.seed(
        "book/one/opening",
        json!({ "content": "# Opening\n\nFirst.\n" }),
    )
    .await;
    app.seed(
        "book/one/the-ferry",
        json!({ "content": "# The Ferry\n\nSecond.\n" }),
    )
    .await;
}

#[tokio::test]
async fn compiles_a_book_into_one_document_with_a_map_back() {
    let app = App::new().await;
    seed_book(&app).await;

    let res = app.get("/api/compile?root=book").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["compiler"], "compile/v1");
    assert_eq!(res.body["target"], 90000);

    let document = res.body["content"].as_str().expect("content");
    // One hierarchy, not four competing ones.
    assert!(document.contains("# The Long Way Round"));
    assert!(document.contains("## Part One"));
    assert!(document.contains("### Opening"));
    assert!(document.contains("### The Ferry"));

    let sections = res.body["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 4);
    assert_eq!(sections[0]["slug"], "book");
    assert_eq!(sections[0]["depth"], 0);
    assert_eq!(sections[3]["slug"], "book/one/the-ferry");
    assert_eq!(sections[3]["depth"], 2);

    // Every offset indexes into the bytes that came back.
    for section in sections {
        if section["status"] != "included" {
            continue;
        }
        let offset = section["offset"].as_u64().unwrap() as usize;
        let length = section["length"].as_u64().unwrap() as usize;
        let slice = &document[offset..offset + length];
        assert!(
            slice.contains(section["title"].as_str().unwrap()),
            "{} was not at the offset the manifest claims",
            section["slug"]
        );
    }
}

/// The card, the badge and the two numbers, over HTTP. Everything an included
/// section says about itself is in the manifest; a gap says nothing, because
/// there is nothing there to say it.
#[tokio::test]
async fn the_manifest_carries_the_synopsis_the_stage_and_both_counts() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "# Book\n", "contents": ["book/one", "book/missing"] }),
    )
    .await;
    app.seed(
        "book/one",
        json!({
            "content": "# Part One\n\n> Four words of epigraph.\n",
            "contents": ["book/one/the-ferry"],
            "target": 5000,
        }),
    )
    .await;
    app.seed(
        "book/one/the-ferry",
        json!({
            "content": "# The Ferry\n\none two three four five\n",
            "synopsis": "He misses the crossing and decides not to mind.",
            "stage": "drafted",
            "target": 3000,
        }),
    )
    .await;

    let sections = app.get("/api/compile?root=book").await.body["sections"].clone();
    let at = |index: usize| sections[index].clone();

    let ferry = at(2);
    assert_eq!(ferry["slug"], "book/one/the-ferry");
    assert_eq!(
        ferry["synopsis"],
        "He misses the crossing and decides not to mind."
    );
    assert_eq!(ferry["stage"], "drafted");
    assert_eq!(ferry["target"], 3000);
    assert_eq!(
        ferry["words"], ferry["subtree"],
        "on a leaf the two counts are one number"
    );

    // The part's own words are its heading and its epigraph; its subtree is what
    // the target actually means.
    let part = at(1);
    assert_eq!(part["words"], 2 + 4);
    assert_eq!(part["subtree"], (2 + 4) + (2 + 5));
    assert_eq!(part["target"], 5000);

    let gap = at(3);
    assert_eq!(gap["status"], "wanted");
    assert!(gap.get("synopsis").is_none());
    assert!(gap.get("stage").is_none());
    assert!(gap.get("target").is_none());
    assert_eq!(gap["words"], 0);
    assert_eq!(gap["subtree"], 0);
}

/// `compile: false` is a sixth status rather than an absence, it takes the whole
/// subtree with it, and none of that changes what the wiki knows about structure.
#[tokio::test]
async fn an_excluded_chapter_leaves_the_book_and_stays_in_the_wiki() {
    let app = App::new().await;
    seed_book(&app).await;
    app.patch("/api/pages/book/one/opening", json!({ "compile": false }))
        .await;

    let compiled = app.get("/api/compile?root=book").await;
    let sections = compiled.body["sections"].as_array().expect("sections");
    let by_slug: Vec<(&str, &str)> = sections
        .iter()
        .map(|section| {
            (
                section["slug"].as_str().unwrap(),
                section["status"].as_str().unwrap(),
            )
        })
        .collect();

    assert_eq!(
        by_slug,
        [
            ("book", "included"),
            ("book/one", "included"),
            ("book/one/opening", "excluded"),
            ("book/one/the-ferry", "included"),
        ],
        "the cut chapter left the manifest instead of holding its position"
    );
    assert!(
        !compiled.body["content"]
            .as_str()
            .unwrap()
            .contains("Opening")
    );

    // Excluded from the book, not from the wiki. Still not an orphan, because a
    // `page_parts` row is what the spine is and `compile` is what this document
    // is; the two are different questions.
    let stats = app.get("/api/stats").await;
    let orphans: Vec<&str> = stats.body["orphans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|orphan| orphan["slug"].as_str().unwrap())
        .collect();
    assert!(
        !orphans.contains(&"book/one/opening"),
        "a cut chapter fell out of the wiki: {orphans:?}"
    );

    // And the graph still draws the line to it.
    let edges = app.get("/api/graph").await.body["edges"].clone();
    let drawn = edges.as_array().unwrap().iter().any(|edge| {
        edge["source"] == "book/one" && edge["target"] == "book/one/opening" && edge["part"] == true
    });
    assert!(drawn, "the spine stopped drawing an excluded chapter");
}

/// The order in the frontmatter is the order in the document, which is the whole
/// reason the spine is its own table keyed by position.
#[tokio::test]
async fn the_contents_order_is_the_document_order() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "# Book\n", "contents": ["b", "a", "c"] }),
    )
    .await;
    for slug in ["a", "b", "c"] {
        app.seed(slug, json!({ "content": format!("# {slug}\n") }))
            .await;
    }

    let res = app.get("/api/compile?root=book").await;
    let slugs: Vec<&str> = res.body["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|section| section["slug"].as_str().unwrap())
        .collect();

    assert_eq!(slugs, ["book", "b", "a", "c"]);
}

/// Reordering the list reorders the document, with nothing left behind. A
/// shorter list is the case a positional key gets wrong if the old rows survive.
#[tokio::test]
async fn shortening_a_contents_list_drops_the_chapters_it_removed() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "# Book\n", "contents": ["a", "b", "c"] }),
    )
    .await;
    for slug in ["a", "b", "c"] {
        app.seed(slug, json!({ "content": format!("# {slug}\n") }))
            .await;
    }

    app.put(
        "/api/pages/book",
        json!({ "content": "# Book\n", "contents": ["c"] }),
    )
    .await;

    let res = app.get("/api/compile?root=book").await;
    let slugs: Vec<&str> = res.body["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|section| section["slug"].as_str().unwrap())
        .collect();

    assert_eq!(slugs, ["book", "c"]);
}

#[tokio::test]
async fn a_gap_and_a_bad_entry_keep_their_positions() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "# Book\n", "contents": ["a", "book/missing", "../etc/passwd", "b"] }),
    )
    .await;
    app.seed("a", json!({ "content": "# A\n" })).await;
    app.seed("b", json!({ "content": "# B\n" })).await;

    let res = app.get("/api/compile?root=book").await;
    let reported: Vec<(&str, &str)> = res.body["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|section| {
            (
                section["slug"].as_str().unwrap(),
                section["status"].as_str().unwrap(),
            )
        })
        .collect();

    assert_eq!(
        reported,
        [
            ("book", "included"),
            ("a", "included"),
            ("book/missing", "wanted"),
            ("../etc/passwd", "invalid"),
            ("b", "included"),
        ]
    );
}

#[tokio::test]
async fn html_is_rendered_from_the_assembled_document() {
    let app = App::new().await;
    seed_book(&app).await;

    let res = app.get("/api/compile?root=book&format=html").await;
    let html = res.body["content"].as_str().expect("content");

    assert!(html.contains("<h1>"), "{html}");
    assert!(
        html.contains("<h3>"),
        "the shifted levels did not survive: {html}"
    );
}

/// The `json` format hands over the parts rather than the whole, so a caller can
/// take one chapter without slicing bytes itself.
#[tokio::test]
async fn the_json_format_carries_each_section_and_no_document() {
    let app = App::new().await;
    seed_book(&app).await;

    let res = app.get("/api/compile?root=book&format=json").await;

    assert!(res.body.get("content").is_none());
    let sections = res.body["sections"].as_array().unwrap();
    assert!(sections[0]["content"].as_str().unwrap().contains("A note."));
}

#[tokio::test]
async fn a_style_page_is_prepended_and_is_in_the_manifest() {
    let app = App::new().await;
    seed_book(&app).await;
    app.seed(
        "rules/voice",
        json!({ "content": "# Voice\n\nPast tense.\n" }),
    )
    .await;

    let res = app.get("/api/compile?root=book&style=rules/voice").await;

    assert!(res.body["content"].as_str().unwrap().starts_with("# Voice"));
    assert_eq!(res.body["sections"][0]["slug"], "rules/voice");
    assert_eq!(res.body["sections"][1]["slug"], "book");
}

#[tokio::test]
async fn compiling_from_nothing_names_the_root_it_looked_for() {
    let app = App::new().await;

    let res = app.get("/api/compile?root=book").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "compile_root_not_found");
    assert_eq!(res.body["error"]["details"]["slug"], "book");
}

#[tokio::test]
async fn a_compile_root_that_is_not_a_slug_is_refused() {
    let app = App::new().await;
    let res = app.get("/api/compile?root=../etc/passwd").await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

/// A refusal rather than a truncation, and it names the slug: that is the one
/// thing the caller cannot work out from the limit alone.
#[tokio::test]
async fn a_chain_past_the_depth_limit_is_refused_rather_than_cut_short() {
    let app = App::new().await;
    for level in 0..20 {
        app.seed(
            &format!("p{level}"),
            json!({ "content": format!("# Level {level}\n"), "contents": [format!("p{}", level + 1)] }),
        )
        .await;
    }

    let res = app.get("/api/compile?root=p0").await;

    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(res.code(), "compile_too_large");
    assert_eq!(res.body["error"]["details"]["limit"], "depth");
    assert_eq!(res.body["error"]["details"]["ceiling"], 16);
    assert_eq!(res.body["error"]["details"]["slug"], "p17");
}

/// The gate that justifies indexing the spine at all: without it every chapter
/// in the wiki is unreferenced, and the orphan count is one of the two numbers
/// `/api/stats` exists for.
#[tokio::test]
async fn a_chapter_named_by_a_contents_list_is_not_an_orphan() {
    let app = App::new().await;
    seed_book(&app).await;
    app.seed("loose", json!({ "content": "Nobody points at this.\n" }))
        .await;

    let stats = app.get("/api/stats").await;
    let orphans: Vec<&str> = stats.body["orphans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|orphan| orphan["slug"].as_str().unwrap())
        .collect();

    // The root is an orphan: nothing assembles the book. Its chapters are not.
    assert!(orphans.contains(&"book"));
    assert!(orphans.contains(&"loose"));
    assert!(
        !orphans.contains(&"book/one"),
        "a part named by its book was reported as an orphan"
    );
    assert!(!orphans.contains(&"book/one/opening"));
    assert_eq!(stats.body["orphan_count"], 2);
}

/// A chapter dropped from the list goes back to being unreferenced, which is the
/// same self-healing the link graph has and needs no reindex of its own.
#[tokio::test]
async fn removing_a_chapter_from_the_spine_makes_it_an_orphan_again() {
    let app = App::new().await;
    app.seed("book", json!({ "content": "# Book\n", "contents": ["a"] }))
        .await;
    app.seed("a", json!({ "content": "# A\n" })).await;

    assert_eq!(app.get("/api/stats").await.body["orphan_count"], 1);

    app.put("/api/pages/book", json!({ "content": "# Book\n" }))
        .await;

    assert_eq!(app.get("/api/stats").await.body["orphan_count"], 2);
}

/// The half of the L1 plan that waited for a panel to exist. Orphans needed the
/// union straight away, because a chapter reported as unreferenced is a number
/// being wrong; drawing needed somebody to decide what a part edge looks like.
#[tokio::test]
async fn the_graph_draws_the_spine_and_says_which_lines_are_parts() {
    let app = App::new().await;
    seed_book(&app).await;

    let graph = app.get("/api/graph").await;
    let edges = graph.body["edges"].as_array().expect("edges");

    let spine: Vec<(&str, &str)> = edges
        .iter()
        .filter(|edge| edge["part"] == true)
        .map(|edge| {
            (
                edge["source"].as_str().unwrap(),
                edge["target"].as_str().unwrap(),
            )
        })
        .collect();

    assert_eq!(
        spine,
        [
            ("book", "book/one"),
            ("book/one", "book/one/opening"),
            ("book/one", "book/one/the-ferry"),
        ]
    );

    // Not a sixth kind of link. A part is not a kind of link, which is the whole
    // reason `page_parts` is its own table.
    for edge in edges {
        assert_eq!(edge["kinds"], json!([]), "nothing in this book links");
    }

    // ...and a chapter's neighbourhood holds the book it belongs to, or the one
    // page it is certain to be connected to would be the one a walk cannot find.
    let walk = app.get("/api/graph?root=book/one/opening&depth=2").await;
    let reached: Vec<&str> = walk.body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["slug"].as_str().unwrap())
        .collect();
    assert!(reached.contains(&"book"), "got {reached:?}");
    assert!(reached.contains(&"book/one/the-ferry"), "got {reached:?}");
}

/// `page_parts.target` is the entry as it was written, so a mistyped one is in
/// the table. Drawing it would advertise a path as a page worth writing.
#[tokio::test]
async fn a_contents_entry_that_is_not_a_slug_is_not_drawn() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "# Book\n", "contents": ["../etc/passwd", "a"] }),
    )
    .await;
    app.seed("a", json!({ "content": "# A\n" })).await;

    let graph = app.get("/api/graph").await;
    let slugs: Vec<&str> = graph.body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["slug"].as_str().unwrap())
        .collect();

    assert_eq!(slugs, ["a", "book"]);
    // It is still reported in position by the manifest, which is where a
    // mistyped chapter belongs.
    let sections = app.get("/api/compile?root=book").await;
    assert_eq!(sections.body["sections"][1]["status"], "invalid");

    // And it is not somewhere to write. The dashboard names a wanted page, so
    // this is the query that must never carry a path.
    let stats = app.get("/api/stats").await;
    assert_eq!(stats.body["wanted_count"], 0);
    assert_eq!(stats.body["wanted"], json!([]));
}

/// The two numbers `/api/stats` exists for, agreeing about one wiki.
///
/// A chapter listed in a `contents:` list is not an orphan and is wanted, and it
/// took a book in the example wiki to notice that only the first half was true:
/// the graph drew the gap as a wanted node while the card beside it counted
/// links alone and said the wiki wanted nothing.
#[tokio::test]
async fn a_chapter_nobody_has_written_is_wanted_and_its_siblings_are_not_orphans() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({ "content": "# The Long Way Round\n", "contents": ["book/one"] }),
    )
    .await;
    app.seed(
        "book/one",
        json!({
            "content": "# Part One\n",
            "contents": ["book/one/opening", "book/one/the-crossing"],
        }),
    )
    .await;
    app.seed("book/one/opening", json!({ "content": "# Opening\n" }))
        .await;

    let stats = app.get("/api/stats").await;

    assert_eq!(stats.body["wanted_count"], 1);
    assert_eq!(stats.body["wanted"][0]["slug"], "book/one/the-crossing");
    assert_eq!(stats.body["wanted"][0]["referrers"], 1);

    // The narrower figure stays narrow: nothing here is a link.
    assert_eq!(stats.body["links"]["wanted"], 0);

    // And the written chapters are still not orphans, which is the half that
    // was already true.
    assert_eq!(stats.body["orphan_count"], 1);
    assert_eq!(stats.body["orphans"][0]["slug"], "book");
}

// -------------------------------------------------------------- word log

/// The fields of one logged line, by name rather than by index.
const AT: usize = 0;
const SLUG: usize = 1;
const ACTOR: usize = 2;
const ACCOUNT: usize = 3;
const KIND: usize = 4;
const ADDED: usize = 5;
const REMOVED: usize = 6;
const TOTAL: usize = 7;
const FROM: usize = 8;

/// The whole reason this feature exists. A rewrite is not "minus one hundred".
#[tokio::test]
async fn a_rewrite_reports_both_halves_rather_than_their_difference() {
    let app = App::new().await;

    app.seed(
        "chapter",
        json!({ "content": "the ferry was late and nobody was surprised at all\n" }),
    )
    .await;

    app.put(
        "/api/pages/chapter",
        json!({ "content": "the ferry was early and everybody was surprised\n" }),
    )
    .await;

    let log = app.log();
    assert_eq!(log.len(), 2);

    // A page written through the API is words that have just arrived, not a
    // wiki that was already there.
    assert_eq!(log[0][KIND], "observed");
    assert_eq!(log[0][ADDED], "10");
    assert_eq!(log[0][REMOVED], "0");
    assert_eq!(log[0][TOTAL], "10");

    // Both halves, and neither of them the whole sentence: the words the two
    // versions share were not written a second time.
    assert_eq!(log[1][ADDED], "2", "early, everybody");
    assert_eq!(log[1][REMOVED], "4", "late, nobody, at, all");
    assert_eq!(log[1][TOTAL], "8");

    // `total` is the check. It has to be the page's own count, and the two
    // halves have to account for the change in it.
    assert_eq!(app.get("/api/pages?prefix=chapter").await.body["words"], 8);

    let series = app.get("/api/word-stats").await;
    assert_eq!(series.body["totals"]["added"], 12);
    assert_eq!(series.body["totals"]["removed"], 4);
    assert_eq!(
        series.body["totals"]["delta"], 8,
        "which is the page's length, and not the only figure on offer"
    );
}

/// Four writes, four distinguishable records. The label is a claim rather than a
/// proof, which is exactly why it is worth recording who claimed what.
#[tokio::test]
async fn a_write_is_labelled_by_the_tool_that_made_it() {
    let app = App::new().await;

    app.post(
        "/api/pages",
        json!({ "slug": "a", "content": "One two three.\n" }),
    )
    .await;
    app.send_as(
        Method::POST,
        "/api/pages",
        Some(json!({ "slug": "b", "content": "Four five six.\n" })),
        "claude-code",
    )
    .await;
    app.send_as(
        Method::PUT,
        "/api/pages/b",
        Some(json!({ "content": "Four five six seven.\n" })),
        "web",
    )
    .await;

    let actors: Vec<String> = app.log().iter().map(|line| line[ACTOR].clone()).collect();

    assert_eq!(actors, ["api", "claude-code", "web"]);
}

/// A label that could not be written into a tab-separated line is refused rather
/// than trimmed to fit, because silently rewriting somebody's provenance is
/// worse than telling them the header was no good.
#[tokio::test]
async fn a_label_that_would_break_the_log_is_refused() {
    let app = App::new().await;

    let res = app
        .send_as(
            Method::POST,
            "/api/pages",
            Some(json!({ "slug": "a", "content": "One.\n" })),
            "two\tfields",
        )
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "invalid_actor");
    assert_eq!(
        res.body["error"]["details"]["header"], "x-rhizolog-actor",
        "and it names the header rather than echoing what was refused"
    );
    assert!(
        !res.body.to_string().contains("two"),
        "the value was just refused for being unprintable; it does not belong in a JSON error"
    );

    // And the write did not happen.
    assert_eq!(app.get("/api/pages/a").await.status, StatusCode::NOT_FOUND);
}

/// A move is one record naming both slugs, not a rewrite of history. A delete
/// and a new page at the same slug are two series, because otherwise the chart
/// would show a page losing forty thousand words and gaining them back.
#[tokio::test]
async fn a_move_keeps_one_series_and_a_delete_starts_another() {
    let app = App::new().await;

    app.seed("old", json!({ "content": "One two three four.\n" }))
        .await;
    app.post("/api/move", json!({ "from": "old", "to": "new" }))
        .await;
    app.put(
        "/api/pages/new",
        json!({ "content": "One two three four five.\n" }),
    )
    .await;

    let log = app.log();
    assert_eq!(log[1][KIND], "moved");
    assert_eq!(log[1][SLUG], "new");
    assert_eq!(log[1][FROM], "old");
    assert_eq!(
        (log[1][ADDED].as_str(), log[1][REMOVED].as_str()),
        ("0", "0"),
        "nothing was written; a move is a marker rather than a churn"
    );

    // The next edit is an ordinary one, diffed against what the page held before
    // it moved rather than treated as a brand new page.
    assert_eq!(log[2][KIND], "observed");
    assert_eq!(log[2][ADDED], "1");
    assert_eq!(log[2][REMOVED], "0");

    // Now delete it and write something else at the same slug.
    app.delete("/api/pages/new").await;
    app.seed(
        "new",
        json!({ "content": "Something else entirely here.\n" }),
    )
    .await;

    let log = app.log();
    assert_eq!(log[3][KIND], "deleted");
    assert_eq!(
        log[3][REMOVED], "0",
        "the words were written and deleting the file does not unwrite them"
    );
    assert_eq!(log[4][KIND], "observed");
    assert_eq!(
        log[4][ADDED], "4",
        "a fresh series, not a four-word edit of a five-word page"
    );
}

/// A page nobody wrote to twice produces one line, not one per save.
#[tokio::test]
async fn saving_a_page_nobody_changed_records_nothing() {
    let app = App::new().await;
    let body = json!({ "content": "One two three.\n" });

    app.seed("a", body.clone()).await;
    app.put("/api/pages/a", body.clone()).await;
    app.put("/api/pages/a", body).await;

    assert_eq!(app.log().len(), 1);
}

/// Formatting is not writing, which is why the diff runs over the extracted text
/// rather than over the markdown.
#[tokio::test]
async fn reflowing_a_paragraph_is_not_words() {
    let app = App::new().await;

    app.seed("a", json!({ "content": "One two three four five six.\n" }))
        .await;
    app.put(
        "/api/pages/a",
        json!({ "content": "One two three\nfour five six.\n" }),
    )
    .await;

    assert_eq!(app.log().len(), 1);
}

/// On a wiki with no accounts there is no name to give, which is the same thing
/// `owner` does on a capture.
#[tokio::test]
async fn an_open_wiki_records_a_tool_and_no_account() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "One.\n" })).await;

    let log = app.log();
    assert_eq!(log[0][ACCOUNT], "");
    assert!(log[0][AT].ends_with('Z'), "got {}", log[0][AT]);
}

// -------------------------------------------------------------- word stats

#[tokio::test]
async fn the_series_comes_back_by_day_by_tool_and_by_page() {
    let app = App::new().await;

    app.send_as(
        Method::POST,
        "/api/pages",
        Some(json!({ "slug": "a", "content": "One two three four.\n" })),
        "claude-code",
    )
    .await;
    app.send_as(
        Method::POST,
        "/api/pages",
        Some(json!({ "slug": "b", "content": "Five six.\n" })),
        "web",
    )
    .await;

    let res = app.get("/api/word-stats").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["resolution"], "observed");
    assert_eq!(res.body["totals"]["added"], 6);
    assert_eq!(res.body["totals"]["removed"], 0);
    assert_eq!(res.body["totals"]["delta"], 6);
    assert_eq!(res.body["totals"]["pages"], 2);

    let actors = res.body["actors"].as_array().expect("actors");
    assert_eq!(actors.len(), 2);
    assert_eq!(actors[0]["actor"], "claude-code");
    assert_eq!(actors[0]["added"], 4);
    assert_eq!(actors[1]["actor"], "web");

    let pages = res.body["pages"].as_array().expect("pages");
    assert_eq!(pages[0]["slug"], "a");
    assert_eq!(pages[0]["title"], "A", "the derived title, not the slug");

    // Every day in the window, so a chart can draw the gaps.
    let days = res.body["days"].as_array().expect("days");
    assert_eq!(days.len(), 90);
    assert_eq!(
        days.iter()
            .map(|day| day["added"].as_u64().unwrap())
            .sum::<u64>(),
        6
    );
}

#[tokio::test]
async fn a_window_the_caller_named_is_the_window_it_gets() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "One two.\n" })).await;

    // A window that ended before anything was written.
    let res = app
        .get("/api/word-stats?from=2020-01-01T00:00:00Z&to=2020-01-08T00:00:00Z")
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["days"].as_array().expect("days").len(), 7);
    assert_eq!(res.body["totals"]["added"], 0);
    assert_eq!(res.body["totals"]["observations"], 0);
}

/// Bookkeeping is not writing, so a baseline is not a day on which somebody
/// wrote a whole wiki.
#[tokio::test]
async fn a_reindex_of_an_untouched_wiki_adds_nothing_to_the_series() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "One two three.\n" }))
        .await;

    let before = app.get("/api/word-stats").await.body["totals"].clone();
    assert_eq!(
        app.post("/api/reindex", json!({})).await.status,
        StatusCode::OK
    );
    let after = app.get("/api/word-stats").await.body["totals"].clone();

    assert_eq!(before, after);
    assert_eq!(app.log().len(), 1, "and no new line was written");
}

/// The word log is read and replaced rather than reconciled, so it is reported
/// differently. Three of the five fields the other trees carry would be zero
/// here for reasons that mean nothing, and zeroes that mean nothing read as news.
#[tokio::test]
async fn a_reindex_reports_what_the_word_log_held() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "One two three.\n" }))
        .await;

    let res = app.post("/api/reindex", json!({})).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["words"]["observations"], 1);
    assert_eq!(res.body["words"]["skipped"], 0);
    assert_eq!(res.body["words"]["read"], true);
    // And nothing that would be a lie about an operation that does not compare
    // files.
    assert!(res.body["words"]["scanned"].is_null());
    assert!(res.body["words"]["unchanged"].is_null());
}

/// A truncated last line after a hard power-off costs that line and nothing
/// else, and the report says how many it cost.
#[tokio::test]
async fn a_reindex_counts_log_lines_it_could_not_read() {
    let app = App::new().await;
    let month = app.directory.path().join(".rhizolog/words/2026-08.log");
    std::fs::create_dir_all(month.parent().expect("a parent")).expect("the words directory");
    std::fs::write(&month, "half a line\nnor this one\n").expect("write a broken log");

    let res = app.post("/api/reindex", json!({})).await;

    assert_eq!(res.body["words"]["skipped"], 2);
    assert_eq!(res.body["words"]["observations"], 0);
    assert_eq!(res.body["words"]["read"], true);
}

/// The manifest says which list named each entry and where in it, which is the
/// whole of what a client needs to move one without reading any page.
#[tokio::test]
async fn the_manifest_says_which_contents_list_named_each_entry() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({
            "content": "# Book\n",
            "contents": ["book/one", "book/gone", "book/one", "../nope"],
        }),
    )
    .await;
    app.seed("book/one", json!({ "content": "# One\n" })).await;

    let res = app.get("/api/compile?root=book").await;
    let sections = res.body["sections"].as_array().expect("sections");

    // The root is the page that was asked for rather than one the spine reaches,
    // so nothing named it.
    assert!(sections[0]["parent"].is_null());
    assert!(sections[0]["ordinal"].is_null());

    // Rebuilt by ordinal, the way a client has to do it, and byte for byte what
    // the frontmatter says, repeat and typo included.
    let mut entries: Vec<(u64, &str)> = sections[1..]
        .iter()
        .map(|section| {
            assert_eq!(section["parent"], "book");
            (
                section["ordinal"].as_u64().expect("an ordinal"),
                section["slug"].as_str().expect("a slug"),
            )
        })
        .collect();
    entries.sort_by_key(|(at, _)| *at);

    assert_eq!(
        entries,
        vec![
            (0, "book/one"),
            (1, "book/gone"),
            (2, "book/one"),
            (3, "../nope"),
        ]
    );
}

/// Moving a chapter is a `PATCH` of the list that names it, and nothing else:
/// no new endpoint, and the entries a compile could not resolve survive it.
#[tokio::test]
async fn reordering_a_spine_is_a_patch_of_its_contents() {
    let app = App::new().await;
    app.seed(
        "book",
        json!({
            "content": "# Book\n",
            "contents": ["book/one", "book/gone", "../nope"],
        }),
    )
    .await;
    app.seed("book/one", json!({ "content": "# One\n" })).await;

    let res = app
        .patch(
            "/api/pages/book",
            json!({ "contents": ["book/gone", "book/one", "../nope"] }),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);

    let compiled = app.get("/api/compile?root=book").await;
    let order: Vec<&str> = compiled.body["sections"]
        .as_array()
        .expect("sections")
        .iter()
        .map(|section| section["slug"].as_str().expect("a slug"))
        .collect();

    assert_eq!(order, vec!["book", "book/gone", "book/one", "../nope"]);

    // A reorder is not an edit. The prose the page holds is the prose it held,
    // and a `PATCH` that named only `contents` left everything else alone.
    let root = app.get("/api/pages/book").await;
    assert_eq!(root.body["content"], "# Book\n");
    assert_eq!(root.body["contents"][0], "book/gone");
}

// ------------------------------------------------------------------ pace

/// Seed a book with a target, a deadline and a scene that was cut.
///
/// The caller asks about an instant taken **after** this returns, so every
/// observation the seeding writes is inside the window whatever the clock says.
/// Taking it before would put the writes on the far side of midnight a few times
/// in a million runs, and a test that fails once a decade is worse than one that
/// tests slightly less.
async fn seed_paced_book(app: &App, due: DateTime<Utc>) {
    app.seed(
        "book",
        json!({
            "content": "# Book\n\nAn epigraph.\n",
            "contents": ["book/one", "book/cut"],
            "target": 1000,
            "due": due.to_rfc3339_opts(SecondsFormat::Secs, true),
        }),
    )
    .await;
    app.seed(
        "book/one",
        json!({ "content": "# Part One\n\nFour words of prose.\n" }),
    )
    .await;
    app.seed(
        "book/cut",
        json!({ "content": "# The Argument\n\nCut, and not thrown away.\n", "compile": false }),
    )
    .await;
}

/// Which instant to ask about, as a query parameter.
///
/// `Z` rather than `+00:00`: a plus sign in a query string decodes as a space,
/// which would make the timestamp unparseable and the failure look like a bug in
/// the handler.
fn asking_at(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The whole feature over HTTP, and every figure in it recomputed from the ones
/// beside it, which is the gate this endpoint is measured against.
#[tokio::test]
async fn the_pace_is_words_remaining_over_days_remaining() {
    let app = App::new().await;
    let due = Utc::now()
        .checked_add_days(Days::new(9))
        .expect("a due date");
    seed_paced_book(&app, due).await;
    let at = Utc::now();

    let res = app
        .get(&format!(
            "/api/pace?root=book&at={}&offset=0",
            asking_at(at)
        ))
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["ruleset"], "pace/v1");
    assert_eq!(res.body["root"], "book");

    // What a reader would get, which is what the target is measured against.
    let compiled = app.get("/api/compile?root=book").await;
    let words = compiled.body["words"].as_u64().expect("a compiled total");
    assert_eq!(res.body["words"], words);
    assert_eq!(res.body["target"], 1000);
    assert_eq!(res.body["remaining"], 1000 - words as i64);

    // Everything in the book was written by the seeding above and nothing was
    // taken away, so the fortnight's net is the book.
    assert_eq!(res.body["window"]["days"], 14);
    assert_eq!(res.body["window"]["removed"], 0);
    assert_eq!(res.body["window"]["net"], words);
    assert_eq!(
        res.body["window"]["per_day"].as_f64(),
        Some(words as f64 / 14.0)
    );
    // Which local day each write fell on is `pace::build`'s question and is
    // pinned there against fixed instants. All this can say is that a run of
    // writes was not filed under no day at all.
    assert!(res.body["window"]["active_days"].as_u64().expect("days") >= 1);

    // The deadline half is wired, and the response's own arithmetic holds: the
    // rate really is the remainder over the days, which is the property the whole
    // receipt is for. Where the day boundary falls is pinned in `pace::build`.
    let left = res.body["days_remaining"].as_i64().expect("days remaining");
    assert!(left > 0, "nine days out is not a deadline that has gone");
    assert_eq!(
        res.body["required_per_day"].as_f64(),
        Some((1000 - words as i64) as f64 / left as f64)
    );
}

/// The contrast the `uncounted` block exists for, and the one way these two
/// numbers get misread. A day spent on a scene that is out of the book is a day
/// the compiled total did not move.
#[tokio::test]
async fn words_written_into_a_cut_scene_are_reported_beside_the_book_and_not_in_it() {
    let app = App::new().await;
    let due = Utc::now()
        .checked_add_days(Days::new(9))
        .expect("a due date");
    seed_paced_book(&app, due).await;
    let at = Utc::now();

    let res = app
        .get(&format!(
            "/api/pace?root=book&at={}&offset=0",
            asking_at(at)
        ))
        .await;

    let cut = app.get("/api/pages/book/cut").await;
    let cut_words = cut.body["words"].as_u64().expect("the cut scene's length");

    assert!(cut_words > 0);
    assert_eq!(res.body["uncounted"]["net"], cut_words);
    assert_eq!(res.body["uncounted"]["observations"], 1);
    assert_eq!(res.body["uncounted"]["pages"][0]["slug"], "book/cut");

    // And nowhere in the rate, which has to be in the same currency as
    // `remaining` or dividing one by the other means nothing.
    let counted: Vec<&str> = res.body["window"]["pages"]
        .as_array()
        .expect("the pages counted")
        .iter()
        .map(|page| page["slug"].as_str().expect("a slug"))
        .collect();
    assert!(
        !counted.contains(&"book/cut"),
        "the cut scene was in the rate"
    );

    // The word log still has all of it. The two answer different questions.
    let series = app.get("/api/word-stats").await;
    assert_eq!(
        series.body["totals"]["added"].as_u64(),
        Some(res.body["window"]["added"].as_u64().expect("added") + cut_words),
    );
}

/// The divisor is the window, and the window is what the caller asked for.
#[tokio::test]
async fn a_shorter_window_is_a_higher_rate_over_the_same_words() {
    let app = App::new().await;
    let due = Utc::now()
        .checked_add_days(Days::new(9))
        .expect("a due date");
    seed_paced_book(&app, due).await;
    let at = Utc::now();

    let asked = asking_at(at);
    let fortnight = app
        .get(&format!("/api/pace?root=book&at={asked}&offset=0"))
        .await;
    let week = app
        .get(&format!("/api/pace?root=book&at={asked}&offset=0&days=7"))
        .await;

    assert_eq!(week.body["window"]["days"], 7);
    assert_eq!(week.body["window"]["net"], fortnight.body["window"]["net"]);
    // Or the halving below is zero against zero, which every broken version of
    // this would also satisfy.
    assert!(week.body["window"]["net"].as_i64().expect("a net") > 0);
    assert_eq!(
        week.body["window"]["per_day"].as_f64(),
        fortnight.body["window"]["per_day"]
            .as_f64()
            .map(|rate| rate * 2.0),
    );

    // Clamped rather than refused, which is what the chart does with a window
    // nobody thought about.
    let silly = app
        .get(&format!("/api/pace?root=book&at={asked}&days=0"))
        .await;
    assert_eq!(silly.body["window"]["days"], 1);
}

/// A manuscript with no first page is not a short manuscript, which is the
/// answer compile already gives and is why this endpoint borrows its error.
#[tokio::test]
async fn the_pace_of_a_manuscript_nobody_has_written_is_a_404() {
    let app = App::new().await;

    let res = app.get("/api/pace?root=book").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "compile_root_not_found");
    assert_eq!(res.body["error"]["details"]["slug"], "book");
}

/// An ordinary page aiming at nothing still answers the half of the question
/// the log can answer, rather than refusing a question that has an answer.
#[tokio::test]
async fn a_page_with_no_target_and_no_deadline_still_reports_what_was_written() {
    let app = App::new().await;
    app.seed(
        "notes",
        json!({ "content": "# Notes\n\nSix words in this one.\n" }),
    )
    .await;

    let res = app.get("/api/pace?root=notes").await;

    assert_eq!(res.status, StatusCode::OK);
    assert!(res.body["target"].is_null());
    assert!(res.body["remaining"].is_null());
    assert!(res.body["days_remaining"].is_null());
    assert!(res.body["required_per_day"].is_null());
    assert!(res.body["window"]["net"].as_i64().expect("a net") > 0);
}

// ----------------------------------------------------------------- prose

/// This repository's own rule, written the way the file has to be written: with
/// an escape, because the file that configures it may not contain the character.
const NO_EM_DASH: &str = "[[rule]]\n\
                          id = \"no-em-dash\"\n\
                          kind = \"forbid\"\n\
                          severity = \"error\"\n\
                          literals = [\"\\u2014\"]\n\
                          message = \"em dash\"\n";

/// The whole promise of `GET /api/prose/rules`, tested as a remote caller would
/// have to live it: reconstruct the finding from the receipt and the ruleset,
/// without reading `prose.toml` and without reading the implementation.
#[tokio::test]
async fn a_finding_can_be_reproduced_from_the_rules_and_its_own_receipt() {
    let app = App::new().await;
    app.rules(
        "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 6\nignore = [\"the\", \"was\"]\n",
    );
    app.seed(
        "chapter",
        json!({ "content": "The ferry was late, and late was all it was.\n" }),
    )
    .await;

    let rules = app.get("/api/prose/rules").await;
    assert_eq!(rules.status, StatusCode::OK);
    assert_eq!(rules.body["analyzer"], "prose/v1");

    let rule = &rules.body["rules"].as_array().expect("rules")[0];
    assert_eq!(rule["id"], "echo");
    assert_eq!(rule["kind"], "echo");
    assert_eq!(rule["within"], 6, "the value the analyzer used, resolved");
    assert_eq!(rule["ignore"], json!(["the", "was"]));

    let res = app.get("/api/prose?slug=chapter").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["offsets"], "page");
    assert_eq!(
        res.body["rules_digest"], rules.body["rules_digest"],
        "a finding and the ruleset it came from have to agree on the stamp"
    );

    let finding = &res.body["findings"].as_array().expect("findings")[0];
    let receipt = &finding["receipt"];
    assert_eq!(receipt["token"], "late");
    assert_eq!(receipt["within"], 6);

    // Everything the rule claims, checked against the page itself.
    let page = app.get("/api/pages/chapter").await;
    let body = page.body["content"].as_str().expect("content");

    let first = receipt["first"].as_u64().unwrap() as usize;
    let second = receipt["second"].as_u64().unwrap() as usize;
    let token = receipt["token"].as_str().unwrap();

    assert_eq!(&body[first..first + token.len()], "late");
    assert_eq!(&body[second..second + token.len()], "late");
    assert!(
        receipt["distance"].as_u64().unwrap() <= 6,
        "the rule fired outside its own window"
    );

    let (start, end) = (
        finding["span"]["start"].as_u64().unwrap() as usize,
        finding["span"]["end"].as_u64().unwrap() as usize,
    );
    assert_eq!(&body[start..end], finding["quote"].as_str().unwrap());
}

/// No rules is a state a wiki is genuinely in, and it is the answer somebody
/// asking what the rules are should get.
#[tokio::test]
async fn a_wiki_with_no_rules_answers_an_empty_ruleset_rather_than_a_404() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "Anything at all.\n" }))
        .await;

    let rules = app.get("/api/prose/rules").await;
    assert_eq!(rules.status, StatusCode::OK);
    assert_eq!(rules.body["rules"], json!([]));
    assert!(
        rules.body["rules_digest"]
            .as_str()
            .expect("a digest")
            .starts_with("sha256:"),
        "an empty ruleset still stamps, so a caller has something to compare"
    );

    // And nothing to check against is not an error at either of the other two.
    let posted = app
        .post("/api/prose", json!({ "content": "Anything at all.\n" }))
        .await;
    assert_eq!(posted.status, StatusCode::OK);
    assert_eq!(posted.body["findings"], json!([]));

    let read = app.get("/api/prose?slug=a").await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["findings"], json!([]));

    // And every report says how many rules ran, so a caller can tell the two
    // silences apart. No findings from no rules is not a clean page.
    assert_eq!(posted.body["rules"], 0);
    assert_eq!(read.body["rules"], 0);
}

/// The other half of the pair above: rules ran and found nothing.
#[tokio::test]
async fn a_report_says_how_many_rules_it_applied() {
    let app = App::new().await;
    app.rules(NO_EM_DASH);

    let clean = app
        .post(
            "/api/prose",
            json!({ "content": "A clause, and another.\n" }),
        )
        .await;

    assert_eq!(clean.body["rules"], 1);
    assert_eq!(clean.body["findings"], json!([]));
}

/// A file that will not parse is a mistake somebody just made, which is a
/// different thing from a file that is not there.
#[tokio::test]
async fn a_rules_file_that_will_not_parse_is_reported_as_itself() {
    let app = App::new().await;
    app.rules("[[rule]\nid = \"echo\"\n");

    let res = app.get("/api/prose/rules").await;

    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.code(), "prose_rules_invalid");
    assert_eq!(res.body["error"]["details"]["file"], ".rhizolog/prose.toml");
    assert!(res.body["error"]["details"]["reason"].is_string());

    // The same answer wherever the rules are read from.
    assert_eq!(
        app.post("/api/prose", json!({ "content": "x\n" }))
            .await
            .code(),
        "prose_rules_invalid"
    );
}

#[tokio::test]
async fn a_rule_that_is_wrong_rather_than_unparseable_says_which_rule() {
    let app = App::new().await;
    app.rules("[[rule]]\nid = \"tells\"\nkind = \"phrase\"\nwithin = 4\n");

    let res = app.get("/api/prose/rules").await;

    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reason = res.body["error"]["details"]["reason"]
        .as_str()
        .expect("a reason");
    assert!(reason.contains("tells"), "got {reason}");
    assert!(reason.contains("within"), "got {reason}");
}

/// The digest is what ties a finding to the rules that produced it. Two calls
/// either side of an edit have to disagree, or it is a number that proves
/// nothing.
#[tokio::test]
async fn editing_the_rules_between_two_calls_makes_the_stamps_disagree() {
    let app = App::new().await;
    app.rules("[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 6\n");

    let before = app.get("/api/prose/rules").await.body["rules_digest"].clone();

    app.rules("[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 7\n");
    let after = app.get("/api/prose/rules").await.body["rules_digest"].clone();

    assert_ne!(before, after);

    // Read on every request rather than cached, so tuning a rule is a matter of
    // saving the file and asking again.
    assert_eq!(
        app.post("/api/prose", json!({ "content": "x\n" }))
            .await
            .body["rules_digest"],
        after
    );
}

/// The editor's path. Nothing is stored, so it is safe on a debounce.
#[tokio::test]
async fn the_editor_can_check_a_body_it_has_not_saved() {
    let app = App::new().await;
    app.rules(NO_EM_DASH);

    let res = app
        .post(
            "/api/prose",
            json!({ "content": "A clause \u{2014} and another.\n" }),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["offsets"], "page");
    assert!(res.body["slug"].is_null(), "nothing was stored");
    assert_eq!(res.body["errors"], 1);
    assert_eq!(res.body["warnings"], 0);

    let finding = &res.body["findings"].as_array().expect("findings")[0];
    assert_eq!(finding["rule"], "no-em-dash");
    assert_eq!(finding["severity"], "error");
    assert_eq!(finding["message"], "em dash");
    assert_eq!(finding["quote"], "\u{2014}");
    assert!(
        finding["slug"].is_null(),
        "a finding names its chapter only when there is a book to name it in"
    );

    // And nothing was written to the wiki by asking.
    assert_eq!(
        app.get("/api/pages").await.body["total"],
        0,
        "checking is not saving"
    );
}

/// The reason `?compiled=true` exists. A word repeated across a chapter break is
/// invisible to anything reading one page at a time.
#[tokio::test]
async fn a_repeat_across_a_chapter_break_is_only_visible_compiled() {
    let app = App::new().await;
    app.rules(
        "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 8\nignore = [\"the\", \"was\"]\n",
    );
    app.seed(
        "novel",
        json!({ "content": "# Novel\n", "contents": ["novel/a", "novel/b"] }),
    )
    .await;
    app.seed("novel/a", json!({ "content": "The ferry was late.\n" }))
        .await;
    app.seed("novel/b", json!({ "content": "The ferry was early.\n" }))
        .await;

    assert_eq!(
        app.get("/api/prose?slug=novel/a").await.body["findings"],
        json!([]),
        "one chapter on its own repeats nothing"
    );

    let res = app.get("/api/prose?slug=novel&compiled=true").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["offsets"], "document");
    assert_eq!(res.body["slug"], "novel");

    let findings = res.body["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["receipt"]["token"], "ferry");
    assert_eq!(
        findings[0]["slug"], "novel/a",
        "a finding that straddles a chapter break belongs where it starts"
    );
}

/// A finding over a whole book is no use if it cannot say which chapter owns it.
#[tokio::test]
async fn a_finding_over_a_manuscript_names_the_chapter_it_fell_in() {
    let app = App::new().await;
    seed_book(&app).await;
    app.rules("[[rule]]\nid = \"tells\"\nkind = \"phrase\"\nphrases = [\"second\"]\n");

    let res = app.get("/api/prose?slug=book&compiled=true").await;
    let findings = res.body["findings"].as_array().expect("findings");

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["slug"], "book/one/the-ferry");
    assert_eq!(findings[0]["quote"], "Second");

    // And the offset indexes the document the compiler returns, which is what
    // `offsets: "document"` says it does.
    let document = app.get("/api/compile?root=book").await.body["content"]
        .as_str()
        .expect("content")
        .to_owned();
    let start = findings[0]["span"]["start"].as_u64().unwrap() as usize;
    let end = findings[0]["span"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&document[start..end], "Second");
}

/// A page nobody has written and a page nobody may read are the same answer, as
/// they are everywhere else.
#[tokio::test]
async fn checking_a_page_that_is_not_there_is_a_404() {
    let app = App::new().await;
    app.rules(NO_EM_DASH);

    let res = app.get("/api/prose?slug=nowhere").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "page_not_found");

    let compiled = app.get("/api/prose?slug=nowhere&compiled=true").await;
    assert_eq!(compiled.status, StatusCode::NOT_FOUND);
    assert_eq!(compiled.code(), "compile_root_not_found");

    assert_eq!(
        app.get("/api/prose?slug=../etc/passwd").await.code(),
        "slug_relative_segment"
    );
}

/// Frontmatter is not checked, so a rule cannot fire on a field the author did
/// not write as prose.
#[tokio::test]
async fn a_rule_reads_the_body_and_not_the_frontmatter() {
    let app = App::new().await;
    app.rules("[[rule]]\nid = \"tells\"\nkind = \"phrase\"\nphrases = [\"delve\"]\n");
    app.seed(
        "a",
        json!({ "title": "How to delve", "tags": ["delve"], "content": "Nothing here.\n" }),
    )
    .await;

    assert_eq!(
        app.get("/api/prose?slug=a").await.body["findings"],
        json!([])
    );
}

// ---------------------------------------------------------------- search

/// The property that makes writes usable immediately: the index is updated
/// before the write responds, not whenever a watcher gets round to it.
#[tokio::test]
async fn a_page_is_searchable_the_moment_it_is_written() {
    let app = App::new().await;

    app.seed(
        "notes/rhizome",
        json!({ "title": "Rhizome", "content": "Knowledge branches off chaotically.\n" }),
    )
    .await;

    let res = app.get("/api/search?q=chaotically").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["total"], 1);
    let hit = &res.body["hits"][0];
    assert_eq!(hit["slug"], "notes/rhizome");
    assert_eq!(hit["title"], "Rhizome");
    assert!(
        hit["snippet"].as_str().unwrap().contains("<mark>"),
        "search hit carried no marked excerpt"
    );
}

#[tokio::test]
async fn edits_and_deletes_are_reflected_in_search_immediately() {
    let app = App::new().await;
    app.seed("notes/rhizome", json!({ "content": "Original wording.\n" }))
        .await;

    app.patch(
        "/api/pages/notes/rhizome",
        json!({ "content": "Replaced wording.\n" }),
    )
    .await;
    assert_eq!(app.get("/api/search?q=Original").await.body["total"], 0);
    assert_eq!(app.get("/api/search?q=Replaced").await.body["total"], 1);

    app.delete("/api/pages/notes/rhizome").await;
    assert_eq!(app.get("/api/search?q=Replaced").await.body["total"], 0);
}

/// Punctuation in a search box must never become a 500.
#[tokio::test]
async fn search_survives_hostile_queries() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "rust-lang notes\n" }))
        .await;

    for query in [
        "%22",
        "*",
        "AND",
        "a%20OR%20b",
        "x%20AND%20(y",
        "",
        "%20%20",
    ] {
        let res = app.get(&format!("/api/search?q={query}")).await;
        assert_eq!(
            res.status,
            StatusCode::OK,
            "query {query:?} was not handled"
        );
    }

    assert_eq!(app.get("/api/search?q=rust-lang").await.body["total"], 1);
}

// ----------------------------------------------------------------- graph

#[tokio::test]
async fn reports_links_in_both_directions() {
    let app = App::new().await;
    app.seed("notes/target", json!({ "content": "The target.\n" }))
        .await;
    app.seed(
        "index",
        json!({ "content": "See [[notes/target]] and [out](https://example.com).\n" }),
    )
    .await;

    let index_links = app.get("/api/links/index").await;
    assert_eq!(index_links.status, StatusCode::OK);
    assert_eq!(index_links.body["exists"], true);
    assert_eq!(index_links.body["outbound"].as_array().unwrap().len(), 2);
    assert!(index_links.body["inbound"].as_array().unwrap().is_empty());

    let target_links = app.get("/api/links/notes/target").await;
    assert_eq!(target_links.body["inbound"][0]["slug"], "index");
    assert_eq!(target_links.body["inbound"][0]["kind"], "wiki");
}

/// A link to a page nobody has written is not an error — it is a wanted page,
/// and it resolves the moment someone writes it, with no reindex.
#[tokio::test]
async fn a_wanted_page_resolves_when_it_is_created() {
    let app = App::new().await;
    app.seed("index", json!({ "content": "See [[notes/later]].\n" }))
        .await;

    let before = app.get("/api/links/index").await;
    assert_eq!(before.body["outbound"][0]["resolved"], false);
    assert_eq!(before.body["outbound"][0]["target"], "notes/later");

    let wanted = app.get("/api/links/notes/later").await;
    assert_eq!(
        wanted.status,
        StatusCode::OK,
        "a wanted page still has links"
    );
    assert_eq!(wanted.body["exists"], false);
    assert_eq!(wanted.body["inbound"][0]["slug"], "index");

    let stats = app.get("/api/stats").await;
    assert_eq!(stats.body["wanted_count"], 1);
    assert_eq!(stats.body["wanted"][0]["slug"], "notes/later");

    // Write only the new page; nothing touches `index`.
    app.seed("notes/later", json!({ "content": "Now it exists.\n" }))
        .await;

    let after = app.get("/api/links/index").await;
    assert_eq!(after.body["outbound"][0]["resolved"], true);
    assert_eq!(app.get("/api/stats").await.body["wanted_count"], 0);
}

#[tokio::test]
async fn reports_tags_with_counts() {
    let app = App::new().await;
    app.seed("a", json!({ "tags": ["theory", "shared"] })).await;
    app.seed("b", json!({ "tags": ["shared"] })).await;

    let res = app.get("/api/tags").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["tags"][0], json!({ "tag": "shared", "pages": 2 }));
    assert_eq!(res.body["tags"][1], json!({ "tag": "theory", "pages": 1 }));
}

#[tokio::test]
async fn stats_describe_the_shape_of_the_wiki() {
    let app = App::new().await;
    app.seed(
        "hub",
        json!({ "tags": ["meta"], "content": "See [[spoke]] and [[missing]].\n" }),
    )
    .await;
    app.seed("spoke", json!({ "content": "Linked to.\n" }))
        .await;
    app.seed("lonely", json!({ "content": "Nothing links here.\n" }))
        .await;

    let res = app.get("/api/stats").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["pages"], 3);
    assert_eq!(res.body["tags"], 1);
    assert_eq!(res.body["links"]["internal"], 2);
    assert_eq!(res.body["links"]["resolved"], 1);
    assert_eq!(res.body["links"]["wanted"], 1);

    // `hub` and `lonely` are unreferenced.
    assert_eq!(res.body["orphan_count"], 2);
    assert_eq!(res.body["wanted"][0]["slug"], "missing");
    assert_eq!(res.body["most_linked"][0]["slug"], "spoke");
    assert_eq!(res.body["most_linked"][0]["referrers"], 1);
    assert!(res.body["last_indexed"].is_null(), "no scan has run here");
}

/// Usage is keyed on the route template, not the URL, or the table would grow a
/// row per page ever fetched.
#[tokio::test]
async fn api_usage_is_counted_per_route_template() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "Body.\n" })).await;
    app.seed("b", json!({ "content": "Body.\n" })).await;

    app.get("/api/pages/a").await;
    app.get("/api/pages/b").await;
    app.get("/api/pages").await;

    let usage = app.get("/api/stats").await.body["api_usage"].clone();
    let usage = usage.as_array().expect("usage list");

    let page_reads = usage
        .iter()
        .find(|entry| entry["route"] == "/api/pages/{slug}" && entry["method"] == "GET")
        .expect("page reads were not counted");
    assert_eq!(
        page_reads["count"], 2,
        "two different pages should share one route counter"
    );

    // The wildcard spelling must not leak here either.
    for entry in usage {
        let route = entry["route"].as_str().unwrap();
        assert!(
            !route.contains("{*"),
            "wildcard syntax leaked into usage: {route}"
        );
    }
}

#[tokio::test]
async fn the_graph_comes_back_as_something_drawable() {
    let app = App::new().await;
    app.seed("index", json!({ "content": "See [[notes/rust]].\n" }))
        .await;
    app.seed(
        "notes/rust",
        json!({ "tags": ["rust"], "content": "Both [[notes/rust/async]] and [a](rust/async.md), plus [[notes/rust/streams]] and [out](https://example.com).\n" }),
    )
    .await;
    app.seed("notes/rust/async", json!({ "content": "A page.\n" }))
        .await;

    let res = app.get("/api/graph").await;

    assert_eq!(res.status, StatusCode::OK);
    let slugs: Vec<&str> = res.body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        slugs,
        [
            "index",
            "notes/rust",
            "notes/rust/async",
            "notes/rust/streams"
        ]
    );

    // The unwritten page is a node, and says so rather than being absent.
    assert_eq!(res.body["nodes"][3]["exists"], false);
    assert_eq!(res.body["nodes"][3]["title"], "notes/rust/streams");

    // Linked as `[[a]]` and as `[a](a.md)`: two rows in the index, one line to
    // draw, and one referrer.
    let doubled = res.body["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| edge["target"] == "notes/rust/async")
        .expect("the doubled edge");
    assert_eq!(doubled["kinds"], json!(["internal", "wiki"]));
    assert_eq!(res.body["nodes"][2]["inbound"], 1);

    // Nothing that leaves the wiki. An external link has no node to land on.
    for edge in res.body["edges"].as_array().unwrap() {
        assert!(
            !edge["target"].as_str().unwrap().starts_with("http"),
            "an external link reached the graph: {edge}"
        );
    }

    assert_eq!(res.body["matched"], 3, "wants are not pages");
    assert_eq!(res.body["truncated"], false);
    assert_eq!(res.body["root"], Value::Null);
    assert_eq!(res.body["depth"], Value::Null);
}

#[tokio::test]
async fn a_graph_can_be_walked_out_from_one_page() {
    let app = App::new().await;
    app.seed("index", json!({ "content": "See [[a]].\n" }))
        .await;
    app.seed("a", json!({ "content": "See [[b]].\n" })).await;
    app.seed("b", json!({ "content": "The far end.\n" })).await;

    let res = app.get("/api/graph?root=a&depth=1").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["root"], "a");
    assert_eq!(res.body["depth"], 1);
    let slugs: Vec<&str> = res.body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["slug"].as_str().unwrap())
        .collect();
    // Both directions: `index` points at `a`, and `a` points at `b`.
    assert_eq!(slugs, ["a", "b", "index"]);
    assert_eq!(res.body["nodes"][0]["distance"], 0);
    assert_eq!(res.body["nodes"][2]["distance"], 1);

    // The depth is clamped rather than refused, like every other numeric bound
    // in this API.
    let deep = app.get("/api/graph?root=a&depth=99").await;
    assert_eq!(deep.body["depth"], 6);
}

#[tokio::test]
async fn a_graph_root_that_is_not_a_slug_is_refused() {
    let app = App::new().await;

    let res = app.get("/api/graph?root=notes/../../etc").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "slug_relative_segment");
}

#[tokio::test]
async fn links_for_an_invalid_slug_are_refused() {
    let app = App::new().await;

    let res = app.get("/api/links/notes/../../etc").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "slug_relative_segment");
}

#[tokio::test]
async fn deleting_a_page_leaves_its_backlinks_wanting() {
    let app = App::new().await;
    app.seed("index", json!({ "content": "See [[notes/target]].\n" }))
        .await;
    app.seed("notes/target", json!({ "content": "The target.\n" }))
        .await;
    assert_eq!(app.get("/api/stats").await.body["links"]["resolved"], 1);

    app.delete("/api/pages/notes/target").await;

    let stats = app.get("/api/stats").await;
    assert_eq!(stats.body["links"]["wanted"], 1);
    assert_eq!(stats.body["wanted"][0]["slug"], "notes/target");
}

#[tokio::test]
async fn reindexing_rebuilds_from_disk() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "Body about rhizomes.\n" }))
        .await;
    app.seed("b", json!({ "content": "Body about rhizomes.\n" }))
        .await;

    let res = app.post("/api/reindex", json!({})).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["pages"]["scanned"], 2);
    assert_eq!(res.body["pages"]["indexed"], 2);
    assert_eq!(res.body["pages"]["failed"], 0);
    // The time log is scanned alongside the pages, and reported beside them.
    assert_eq!(res.body["times"]["scanned"], 0);
    // Still searchable afterwards.
    assert_eq!(app.get("/api/search?q=rhizomes").await.body["total"], 2);
}

// ------------------------------------------------------------------ pins

impl App {
    async fn pin(&self, slug: &str) -> Res {
        self.send(Method::PUT, &format!("/api/pins/{slug}"), None)
            .await
    }

    async fn unpin(&self, slug: &str) -> Res {
        self.delete(&format!("/api/pins/{slug}")).await
    }

    async fn pinned_slugs(&self) -> Vec<String> {
        self.get("/api/pins").await.body["pins"]
            .as_array()
            .expect("pins array")
            .iter()
            .map(|pin| pin["slug"].as_str().expect("slug").to_owned())
            .collect()
    }
}

#[tokio::test]
async fn pins_a_page_and_lists_it_with_its_title() {
    let app = App::new().await;
    app.seed(
        "notes/quick",
        json!({ "title": "Quick Notes", "content": "Scratch.\n" }),
    )
    .await;

    let res = app.pin("notes/quick").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["slug"], "notes/quick");
    assert_eq!(res.body["title"], "Quick Notes");
    assert_eq!(res.body["exists"], true);

    let listed = app.get("/api/pins").await;
    assert_eq!(listed.body["pins"][0]["title"], "Quick Notes");
    assert_eq!(listed.body["limit"], 50);
}

/// Slugs contain `/`, so the pin routes are wildcards like the page routes.
/// A nested slug arriving as several path segments is the bug this catches.
#[tokio::test]
async fn pins_a_deeply_nested_slug() {
    let app = App::new().await;
    app.seed("notes/rust/async", json!({ "content": "Body.\n" }))
        .await;

    assert_eq!(app.pin("notes/rust/async").await.status, StatusCode::OK);
    assert_eq!(app.pinned_slugs().await, ["notes/rust/async"]);
}

/// Idempotent in both senses: no second entry, and no new position. A client
/// should not have to check whether something is pinned before pinning it.
#[tokio::test]
async fn pinning_twice_changes_nothing() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "Body.\n" })).await;
    app.seed("b", json!({ "content": "Body.\n" })).await;

    app.pin("a").await;
    app.pin("b").await;
    let first = app.get("/api/pins").await.body["pins"][0]["pinned_at"].clone();
    assert_eq!(app.pin("a").await.status, StatusCode::OK);

    assert_eq!(app.pinned_slugs().await, ["a", "b"]);
    assert_eq!(
        app.get("/api/pins").await.body["pins"][0]["pinned_at"],
        first
    );
}

#[tokio::test]
async fn pinning_a_page_that_does_not_exist_is_a_404() {
    let app = App::new().await;

    let res = app.pin("nothing/here").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "page_not_found");
}

#[tokio::test]
async fn pinning_an_invalid_slug_is_refused_by_rule() {
    let app = App::new().await;

    let res = app.pin("notes/CON").await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "slug_reserved_name");
}

#[tokio::test]
async fn unpinning_removes_the_pin_but_not_the_page() {
    let app = App::new().await;
    app.seed("notes/quick", json!({ "content": "Scratch.\n" }))
        .await;
    app.pin("notes/quick").await;

    assert_eq!(
        app.unpin("notes/quick").await.status,
        StatusCode::NO_CONTENT
    );

    assert!(app.pinned_slugs().await.is_empty());
    assert_eq!(
        app.get("/api/pages/notes/quick").await.status,
        StatusCode::OK,
        "unpinning must not touch the page"
    );
}

/// Not `page_not_found`: the page is right there, it is the pin that is
/// missing, and a caller that cannot tell them apart would retry the wrong
/// thing.
#[tokio::test]
async fn unpinning_something_that_was_not_pinned_is_a_404_of_its_own() {
    let app = App::new().await;
    app.seed("notes/quick", json!({ "content": "Scratch.\n" }))
        .await;

    let res = app.unpin("notes/quick").await;

    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.code(), "pin_not_found");
    assert_eq!(res.body["error"]["details"]["slug"], "notes/quick");
}

/// A bookmark that stopped working because you renamed the thing it points at
/// is a bug. Unlike inbound links, which belong to the pages that wrote them, a
/// pin follows the page.
#[tokio::test]
async fn a_pin_follows_a_move() {
    let app = App::new().await;
    app.seed(
        "notes/old",
        json!({ "title": "Kept", "content": "Body.\n" }),
    )
    .await;
    app.pin("notes/old").await;

    app.post(
        "/api/move",
        json!({ "from": "notes/old", "to": "archive/new" }),
    )
    .await;

    let pins = app.get("/api/pins").await;
    assert_eq!(pins.body["pins"][0]["slug"], "archive/new");
    assert_eq!(pins.body["pins"][0]["title"], "Kept");
    assert_eq!(pins.body["pins"][0]["exists"], true);
}

/// Deleting a page through the API is a deliberate act on that page, so the
/// shortcut to it goes too rather than lingering as a dead menu entry.
#[tokio::test]
async fn deleting_a_page_takes_its_pin_with_it() {
    let app = App::new().await;
    app.seed("notes/quick", json!({ "content": "Scratch.\n" }))
        .await;
    app.pin("notes/quick").await;

    app.delete("/api/pages/notes/quick").await;

    assert!(app.pinned_slugs().await.is_empty());
}

/// The menu is a shortcut, not a second listing.
#[tokio::test]
async fn the_pin_limit_is_enforced_and_names_itself() {
    let app = App::new().await;
    for n in 0..51 {
        app.seed(&format!("page-{n:02}"), json!({ "content": "Body.\n" }))
            .await;
    }

    for n in 0..50 {
        assert_eq!(
            app.pin(&format!("page-{n:02}")).await.status,
            StatusCode::OK,
            "pin {n} was refused early"
        );
    }

    let refused = app.pin("page-50").await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.code(), "too_many_pins");
    assert_eq!(refused.body["error"]["details"]["limit"], 50);

    // Re-pinning at the limit adds nothing, so it must still be allowed.
    assert_eq!(app.pin("page-00").await.status, StatusCode::OK);
}
