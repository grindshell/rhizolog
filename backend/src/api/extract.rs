//! Extractors that fail in Rhizowiki's error envelope.
//!
//! `axum::Json` rejects a bad body with its own response shape, which would
//! leave one class of error — the one a caller is most likely to hit while
//! finding its footing — looking nothing like every other error the API
//! returns. Since the OpenAPI document promises that every failure has the
//! shape `{"error": {"code", "message", "details"}}`, that promise has to hold
//! for extraction too.

use axum::extract::{FromRequest, Request, rejection::JsonRejection};
use serde::de::DeserializeOwned;

use crate::error::AppError;

/// Drop-in replacement for [`axum::Json`] as a **request** extractor.
///
/// Responses still use `axum::Json`; only the failure path differs.
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(request, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(AppError::InvalidRequestBody {
                // `body_text` carries serde's own message, which names the
                // offending field and why it was refused — for a slug, that is
                // the validation rule it broke. Dropping it would turn a fixable
                // mistake into a guessing game.
                message: rejection.body_text(),
                kind: kind_of(&rejection),
            }),
        }
    }
}

/// A coarse label for what went wrong, since `JsonRejection` is non-exhaustive.
fn kind_of(rejection: &JsonRejection) -> &'static str {
    match rejection {
        JsonRejection::JsonDataError(_) => "data",
        JsonRejection::JsonSyntaxError(_) => "syntax",
        JsonRejection::MissingJsonContentType(_) => "content_type",
        JsonRejection::BytesRejection(_) => "body",
        _ => "unknown",
    }
}
