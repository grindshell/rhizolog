//! The HTTP API.
//!
//! Routes are registered through `utoipa-axum`'s [`OpenApiRouter`], so a
//! handler cannot be added to the server without also appearing in the
//! generated OpenAPI document. For a tool-using agent the spec *is* the manual,
//! and a spec that drifts from the routes is worse than no spec at all.

pub mod extract;
pub mod graph;
pub mod meta;
pub mod pages;
pub mod pins;
pub mod search;
pub mod times;
pub mod usage;

use axum::Router;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::assets::{self, Assets};
use crate::error::AppError;
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDocument;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use crate::index::Index;
use crate::store::Store;
use crate::times::TimeStore;

pub const OPENAPI_PATH: &str = "/api-docs/openapi.json";
pub const SWAGGER_UI_PATH: &str = "/swagger-ui";

#[derive(Clone)]
pub struct AppState {
    /// The wiki directory: the source of truth.
    pub store: Store,
    /// The time log under `.rhizolog/times/`. Also files, also authoritative.
    pub times: TimeStore,
    /// The derived index. Everything in it can be rebuilt from `store` and
    /// `times`.
    pub index: Index,
    /// API calls since the last flush to the index.
    pub usage: usage::UsageTally,
    /// The built frontend, wherever it turned out to be.
    pub assets: Assets,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rhizolog",
        description = "A wiki over a directory of markdown files. Markdown on \
                       disk is the source of truth; the search index is derived \
                       from it and can be rebuilt at any time.\n\n\
                       Page content is served as raw markdown by default — pass \
                       `render=true` to also receive rendered HTML.\n\n\
                       Slugs may contain `/` (`notes/rust/async`), so the page \
                       path segment spans the rest of the URL.\n\n\
                       Every error, whatever the status, has the shape \
                       `{\"error\": {\"code\", \"message\", \"details\"}}`. \
                       Branch on `code`; it is stable. `message` is prose.",
    ),
    tags(
        (name = "pages", description = "Reading and writing wiki pages"),
        (name = "search", description = "Full-text search and index maintenance"),
        (name = "graph", description = "Links between pages, tags, and meta-stats"),
        (name = "pins", description = "Pages kept within reach"),
        (name = "times", description = "Time tracking: timers, entries, groups, and statistics"),
        (name = "meta", description = "Server and index status"),
    ),
)]
pub struct ApiDoc;

/// The published OpenAPI document, with no server and no state.
///
/// Exists so the spec can be written out of a checkout that has never been run
/// — see `examples/dump-openapi.rs` for why fetching it over HTTP on Windows
/// is a trap. `ApiDoc::openapi()` on its own is **not** the document: it is
/// only the `info` and `tags` skeleton, and every path comes from the routes
/// registered below. Building it through the same function the server uses is
/// what stops the written spec and the served one drifting apart.
pub fn openapi() -> OpenApiDocument {
    parts().1
}

/// Build the application router, including Swagger UI and the OpenAPI document.
pub fn router(state: AppState) -> Router {
    let (router, api) = parts();

    // Registered explicitly rather than left to the fallback: with the SPA
    // mounted as the fallback, an unmatched `/api` path would otherwise be
    // answered with `index.html` and a 200. A catch-all route claims those
    // first, and static segments still beat it, so the real endpoints are
    // unaffected.
    let router = router
        .route("/api", any(missing_route))
        .route("/api/{*rest}", any(missing_route));

    let router = match &state.assets {
        Assets::Dir(directory) => router.fallback_service(spa(directory)),
        Assets::Embedded => router.fallback(assets::serve),
        Assets::None => router.fallback(missing_route),
    };

    router
        .merge(SwaggerUi::new(SWAGGER_UI_PATH).url(OPENAPI_PATH, api))
        // Counting sits inside the trace layer so it sees the matched route,
        // and applies before `with_state` so it can take the state it needs.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            usage::count,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The routes and the document they describe, built together so neither can
/// exist without the other.
fn parts() -> (Router<AppState>, OpenApiDocument) {
    let (router, mut api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(meta::health))
        .routes(routes!(pages::list, pages::create))
        .routes(routes!(
            pages::read,
            pages::replace,
            pages::patch,
            pages::delete
        ))
        .routes(routes!(pages::move_page))
        .routes(routes!(pages::render_markdown))
        .routes(routes!(search::search))
        .routes(routes!(search::reindex))
        .routes(routes!(graph::links))
        .routes(routes!(graph::link_graph))
        .routes(routes!(graph::tags))
        .routes(routes!(graph::stats))
        .routes(routes!(pins::list_pins))
        .routes(routes!(pins::pin_page, pins::unpin_page))
        .routes(routes!(times::list_times, times::create_time))
        .routes(routes!(
            times::read_time,
            times::patch_time,
            times::delete_time
        ))
        // A static segment after the id, which the page routes cannot have:
        // a time id contains no `/`, so it is an ordinary parameter rather
        // than the catch-all a slug needs.
        .routes(routes!(times::stop_time))
        .routes(routes!(times::list_time_groups))
        .routes(routes!(times::time_statistics))
        .split_for_parts();

    normalize_wildcard_paths(&mut api);
    (router, api)
}

/// What to say when there is no frontend to serve.
///
/// Shared with [`crate::assets::serve`], which answers for the embedded case
/// and reaches the same dead end when nothing was embedded.
pub const NO_FRONTEND: &str = "No frontend has been built. Run `pnpm build` in frontend/, or set \
                               RHIZOLOG_ASSETS to a built directory. The API is unaffected and is \
                               available under /api.";

/// Serve the built frontend, falling back to `index.html`.
///
/// The fallback is what makes deep links work. `/pages/notes/rust/async` is a
/// client-side route with no file behind it, so without this a hard refresh or
/// a pasted link would 404 — the page would only ever be reachable by
/// navigating to it from inside the app.
///
/// `ServeDir` still wins for anything that does exist, so real assets are not
/// shadowed by the fallback. [`crate::assets::serve`] follows the same rule for
/// the embedded copy, so the two are indistinguishable from outside.
fn spa(assets: &std::path::Path) -> ServeDir<ServeFile> {
    ServeDir::new(assets).fallback(ServeFile::new(assets.join("index.html")))
}

/// Answer a request that matched no route.
///
/// An agent that mistypes an endpoint should get the error envelope with a code
/// it can act on, not a page of HTML that happens to be a 200.
async fn missing_route(request: Request) -> Response {
    let path = request.uri().path();

    if is_api_path(path) {
        return AppError::RouteNotFound {
            path: path.to_owned(),
        }
        .into_response();
    }

    // A browser route, but there is no frontend built to serve it.
    (StatusCode::NOT_FOUND, NO_FRONTEND).into_response()
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Rewrite `{*slug}` to `{slug}` in the published OpenAPI paths.
///
/// `utoipa-axum` hands the path string from `#[utoipa::path]` straight to
/// `axum::Router::route`, so the wildcard has to be written in the macro for
/// nested slugs to route at all. But `{*slug}` is an axum spelling, not an
/// OpenAPI one: left alone it produces a parameter named `*slug`, which reads
/// wrong in Swagger UI and would generate a mangled name in any client built
/// from this document.
///
/// Rewriting here keeps both halves honest — routes and spec still come from
/// the same declaration, and only the published spelling is normalised.
fn normalize_wildcard_paths(api: &mut OpenApiDocument) {
    let paths = std::mem::take(&mut api.paths.paths);

    api.paths.paths = paths
        .into_iter()
        .map(|(path, item)| (path.replace("{*", "{"), item))
        .collect();
}
