//! End-to-end tests over the assembled router.
//!
//! The app is driven in-process with `tower::ServiceExt::oneshot` against a
//! throwaway wiki directory: no ports to allocate, no server task to tear down,
//! and no chance of two tests colliding on the same wiki.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use rhizolog::{AppState, Assets, Index, Store, TimeStore, UserStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct App {
    _directory: TempDir,
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
        Self {
            router: rhizolog::router(AppState {
                store,
                times,
                users,
                index,
                usage: rhizolog::UsageTally::new(),
                // API-only: the SPA fallback is covered in tests/frontend.rs.
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read: false,
            }),
            _directory: directory,
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

    async fn put(&self, path: &str, body: Value) -> Res {
        self.send(Method::PUT, path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: Value) -> Res {
        self.send(Method::PATCH, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> Res {
        self.send(Method::DELETE, path, None).await
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
        let index = Index::open(Some(&database)).await.expect("open index");
        let usage = rhizolog::UsageTally::new();
        let app = App {
            router: rhizolog::router(AppState {
                store,
                times,
                users,
                index: index.clone(),
                usage: usage.clone(),
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read: false,
            }),
            _directory: wiki,
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
    let index = Index::open(Some(&database)).await.expect("reopen index");
    let app = App {
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
        _directory: wiki,
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
