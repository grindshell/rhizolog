//! Accounts, sessions, and the gate in front of the API.
//!
//! Driven in-process with `tower::ServiceExt::oneshot`, like `tests/api.rs`, but
//! against a wiki whose account directory changes underneath the router — which
//! is the point. Nothing here restarts the server, because nothing should have
//! to: accounts are files, and how many there are is read from the directory on
//! every request.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use rhizolog::{AppState, Assets, Index, Store, TimeStore, UserStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

/// The password every account in this file gets. Long enough to pass the only
/// rule there is.
const PASSWORD: &str = "correct horse battery staple";

struct App {
    directory: TempDir,
    router: Router,
}

struct Res {
    status: StatusCode,
    body: Value,
    set_cookie: Option<String>,
}

impl Res {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("<no code>")
    }
}

impl App {
    async fn new() -> Self {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        let times = TimeStore::open(directory.path())
            .await
            .expect("open time log");
        let users = UserStore::open(directory.path()).await.expect("open users");
        let ideas = rhizolog::IdeaStore::open(directory.path())
            .await
            .expect("open ideas");
        let words = rhizolog::WordLog::open(directory.path())
            .await
            .expect("open word log");
        let index = Index::open(None).await.expect("open index");

        Self {
            router: rhizolog::router(AppState {
                store,
                times,
                ideas: rhizolog::IdeaService::new(ideas),
                users,
                words,
                index,
                usage: rhizolog::UsageTally::new(),
                assets: Assets::None,
                secure_cookies: false,
                anonymous_read: false,
            }),
            directory,
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        auth: Option<&str>,
    ) -> Res {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(header_value) = auth {
            builder = builder.header(header::AUTHORIZATION, header_value);
        }

        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .expect("build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router response");

        let status = response.status();
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");

        Res {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            set_cookie,
        }
    }

    /// A request with no credentials at all.
    async fn anonymous(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        self.send(method, path, body, None).await
    }

    /// A request carrying a session token as a bearer.
    async fn as_user(&self, token: &str, method: Method, path: &str, body: Option<Value>) -> Res {
        self.send(method, path, body, Some(&format!("Bearer {token}")))
            .await
    }

    /// A request carrying a session token as a cookie, the way a browser does.
    async fn with_cookie(&self, cookie: &str, path: &str) -> Res {
        let request = Request::builder()
            .method(Method::GET)
            .uri(path)
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .expect("build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router response");

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");

        Res {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            set_cookie: None,
        }
    }

    /// Create the first account, which is always an owner.
    async fn bootstrap(&self, username: &str) -> Res {
        self.anonymous(
            Method::POST,
            "/api/users",
            Some(json!({ "username": username, "password": PASSWORD })),
        )
        .await
    }

    /// Sign in, and return the token.
    async fn sign_in(&self, username: &str) -> String {
        let res = self
            .anonymous(
                Method::POST,
                "/api/auth/login",
                Some(json!({ "username": username, "password": PASSWORD })),
            )
            .await;

        assert_eq!(res.status, StatusCode::OK, "sign-in failed: {:?}", res.body);
        res.body["token"].as_str().expect("a token").to_owned()
    }

    /// Create the first account and sign in as it, in one step.
    async fn owner(&self) -> String {
        assert_eq!(self.bootstrap("tim").await.status, StatusCode::CREATED);
        self.sign_in("tim").await
    }
}

// --------------------------------------------------------- an open wiki

/// The state a wiki starts in and the one the desktop app spends its life in:
/// no accounts, no login page, nothing refused. If this ever fails, accounts
/// have stopped being opt-in.
#[tokio::test]
async fn a_wiki_with_no_accounts_asks_for_nothing() {
    let app = App::new().await;

    let session = app.anonymous(Method::GET, "/api/auth/session", None).await;
    assert_eq!(session.status, StatusCode::OK);
    assert_eq!(session.body["authentication_required"], false);
    assert_eq!(session.body["authenticated"], true);
    // Deliberately nameless. Inventing a username here would put a value in
    // `owner:` frontmatter that means nothing once a real account exists.
    assert!(session.body["user"].is_null());

    // Reading and writing both work, with no credentials anywhere.
    assert_eq!(
        app.anonymous(Method::GET, "/api/pages", None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.anonymous(
            Method::POST,
            "/api/pages",
            Some(json!({ "slug": "notes/rhizome", "content": "Branches off.\n" })),
        )
        .await
        .status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn health_on_an_open_wiki_still_reports_everything() {
    let app = App::new().await;

    let res = app.anonymous(Method::GET, "/api/health", None).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["authentication_required"], false);
    assert_eq!(res.body["pages"], 0);
    assert!(res.body["wiki_root"].is_string());
}

// ------------------------------------------------------- the first account

/// The bootstrap: anybody may create the first account, because a wiki with no
/// accounts is already fully readable and writable by anybody who can reach it.
/// The door closes behind it.
#[tokio::test]
async fn the_first_account_may_be_created_by_anybody_and_the_second_may_not() {
    let app = App::new().await;

    let first = app.bootstrap("tim").await;
    assert_eq!(first.status, StatusCode::CREATED);
    assert_eq!(first.body["username"], "tim");

    let second = app
        .anonymous(
            Method::POST,
            "/api/users",
            Some(json!({ "username": "alice", "password": PASSWORD })),
        )
        .await;

    assert_eq!(second.status, StatusCode::UNAUTHORIZED);
    assert_eq!(second.code(), "unauthorized");
}

/// A wiki that requires authentication and has nobody able to administer it is
/// recoverable only by editing files on the server's disk — which is the one
/// thing somebody administering a remote instance cannot do.
#[tokio::test]
async fn the_first_account_is_an_owner_whatever_it_asked_for() {
    let app = App::new().await;

    let res = app
        .anonymous(
            Method::POST,
            "/api/users",
            Some(json!({ "username": "tim", "password": PASSWORD, "role": "member" })),
        )
        .await;

    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(res.body["role"], "owner");
}

#[tokio::test]
async fn a_password_that_is_too_short_is_refused_by_rule() {
    let app = App::new().await;

    let res = app
        .anonymous(
            Method::POST,
            "/api/users",
            Some(json!({ "username": "tim", "password": "short" })),
        )
        .await;

    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.code(), "password_too_short");
    // Named, so a client can say what would have worked without reading prose.
    assert_eq!(res.body["error"]["details"]["minimum"], 8);
}

/// A name in a *body* is refused while the body is being deserialised, so it
/// arrives as `invalid_request_body` carrying serde's own message — which names
/// the rule that was broken. Exactly what a bad slug in a body does, and the
/// reason that path returns 400 rather than axum's 422: the identical mistake
/// must not have two statuses depending on where it appeared.
#[tokio::test]
async fn a_bad_username_in_a_body_is_refused_with_the_reason() {
    let app = App::new().await;

    for (username, reason) in [
        ("Tim", "uppercase"),
        ("-tim", "must start with"),
        ("tim yuen", "must not contain"),
        ("con", "reserved device name"),
        ("../etc/passwd", "must not contain"),
    ] {
        let res = app
            .anonymous(
                Method::POST,
                "/api/users",
                Some(json!({ "username": username, "password": PASSWORD })),
            )
            .await;

        assert_eq!(res.status, StatusCode::BAD_REQUEST, "for {username:?}");
        assert_eq!(res.code(), "invalid_request_body", "for {username:?}");
        assert!(
            res.body["error"]["message"]
                .as_str()
                .expect("a message")
                .contains(reason),
            "{username:?} did not say why: {:?}",
            res.body
        );
    }
}

/// A name in a *path* is parsed by the handler, which is where the `username_*`
/// codes come from — the same contract a bad slug in a URL has. A caller that
/// built the name can correct itself from `details.rule` without a human
/// reading the prose.
#[tokio::test]
async fn a_bad_username_in_a_path_names_the_rule_it_broke() {
    let app = App::new().await;
    let token = app.owner().await;

    for (username, rule) in [
        ("Tim", "username_uppercase"),
        ("-tim", "username_bad_start"),
        ("con", "username_reserved_name"),
    ] {
        let res = app
            .as_user(&token, Method::GET, &format!("/api/users/{username}"), None)
            .await;

        assert_eq!(res.status, StatusCode::BAD_REQUEST, "for {username:?}");
        assert_eq!(res.code(), rule, "for {username:?}");
        assert_eq!(res.body["error"]["details"]["rule"], rule);
        assert_eq!(res.body["error"]["details"]["username"], username);
    }
}

// ----------------------------------------------------------- the gate

/// The switch. One account exists, so the API stops answering to nobody.
#[tokio::test]
async fn creating_an_account_closes_the_wiki() {
    let app = App::new().await;

    assert_eq!(
        app.anonymous(Method::GET, "/api/pages", None).await.status,
        StatusCode::OK
    );

    assert_eq!(app.bootstrap("tim").await.status, StatusCode::CREATED);

    let refused = app.anonymous(Method::GET, "/api/pages", None).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(refused.code(), "unauthorized");
}

/// Files are the truth for accounts as much as for pages. Dropping one in turns
/// authentication on with nothing to reindex and no server to restart.
#[tokio::test]
async fn an_account_written_by_hand_closes_the_wiki_too() {
    let app = App::new().await;
    let users = app.directory.path().join(".rhizolog").join("users");

    assert_eq!(
        app.anonymous(Method::GET, "/api/pages", None).await.status,
        StatusCode::OK
    );

    std::fs::create_dir_all(&users).expect("accounts directory");
    std::fs::write(users.join("alice.md"), "---\nrole: owner\n---\n")
        .expect("write an account by hand");

    assert_eq!(
        app.anonymous(Method::GET, "/api/pages", None).await.status,
        StatusCode::UNAUTHORIZED
    );
}

/// Every route the gate lets through, and the reason each one is there. A
/// sixth appearing in this list should be a deliberate act.
#[tokio::test]
async fn only_the_documented_routes_survive_the_gate() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    for (method, path, body) in [
        (Method::GET, "/api/health", None),
        (Method::GET, "/api/auth/session", None),
        (Method::POST, "/api/auth/logout", None),
    ] {
        let res = app.anonymous(method.clone(), path, body).await;
        assert_ne!(
            res.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} should be reachable"
        );
    }

    for (method, path) in [
        (Method::GET, "/api/pages"),
        (Method::GET, "/api/search?q=x"),
        (Method::GET, "/api/stats"),
        (Method::GET, "/api/tags"),
        (Method::GET, "/api/graph"),
        (Method::GET, "/api/pins"),
        (Method::GET, "/api/times"),
        (Method::GET, "/api/users"),
        (Method::GET, "/api/users/tim"),
        (Method::POST, "/api/reindex"),
    ] {
        let res = app.anonymous(method.clone(), path, None).await;
        assert_eq!(
            res.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} should need an account"
        );
    }
}

/// The desktop app's single-instance check confirms a published `server.json`
/// by reading `wiki_root` from here, before anybody could have signed in.
/// Gating this endpoint would break that on every wiki with accounts.
#[tokio::test]
async fn health_stays_reachable_but_stops_describing_the_wiki() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    let res = app.anonymous(Method::GET, "/api/health", None).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["status"], "ok");
    assert_eq!(res.body["authentication_required"], true);
    // The half the handshake needs survives.
    assert!(res.body["wiki_root"].is_string());
    // The half that describes the contents does not.
    for hidden in ["pages", "times", "running_timers", "last_indexed"] {
        assert!(
            res.body.get(hidden).is_none(),
            "{hidden} was reported to an anonymous caller: {:?}",
            res.body
        );
    }

    // Signed in, it is the endpoint it always was.
    let token = app.sign_in("tim").await;
    let res = app.as_user(&token, Method::GET, "/api/health", None).await;
    assert_eq!(res.body["pages"], 0);
}

// --------------------------------------------------------- signing in

#[tokio::test]
async fn signing_in_returns_a_session_as_a_token_and_as_a_cookie() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    let res = app
        .anonymous(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "username": "tim", "password": PASSWORD })),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["user"]["username"], "tim");
    assert_eq!(res.body["user"]["role"], "owner");
    // A hash is not something a client ever needs, and the response type has
    // nowhere to put one.
    assert!(res.body["user"].get("password").is_none());

    let token = res.body["token"].as_str().expect("a token");
    let cookie = res.set_cookie.expect("a Set-Cookie header");

    assert!(cookie.contains("rhizolog_session="));
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    // Standing in for a CSRF token — see the auth module.
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    // The server speaks HTTP; a Secure cookie here would be discarded.
    assert!(!cookie.contains("Secure"), "{cookie}");

    // The same session, reached both ways.
    assert_eq!(
        app.as_user(token, Method::GET, "/api/pages", None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        app.with_cookie(&format!("rhizolog_session={token}"), "/api/pages")
            .await
            .status,
        StatusCode::OK
    );
}

/// Telling a wrong password apart from an account that does not exist is a list
/// of the accounts on the instance, one guess at a time.
#[tokio::test]
async fn a_wrong_password_and_an_unknown_account_are_the_same_answer() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    let wrong_password = app
        .anonymous(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "username": "tim", "password": "not the password" })),
        )
        .await;
    let no_such_account = app
        .anonymous(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "username": "nobody", "password": PASSWORD })),
        )
        .await;

    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_password.code(), "invalid_credentials");
    assert_eq!(no_such_account.status, StatusCode::UNAUTHORIZED);
    assert_eq!(no_such_account.code(), "invalid_credentials");
    // Not even a `details` to tell them apart by.
    assert!(wrong_password.body["error"].get("details").is_none());
    assert_eq!(
        wrong_password.body["error"]["message"],
        no_such_account.body["error"]["message"]
    );
}

/// Only reachable for a file somebody wrote by hand and did not finish. An
/// operator staring at "incorrect password" has no way to work out why, and no
/// password can ever be right for it, so there is nothing to protect.
#[tokio::test]
async fn an_account_with_no_password_says_so_rather_than_blaming_the_password() {
    let app = App::new().await;
    let users = app.directory.path().join(".rhizolog").join("users");
    std::fs::create_dir_all(&users).expect("accounts directory");
    std::fs::write(users.join("alice.md"), "---\nrole: owner\n---\n").expect("write account");

    let res = app
        .anonymous(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "username": "alice", "password": PASSWORD })),
        )
        .await;

    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.code(), "no_password_set");
    assert_eq!(res.body["error"]["details"]["username"], "alice");
}

#[tokio::test]
async fn a_bearer_token_beats_a_cookie() {
    let app = App::new().await;
    let owner = app.owner().await;

    let created = app
        .as_user(
            &owner,
            Method::POST,
            "/api/users",
            Some(json!({ "username": "alice", "password": PASSWORD })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let member = app.sign_in("alice").await;

    // Signed in as tim by cookie, driving the API as alice by header.
    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/session")
        .header(header::COOKIE, format!("rhizolog_session={owner}"))
        .header(header::AUTHORIZATION, format!("Bearer {member}"))
        .body(Body::empty())
        .expect("build request");

    let response = app
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");

    assert_eq!(body["user"]["username"], "alice");
}

#[tokio::test]
async fn signing_out_ends_the_session_and_clears_the_cookie() {
    let app = App::new().await;
    let token = app.owner().await;

    let out = app
        .as_user(&token, Method::POST, "/api/auth/logout", None)
        .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);

    let cookie = out.set_cookie.expect("a Set-Cookie header");
    assert!(cookie.contains("rhizolog_session="), "{cookie}");
    assert!(cookie.contains("Max-Age=0"), "{cookie}");

    assert_eq!(
        app.as_user(&token, Method::GET, "/api/pages", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the token still works after signing out"
    );
}

/// The state being asked for is the state you are in.
#[tokio::test]
async fn signing_out_when_not_signed_in_is_not_an_error() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    assert_eq!(
        app.anonymous(Method::POST, "/api/auth/logout", None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}

/// A client cannot tell a 401 apart from a session that has just expired, so
/// the endpoint that answers "do I need to sign in" must never be one.
#[tokio::test]
async fn the_session_endpoint_answers_rather_than_refusing() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    let anonymous = app.anonymous(Method::GET, "/api/auth/session", None).await;
    assert_eq!(anonymous.status, StatusCode::OK);
    assert_eq!(anonymous.body["authentication_required"], true);
    assert_eq!(anonymous.body["authenticated"], false);
    assert!(anonymous.body["user"].is_null());

    let token = app.sign_in("tim").await;
    let signed_in = app
        .as_user(&token, Method::GET, "/api/auth/session", None)
        .await;
    assert_eq!(signed_in.body["authenticated"], true);
    assert_eq!(signed_in.body["user"]["username"], "tim");
}

/// RFC 9110 asks for it, and `Bearer` rather than `Basic` is what stops a
/// browser popping its own credentials dialog over the dashboard's login page.
#[tokio::test]
async fn a_401_says_how_to_authenticate() {
    let app = App::new().await;
    app.bootstrap("tim").await;

    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/pages")
        .body(Body::empty())
        .expect("build request");
    let response = app
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("router response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get(header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer")
    );
}

// ------------------------------------------------------- administering

#[tokio::test]
async fn only_an_owner_may_create_and_delete_accounts() {
    let app = App::new().await;
    let owner = app.owner().await;

    assert_eq!(
        app.as_user(
            &owner,
            Method::POST,
            "/api/users",
            Some(json!({ "username": "alice", "password": PASSWORD })),
        )
        .await
        .status,
        StatusCode::CREATED
    );

    let member = app.sign_in("alice").await;

    let refused = app
        .as_user(
            &member,
            Method::POST,
            "/api/users",
            Some(json!({ "username": "bob", "password": PASSWORD })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "forbidden");

    assert_eq!(
        app.as_user(&member, Method::DELETE, "/api/users/tim", None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.as_user(&owner, Method::DELETE, "/api/users/alice", None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}

/// Naming somebody in a page's `readers:` means knowing they exist, so the
/// listing is not a privilege. What it must not carry is a password hash.
#[tokio::test]
async fn any_account_may_list_the_others_and_none_of_them_carry_a_hash() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.as_user(
        &owner,
        Method::POST,
        "/api/users",
        Some(json!({ "username": "alice", "password": PASSWORD })),
    )
    .await;

    let member = app.sign_in("alice").await;
    let res = app.as_user(&member, Method::GET, "/api/users", None).await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["total"], 2);
    assert!(
        !res.body.to_string().contains("argon2"),
        "a password hash reached the wire: {:?}",
        res.body
    );
    assert_eq!(res.body["users"][0]["username"], "alice");
    assert_eq!(res.body["users"][0]["has_password"], true);
}

/// A wiki that requires authentication with nobody able to administer it is
/// recoverable only from the server's disk.
#[tokio::test]
async fn the_last_owner_can_be_neither_deleted_nor_demoted() {
    let app = App::new().await;
    let owner = app.owner().await;

    let deleted = app
        .as_user(&owner, Method::DELETE, "/api/users/tim", None)
        .await;
    assert_eq!(deleted.status, StatusCode::CONFLICT);
    assert_eq!(deleted.code(), "last_owner");

    let demoted = app
        .as_user(
            &owner,
            Method::PATCH,
            "/api/users/tim",
            Some(json!({ "role": "member" })),
        )
        .await;
    assert_eq!(demoted.status, StatusCode::CONFLICT);
    assert_eq!(demoted.code(), "last_owner");

    // With a second owner there is nothing to protect against.
    app.as_user(
        &owner,
        Method::POST,
        "/api/users",
        Some(json!({ "username": "alice", "password": PASSWORD, "role": "owner" })),
    )
    .await;
    assert_eq!(
        app.as_user(
            &owner,
            Method::PATCH,
            "/api/users/tim",
            Some(json!({ "role": "member" })),
        )
        .await
        .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_account_may_change_its_own_name_but_not_its_own_role() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.as_user(
        &owner,
        Method::POST,
        "/api/users",
        Some(json!({ "username": "alice", "password": PASSWORD })),
    )
    .await;
    let member = app.sign_in("alice").await;

    let renamed = app
        .as_user(
            &member,
            Method::PATCH,
            "/api/users/alice",
            Some(json!({ "display_name": "Alice A." })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(renamed.body["display_name"], "Alice A.");

    let promoted = app
        .as_user(
            &member,
            Method::PATCH,
            "/api/users/alice",
            Some(json!({ "role": "owner" })),
        )
        .await;
    assert_eq!(promoted.status, StatusCode::FORBIDDEN);

    let meddling = app
        .as_user(
            &member,
            Method::PATCH,
            "/api/users/tim",
            Some(json!({ "display_name": "not yours" })),
        )
        .await;
    assert_eq!(meddling.status, StatusCode::FORBIDDEN);
}

/// The difference between changing a password and revoking access: a token
/// handed out before the change has to stop working, or it goes on working for
/// its full thirty days.
#[tokio::test]
async fn changing_a_password_signs_that_account_out_everywhere() {
    let app = App::new().await;
    let owner = app.owner().await;
    let second_session = app.sign_in("tim").await;

    let changed = app
        .as_user(
            &owner,
            Method::PATCH,
            "/api/users/tim",
            Some(json!({ "password": "a completely different password" })),
        )
        .await;

    assert_eq!(changed.status, StatusCode::OK);
    assert_eq!(changed.body["sessions_ended"], 2);

    for token in [&owner, &second_session] {
        assert_eq!(
            app.as_user(token, Method::GET, "/api/pages", None)
                .await
                .status,
            StatusCode::UNAUTHORIZED
        );
    }

    // And the new password is the one that works.
    let res = app
        .anonymous(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "username": "tim", "password": "a completely different password" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn deleting_an_account_ends_its_sessions() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.as_user(
        &owner,
        Method::POST,
        "/api/users",
        Some(json!({ "username": "alice", "password": PASSWORD })),
    )
    .await;
    let member = app.sign_in("alice").await;

    assert_eq!(
        app.as_user(&member, Method::GET, "/api/pages", None)
            .await
            .status,
        StatusCode::OK
    );

    app.as_user(&owner, Method::DELETE, "/api/users/alice", None)
        .await;

    assert_eq!(
        app.as_user(&member, Method::GET, "/api/pages", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

/// A page owned by a deleted account keeps saying so, which is recoverable.
/// Deleting somebody's pages along with their account is not.
#[tokio::test]
async fn deleting_an_account_leaves_the_pages_alone() {
    let app = App::new().await;
    let owner = app.owner().await;
    app.as_user(
        &owner,
        Method::POST,
        "/api/pages",
        Some(json!({ "slug": "notes/rhizome", "content": "Branches off.\n" })),
    )
    .await;
    app.as_user(
        &owner,
        Method::POST,
        "/api/users",
        Some(json!({ "username": "alice", "password": PASSWORD, "role": "owner" })),
    )
    .await;

    let alice = app.sign_in("alice").await;
    app.as_user(&alice, Method::DELETE, "/api/users/tim", None)
        .await;

    let res = app
        .as_user(&alice, Method::GET, "/api/pages/notes/rhizome", None)
        .await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn an_account_cannot_be_created_twice() {
    let app = App::new().await;
    let owner = app.owner().await;

    let res = app
        .as_user(
            &owner,
            Method::POST,
            "/api/users",
            Some(json!({ "username": "tim", "password": PASSWORD })),
        )
        .await;

    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.code(), "user_already_exists");
    assert_eq!(res.body["error"]["details"]["username"], "tim");
}
