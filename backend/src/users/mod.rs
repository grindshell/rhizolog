//! Accounts.
//!
//! An account is a file, for the same reason a page and a time entry are one:
//! it is authored data with no other copy, and a developer should be able to
//! read the list in an editor without a running server. The tree is
//! `<wiki>/.rhizolog/users/<username>.md`, and the shape is the one this
//! codebase already has twice — YAML frontmatter, then markdown:
//!
//! ```markdown
//! ---
//! display_name: Tim Yuen
//! role: owner
//! password: $argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$...
//! created: 2026-08-19T10:00:00Z
//! ---
//!
//! Anything worth saying about this account.
//! ```
//!
//! The username is **not** in the file. It is the filename, exactly as a page's
//! slug is its path — one name in one place, so renaming cannot leave the two
//! disagreeing.
//!
//! ## These files are secrets, and the wiki is probably a git repository
//!
//! `password` is an Argon2id PHC string, which is designed to be stored and is
//! not reversible. It is still a hash of a real password and it does not belong
//! in a commit, so `.rhizolog/users/` is gitignored by name — the same
//! treatment `index.db` and `server.json` get, and for the same reason the
//! directory as a whole is not ignored: `times/` beside it is authored data
//! that *should* be committed. See `knowledge-base/accounts.md`.
//!
//! Anyone who can read the wiki directory can read every page in it whatever
//! its visibility says, and can replace a password hash with one they know.
//! Accounts are a boundary between *network* callers, not a boundary against
//! whoever holds the disk.

pub mod password;
pub mod store;

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use thiserror::Error;
use utoipa::ToSchema;

use crate::frontmatter::{self, FrontmatterError};
use crate::slug::RESERVED_STEMS;

pub use store::{UserStore, UserStoreError};

/// Directory holding accounts, under `.rhizolog/`.
pub const USERS_DIR: &str = "users";

/// Maximum length of a username, in bytes.
///
/// Short on purpose. It is a filename, it appears in `readers:` lists that
/// people type by hand, and nothing is improved by allowing a paragraph.
pub const MAX_USERNAME_LEN: usize = 39;

/// A validated account name.
///
/// The only way to construct one is [`Username::parse`], so holding one is
/// proof that it is safe to use as a filename.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
#[schema(
    value_type = String,
    example = "tim",
    description = "An account name, which is also the name of the file the account \
                   lives in.\n\n\
                   Lowercase ASCII letters, digits, `-` and `_`, starting with a \
                   letter or a digit, at most 39 characters. Uppercase is refused \
                   rather than folded: Windows filenames are case-insensitive, so \
                   `Tim` and `tim` would be one account on one machine and two on \
                   another.\n\n\
                   A rejected name comes back as a `400` whose `details.rule` names \
                   which rule was broken."
)]
pub struct Username(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UsernameError {
    #[error("username is empty")]
    Empty,

    #[error("username is longer than {MAX_USERNAME_LEN} bytes")]
    TooLong,

    #[error("username must not contain {character:?}: use lowercase letters, digits, '-' and '_'")]
    ForbiddenCharacter { character: char },

    #[error("username must not contain uppercase letters")]
    Uppercase,

    #[error("username must start with a lowercase letter or a digit")]
    BadStart,

    #[error("{name:?} is a reserved device name on Windows")]
    ReservedName { name: String },
}

impl UsernameError {
    /// A stable, machine-readable identifier for this rule, for the `details` of
    /// an error response. Same contract as [`crate::slug::SlugError::code`]: a
    /// caller that built a bad name should be able to fix it from the response.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "username_empty",
            Self::TooLong => "username_too_long",
            Self::ForbiddenCharacter { .. } => "username_forbidden_character",
            Self::Uppercase => "username_uppercase",
            Self::BadStart => "username_bad_start",
            Self::ReservedName { .. } => "username_reserved_name",
        }
    }
}

impl Username {
    /// Validate `raw` as a username.
    pub fn parse(raw: &str) -> Result<Self, UsernameError> {
        if raw.is_empty() {
            return Err(UsernameError::Empty);
        }
        if raw.len() > MAX_USERNAME_LEN {
            return Err(UsernameError::TooLong);
        }

        for character in raw.chars() {
            if character.is_ascii_uppercase() {
                return Err(UsernameError::Uppercase);
            }
            if !(character.is_ascii_lowercase() || character.is_ascii_digit())
                && character != '-'
                && character != '_'
            {
                return Err(UsernameError::ForbiddenCharacter { character });
            }
        }

        let first = raw.chars().next().expect("non-empty");
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(UsernameError::BadStart);
        }

        // The character set above already excludes `.`, so there is no extension
        // to strip: the whole name is the stem. `con` is still the console.
        if RESERVED_STEMS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(raw))
        {
            return Err(UsernameError::ReservedName {
                name: raw.to_owned(),
            });
        }

        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The file this account lives in, under the accounts directory.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.md", self.0))
    }

    /// The account a filename names, or `None` if it is not one of ours.
    ///
    /// Used by the walker, which sees whatever is in the directory. A file
    /// somebody dropped in there by hand is skipped rather than half-read.
    pub fn from_file_name(name: &str) -> Option<Self> {
        Self::parse(name.strip_suffix(".md")?).ok()
    }
}

impl fmt::Display for Username {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for Username {
    type Err = UsernameError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

impl Serialize for Username {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// So a username can be **bound** into a query rather than interpolated.
///
/// The character set makes interpolation provably safe today, which is exactly
/// the argument that stops being true the first time somebody relaxes
/// [`Username::parse`]. Binding costs nothing and does not depend on that.
impl rusqlite::ToSql for Username {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::Borrowed(
            rusqlite::types::ValueRef::Text(self.0.as_bytes()),
        ))
    }
}

/// Validated on the way in, so a username in a request body is refused by the
/// same rules as one in a URL. See [`crate::error::AppError::InvalidRequestBody`]
/// for why that is a 400 either way.
impl<'de> Deserialize<'de> for Username {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(D::Error::custom)
    }
}

/// What an account is allowed to do to *the instance*.
///
/// Deliberately two values. This is not a permission system: who may read a
/// given page is decided by that page, and the only instance-wide question is
/// whether somebody may administer accounts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// May create, edit and delete accounts, and is the role the first account
    /// gets. Not a licence to read other people's private pages — see
    /// `knowledge-base/accounts.md`.
    Owner,
    /// Everything else.
    #[default]
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }

    pub fn is_owner(self) -> bool {
        matches!(self, Self::Owner)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The frontmatter block of an account file, exactly as it appears on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFrontmatter {
    /// A human's name for this account. Falls back to the username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    #[serde(default)]
    pub role: Role,

    /// An Argon2id PHC string.
    ///
    /// Optional, because a file written by hand may not have one yet and
    /// refusing to parse it would turn a typo into an account that cannot be
    /// listed or repaired. An account with no password cannot be signed in to;
    /// [`crate::api::auth::login`] says so rather than reporting bad
    /// credentials, so the state is diagnosable rather than merely broken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,

    /// Set once, when the account is created.
    ///
    /// Reads a bare `2026-08-19` too, as a page's does. An account file is
    /// hand-edited more often than a page is — it is where a display name or a
    /// role gets changed — and a frontmatter block that will not parse takes the
    /// password hash with it, which is a locked-out account rather than a
    /// missing date.
    #[serde(
        default,
        deserialize_with = "frontmatter::timestamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub created: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum UserError {
    /// Wrapped rather than restated, for the reason [`crate::page::PageError`]'s
    /// is: one message, defined once, wherever a frontmatter block is read.
    #[error(transparent)]
    Frontmatter(#[from] FrontmatterError),
}

/// An account, as parsed from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub username: Username,
    pub frontmatter: UserFrontmatter,
    /// Free markdown: whatever the account holder wants to say about itself.
    pub profile: String,
    /// The file's mtime. Not part of its contents.
    pub updated: DateTime<Utc>,
}

impl User {
    /// Parse the text of an account file.
    pub fn from_markdown(
        username: Username,
        text: &str,
        updated: DateTime<Utc>,
    ) -> Result<Self, UserError> {
        // Same BOM hazard as pages and time entries, and the same answer: a file
        // saved by Notepad starts with three invisible bytes, and without this
        // its whole frontmatter — the password included — reads as body.
        let text = frontmatter::strip_bom(text);

        let (frontmatter, profile) = match frontmatter::split(text)? {
            Some((yaml, body)) => (frontmatter::parse(yaml)?, body),
            None => (UserFrontmatter::default(), text),
        };

        Ok(Self {
            username,
            frontmatter,
            profile: profile.to_owned(),
            updated,
        })
    }

    /// Render the account back to the text that belongs in its file.
    pub fn to_markdown(&self) -> String {
        let yaml = serde_yaml_ng::to_string(&self.frontmatter)
            .expect("account frontmatter is a plain struct of strings and timestamps");

        frontmatter::compose(&yaml, &self.profile)
    }

    /// The name to show, falling back to the username.
    pub fn display_name(&self) -> String {
        self.frontmatter
            .display_name
            .as_ref()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| self.username.to_string())
    }

    pub fn role(&self) -> Role {
        self.frontmatter.role
    }

    pub fn is_owner(&self) -> bool {
        self.frontmatter.role.is_owner()
    }

    /// When the account was created, defaulting to its mtime for a file written
    /// by hand that never carried the field.
    pub fn created(&self) -> DateTime<Utc> {
        self.frontmatter.created.unwrap_or(self.updated)
    }

    /// Whether `candidate` is this account's password.
    ///
    /// `false` for an account with no password at all, which is a file somebody
    /// wrote by hand and has not finished.
    pub fn verify_password(&self, candidate: &str) -> bool {
        match &self.frontmatter.password {
            Some(hash) => password::verify(candidate, hash),
            None => false,
        }
    }

    pub fn has_password(&self) -> bool {
        self.frontmatter.password.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn user(text: &str) -> User {
        User::from_markdown(
            Username::parse("tim").unwrap(),
            text,
            at("2026-08-19T12:00:00Z"),
        )
        .expect("account should parse")
    }

    /// An account file gets hand-edited — it is where a display name or a role
    /// is changed — and a frontmatter block that will not parse takes the
    /// password hash down with it. A date written the ordinary way must not be
    /// the thing that locks somebody out.
    #[test]
    fn a_bare_date_does_not_cost_an_account_its_password() {
        let parsed = user(
            "---\ndisplay_name: Tim Yuen\nrole: owner\npassword: $argon2id$abc\ncreated: 2026-08-19\n---\n\nHello.\n",
        );

        assert_eq!(parsed.created(), at("2026-08-19T00:00:00Z"));
        assert!(parsed.has_password());
        assert!(parsed.is_owner());
    }

    #[test]
    fn accepts_ordinary_names() {
        for raw in ["tim", "a", "tim-yuen", "user_2", "0", "agent-7"] {
            assert!(Username::parse(raw).is_ok(), "{raw} should be a username");
        }
    }

    /// Every one of these would otherwise become a filename.
    #[test]
    fn rejects_names_that_are_not_safe_as_filenames() {
        for (raw, expected) in [
            ("", "username_empty"),
            ("..", "username_forbidden_character"),
            ("a/b", "username_forbidden_character"),
            ("a\\b", "username_forbidden_character"),
            ("a:b", "username_forbidden_character"),
            ("a.b", "username_forbidden_character"),
            ("a b", "username_forbidden_character"),
            ("a\0b", "username_forbidden_character"),
            ("-tim", "username_bad_start"),
            ("_tim", "username_bad_start"),
            ("con", "username_reserved_name"),
            ("nul", "username_reserved_name"),
            ("lpt1", "username_reserved_name"),
        ] {
            let error = Username::parse(raw).expect_err("{raw} should be refused");
            assert_eq!(error.code(), expected, "for {raw:?}");
        }

        assert_eq!(
            Username::parse(&"a".repeat(MAX_USERNAME_LEN + 1))
                .unwrap_err()
                .code(),
            "username_too_long"
        );
    }

    /// Windows filenames are case-insensitive, so folding `Tim` to `tim` would
    /// make two accounts on Linux and one here. Refusing is the only answer that
    /// means the same thing on both.
    #[test]
    fn rejects_uppercase_rather_than_folding_it() {
        assert_eq!(
            Username::parse("Tim").unwrap_err().code(),
            "username_uppercase"
        );
        assert_eq!(
            Username::parse("CON").unwrap_err().code(),
            "username_uppercase"
        );
    }

    #[test]
    fn parses_frontmatter_and_profile() {
        let parsed = user(
            "---\ndisplay_name: Tim Yuen\nrole: owner\npassword: $argon2id$abc\ncreated: 2026-08-19T10:00:00Z\n---\n\nHello.\n",
        );

        assert_eq!(parsed.display_name(), "Tim Yuen");
        assert_eq!(parsed.role(), Role::Owner);
        assert!(parsed.has_password());
        assert_eq!(parsed.created(), at("2026-08-19T10:00:00Z"));
        assert_eq!(parsed.profile, "\nHello.\n");
    }

    #[test]
    fn a_missing_role_is_a_member_and_a_missing_name_is_the_username() {
        let parsed = user("---\npassword: $argon2id$abc\n---\n");

        assert_eq!(parsed.role(), Role::Member);
        assert!(!parsed.is_owner());
        assert_eq!(parsed.display_name(), "tim");
    }

    /// The same three invisible bytes that once hid a page's frontmatter. Here
    /// they would hide the password and the role, leaving an account that looks
    /// like an ordinary member with no credentials.
    #[test]
    fn a_utf8_bom_does_not_hide_the_frontmatter() {
        let parsed = user("\u{feff}---\nrole: owner\npassword: $argon2id$abc\n---\n\nHi.\n");

        assert_eq!(parsed.role(), Role::Owner);
        assert!(parsed.has_password());
    }

    #[test]
    fn round_trips_through_the_file() {
        let original = user(
            "---\ndisplay_name: Tim Yuen\nrole: owner\npassword: $argon2id$abc\ncreated: 2026-08-19T10:00:00Z\n---\n\nHello.\n",
        );
        let reparsed = user(&original.to_markdown());

        assert_eq!(original.frontmatter, reparsed.frontmatter);
        assert_eq!(original.profile, reparsed.profile);
    }

    /// An account nobody has set a password on cannot be signed in to, and
    /// saying so is different from saying the password was wrong.
    #[test]
    fn an_account_with_no_password_verifies_nothing() {
        let parsed = user("---\nrole: member\n---\n");

        assert!(!parsed.has_password());
        assert!(!parsed.verify_password(""));
        assert!(!parsed.verify_password("anything"));
    }

    #[test]
    fn a_file_name_names_an_account_and_other_files_do_not() {
        assert_eq!(
            Username::from_file_name("tim.md"),
            Some(Username::parse("tim").unwrap())
        );
        assert_eq!(Username::from_file_name("tim.txt"), None);
        assert_eq!(Username::from_file_name("tim"), None);
        assert_eq!(Username::from_file_name("Tim.md"), None);
        assert_eq!(Username::from_file_name(".tim.md"), None);
    }
}
