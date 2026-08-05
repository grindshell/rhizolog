//! End-to-end tests over the assembled router.
//!
//! The app is driven in-process with `tower::ServiceExt::oneshot` against a
//! throwaway wiki directory: no ports to allocate, no server task to tear down,
//! and no chance of two tests colliding on the same wiki.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use rhizowiki::{AppState, Index, Store};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

async fn app() -> (TempDir, Router) {
    let directory = TempDir::new().expect("temp dir");
    let store = Store::open(directory.path()).await.expect("open store");
    // In-memory index: these tests are about the HTTP surface, not persistence.
    let index = Index::open(None).await.expect("open index");
    (directory, rhizowiki::router(AppState { store, index }))
}

async fn get(router: Router, path: &str) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router response");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

    (status, json)
}

#[tokio::test]
async fn health_reports_the_wiki_it_is_serving() {
    let (directory, router) = app().await;

    let (status, body) = get(router, "/api/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["pages"], 0);
    assert!(
        body["last_indexed"].is_null(),
        "nothing has been scanned yet"
    );

    // The root is canonicalised internally, so compare against the canonical
    // form of the temp dir rather than the path we happened to pass in — but
    // without the `\\?\` prefix Windows canonicalisation adds, which must not
    // reach the wire.
    let canonical = std::fs::canonicalize(directory.path()).expect("canonicalize");
    let expected = rhizowiki::store::display_path(&canonical);

    assert_eq!(body["wiki_root"], expected);
    assert!(
        !expected.starts_with(r"\\?\"),
        "verbatim prefix leaked into the API"
    );
}

#[tokio::test]
async fn the_openapi_document_is_served() {
    let (_directory, router) = app().await;

    let (status, body) = get(router, "/api-docs/openapi.json").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["info"]["title"], "Rhizowiki");
    assert!(body["paths"]["/api/health"]["get"].is_object());
}

/// Every route must appear in the spec. If this fails, a handler was added to
/// the router without going through `routes!`, and agents reading the document
/// will not know the endpoint exists.
#[tokio::test]
async fn every_api_route_is_documented() {
    let (_directory, router) = app().await;

    let (_, spec) = get(router, "/api-docs/openapi.json").await;
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

#[tokio::test]
async fn unknown_routes_are_404() {
    let (_directory, router) = app().await;

    let (status, _) = get(router, "/api/nonsense").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
