//! End-to-end tests over the assembled router.
//!
//! The app is driven in-process with `tower::ServiceExt::oneshot` against a
//! throwaway wiki directory: no ports to allocate, no server task to tear down,
//! and no chance of two tests colliding on the same wiki.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use rhizowiki::{AppState, Index, Store};
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
        // In-memory index: these tests are about the HTTP surface, not
        // persistence, which `index::sync` covers.
        let index = Index::open(None).await.expect("open index");
        Self {
            router: rhizowiki::router(AppState { store, index }),
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

// -------------------------------------------------------------- openapi

#[tokio::test]
async fn the_openapi_document_is_served() {
    let app = App::new().await;

    let res = app.get("/api-docs/openapi.json").await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["info"]["title"], "Rhizowiki");
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

#[tokio::test]
async fn reindexing_rebuilds_from_disk() {
    let app = App::new().await;
    app.seed("a", json!({ "content": "Body about rhizomes.\n" }))
        .await;
    app.seed("b", json!({ "content": "Body about rhizomes.\n" }))
        .await;

    let res = app.post("/api/reindex", json!({})).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["scanned"], 2);
    assert_eq!(res.body["indexed"], 2);
    assert_eq!(res.body["failed"], 0);
    // Still searchable afterwards.
    assert_eq!(app.get("/api/search?q=rhizomes").await.body["total"], 2);
}
