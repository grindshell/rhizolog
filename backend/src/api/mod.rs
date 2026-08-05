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
pub mod search;
pub mod usage;

use axum::Router;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDocument;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use crate::index::Index;
use crate::store::Store;

pub const OPENAPI_PATH: &str = "/api-docs/openapi.json";
pub const SWAGGER_UI_PATH: &str = "/swagger-ui";

#[derive(Clone)]
pub struct AppState {
    /// The wiki directory: the source of truth.
    pub store: Store,
    /// The derived index. Everything in it can be rebuilt from `store`.
    pub index: Index,
    /// API calls since the last flush to the index.
    pub usage: usage::UsageTally,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rhizowiki",
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
        (name = "meta", description = "Server and index status"),
    ),
)]
pub struct ApiDoc;

/// Build the application router, including Swagger UI and the OpenAPI document.
pub fn router(state: AppState) -> Router {
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
        .routes(routes!(search::search))
        .routes(routes!(search::reindex))
        .routes(routes!(graph::links))
        .routes(routes!(graph::tags))
        .routes(routes!(graph::stats))
        .split_for_parts();

    normalize_wildcard_paths(&mut api);

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
