//! Signing in and out.
//!
//! Three endpoints, and all three are reachable without an account — for
//! reasons [`crate::auth::gate`] lists. The interesting one is
//! `GET /api/auth/session`: a client needs to be able to ask "does this instance
//! want me to sign in, and am I signed in already" and get an answer rather than
//! a refusal, because a 401 cannot be told apart from a session that has just
//! expired.
//!
//! ## One session, handed over twice
//!
//! [`login`] mints a single token and returns it in the body *and* sets it as an
//! `HttpOnly` cookie. The dashboard uses the cookie and never touches the body's
//! copy; an agent uses the body's copy in an `Authorization: Bearer` header and
//! ignores the cookie. They are the same session and the same row, so there is
//! one expiry and one way to revoke.
//!
//! ## What a failed sign-in is allowed to say
//!
//! Nothing that distinguishes a username nobody has from a password that is
//! wrong. Both are `invalid_credentials` with no `details`, and
//! [`crate::users::password::verify_absent`] spends an Argon2 run on the
//! missing-account case so that the timing does not say what the message will
//! not.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AppState;
use crate::api::extract::Json as RequestJson;
use crate::api::users::UserView;
use crate::auth::{self, SESSION_LIFETIME, SessionToken, Viewer};
use crate::error::{AppError, AppResult};
use crate::users::{Username, password};

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "tim")]
    pub username: Username,
    #[schema(example = "correct horse battery staple")]
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    /// The account that was signed in.
    pub user: UserView,

    /// The session token.
    ///
    /// The same session as the `rhizolog_session` cookie set alongside it, not a
    /// second one. Send it as `Authorization: Bearer <token>` from anything that
    /// is not a browser; a browser should ignore this field and let the cookie
    /// do the work, since the cookie is `HttpOnly` and this is not.
    #[schema(example = "3f2a…")]
    pub token: String,

    /// When the session expires if it is not used.
    ///
    /// Using it pushes this out, so an account in daily use is never signed out.
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionStatus {
    /// Whether this wiki has any accounts at all.
    ///
    /// False means it is open: no sign-in is asked for, nothing is refused, and
    /// the dashboard shows no login page. That is the state a fresh wiki is in.
    pub authentication_required: bool,

    /// Whether this request is signed in. Always true on an open wiki, where
    /// every request is the single user.
    pub authenticated: bool,

    /// The signed-in account. Null on an open wiki, which deliberately has no
    /// account to name, and null when not signed in.
    pub user: Option<UserView>,
}

/// Sign in.
///
/// Returns a session as a cookie and as a token; see the module docs for why
/// both. A wrong password and an account that does not exist are the same
/// answer and take the same time.
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "accounts",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in", body = LoginResponse),
        (status = 401, description = "Wrong username or password", body = crate::error::ErrorResponse),
        (status = 409, description = "The account has no password set", body = crate::error::ErrorResponse),
    ),
)]
pub async fn login(
    State(state): State<AppState>,
    RequestJson(request): RequestJson<LoginRequest>,
) -> AppResult<Response> {
    let Some(user) = state.users.find(&request.username).await? else {
        // Spend the same time as a real verification would, or the difference
        // enumerates the accounts on this instance one guess at a time.
        password::verify_absent(request.password).await;
        return Err(AppError::InvalidCredentials);
    };

    let Some(stored) = user.frontmatter.password.clone() else {
        // Told plainly rather than reported as a bad password. It is only
        // reachable for an account file somebody wrote by hand and did not
        // finish, no password can ever be right for it, and an operator staring
        // at "incorrect password" has no way to work out why.
        return Err(AppError::NoPasswordSet {
            username: user.username,
        });
    };

    if !password::verify_in_background(request.password, stored).await {
        tracing::info!(username = %user.username, "a sign-in was refused");
        return Err(AppError::InvalidCredentials);
    }

    let now = Utc::now();
    let expires = now + SESSION_LIFETIME;
    let token = SessionToken::mint();

    state
        .index
        .create_session(&token.hash(), &user.username, now, expires)
        .await?;

    // One delete on a table that only grows otherwise. Sign-in is the natural
    // moment for it: it is rare, it is already writing, and nothing is waiting
    // on the answer.
    if let Err(error) = state.index.purge_expired_sessions(now).await {
        tracing::warn!(%error, "could not purge expired sessions");
    }

    tracing::info!(username = %user.username, "signed in");

    let body = LoginResponse {
        user: user.into(),
        token: token.expose().to_owned(),
        expires,
    };

    Ok(with_cookie(
        StatusCode::OK,
        auth::session_cookie(&token, state.secure_cookies).to_string(),
        Json(body),
    ))
}

/// Sign out.
///
/// Ends the session the request arrived with, and clears the cookie. Signing out
/// when not signed in is a `204` rather than an error: the state being asked for
/// is the state you are in.
///
/// Only this session ends. Signing out everywhere is what changing the password
/// does — see [`crate::api::users::patch_user`].
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "accounts",
    responses((status = 204, description = "Signed out, or was not signed in")),
)]
pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Response> {
    if let Some(token) = auth::token_of(&headers) {
        state
            .index
            .delete_session(&auth::hash_token(&token))
            .await?;
    }

    // Cleared whether or not there was a session to end. A cookie the server
    // does not recognise is exactly the cookie a browser should stop sending.
    Ok(with_cookie(
        StatusCode::NO_CONTENT,
        auth::cleared_cookie(state.secure_cookies).to_string(),
        (),
    ))
}

/// Whether this instance wants a sign-in, and whether this request has one.
///
/// The first call a client makes. It never refuses: a 401 here would be an
/// answer a client cannot tell apart from a session that has just expired,
/// which is the one thing it is asking about.
#[utoipa::path(
    get,
    path = "/api/auth/session",
    tag = "accounts",
    responses((status = 200, description = "What this request is", body = SessionStatus)),
)]
pub async fn read_session(viewer: Viewer) -> AppResult<Json<SessionStatus>> {
    Ok(Json(SessionStatus {
        authentication_required: viewer.authentication_required(),
        authenticated: viewer.is_permitted(),
        user: viewer.account().cloned().map(UserView::from),
    }))
}

/// A response carrying a `Set-Cookie`.
fn with_cookie(status: StatusCode, cookie: String, body: impl IntoResponse) -> Response {
    (status, [(header::SET_COOKIE, cookie)], body).into_response()
}
