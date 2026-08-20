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
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use utoipa::ToSchema;

use crate::ideas::{CaptureId, IdError, IdeaId, IdeaServiceError, IdeaStoreError, RecordKind};
use crate::index::IndexError;
use crate::page::PageError;
use crate::slug::{Slug, SlugError};
use crate::store::StoreError;
use crate::times::{TimeId, TimeIdError, TimeStoreError};
use crate::users::password::PasswordError;
use crate::users::{UserStoreError, Username, UsernameError};

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

    #[error("invalid time id {raw:?}: {source}")]
    InvalidTimeId {
        raw: String,
        #[source]
        source: TimeIdError,
    },

    #[error(transparent)]
    Times(#[from] TimeStoreError),

    /// Stopping a timer that is already stopped.
    ///
    /// A conflict rather than a bad request: the request was well formed and
    /// would have worked a moment earlier, which is exactly the case a caller
    /// wants to tell apart from a typo.
    #[error("{id} is not running")]
    TimeNotRunning { id: TimeId },

    /// An entry whose end precedes its start.
    ///
    /// Refused on the way in, so the API can never be the source of one. A file
    /// edited by hand can still say it, and there it counts as zero rather than
    /// subtracting from every total it appears in.
    #[error("a time entry cannot end before it starts")]
    TimeRangeInverted {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },

    /// This wiki has accounts and the request did not name one.
    ///
    /// Deliberately says nothing about *which* accounts exist. The response is
    /// identical for a missing token, an expired one, and one belonging to an
    /// account that has since been deleted — a caller's next move is the same in
    /// all three, and the differences are exactly what an attacker would like to
    /// be told.
    #[error("this wiki requires authentication")]
    Unauthorized,

    /// The request said who it was, and that is not enough.
    ///
    /// Distinct from [`AppError::Unauthorized`] because the remedies are
    /// opposites: signing in again fixes one and cannot fix the other.
    #[error("not permitted to {action}")]
    Forbidden { action: &'static str },

    /// A sign-in that did not work.
    ///
    /// One code for a username nobody has and a password that is wrong,
    /// because telling them apart is a list of the accounts on the instance,
    /// one guess at a time. [`crate::users::password::verify_absent`] makes the
    /// two cost the same, so the timing does not say what the message will not.
    #[error("incorrect username or password")]
    InvalidCredentials,

    /// The account exists, and nobody has given it a password.
    ///
    /// Not folded into [`AppError::InvalidCredentials`]: this one is only
    /// reachable for an account file somebody wrote by hand and did not finish,
    /// and there is nothing an operator can do about it if the server insists on
    /// calling it a bad password. It reveals that the account exists, which is
    /// acceptable precisely because no password can ever be right for it.
    #[error("the account {username} has no password set")]
    NoPasswordSet { username: Username },

    #[error("invalid username {raw:?}: {source}")]
    InvalidUsername {
        raw: String,
        #[source]
        source: UsernameError,
    },

    #[error(transparent)]
    Users(#[from] UserStoreError),

    #[error(transparent)]
    Password(#[from] PasswordError),

    /// Deleting or demoting the last account that can administer accounts.
    ///
    /// Refused rather than allowed, because the result is a wiki that requires
    /// authentication and has nobody able to add an account to it — recoverable
    /// only by editing files on the server's disk, which is the one thing
    /// somebody administering a remote instance cannot do.
    #[error("this is the only owner; promote another account first")]
    LastOwner,

    /// A page narrowed to `restricted` or `private` with nobody able to read it.
    ///
    /// Refused rather than written. The result would be a page invisible to
    /// everyone including its author, recoverable only by editing the file on
    /// the server's disk — which is the one thing somebody using a remote
    /// instance cannot do.
    #[error("a {visibility} page needs an owner, and {slug} would have none")]
    OwnerlessPage {
        slug: Slug,
        visibility: &'static str,
    },

    /// An Idea Inbox id in a URL that is not one.
    #[error("invalid {record} id {raw:?}: {source}")]
    InvalidRecordId {
        record: RecordKind,
        raw: String,
        #[source]
        source: IdError,
    },

    #[error(transparent)]
    Ideas(#[from] IdeaServiceError),

    /// Deleting a capture an idea has nothing else to stand on.
    ///
    /// A conflict rather than a bad request: the request is well formed and
    /// would work the moment the idea holds something else or is retired. The
    /// alternative is an idea with no authored evidence, which cannot be given a
    /// lifecycle state at all, and manufacturing one out of nothing is the thing
    /// this feature must never do.
    #[error("{id} is the only capture {name} still holds; connect another or retire the idea")]
    CaptureRequiredByIdea {
        id: CaptureId,
        idea: IdeaId,
        name: String,
    },

    /// Disconnecting an idea's last capture.
    ///
    /// Refused for the same reason, and pointed at the reversible answer:
    /// retiring an idea sets it aside without leaving it unanswerable.
    #[error("an idea has to keep at least one capture; retire {id} instead")]
    IdeaWouldBeEmpty { id: IdeaId },

    /// Retiring what is already retired, or reopening what is not.
    ///
    /// Conflicts rather than no-ops. Affirming twice is two affirmations at two
    /// times and is meaningful; retiring twice is a caller that thinks the state
    /// is something it is not, and telling it so is more use than a second event
    /// nobody asked for.
    #[error("{id} is already retired")]
    IdeaAlreadyRetired { id: IdeaId },

    #[error("{id} is not retired")]
    IdeaNotRetired { id: IdeaId },

    /// Candidates were asked for and the derived half could not answer.
    ///
    /// Analysis is derived and retryable, so this never means anything is lost:
    /// the capture's text is on disk and a reindex builds its terms again. It is
    /// deliberately not folded into a generic failure, because the caller's next
    /// move is specific and the response is the only place to say what it is.
    #[error(
        "the analyzer has no terms for {id}. Nothing is lost; run POST /api/reindex and ask again."
    )]
    IdeaAnalysisUnavailable { id: CaptureId },

    /// The authored file was written and the index would not take it.
    ///
    /// Says so plainly, because the two halves of a write have come apart and a
    /// caller that retried would write the record twice. Nothing is lost: the
    /// files are the truth and `POST /api/reindex` puts the derived half back in
    /// step.
    #[error(
        "{what} was written to disk, but the index would not take it. Nothing is lost; run \
         POST /api/reindex to bring the index back in step."
    )]
    WrittenButNotIndexed {
        what: &'static str,
        #[source]
        source: IndexError,
    },

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
            Self::Times(error) => match error {
                TimeStoreError::NotFound { .. } => StatusCode::NOT_FOUND,
                TimeStoreError::EscapesRoot { .. } => StatusCode::BAD_REQUEST,
                TimeStoreError::NotUtf8 { .. } | TimeStoreError::Malformed { .. } => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
                TimeStoreError::NoFreeId { .. } | TimeStoreError::Io(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            },
            Self::Users(error) => match error {
                UserStoreError::NotFound { .. } => StatusCode::NOT_FOUND,
                UserStoreError::AlreadyExists { .. } => StatusCode::CONFLICT,
                UserStoreError::EscapesRoot { .. } => StatusCode::BAD_REQUEST,
                UserStoreError::NotUtf8 { .. } | UserStoreError::Malformed { .. } => {
                    StatusCode::UNPROCESSABLE_ENTITY
                }
                UserStoreError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            },
            // A sign-in that failed is a 401, not a 400: the request was
            // perfectly well formed and the credentials were not accepted.
            Self::Unauthorized | Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            // The account is real and unusable, which is a state of the server
            // rather than a fault in the request.
            Self::NoPasswordSet { .. } | Self::LastOwner => StatusCode::CONFLICT,
            // A bad request rather than a conflict: the caller asked for a state
            // that is not allowed to exist, and adding one field fixes it.
            Self::OwnerlessPage { .. } => StatusCode::BAD_REQUEST,
            Self::Ideas(error) => match error {
                IdeaServiceError::CaptureNotFound { .. }
                | IdeaServiceError::IdeaNotFound { .. } => StatusCode::NOT_FOUND,
                IdeaServiceError::EmptyCapture
                | IdeaServiceError::EmptyName
                | IdeaServiceError::NoSeeds
                | IdeaServiceError::TooManySeeds { .. } => StatusCode::BAD_REQUEST,
                // A draft the reader would refuse means the server built one
                // wrong, which is not something the caller phrased.
                IdeaServiceError::Record(_) => StatusCode::INTERNAL_SERVER_ERROR,
                IdeaServiceError::Store(store) => match store {
                    IdeaStoreError::NotFound { .. } => StatusCode::NOT_FOUND,
                    IdeaStoreError::EscapesRoot { .. } => StatusCode::BAD_REQUEST,
                    IdeaStoreError::NotUtf8 { .. } | IdeaStoreError::Malformed { .. } => {
                        StatusCode::UNPROCESSABLE_ENTITY
                    }
                    IdeaStoreError::NoFreeId { .. } | IdeaStoreError::Io(_) => {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                },
            },
            Self::CaptureRequiredByIdea { .. }
            | Self::IdeaWouldBeEmpty { .. }
            | Self::IdeaAlreadyRetired { .. }
            | Self::IdeaNotRetired { .. } => StatusCode::CONFLICT,
            Self::WrittenButNotIndexed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            // Not a 500: nothing is broken and nothing is lost. The derived half
            // is behind the authored half, which a reindex fixes, and 503 is the
            // status that means come back rather than something went wrong.
            Self::IdeaAnalysisUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::RouteNotFound { .. } | Self::PinNotFound { .. } => StatusCode::NOT_FOUND,
            Self::TooManyPins { .. } | Self::TimeNotRunning { .. } => StatusCode::CONFLICT,
            Self::InvalidRecordId { .. }
            | Self::InvalidTimeId { .. }
            | Self::TimeRangeInverted { .. }
            | Self::InvalidRequestBody { .. }
            | Self::InvalidUsername { .. }
            | Self::Password(_)
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
            Self::Times(error) => match error {
                TimeStoreError::NotFound { .. } => "time_not_found",
                TimeStoreError::EscapesRoot { .. } => "time_escapes_root",
                TimeStoreError::NotUtf8 { .. } => "time_not_utf8",
                TimeStoreError::Malformed { .. } => "time_malformed",
                TimeStoreError::NoFreeId { .. } => "time_id_exhausted",
                TimeStoreError::Io(_) => "io_error",
            },
            Self::Users(error) => match error {
                UserStoreError::NotFound { .. } => "user_not_found",
                UserStoreError::AlreadyExists { .. } => "user_already_exists",
                UserStoreError::EscapesRoot { .. } => "username_escapes_root",
                UserStoreError::NotUtf8 { .. } => "user_not_utf8",
                UserStoreError::Malformed { .. } => "user_malformed",
                UserStoreError::Io(_) => "io_error",
            },
            // The specific rule, as for a slug, so a caller that built a bad
            // name can correct it from the response.
            Self::InvalidUsername { source, .. } => source.code(),
            Self::Password(error) => error.code(),
            Self::Unauthorized => "unauthorized",
            Self::Forbidden { .. } => "forbidden",
            Self::InvalidCredentials => "invalid_credentials",
            Self::NoPasswordSet { .. } => "no_password_set",
            Self::LastOwner => "last_owner",
            Self::OwnerlessPage { .. } => "ownerless_page",
            Self::Ideas(error) => error.code(),
            Self::InvalidRecordId { record, .. } => match record {
                RecordKind::Capture => "invalid_capture_id",
                RecordKind::Idea => "invalid_idea_id",
                RecordKind::Event => "invalid_idea_event_id",
            },
            Self::CaptureRequiredByIdea { .. } => "capture_required_by_idea",
            Self::IdeaWouldBeEmpty { .. } => "idea_would_be_empty",
            Self::IdeaAlreadyRetired { .. } => "idea_already_retired",
            Self::IdeaNotRetired { .. } => "idea_not_retired",
            Self::IdeaAnalysisUnavailable { .. } => "idea_analysis_unavailable",
            Self::WrittenButNotIndexed { .. } => "written_but_not_indexed",
            Self::InvalidTimeId { .. } => "invalid_time_id",
            Self::TimeNotRunning { .. } => "time_not_running",
            Self::TimeRangeInverted { .. } => "time_range_inverted",
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
            Self::Times(error) => match error {
                TimeStoreError::NotFound { id }
                | TimeStoreError::EscapesRoot { id }
                | TimeStoreError::NotUtf8 { id } => Some(json!({ "id": id })),
                TimeStoreError::Malformed { id, source } => Some(json!({
                    "id": id,
                    "reason": source.to_string(),
                })),
                TimeStoreError::NoFreeId { start } => Some(json!({ "start": start })),
                TimeStoreError::Io(_) => None,
            },
            Self::Users(error) => match error {
                UserStoreError::NotFound { username }
                | UserStoreError::AlreadyExists { username }
                | UserStoreError::EscapesRoot { username }
                | UserStoreError::NotUtf8 { username } => Some(json!({ "username": username })),
                UserStoreError::Malformed { username, source } => Some(json!({
                    "username": username,
                    "reason": source.to_string(),
                })),
                UserStoreError::Io(_) => None,
            },
            Self::InvalidUsername { raw, source } => Some(json!({
                "username": raw,
                "rule": source.code(),
                "reason": source.to_string(),
            })),
            Self::Password(error) => Some(json!({
                "rule": error.code(),
                "minimum": crate::users::password::MIN_PASSWORD_LEN,
                "maximum": crate::users::password::MAX_PASSWORD_LEN,
            })),
            Self::Forbidden { action } => Some(json!({ "action": action })),
            Self::OwnerlessPage { slug, visibility } => Some(json!({
                "slug": slug,
                "visibility": visibility,
            })),
            Self::NoPasswordSet { username } => Some(json!({ "username": username })),
            // Nothing. Which of the three ways a request can be nobody is
            // exactly what an attacker would like to be told, and a caller's
            // next move — sign in — is the same for all of them.
            Self::Unauthorized | Self::InvalidCredentials | Self::LastOwner => None,
            Self::Ideas(error) => match error {
                // The id, and nothing about whose it was. A caller who may not
                // read a capture must not be able to tell it exists.
                IdeaServiceError::CaptureNotFound { id } => Some(json!({ "id": id })),
                IdeaServiceError::IdeaNotFound { id } => Some(json!({ "id": id })),
                IdeaServiceError::TooManySeeds { count } => Some(json!({
                    "count": count,
                    "maximum": crate::ideas::MAX_SEEDS,
                })),
                IdeaServiceError::Store(store) => store
                    .id()
                    .map(|id| json!({ "id": id, "reason": store.to_string() })),
                IdeaServiceError::EmptyCapture
                | IdeaServiceError::EmptyName
                | IdeaServiceError::NoSeeds
                | IdeaServiceError::Record(_) => None,
            },
            Self::InvalidRecordId { raw, source, .. } => Some(json!({
                "id": raw,
                "reason": source.to_string(),
            })),
            // Names the idea that would be left with nothing, so a caller can
            // say which one and offer the two ways out rather than a refusal.
            Self::CaptureRequiredByIdea { id, idea, name } => Some(json!({
                "id": id,
                "idea": idea,
                "name": name,
            })),
            Self::IdeaWouldBeEmpty { id }
            | Self::IdeaAlreadyRetired { id }
            | Self::IdeaNotRetired { id } => Some(json!({ "id": id })),
            Self::IdeaAnalysisUnavailable { id } => Some(json!({ "id": id })),
            Self::WrittenButNotIndexed { what, .. } => Some(json!({ "written": what })),
            Self::InvalidTimeId { raw, source } => Some(json!({
                "id": raw,
                "reason": source.to_string(),
            })),
            Self::TimeNotRunning { id } => Some(json!({ "id": id })),
            Self::TimeRangeInverted { start, end } => Some(json!({
                "start": start,
                "end": end,
            })),
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
            Self::Store(StoreError::Io(_))
            | Self::Times(TimeStoreError::Io(_))
            | Self::Users(UserStoreError::Io(_))
            | Self::Ideas(IdeaServiceError::Store(IdeaStoreError::Io(_)))
            | Self::Index(_)
            | Self::Internal { .. } => "the server failed to handle the request".to_owned(),
            // The exceptions to the rule above. Both are server-side failures
            // the caller has to be told about, because in both cases the
            // authored files are fine and the useful next move is a reindex
            // rather than a retry: retrying the first would write the record a
            // second time, and retrying the second would fail again.
            Self::WrittenButNotIndexed { .. } | Self::IdeaAnalysisUnavailable { .. } => {
                self.to_string()
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

        let mut response = (status, Json(body)).into_response();

        // What RFC 9110 requires of a 401, and what tells a caller which of the
        // two transports to reach for. `Bearer` rather than `Basic` matters to a
        // browser as well as to an agent: `Basic` would make it pop its own
        // credentials dialog over the dashboard's login page.
        if status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::HeaderValue::from_static("Bearer"),
            );
        }

        response
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

impl From<UsernameError> for AppError {
    fn from(source: UsernameError) -> Self {
        Self::InvalidUsername {
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
