//! Accounts.
//!
//! Handlers are named `list_users`/`create_user`/… rather than `list`/`create`,
//! for the reason [`crate::api::pins`] records: utoipa takes an operation id
//! from the function name and those ids are global to the document, so short
//! names collide and a generated client silently drops one of each pair.
//!
//! ## Who may do what
//!
//! - **Listing and reading** is open to any signed-in account. Setting a page's
//!   `readers:` means naming other accounts, so knowing which ones exist is not
//!   a privilege — it is a prerequisite for using the feature at all. What comes
//!   back never includes a password hash.
//! - **Creating, deleting and changing a role** is for owners.
//! - **Changing a display name, a password or a profile** is for owners and for
//!   the account itself.
//!
//! ## Creating the first account is deliberately unauthenticated
//!
//! `POST /api/users` is in [the gate's public list](crate::auth), and it only
//! succeeds while the wiki has no accounts at all. That is safe rather than
//! merely convenient: a wiki with no accounts is already fully readable and
//! writable by anybody who can reach it, so claiming the first account grants
//! nothing that was not already on offer. The moment it exists the door closes
//! behind it.
//!
//! The first account is an **owner** whatever the request asked for. The
//! alternative is a wiki that requires authentication and has nobody able to add
//! an account to it, recoverable only by editing files on the server's disk —
//! which is the one thing somebody administering a remote instance cannot do.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::AppState;
use crate::api::extract::Json as RequestJson;
use crate::api::pages::present_or_absent;
use crate::auth::Viewer;
use crate::error::{AppError, AppResult};
use crate::users::{Role, User, UserFrontmatter, Username, password};

/// An account, as the API reports it.
///
/// There is no field for the password and there is no `?include=` that adds
/// one. A hash is not something a client ever needs, and the surest way to keep
/// it off the wire is for the type that goes on the wire not to have a place to
/// put it.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserView {
    pub username: Username,
    /// The name to show. Falls back to the username.
    #[schema(example = "Tim Yuen")]
    pub display_name: String,
    pub role: Role,
    /// Whether this account can be signed in to at all.
    ///
    /// False for a file somebody wrote by hand and has not finished. Such an
    /// account still counts towards "this wiki has accounts", so it is worth
    /// being able to see one.
    pub has_password: bool,
    /// Free markdown the account holder wrote about itself.
    pub profile: String,
    pub created: DateTime<Utc>,
    /// The account file's mtime.
    pub updated: DateTime<Utc>,
}

impl From<User> for UserView {
    fn from(user: User) -> Self {
        Self {
            display_name: user.display_name(),
            role: user.role(),
            has_password: user.has_password(),
            created: user.created(),
            updated: user.updated,
            profile: user.profile,
            username: user.username,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UsersResponse {
    /// Every account, by name.
    pub users: Vec<UserView>,
    /// How many there are. Zero means this wiki is open: no sign-in is asked
    /// for and nothing is refused.
    #[schema(example = 2)]
    pub total: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUser {
    #[schema(example = "tim")]
    pub username: Username,
    /// At least 8 characters. There are no composition rules.
    #[schema(example = "correct horse battery staple")]
    pub password: String,
    #[serde(default)]
    #[schema(example = "Tim Yuen")]
    pub display_name: Option<String>,
    /// Ignored for the first account on a wiki, which is always an owner.
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchUser {
    /// Omit to leave unchanged; send `null` to clear it and fall back to the
    /// username.
    #[serde(default, deserialize_with = "present_or_absent")]
    #[schema(value_type = Option<String>, example = "Tim Yuen")]
    pub display_name: Option<Option<String>>,

    /// Owners only. Demoting the last owner is refused with `last_owner`: it
    /// would leave a wiki that requires authentication with nobody able to add
    /// an account to it.
    #[serde(default)]
    pub role: Option<Role>,

    /// A new password, at least 8 characters.
    ///
    /// Setting it ends **every** session this account has, including the one
    /// that sent the request — so the caller has to sign in again.
    /// `sessions_ended` in the response says how many were ended.
    #[serde(default)]
    #[schema(example = "correct horse battery staple")]
    pub password: Option<String>,

    #[serde(default)]
    pub profile: Option<String>,
}

/// What a password change did.
#[derive(Debug, Serialize, ToSchema)]
pub struct PatchUserResponse {
    #[serde(flatten)]
    pub user: UserView,
    /// How many sessions this request ended. Non-zero only when the password
    /// changed, and it includes the session that made the request.
    #[schema(example = 2)]
    pub sessions_ended: usize,
}

/// Every account on this wiki.
///
/// Open to any signed-in account: naming somebody in a page's `readers:` list
/// means knowing they exist. An empty list means the wiki is open and asks
/// nobody to sign in.
#[utoipa::path(
    get,
    path = "/api/users",
    tag = "accounts",
    responses(
        (status = 200, description = "Every account", body = UsersResponse),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_users(
    State(state): State<AppState>,
    viewer: Viewer,
) -> AppResult<Json<UsersResponse>> {
    viewer.require_account()?;

    let users: Vec<UserView> = state
        .users
        .list()
        .await?
        .into_iter()
        .map(UserView::from)
        .collect();

    Ok(Json(UsersResponse {
        total: users.len(),
        users,
    }))
}

/// Create an account.
///
/// The first one on a wiki may be created by anybody and is always an owner;
/// see the module docs for why both halves of that are deliberate. Every one
/// after it needs an owner.
#[utoipa::path(
    post,
    path = "/api/users",
    tag = "accounts",
    request_body = CreateUser,
    responses(
        (status = 201, description = "The account was created", body = UserView),
        (status = 400, description = "The username or password was refused", body = crate::error::ErrorResponse),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
        (status = 403, description = "Only an owner may create accounts", body = crate::error::ErrorResponse),
        (status = 409, description = "That account already exists", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_user(
    State(state): State<AppState>,
    viewer: Viewer,
    RequestJson(request): RequestJson<CreateUser>,
) -> AppResult<(StatusCode, Json<UserView>)> {
    // Read before anything else is decided: it answers both "may this request
    // create an account at all" and "is this the first one", and the two must
    // agree with each other.
    let bootstrapping = state.users.is_empty().await?;

    if !bootstrapping {
        viewer.require_owner()?;
    }

    password::check(&request.password)?;

    let role = if bootstrapping {
        Role::Owner
    } else {
        request.role.unwrap_or_default()
    };

    let frontmatter = UserFrontmatter {
        display_name: request.display_name,
        role,
        password: Some(password::hash_in_background(request.password).await),
        created: Some(Utc::now()),
    };

    let user = state
        .users
        .create(
            &request.username,
            frontmatter,
            &request.profile.unwrap_or_default(),
        )
        .await?;

    if bootstrapping {
        tracing::info!(
            username = %user.username,
            "the first account was created; this wiki now requires authentication"
        );

        // Every idea record written while this wiki was open belongs to nobody
        // as of the line above, so it is handed to the account that just claimed
        // the wiki. Doing it here rather than only at startup is what keeps
        // somebody's inbox from disappearing between creating an account and
        // restarting; startup runs it too, because an account can also be
        // created by dropping a file in. See `crate::ideas::adoption`.
        crate::ideas::adoption::adopt_and_report(state.ideas.store(), &state.users, &state.index)
            .await;
    }

    Ok((StatusCode::CREATED, Json(user.into())))
}

/// One account.
#[utoipa::path(
    get,
    path = "/api/users/{username}",
    tag = "accounts",
    params(("username" = String, Path, description = "Account name", example = "tim")),
    responses(
        (status = 200, description = "The account", body = UserView),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
        (status = 404, description = "No such account", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_user(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<UserView>> {
    viewer.require_account()?;

    let username = parse_username(&raw)?;
    Ok(Json(state.users.read(&username).await?.into()))
}

/// Change an account.
///
/// Merges only the fields present, as every `PATCH` here does.
///
/// **A password change signs that account out everywhere**, including the
/// request that made the change — the response says how many sessions ended.
/// Anything less is not a password change: a token handed out before it would
/// go on working for its full thirty days, which is the difference between
/// changing a password and revoking access. Signing back in is the cost, and it
/// is the right way round for the case this exists for, which is a password
/// somebody thinks has leaked.
#[utoipa::path(
    patch,
    path = "/api/users/{username}",
    tag = "accounts",
    params(("username" = String, Path, description = "Account name", example = "tim")),
    request_body = PatchUser,
    responses(
        (status = 200, description = "The account as it now stands", body = PatchUserResponse),
        (status = 400, description = "The password was refused", body = crate::error::ErrorResponse),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
        (status = 403, description = "Not yours to change", body = crate::error::ErrorResponse),
        (status = 404, description = "No such account", body = crate::error::ErrorResponse),
        (status = 409, description = "This is the only owner", body = crate::error::ErrorResponse),
    ),
)]
pub async fn patch_user(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
    RequestJson(request): RequestJson<PatchUser>,
) -> AppResult<Json<PatchUserResponse>> {
    viewer.require_account()?;

    let username = parse_username(&raw)?;
    let is_self = viewer.username() == Some(&username);

    if !is_self && !viewer.may_administer_accounts() {
        return Err(AppError::Forbidden {
            action: "change another account",
        });
    }

    let existing = state.users.read(&username).await?;
    let mut frontmatter = existing.frontmatter.clone();

    if let Some(display_name) = request.display_name {
        frontmatter.display_name = display_name;
    }

    if let Some(role) = request.role {
        // Not a permission check so much as a lockout check. An owner demoting
        // themselves while they are the only one leaves a wiki that requires
        // authentication and has nobody able to administer it.
        if !viewer.may_administer_accounts() {
            return Err(AppError::Forbidden {
                action: "change a role",
            });
        }
        if role != Role::Owner && existing.is_owner() && only_owner(&state, &username).await? {
            return Err(AppError::LastOwner);
        }
        frontmatter.role = role;
    }

    let changing_password = request.password.is_some();
    if let Some(new_password) = request.password {
        password::check(&new_password)?;
        frontmatter.password = Some(password::hash_in_background(new_password).await);
    }

    let profile = request.profile.as_deref().unwrap_or(&existing.profile);
    let user = state.users.write(&username, frontmatter, profile).await?;

    let sessions_ended = if changing_password {
        let ended = state.index.delete_sessions_for(&username).await?;
        tracing::info!(%username, ended, "password changed; every session for this account ended");
        ended
    } else {
        0
    };

    Ok(Json(PatchUserResponse {
        user: user.into(),
        sessions_ended,
    }))
}

/// Delete an account, and every session it had.
///
/// Refused for the last owner: the result would be a wiki that requires
/// authentication with nobody able to add an account to it.
///
/// The account's pages are **not** touched. A page owned by a deleted account
/// keeps saying so, which is recoverable — recreate the account, or change the
/// page — where deleting somebody's pages along with their account is not.
#[utoipa::path(
    delete,
    path = "/api/users/{username}",
    tag = "accounts",
    params(("username" = String, Path, description = "Account name", example = "tim")),
    responses(
        (status = 204, description = "The account is gone"),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
        (status = 403, description = "Only an owner may delete accounts", body = crate::error::ErrorResponse),
        (status = 404, description = "No such account", body = crate::error::ErrorResponse),
        (status = 409, description = "This is the only owner", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_user(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<StatusCode> {
    viewer.require_owner()?;

    let username = parse_username(&raw)?;
    let existing = state.users.read(&username).await?;

    if existing.is_owner() && only_owner(&state, &username).await? {
        return Err(AppError::LastOwner);
    }

    state.users.delete(&username).await?;

    // After the file, not before: a revocation that ran and then failed to
    // delete would sign somebody out of an account they still have.
    let ended = state.index.delete_sessions_for(&username).await?;
    tracing::info!(%username, sessions_ended = ended, "account deleted");

    Ok(StatusCode::NO_CONTENT)
}

/// Whether `username` is the only account that can administer accounts.
async fn only_owner(state: &AppState, username: &Username) -> AppResult<bool> {
    let others = state
        .users
        .list()
        .await?
        .into_iter()
        .filter(|user| user.is_owner() && &user.username != username)
        .count();

    Ok(others == 0)
}

/// Parse a username from a URL path, reporting which rule it broke.
pub fn parse_username(raw: &str) -> AppResult<Username> {
    Username::parse(raw).map_err(|source| AppError::InvalidUsername {
        raw: raw.to_owned(),
        source,
    })
}
