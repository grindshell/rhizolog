//! The HTTP API.
//!
//! Routes are registered through `utoipa-axum`'s [`OpenApiRouter`], so a
//! handler cannot be added to the server without also appearing in the
//! generated OpenAPI document. For a tool-using agent the spec *is* the manual,
//! and a spec that drifts from the routes is worse than no spec at all.

pub mod meta;

use axum::Router;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
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
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Rhizowiki",
        description = "A wiki over a directory of markdown files. Markdown on \
                       disk is the source of truth; the search index is derived \
                       from it and can be rebuilt at any time.\n\n\
                       Page content is served as raw markdown by default — pass \
                       `render=true` to also receive rendered HTML.",
    ),
    tags(
        (name = "meta", description = "Server and index status"),
    ),
)]
pub struct ApiDoc;

/// Build the application router, including Swagger UI and the OpenAPI document.
pub fn router(state: AppState) -> Router {
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(meta::health))
        .split_for_parts();

    router
        .merge(SwaggerUi::new(SWAGGER_UI_PATH).url(OPENAPI_PATH, api))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
