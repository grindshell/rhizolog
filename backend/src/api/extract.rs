//! Extractors that fail in Rhizolog's error envelope.
//!
//! `axum::Json` rejects a bad body with its own response shape, which would
//! leave one class of error — the one a caller is most likely to hit while
//! finding its footing — looking nothing like every other error the API
//! returns. Since the OpenAPI document promises that every failure has the
//! shape `{"error": {"code", "message", "details"}}`, that promise has to hold
//! for extraction too.

use axum::extract::{FromRequest, FromRequestParts, Request, rejection::JsonRejection};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::AppError;
use crate::words::{self as words, ACTOR_API, ACTOR_HEADER, MAX_ACTOR};

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

/// Which tool a write says it is.
///
/// Read from the `X-Rhizolog-Actor` header, and `api` when there is none. It
/// goes into the word log so that "how much of today came through Claude" has an
/// answer, and it is **a claim rather than a proof**: on an open wiki anything
/// that can write can say anything. That is fine, because the question is
/// bookkeeping about your own tools. It is said out loud rather than assumed.
///
/// A header that could not be written into a tab-separated line is refused
/// rather than trimmed to fit. Silently rewriting somebody's provenance is worse
/// than telling them the header was no good, and the fix is one character.
pub struct Actor(pub String);

impl<S> FromRequestParts<S> for Actor
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(header) = parts.headers.get(ACTOR_HEADER) else {
            return Ok(Self(ACTOR_API.to_owned()));
        };

        header
            .to_str()
            .ok()
            .and_then(words::actor)
            .map(Self)
            .ok_or_else(|| AppError::InvalidActor {
                // Not the header's own bytes: it is what was just refused for
                // being unprintable, and echoing it into a JSON error would be
                // putting it somewhere else it does not belong.
                maximum: MAX_ACTOR,
            })
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
