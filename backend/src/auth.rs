//! Who is making this request.
//!
//! ## Accounts existing is the switch
//!
//! A wiki with no accounts is **open**: every request is the single user the
//! product was built around, nothing is refused, and no login page is ever
//! shown. That is not a special mode, it is the absence of one — it is exactly
//! how Rhizolog behaved before this module existed, and it is what keeps the
//! desktop app, the README's quick start and every existing test working
//! unchanged.
//!
//! Creating the first account turns authentication on for that wiki, and from
//! then on every `/api` request has to say who it is. There is no separate
//! setting to disagree with the directory; see `knowledge-base/accounts.md`.
//!
//! ## One session, two transports
//!
//! Signing in mints a 256-bit token and hands it back two ways at once: as an
//! `HttpOnly` cookie, which is what the dashboard uses and never lets JavaScript
//! near, and in the response body, which is what an agent or a script puts in an
//! `Authorization: Bearer` header. They name the same session and the same row,
//! so there is one lifetime, one expiry and one way to revoke.
//!
//! That matters more here than it would elsewhere. The whole premise is that
//! [the API is the only interface](../../knowledge-base/api-design.md) and that
//! it is as usable by an agent as by a browser. A cookie-only design would make
//! authentication the point where those two stop being the same API.
//!
//! ## `SameSite=Lax` is doing real work
//!
//! There is no CSRF token anywhere in this codebase, and the cookie's
//! `SameSite=Lax` is why one is not needed: a browser will not attach it to a
//! cross-site `POST`, `PUT`, `PATCH` or `DELETE`, only to a top-level `GET`
//! navigation. Every state-changing endpoint here is one of the former. Weaken
//! that attribute and the gap has to be filled with something else.

use std::fmt;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, SameSite};
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

use crate::api::AppState;
use crate::error::{AppError, AppResult};
use crate::users::{Role, User, Username};

/// The cookie a browser session travels in.
pub const SESSION_COOKIE: &str = "rhizolog_session";

/// How long a session lasts without being used.
pub const SESSION_LIFETIME: Duration = Duration::days(30);

/// How much of the lifetime has to have gone before a request pushes the expiry
/// back out.
///
/// Sessions slide, so somebody who uses the wiki daily is never signed out. The
/// threshold is what stops that costing a database write on every single
/// request: the expiry moves at most once a day, and the only cost of the delay
/// is that a session may expire up to a day earlier than a strictly sliding one
/// would.
pub const SESSION_RENEW_AFTER: Duration = Duration::days(1);

/// Bytes of randomness in a session token.
///
/// 256 bits from the OS. There is nothing to guess, which is the property the
/// [session table](crate::index::sessions) relies on when it stores a plain
/// SHA-256 rather than a password hash.
const TOKEN_BYTES: usize = 32;

/// A freshly minted session token, before it is handed to anybody.
///
/// Deliberately not `Display` and not `Serialize`: the two places it is allowed
/// to go — a `Set-Cookie` header and one field of the login response — both ask
/// for it by name. Anything else has to reach for [`SessionToken::expose`],
/// which is a thing a reader can search for.
pub struct SessionToken(String);

impl SessionToken {
    /// Mint a token from the operating system's random source.
    pub fn mint() -> Self {
        use rand_core::{OsRng, RngCore};

        let mut bytes = [0_u8; TOKEN_BYTES];
        OsRng.fill_bytes(&mut bytes);
        Self(to_hex(&bytes))
    }

    /// The token itself, for the one response and the one header that carry it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// What the index stores.
    pub fn hash(&self) -> String {
        hash_token(&self.0)
    }
}

/// Keeps a token out of a log line that formats a struct with `{:?}`.
impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken(...)")
    }
}

/// The index's key for a token.
///
/// SHA-256 and not Argon2: see [`crate::index::sessions`]. A slow hash buys
/// nothing against 256 bits of randomness and would be paid on every request
/// rather than once per sign-in.
pub fn hash_token(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Who a request is.
#[derive(Debug, Clone)]
pub enum Viewer {
    /// This wiki has no accounts. Everybody is the single user, and nothing is
    /// refused — the behaviour Rhizolog had before accounts existed.
    Open,
    /// This wiki has accounts and the request proved it holds one.
    Account(Box<User>),
    /// This wiki has accounts and the request proved nothing.
    Anonymous,
}

impl Viewer {
    /// Whether this wiki is asking anybody to sign in at all.
    pub fn authentication_required(&self) -> bool {
        !matches!(self, Self::Open)
    }

    /// Whether the request is allowed to act.
    pub fn is_permitted(&self) -> bool {
        !matches!(self, Self::Anonymous)
    }

    pub fn account(&self) -> Option<&User> {
        match self {
            Self::Account(user) => Some(user),
            Self::Open | Self::Anonymous => None,
        }
    }

    /// The account's name, or `None` for an open wiki and for anonymous.
    ///
    /// An open wiki deliberately has no name to give. Inventing one — `local`,
    /// `owner`, the OS user — would put a value in `owner:` frontmatter that
    /// means nothing the moment a real account is created.
    pub fn username(&self) -> Option<&Username> {
        self.account().map(|user| &user.username)
    }

    pub fn role(&self) -> Option<Role> {
        self.account().map(User::role)
    }

    /// Whether this request may create, change and delete accounts.
    ///
    /// True on an open wiki, because there is nobody to withhold it from and
    /// somebody has to be able to create the first account.
    pub fn may_administer_accounts(&self) -> bool {
        match self {
            Self::Open => true,
            Self::Account(user) => user.is_owner(),
            Self::Anonymous => false,
        }
    }

    /// Refuse an anonymous request.
    pub fn require_account(&self) -> AppResult<()> {
        match self {
            Self::Open | Self::Account(_) => Ok(()),
            Self::Anonymous => Err(AppError::Unauthorized),
        }
    }

    /// Refuse anybody who may not administer accounts.
    pub fn require_owner(&self) -> AppResult<()> {
        self.require_account()?;

        if self.may_administer_accounts() {
            return Ok(());
        }

        Err(AppError::Forbidden {
            action: "administer accounts",
        })
    }
}

/// The token a request carries, from either transport.
///
/// The header is checked first. A browser that is signed in as one account and
/// a script driving it as another is a real situation, and an explicit
/// `Authorization` header is the more deliberate of the two.
pub fn token_of(headers: &HeaderMap) -> Option<String> {
    if let Some(bearer) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            // The scheme is case-insensitive per RFC 7235, and clients spell it
            // both ways.
            let (scheme, token) = value.split_once(' ')?;
            scheme.eq_ignore_ascii_case("bearer").then_some(token)
        })
    {
        let bearer = bearer.trim();
        if !bearer.is_empty() {
            return Some(bearer.to_owned());
        }
    }

    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(Cookie::split_parse_encoded)
        .filter_map(Result::ok)
        .find(|cookie| cookie.name() == SESSION_COOKIE)
        .map(|cookie| cookie.value().to_owned())
}

/// Work out who a request is.
///
/// The account list is read from the directory rather than from anything
/// cached, so an account added or removed by hand takes effect on the next
/// request. On an authenticated request the account's own file is read too,
/// which is what makes a role change or a deletion immediate rather than
/// pending until the session expires. Both are small reads from a directory
/// with a handful of files in it; see [`crate::users::store`] for why there is
/// no index over them.
pub async fn resolve(state: &AppState, headers: &HeaderMap) -> AppResult<Viewer> {
    if state.users.is_empty().await? {
        return Ok(Viewer::Open);
    }

    let Some(token) = token_of(headers) else {
        return Ok(Viewer::Anonymous);
    };

    let hash = hash_token(&token);
    let now = Utc::now();

    let Some(session) = state.index.session(&hash, now).await? else {
        return Ok(Viewer::Anonymous);
    };

    // The session names an account; the account is the file. A session whose
    // account has been deleted is not a session — this is what makes deleting
    // an account take effect now rather than in thirty days, even for a token
    // whose row somehow outlived the revocation.
    let Some(user) = state.users.find(&session.username).await? else {
        tracing::info!(
            username = %session.username,
            "a session names an account that no longer exists; ending it"
        );
        state.index.delete_session(&hash).await?;
        return Ok(Viewer::Anonymous);
    };

    renew_if_stale(state, &hash, session.expires, now).await;

    Ok(Viewer::Account(Box::new(user)))
}

/// Push a session's expiry out, at most once every [`SESSION_RENEW_AFTER`].
///
/// Failures are logged and swallowed. The session is valid; refusing the request
/// because its expiry could not be extended would turn a housekeeping problem
/// into an outage.
async fn renew_if_stale(state: &AppState, hash: &str, expires: DateTime<Utc>, now: DateTime<Utc>) {
    if expires - now > SESSION_LIFETIME - SESSION_RENEW_AFTER {
        return;
    }

    if let Err(error) = state
        .index
        .renew_session(hash, now + SESSION_LIFETIME)
        .await
    {
        tracing::warn!(%error, "could not extend a session's expiry");
    }
}

/// The cookie that carries a session to a browser.
pub fn session_cookie(token: &SessionToken, secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, token.expose().to_owned());
    // Never readable from JavaScript: a wiki renders markdown somebody else may
    // have written, and this is the one value on the page that must survive
    // that being wrong.
    cookie.set_http_only(true);
    // Load-bearing, and the reason there is no CSRF token — see the module docs.
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie.set_secure(secure);
    cookie.set_max_age(time::Duration::seconds(SESSION_LIFETIME.num_seconds()));
    cookie
}

/// The cookie that takes one away.
///
/// Same name, path and flags, an empty value and an expiry in the past. All
/// three of those attributes have to match the original or the browser treats it
/// as a different cookie and quietly keeps the one that is signed in.
pub fn cleared_cookie(secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::new(SESSION_COOKIE, "");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie.set_secure(secure);
    cookie.set_max_age(time::Duration::ZERO);
    cookie
}

/// Whether a route may be reached without an account.
///
/// The list is short and every entry earns its place:
///
/// - **`/api/health`** is the liveness signal, and
///   [`crate::endpoint::live`] confirms a published server with it *before*
///   anybody could have signed in. Gating it would break the desktop app's
///   single-instance check. What it reports to an anonymous caller is trimmed
///   instead — see [`crate::api::meta::health`].
/// - **`/api/auth/session`** is how a client finds out whether it needs to sign
///   in at all. A 401 would be an answer, but a client cannot tell that one
///   apart from "your session just expired".
/// - **`/api/auth/login`** for the obvious reason, and **`/api/auth/logout`**
///   because signing out when you were not signed in should be a no-op rather
///   than an error about not being signed in.
/// - **`POST /api/users`** is the bootstrap, and it is safe for a reason worth
///   stating: it only succeeds while the wiki has no accounts, and a wiki with
///   no accounts is already fully readable and writable by anybody who can
///   reach it. Claiming the first account there grants nothing that was not
///   already on offer. The handler enforces that, not this list.
fn is_public(method: &axum::http::Method, path: &str) -> bool {
    use axum::http::Method;

    matches!(
        (method, path),
        (&Method::GET, "/api/health")
            | (&Method::GET, "/api/auth/session")
            | (&Method::POST, "/api/auth/login")
            | (&Method::POST, "/api/auth/logout")
            | (&Method::POST, "/api/users")
    )
}

/// Decide who a request is, and turn away the ones that are nobody.
///
/// Runs once per request, and the answer is put in the request's extensions so
/// that the [`Viewer`] extractor is a lookup rather than a second session
/// resolution.
///
/// Only `/api` is gated. The dashboard itself is served to anybody, because the
/// login page is part of the dashboard and a login page behind a login is not a
/// way in.
pub async fn gate(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();

    if !crate::api::is_api_path(&path) {
        return next.run(request).await;
    }

    let viewer = match resolve(&state, request.headers()).await {
        Ok(viewer) => viewer,
        Err(error) => return error.into_response(),
    };

    if !viewer.is_permitted() && !is_public(request.method(), &path) {
        return AppError::Unauthorized.into_response();
    }

    request.extensions_mut().insert(viewer);
    next.run(request).await
}

/// Read the viewer the [`gate`] worked out.
impl<S: Send + Sync> FromRequestParts<S> for Viewer {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Viewer>().cloned().ok_or_else(|| {
            // Only reachable if a handler is mounted outside the gate, which is
            // a wiring mistake rather than anything a caller did. Failing closed
            // is the only safe answer: the alternative is a route that silently
            // treats everyone as the single user.
            tracing::error!(
                "a handler asked who the request was, but the authentication gate did not run"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AppError::internal("authentication did not run for this route"),
            )
                .into_response()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::http::HeaderValue;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name.clone(), HeaderValue::from_str(value).expect("header"));
        }
        map
    }

    #[test]
    fn a_minted_token_is_random_and_hex() {
        let first = SessionToken::mint();
        let second = SessionToken::mint();

        assert_ne!(first.expose(), second.expose());
        assert_eq!(first.expose().len(), TOKEN_BYTES * 2);
        assert!(first.expose().chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// What the index holds must not be what a client sends, or a copy of the
    /// database is a pile of live credentials.
    #[test]
    fn the_stored_hash_is_not_the_token() {
        let token = SessionToken::mint();

        assert_ne!(token.hash(), token.expose());
        assert_eq!(token.hash(), hash_token(token.expose()));
        assert_eq!(token.hash().len(), 64, "sha-256, hex encoded");
    }

    /// The one place a token is printed by accident is a `{:?}` on a struct
    /// holding one.
    #[test]
    fn a_token_does_not_print_itself() {
        let token = SessionToken::mint();
        assert!(!format!("{token:?}").contains(token.expose()));
    }

    #[test]
    fn a_bearer_token_is_read_from_the_authorization_header() {
        assert_eq!(
            token_of(&headers(&[(header::AUTHORIZATION, "Bearer abc123")])),
            Some("abc123".to_owned())
        );
        // RFC 7235 makes the scheme case-insensitive, and clients spell it both
        // ways.
        assert_eq!(
            token_of(&headers(&[(header::AUTHORIZATION, "bearer abc123")])),
            Some("abc123".to_owned())
        );
    }

    #[test]
    fn a_session_cookie_is_read_from_among_others() {
        let value = format!("theme=dark; {SESSION_COOKIE}=abc123; other=1");
        assert_eq!(
            token_of(&headers(&[(header::COOKIE, &value)])),
            Some("abc123".to_owned())
        );
    }

    /// A script driving a browser session as a different account is a real
    /// situation, and the explicit header is the more deliberate of the two.
    #[test]
    fn an_explicit_header_beats_the_cookie() {
        let cookie = format!("{SESSION_COOKIE}=from-cookie");
        let map = headers(&[
            (header::AUTHORIZATION, "Bearer from-header"),
            (header::COOKIE, &cookie),
        ]);

        assert_eq!(token_of(&map), Some("from-header".to_owned()));
    }

    #[test]
    fn nothing_that_is_not_a_token_reads_as_one() {
        assert_eq!(token_of(&HeaderMap::new()), None);
        assert_eq!(
            token_of(&headers(&[(header::AUTHORIZATION, "Bearer ")])),
            None
        );
        assert_eq!(
            token_of(&headers(&[(header::AUTHORIZATION, "Basic abc")])),
            None
        );
        assert_eq!(token_of(&headers(&[(header::COOKIE, "theme=dark")])), None);
    }

    /// `HttpOnly` because a wiki renders markdown somebody else wrote;
    /// `SameSite=Lax` because it is standing in for a CSRF token.
    #[test]
    fn the_session_cookie_carries_the_attributes_it_relies_on() {
        let cookie = session_cookie(&SessionToken::mint(), false);

        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.secure(), Some(false));
    }

    #[test]
    fn secure_is_set_when_the_instance_says_it_is_behind_tls() {
        assert_eq!(
            session_cookie(&SessionToken::mint(), true).secure(),
            Some(true)
        );
        assert_eq!(cleared_cookie(true).secure(), Some(true));
    }

    /// A browser matches a replacement cookie on name, path and flags. Get any
    /// of them wrong and signing out leaves the signed-in cookie in place.
    #[test]
    fn clearing_matches_the_cookie_it_replaces() {
        let live = session_cookie(&SessionToken::mint(), false);
        let cleared = cleared_cookie(false);

        assert_eq!(cleared.name(), live.name());
        assert_eq!(cleared.path(), live.path());
        assert_eq!(cleared.same_site(), live.same_site());
        assert_eq!(cleared.http_only(), live.http_only());
        assert_eq!(cleared.value(), "");
        assert_eq!(cleared.max_age(), Some(time::Duration::ZERO));
    }

    /// The gate's allow-list. Everything else under `/api` needs an account, and
    /// this is the test that notices when something is added to it.
    #[test]
    fn only_these_five_routes_are_reachable_without_an_account() {
        use axum::http::Method;

        assert!(is_public(&Method::GET, "/api/health"));
        assert!(is_public(&Method::GET, "/api/auth/session"));
        assert!(is_public(&Method::POST, "/api/auth/login"));
        assert!(is_public(&Method::POST, "/api/auth/logout"));
        assert!(is_public(&Method::POST, "/api/users"));

        for (method, path) in [
            (Method::GET, "/api/pages"),
            (Method::GET, "/api/search"),
            (Method::GET, "/api/stats"),
            (Method::GET, "/api/times"),
            (Method::GET, "/api/users"),
            (Method::POST, "/api/reindex"),
            // The method matters as much as the path: listing accounts is not
            // the bootstrap, and neither is creating a page.
            (Method::GET, "/api/health/../pages"),
            (Method::DELETE, "/api/users"),
            (Method::POST, "/api/pages"),
        ] {
            assert!(!is_public(&method, path), "{method} {path} is not public");
        }
    }

    #[test]
    fn an_open_wiki_refuses_nothing_and_names_nobody() {
        let viewer = Viewer::Open;

        assert!(!viewer.authentication_required());
        assert!(viewer.is_permitted());
        assert!(viewer.may_administer_accounts());
        assert!(viewer.require_account().is_ok());
        assert!(viewer.require_owner().is_ok());
        // Deliberately nameless: an invented username would end up in `owner:`
        // frontmatter and mean nothing once a real account existed.
        assert_eq!(viewer.username(), None);
    }

    #[test]
    fn anonymous_may_do_nothing() {
        let viewer = Viewer::Anonymous;

        assert!(viewer.authentication_required());
        assert!(!viewer.is_permitted());
        assert!(!viewer.may_administer_accounts());
        assert!(matches!(
            viewer.require_account(),
            Err(AppError::Unauthorized)
        ));
        assert!(matches!(
            viewer.require_owner(),
            Err(AppError::Unauthorized)
        ));
    }

    #[test]
    fn a_member_may_act_but_may_not_administer_accounts() {
        let viewer = Viewer::Account(Box::new(account("tim", Role::Member)));

        assert!(viewer.is_permitted());
        assert!(viewer.require_account().is_ok());
        assert!(!viewer.may_administer_accounts());
        assert!(matches!(
            viewer.require_owner(),
            Err(AppError::Forbidden { .. })
        ));
        assert_eq!(
            viewer.username().map(Username::to_string),
            Some("tim".to_owned())
        );
    }

    #[test]
    fn an_owner_may_administer_accounts() {
        let viewer = Viewer::Account(Box::new(account("tim", Role::Owner)));

        assert!(viewer.may_administer_accounts());
        assert!(viewer.require_owner().is_ok());
        assert_eq!(viewer.role(), Some(Role::Owner));
    }

    fn account(username: &str, role: Role) -> User {
        User {
            username: Username::parse(username).expect("valid username"),
            frontmatter: crate::users::UserFrontmatter {
                role,
                ..crate::users::UserFrontmatter::default()
            },
            profile: String::new(),
            updated: Utc::now(),
        }
    }
}
