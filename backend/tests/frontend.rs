//! Serving the built frontend.
//!
//! The backend serves the SPA in production, which means it owns two things the
//! frontend cannot fix for itself: deep links must survive a hard refresh, and
//! an unknown `/api` path must not be answered with a page of HTML.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use rhizolog::{AppState, Assets, Index, Store, TimeStore};
use tempfile::TempDir;
use tower::ServiceExt;

/// A wiki plus a pretend `dist/` directory.
async fn app_with_assets() -> (TempDir, TempDir, Router) {
    let wiki = TempDir::new().expect("wiki dir");
    let assets = TempDir::new().expect("assets dir");

    std::fs::write(
        assets.path().join("index.html"),
        "<!doctype html><title>Rhizolog</title><div id=root></div>",
    )
    .expect("write index.html");
    std::fs::create_dir_all(assets.path().join("assets")).expect("create assets dir");
    std::fs::write(
        assets.path().join("assets").join("index-abc123.js"),
        "console.log('app')",
    )
    .expect("write bundle");

    let router = app_serving(&wiki, Assets::Dir(assets.path().to_path_buf())).await;

    (wiki, assets, router)
}

/// A wiki, and a router told where the dashboard is.
async fn app_serving(wiki: &TempDir, assets: Assets) -> Router {
    let store = Store::open(wiki.path()).await.expect("open store");
    let times = TimeStore::open(wiki.path()).await.expect("open time log");
    let users = rhizolog::UserStore::open(wiki.path())
        .await
        .expect("open users");
    let index = Index::open(None).await.expect("open index");

    rhizolog::router(AppState {
        store,
        times,
        users,
        index,
        usage: rhizolog::UsageTally::new(),
        assets,
        secure_cookies: false,
        anonymous_read: false,
    })
}

async fn get(router: &Router, path: &str) -> (StatusCode, String, Option<String>) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router response");

    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");

    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        content_type,
    )
}

#[tokio::test]
async fn serves_the_app_shell_at_the_root() {
    let (_wiki, _assets, router) = app_with_assets().await;

    let (status, body, _) = get(&router, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("id=root"), "got {body}");
}

#[tokio::test]
async fn serves_real_asset_files() {
    let (_wiki, _assets, router) = app_with_assets().await;

    let (status, body, content_type) = get(&router, "/assets/index-abc123.js").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "console.log('app')");
    assert!(
        content_type
            .as_deref()
            .is_some_and(|ct| ct.contains("javascript")),
        "the bundle was not served as JavaScript: {content_type:?}"
    );
}

/// Client routes have no file behind them. Without the fallback a pasted link
/// or a hard refresh would 404, and pages would only be reachable by navigating
/// from inside the app.
#[tokio::test]
async fn deep_links_fall_back_to_the_app_shell() {
    let (_wiki, _assets, router) = app_with_assets().await;

    for path in [
        "/pages",
        "/pages/notes/rust/async",
        "/tags",
        "/anything/else",
    ] {
        let (status, body, _) = get(&router, path).await;
        assert_eq!(status, StatusCode::OK, "{path} did not fall back");
        assert!(body.contains("id=root"), "{path} served something else");
    }
}

/// The reason the SPA fallback cannot simply catch everything: an agent that
/// mistypes an endpoint must get the error envelope, not a 200 of HTML.
#[tokio::test]
async fn unknown_api_routes_still_return_the_error_envelope() {
    let (_wiki, _assets, router) = app_with_assets().await;

    for path in ["/api/nonsense", "/api/nope/deeper/still", "/api"] {
        let (status, body, content_type) = get(&router, path).await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{path} was not a 404");
        assert!(
            content_type
                .as_deref()
                .is_some_and(|ct| ct.contains("json")),
            "{path} answered with {content_type:?}, not JSON"
        );

        let json: serde_json::Value = serde_json::from_str(&body)
            .unwrap_or_else(|_| panic!("{path} did not return JSON: {body}"));
        assert_eq!(json["error"]["code"], "route_not_found", "{path}");
        assert_eq!(json["error"]["details"]["path"], path);
    }
}

/// A long path under `/api/pages` is not an unknown route — it is a request for
/// a deeply nested slug. The catch-all must not intercept it, and the answer
/// should be `page_not_found`, naming the slug that was looked for.
#[tokio::test]
async fn a_deep_page_path_is_a_missing_page_not_a_missing_route() {
    let (_wiki, _assets, router) = app_with_assets().await;

    let (status, body, _) = get(&router, "/api/pages/a/b/c/d/e").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    assert_eq!(json["error"]["code"], "page_not_found");
    assert_eq!(json["error"]["details"]["slug"], "a/b/c/d/e");
}

/// Registering a catch-all under `/api` must not shadow the real endpoints.
#[tokio::test]
async fn the_api_catch_all_does_not_shadow_real_endpoints() {
    let (_wiki, _assets, router) = app_with_assets().await;

    for path in ["/api/health", "/api/pages", "/api/tags", "/api/stats"] {
        let (status, _, content_type) = get(&router, path).await;
        assert_eq!(status, StatusCode::OK, "{path} was shadowed");
        assert!(
            content_type
                .as_deref()
                .is_some_and(|ct| ct.contains("json")),
            "{path} answered with {content_type:?}"
        );
    }
}

#[tokio::test]
async fn the_openapi_document_survives_the_spa_fallback() {
    let (_wiki, _assets, router) = app_with_assets().await;

    let (status, body, _) = get(&router, "/api-docs/openapi.json").await;

    assert_eq!(status, StatusCode::OK);
    let spec: serde_json::Value = serde_json::from_str(&body).expect("spec is JSON");
    assert_eq!(spec["info"]["title"], "Rhizolog");
}

/// Swagger UI must actually serve its own assets.
///
/// `utoipa-swagger-ui` embeds them at compile time from an **absolute** path
/// that its build script bakes into the crate. Move or rename the repository
/// and cargo keeps the cached build output — which now points at a directory
/// that does not exist. Nothing fails: the crate still compiles, the route is
/// still registered, `/swagger-ui` still redirects. Every asset behind it is
/// simply gone, and the page 404s.
///
/// That is not hypothetical. Renaming this project from `rhizowiki` to
/// `rhizolog` did exactly that, and it went unnoticed for several commits
/// because no test opened the page. `cargo clean -p utoipa-swagger-ui` is the
/// fix; this is what says it is needed.
#[tokio::test]
async fn swagger_ui_serves_its_own_assets() {
    let (_wiki, _assets, router) = app_with_assets().await;

    let (status, body, content_type) = get(&router, "/swagger-ui/").await;

    assert_eq!(status, StatusCode::OK, "swagger-ui served nothing");
    assert!(
        content_type
            .as_deref()
            .is_some_and(|ct| ct.contains("html")),
        "the shell was served as {content_type:?}"
    );
    assert!(
        body.contains("swagger-ui"),
        "not the Swagger UI shell: {body}"
    );

    // The shell is useless on its own, and the bundle is the asset that
    // actually goes missing — a stale embed serves neither.
    let (status, bundle, _) = get(&router, "/swagger-ui/swagger-ui-bundle.js").await;

    assert_eq!(status, StatusCode::OK, "the Swagger UI bundle is missing");
    assert!(
        bundle.len() > 100_000,
        "the bundle is suspiciously small at {} bytes",
        bundle.len()
    );
}

/// The dashboard's navbar links to `/swagger-ui` with no trailing slash, so
/// that spelling has to lead somewhere.
#[tokio::test]
async fn the_swagger_ui_link_in_the_navbar_resolves() {
    let (_wiki, _assets, router) = app_with_assets().await;

    let (status, _, _) = get(&router, "/swagger-ui").await;

    assert!(
        status.is_redirection(),
        "expected a redirect to the trailing-slash form, got {status}"
    );
}

/// With no build present the API must still work, and the browser routes should
/// say what to do rather than just failing.
#[tokio::test]
async fn a_missing_frontend_build_leaves_the_api_working() {
    let wiki = TempDir::new().expect("wiki dir");
    let router = app_serving(&wiki, Assets::None).await;

    let (status, _, _) = get(&router, "/api/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the API should not depend on a build"
    );

    let (status, body, _) = get(&router, "/pages").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body.contains("pnpm build"),
        "the message should say how to fix it: {body}"
    );

    // And /api paths still get the envelope, not that message.
    let (status, body, _) = get(&router, "/api/nonsense").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    assert_eq!(json["error"]["code"], "route_not_found");
}

/// The same promises again, from the copy compiled into the binary.
///
/// A portable build serves the dashboard out of its own executable rather than
/// a directory beside it, and the point of doing it that way is that nothing
/// downstream can tell — so these are the directory tests over again, against
/// the real `frontend/dist` this binary was built with.
///
/// Only compiled under `--features embed-assets`, because that is the only
/// configuration in which there is anything embedded to ask about.
#[cfg(feature = "embed-assets")]
mod embedded {
    use super::*;

    async fn app(wiki: &TempDir) -> Router {
        app_serving(wiki, Assets::Embedded).await
    }

    #[tokio::test]
    async fn serves_the_app_shell_at_the_root() {
        let wiki = TempDir::new().expect("wiki dir");
        let router = app(&wiki).await;

        let (status, body, content_type) = get(&router, "/").await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("<div id=\"root\">"), "got {body}");
        assert!(
            content_type
                .as_deref()
                .is_some_and(|ct| ct.contains("html")),
            "the shell was served as {content_type:?}"
        );
    }

    /// Hashed bundle names change on every build, so the file to ask for is
    /// discovered rather than written down.
    #[tokio::test]
    async fn serves_real_asset_files_with_their_content_types() {
        let wiki = TempDir::new().expect("wiki dir");
        let router = app(&wiki).await;

        let paths = rhizolog::assets::embedded_paths();
        let script = paths
            .iter()
            .find(|path| path.ends_with(".js"))
            .unwrap_or_else(|| panic!("no bundle was embedded: {paths:?}"));

        let (status, body, content_type) = get(&router, &format!("/{script}")).await;

        assert_eq!(status, StatusCode::OK);
        assert!(!body.is_empty(), "{script} was served empty");
        assert!(
            content_type
                .as_deref()
                .is_some_and(|ct| ct.contains("javascript")),
            "{script} was not served as JavaScript: {content_type:?}"
        );
    }

    /// The reason `Embedded` cannot be a plain file lookup: a client route has
    /// no file behind it, and a hard refresh has to work in the desktop app for
    /// exactly the reason it has to work in a browser.
    #[tokio::test]
    async fn deep_links_fall_back_to_the_app_shell() {
        let wiki = TempDir::new().expect("wiki dir");
        let router = app(&wiki).await;

        for path in ["/pages", "/pages/notes/rust/async", "/tags", "/times"] {
            let (status, body, _) = get(&router, path).await;
            assert_eq!(status, StatusCode::OK, "{path} did not fall back");
            assert!(
                body.contains("<div id=\"root\">"),
                "{path} served something other than the shell"
            );
        }
    }

    /// The catch-all has to keep winning. An embedded SPA that swallowed
    /// `/api/nonsense` would answer an agent's typo with a page of HTML and a
    /// 200, which is the failure this whole arrangement exists to avoid.
    #[tokio::test]
    async fn unknown_api_routes_still_return_the_error_envelope() {
        let wiki = TempDir::new().expect("wiki dir");
        let router = app(&wiki).await;

        for path in ["/api/nonsense", "/api/nope/deeper/still", "/api"] {
            let (status, body, content_type) = get(&router, path).await;

            assert_eq!(status, StatusCode::NOT_FOUND, "{path} was not a 404");
            assert!(
                content_type
                    .as_deref()
                    .is_some_and(|ct| ct.contains("json")),
                "{path} answered with {content_type:?}, not JSON"
            );

            let json: serde_json::Value = serde_json::from_str(&body)
                .unwrap_or_else(|_| panic!("{path} did not return JSON: {body}"));
            assert_eq!(json["error"]["code"], "route_not_found", "{path}");
        }
    }

    #[tokio::test]
    async fn the_real_endpoints_and_the_spec_are_not_shadowed() {
        let wiki = TempDir::new().expect("wiki dir");
        let router = app(&wiki).await;

        for path in ["/api/health", "/api/pages", "/api-docs/openapi.json"] {
            let (status, _, content_type) = get(&router, path).await;
            assert_eq!(status, StatusCode::OK, "{path} was shadowed");
            assert!(
                content_type
                    .as_deref()
                    .is_some_and(|ct| ct.contains("json")),
                "{path} answered with {content_type:?}"
            );
        }
    }

    /// Swagger UI is embedded by a different crate with its own opinions about
    /// compile-time paths, and it has gone missing here before. A build that
    /// carries the dashboard should carry the manual too.
    #[tokio::test]
    async fn swagger_ui_survives_the_embedded_fallback() {
        let wiki = TempDir::new().expect("wiki dir");
        let router = app(&wiki).await;

        let (status, bundle, _) = get(&router, "/swagger-ui/swagger-ui-bundle.js").await;

        assert_eq!(status, StatusCode::OK, "the Swagger UI bundle is missing");
        assert!(
            bundle.len() > 100_000,
            "the bundle is suspiciously small at {} bytes",
            bundle.len()
        );
    }
}
