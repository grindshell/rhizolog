//! One error shape for the whole API.
//!
//! Every failure — 404, 409, 422, 500 — comes back as the same envelope with a
//! stable `code`:
//!
//! ```json
//! { "error": { "code": "page_not_found", "message": "no page at notes/asnyc",
//!              "details": { "slug": "notes/asnyc" } } }
//! ```
//!
//! The `code` is the part clients should branch on; `message` is prose and may
//! be reworded. `details` carries whatever the caller needs to fix the request
//! without a human reading the message — for a rejected slug, that includes
//! which validation rule it broke.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use utoipa::ToSchema;

use crate::index::IndexError;
use crate::page::PageError;
use crate::slug::{Slug, SlugError};
use crate::store::StoreError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid slug {raw:?}: {source}")]
    InvalidSlug {
        raw: String,
        #[source]
        source: SlugError,
    },

    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(transparent)]
    Index(#[from] IndexError),

    /// A request body that could not be parsed, or that failed validation
    /// during deserialisation — a malformed slug, most often.
    ///
    /// This is a 400 rather than axum's default 422 so that a slug refused in a
    /// body and the same slug refused in a URL come back the same way. A caller
    /// should not have to learn that the identical mistake has two statuses
    /// depending on where it appeared.
    #[error("invalid request body: {message}")]
    InvalidRequestBody { message: String, kind: &'static str },

    /// No route matched an `/api` path.
    ///
    /// axum's own 404 has an empty body, which would make unknown routes the
    /// one failure that does not arrive in the envelope the spec promises.
    #[error("no API route at {path}")]
    RouteNotFound { path: String },

    #[error("unknown field(s): {}", .unknown.join(", "))]
    UnknownFields {
        unknown: Vec<String>,
        valid: &'static [&'static str],
    },

    #[error("{parameter} must be one of: {}", .allowed.join(", "))]
    InvalidParameter {
        parameter: &'static str,
        value: String,
        allowed: &'static [&'static str],
    },

    /// Unpinning a page that was not pinned.
    ///
    /// Distinct from `page_not_found`, because the page is very likely there —
    /// it is the pin that is missing, and a caller that cannot tell the two
    /// apart would retry the wrong thing.
    #[error("{slug} is not pinned")]
    PinNotFound { slug: Slug },

    #[error("at most {limit} pages may be pinned")]
    TooManyPins { limit: usize },

    #[error("{message}")]
    Internal { message: String },
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::InvalidSlug { .. } => StatusCode::BAD_REQUEST,
            Self::Store(error) => match error {
                StoreError::NotFound { .. } => StatusCode::NOT_FOUND,
                StoreError::AlreadyExists { .. } => StatusCode::CONFLICT,
                StoreError::EscapesRoot { .. } => StatusCode::BAD_REQUEST,
                StoreError::NotUtf8 { .. } | StoreError::Malformed { .. } => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
                StoreError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            },
            Self::RouteNotFound { .. } | Self::PinNotFound { .. } => StatusCode::NOT_FOUND,
            Self::TooManyPins { .. } => StatusCode::CONFLICT,
            Self::InvalidRequestBody { .. }
            | Self::UnknownFields { .. }
            | Self::InvalidParameter { .. } => StatusCode::BAD_REQUEST,
            // The index is derived and rebuildable, so a failure here is the
            // server's problem, never something the caller phrased wrong.
            Self::Index(_) | Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            // The specific rule, not a generic `invalid_slug`, so a caller can
            // tell a traversal attempt from a name that is merely too long.
            Self::InvalidSlug { source, .. } => source.code(),
            Self::Store(error) => match error {
                StoreError::NotFound { .. } => "page_not_found",
                StoreError::AlreadyExists { .. } => "page_already_exists",
                StoreError::EscapesRoot { .. } => "slug_escapes_root",
                StoreError::NotUtf8 { .. } => "page_not_utf8",
                StoreError::Malformed { .. } => "page_malformed",
                StoreError::Io(_) => "io_error",
            },
            Self::Index(_) => "index_error",
            Self::RouteNotFound { .. } => "route_not_found",
            Self::PinNotFound { .. } => "pin_not_found",
            Self::TooManyPins { .. } => "too_many_pins",
            Self::InvalidRequestBody { .. } => "invalid_request_body",
            Self::UnknownFields { .. } => "unknown_fields",
            Self::InvalidParameter { .. } => "invalid_parameter",
            Self::Internal { .. } => "internal_error",
        }
    }

    fn details(&self) -> Option<Value> {
        match self {
            Self::InvalidSlug { raw, source } => Some(json!({
                "slug": raw,
                "rule": source.code(),
                "reason": source.to_string(),
            })),
            Self::Store(error) => match error {
                StoreError::NotFound { slug }
                | StoreError::AlreadyExists { slug }
                | StoreError::EscapesRoot { slug }
                | StoreError::NotUtf8 { slug } => Some(json!({ "slug": slug })),
                StoreError::Malformed { slug, source } => Some(json!({
                    "slug": slug,
                    "reason": source.to_string(),
                })),
                StoreError::Io(_) => None,
            },
            Self::RouteNotFound { path } => Some(json!({ "path": path })),
            Self::PinNotFound { slug } => Some(json!({ "slug": slug })),
            Self::TooManyPins { limit } => Some(json!({ "limit": limit })),
            Self::InvalidRequestBody { kind, .. } => Some(json!({ "kind": kind })),
            // Both of these name what was accepted, not just what was refused:
            // a caller that guessed a field or a sort key wrong can correct
            // itself from the response instead of going back to the spec.
            Self::UnknownFields { unknown, valid } => Some(json!({
                "unknown": unknown,
                "valid": valid,
            })),
            Self::InvalidParameter {
                parameter,
                value,
                allowed,
            } => Some(json!({
                "parameter": parameter,
                "value": value,
                "allowed": allowed,
            })),
            Self::Index(_) | Self::Internal { .. } => None,
        }
    }

    /// The message to put on the wire.
    ///
    /// Internal failures are not described to the client — an I/O error can
    /// carry a filesystem path, and there is nothing the caller can do with it
    /// anyway. The full error goes to the log instead.
    fn public_message(&self) -> String {
        match self {
            Self::Store(StoreError::Io(_)) | Self::Index(_) | Self::Internal { .. } => {
                "the server failed to handle the request".to_owned()
            }
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Every failure has this shape, whatever the status. Branch on
    /// `error.code`.
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorDetail {
    /// Stable, machine-readable identifier for what went wrong.
    #[schema(example = "page_not_found")]
    pub code: String,

    /// Human-readable explanation. Prose; do not branch on it.
    #[schema(example = "no page at notes/asnyc")]
    pub message: String,

    /// Context for correcting the request, when there is any. Where something
    /// was refused for being outside a fixed set, this names the values that
    /// would have worked.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({ "slug": "notes/asnyc" }))]
    pub details: Option<Value>,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        } else {
            tracing::debug!(error = %self, "request rejected");
        }

        let body = ErrorResponse {
            error: ErrorDetail {
                code: self.code().to_owned(),
                message: self.public_message(),
                details: self.details(),
            },
        };

        (status, Json(body)).into_response()
    }
}

impl From<SlugError> for AppError {
    fn from(source: SlugError) -> Self {
        Self::InvalidSlug {
            raw: String::new(),
            source,
        }
    }
}

impl From<PageError> for AppError {
    fn from(source: PageError) -> Self {
        Self::internal(source.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::slug::Slug;

    fn body_of(error: AppError) -> (StatusCode, Value) {
        let status = error.status();
        let body = ErrorResponse {
            error: ErrorDetail {
                code: error.code().to_owned(),
                message: error.public_message(),
                details: error.details(),
            },
        };
        (status, serde_json::to_value(body).unwrap())
    }

    #[test]
    fn a_missing_page_is_a_404_naming_the_slug() {
        let slug = Slug::parse("notes/asnyc").unwrap();
        let (status, body) = body_of(AppError::Store(StoreError::NotFound { slug }));

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "page_not_found");
        assert_eq!(body["error"]["details"]["slug"], "notes/asnyc");
    }

    #[test]
    fn a_duplicate_page_is_a_409() {
        let slug = Slug::parse("notes/rhizome").unwrap();
        let (status, body) = body_of(AppError::Store(StoreError::AlreadyExists { slug }));

        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "page_already_exists");
    }

    /// A caller that builds a bad slug should be able to fix it from the
    /// response alone, which means naming the rule rather than just refusing.
    #[test]
    fn a_rejected_slug_reports_which_rule_it_broke() {
        let source = Slug::parse("../etc/passwd").unwrap_err();
        let (status, body) = body_of(AppError::InvalidSlug {
            raw: "../etc/passwd".to_owned(),
            source,
        });

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "slug_relative_segment");
        assert_eq!(body["error"]["details"]["rule"], "slug_relative_segment");
        assert_eq!(body["error"]["details"]["slug"], "../etc/passwd");
        assert!(body["error"]["details"]["reason"].is_string());
    }

    /// Internal failures must not leak filesystem paths to the client.
    #[test]
    fn internal_errors_are_not_described_on_the_wire() {
        let io = std::io::Error::other("C:\\Users\\theaz\\secret\\wiki is on fire");
        let (status, body) = body_of(AppError::Store(StoreError::Io(io)));

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "io_error");
        assert_eq!(
            body["error"]["message"],
            "the server failed to handle the request"
        );
        assert!(body["error"]["details"].is_null());
        assert!(!body.to_string().contains("secret"));
    }

    #[test]
    fn details_are_omitted_rather_than_null_when_absent() {
        let (_, body) = body_of(AppError::internal("boom"));
        assert!(body["error"].get("details").is_none());
    }
}
