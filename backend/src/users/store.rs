//! The accounts directory.
//!
//! `<wiki>/.rhizolog/users/<username>.md`, flat — accounts have no hierarchy and
//! there are never many, so this needs none of the month-bucketing the time log
//! does. Writes go through the same temporary-file-and-rename dance as pages, so
//! nothing ever reads half an account.
//!
//! ## There is no index over this, on purpose
//!
//! Pages and time entries are mirrored into SQLite because they are searched,
//! filtered, sorted and counted in ways a directory scan cannot answer. Accounts
//! are none of those things: there are a handful, they are listed whole, and the
//! only hot question is "how many are there", which is a `read_dir` over a
//! directory with five entries in it.
//!
//! Keeping them out of the index buys the property that matters more —
//! **adding an account by hand works immediately**. Drop a file in, and the next
//! request sees it. No reindex, no restart, and no way for a derived copy of a
//! credential to drift out of step with the file that holds it.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::store::{INTERNAL_DIR, display_path, modified_at, write_atomically};
use crate::users::{USERS_DIR, User, UserError, UserFrontmatter, Username};

#[derive(Debug, Error)]
pub enum UserStoreError {
    #[error("no account named {username}")]
    NotFound { username: Username },

    #[error("an account named {username} already exists")]
    AlreadyExists { username: Username },

    #[error("{username} resolves outside the accounts directory")]
    EscapesRoot { username: Username },

    #[error("{username} is not valid UTF-8")]
    NotUtf8 { username: Username },

    #[error("could not parse the account {username}: {source}")]
    Malformed {
        username: Username,
        #[source]
        source: UserError,
    },

    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A directory of accounts.
#[derive(Debug, Clone)]
pub struct UserStore {
    root: PathBuf,
}

impl UserStore {
    /// Open (creating if necessary) the accounts directory for the wiki at
    /// `wiki_root`.
    pub async fn open(wiki_root: impl AsRef<Path>) -> Result<Self, UserStoreError> {
        let root = wiki_root.as_ref().join(INTERNAL_DIR).join(USERS_DIR);
        tokio::fs::create_dir_all(&root).await?;
        // Canonicalised so the containment check in `resolve` compares like with
        // like — on Windows that means both sides carry the `\\?\` prefix.
        let root = tokio::fs::canonicalize(&root).await?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn root_display(&self) -> String {
        display_path(&self.root)
    }

    /// The path a username names, having confirmed it does not leave the
    /// directory.
    ///
    /// A username cannot traverse on its own — [`Username::parse`] allows only
    /// lowercase alphanumerics, `-` and `_` — but a symlinked account file could
    /// point anywhere, and reading it would be an arbitrary file read.
    async fn resolve(&self, username: &Username) -> Result<PathBuf, UserStoreError> {
        let path = username.to_path(&self.root);

        match tokio::fs::canonicalize(&path).await {
            Ok(canonical) if !canonical.starts_with(&self.root) => {
                Err(UserStoreError::EscapesRoot {
                    username: username.clone(),
                })
            }
            Ok(_) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn exists(&self, username: &Username) -> Result<bool, UserStoreError> {
        let path = self.resolve(username).await?;
        Ok(tokio::fs::try_exists(&path).await?)
    }

    pub async fn read(&self, username: &Username) -> Result<User, UserStoreError> {
        let path = self.resolve(username).await?;

        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(error) => {
                return Err(match error.kind() {
                    io::ErrorKind::NotFound => UserStoreError::NotFound {
                        username: username.clone(),
                    },
                    io::ErrorKind::InvalidData => UserStoreError::NotUtf8 {
                        username: username.clone(),
                    },
                    _ => error.into(),
                });
            }
        };

        let updated = modified_at(&tokio::fs::metadata(&path).await?)?;

        User::from_markdown(username.clone(), &text, updated).map_err(|source| {
            UserStoreError::Malformed {
                username: username.clone(),
                source,
            }
        })
    }

    /// The account, or `None` if there is no such file.
    ///
    /// What a login wants: a username nobody has is not an error there, it is
    /// one of the two ordinary answers.
    pub async fn find(&self, username: &Username) -> Result<Option<User>, UserStoreError> {
        match self.read(username).await {
            Ok(user) => Ok(Some(user)),
            Err(UserStoreError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Write an account, replacing any existing one.
    pub async fn write(
        &self,
        username: &Username,
        frontmatter: UserFrontmatter,
        profile: &str,
    ) -> Result<User, UserStoreError> {
        let path = self.resolve(username).await?;

        let user = User {
            username: username.clone(),
            frontmatter,
            profile: profile.to_owned(),
            // Replaced below with what the filesystem actually recorded.
            updated: Utc::now(),
        };

        write_atomically(&path, user.to_markdown().as_bytes()).await?;

        Ok(User {
            updated: modified_at(&tokio::fs::metadata(&path).await?)?,
            ..user
        })
    }

    /// Write an account that must not already exist.
    pub async fn create(
        &self,
        username: &Username,
        frontmatter: UserFrontmatter,
        profile: &str,
    ) -> Result<User, UserStoreError> {
        if self.exists(username).await? {
            return Err(UserStoreError::AlreadyExists {
                username: username.clone(),
            });
        }
        self.write(username, frontmatter, profile).await
    }

    pub async fn delete(&self, username: &Username) -> Result<(), UserStoreError> {
        let path = self.resolve(username).await?;

        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(UserStoreError::NotFound {
                    username: username.clone(),
                })
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Every account name in the directory, sorted.
    ///
    /// Names only, and no file is opened: this is what [`UserStore::count`] and
    /// the authentication gate are built on, and it runs on every request.
    pub async fn names(&self) -> Result<Vec<Username>, UserStoreError> {
        let mut directory = match tokio::fs::read_dir(&self.root).await {
            Ok(directory) => directory,
            // The directory is created by `open`, so this means somebody removed
            // it underneath us. No directory is no accounts, which is a state the
            // wiki is allowed to be in rather than a failure.
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        let mut names = Vec::new();
        while let Some(entry) = directory.next_entry().await? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            // Anything else in the directory — a stray note, a leftover `.tmp`
            // from an interrupted write — is not an account and is skipped
            // rather than half-read.
            let Some(username) = Username::from_file_name(name) else {
                continue;
            };
            names.push(username);
        }

        names.sort();
        Ok(names)
    }

    /// Every account, read and parsed, sorted by name.
    ///
    /// An account whose file will not parse is **skipped with a warning** rather
    /// than failing the listing. One corrupt file should not make the account
    /// list unreadable, which is the state somebody would be in when they most
    /// need it.
    pub async fn list(&self) -> Result<Vec<User>, UserStoreError> {
        let mut users = Vec::new();

        for username in self.names().await? {
            match self.read(&username).await {
                Ok(user) => users.push(user),
                Err(error) => {
                    tracing::warn!(%username, %error, "skipping an account that could not be read");
                }
            }
        }

        Ok(users)
    }

    /// How many accounts there are.
    ///
    /// **This is the authentication switch.** Zero accounts is an open wiki that
    /// behaves exactly as it did before accounts existed; one or more means every
    /// request has to say who it is. It is answered from the directory rather
    /// than from anything cached, so creating an account — through the API or by
    /// dropping a file in — takes effect on the very next request, and no stale
    /// copy of the answer can leave an instance open that should not be.
    pub async fn count(&self) -> Result<usize, UserStoreError> {
        Ok(self.names().await?.len())
    }

    /// Whether this wiki has any accounts at all.
    pub async fn is_empty(&self) -> Result<bool, UserStoreError> {
        Ok(self.count().await? == 0)
    }

    /// The mtime of the newest account file, for cache validation.
    ///
    /// `None` when there are no accounts.
    pub async fn newest(&self) -> Result<Option<DateTime<Utc>>, UserStoreError> {
        let mut newest = None;

        for username in self.names().await? {
            let path = username.to_path(&self.root);
            let Ok(metadata) = tokio::fs::metadata(&path).await else {
                continue;
            };
            let Ok(updated) = modified_at(&metadata) else {
                continue;
            };
            newest = Some(newest.map_or(updated, |current: DateTime<Utc>| current.max(updated)));
        }

        Ok(newest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::users::Role;

    async fn store() -> (TempDir, UserStore) {
        let directory = TempDir::new().expect("temp dir");
        let store = UserStore::open(directory.path()).await.expect("open");
        (directory, store)
    }

    fn name(raw: &str) -> Username {
        Username::parse(raw).expect("valid username")
    }

    fn frontmatter(role: Role) -> UserFrontmatter {
        UserFrontmatter {
            role,
            password: Some(crate::users::password::hash("correct horse battery")),
            created: Some(Utc::now()),
            ..UserFrontmatter::default()
        }
    }

    #[tokio::test]
    async fn writes_and_reads_an_account() {
        let (_directory, store) = store().await;

        store
            .write(&name("tim"), frontmatter(Role::Owner), "Hello.\n")
            .await
            .expect("write");

        let read = store.read(&name("tim")).await.expect("read");
        assert_eq!(read.role(), Role::Owner);
        assert_eq!(read.profile, "Hello.\n");
        assert!(read.verify_password("correct horse battery"));
    }

    /// The switch the whole feature turns on. A wiki with no accounts is open,
    /// exactly as it was before accounts existed.
    #[tokio::test]
    async fn a_fresh_wiki_has_no_accounts() {
        let (_directory, store) = store().await;

        assert_eq!(store.count().await.expect("count"), 0);
        assert!(store.is_empty().await.expect("is_empty"));
    }

    /// Files are the truth here as much as anywhere else: an account written by
    /// hand is an account, with nothing to reindex and no server to restart.
    #[tokio::test]
    async fn an_account_dropped_in_by_hand_is_found_immediately() {
        let (directory, store) = store().await;
        let path = directory
            .path()
            .join(INTERNAL_DIR)
            .join(USERS_DIR)
            .join("alice.md");

        tokio::fs::write(&path, "---\nrole: member\n---\n")
            .await
            .expect("write by hand");

        assert_eq!(store.count().await.expect("count"), 1);
        assert_eq!(store.names().await.expect("names"), [name("alice")]);
    }

    #[tokio::test]
    async fn create_refuses_to_clobber() {
        let (_directory, store) = store().await;

        store
            .create(&name("tim"), frontmatter(Role::Owner), "First.\n")
            .await
            .expect("first create");
        let error = store
            .create(&name("tim"), frontmatter(Role::Member), "Second.\n")
            .await
            .unwrap_err();

        assert!(matches!(error, UserStoreError::AlreadyExists { .. }));
        // The refused create left the original account alone, role included.
        let read = store.read(&name("tim")).await.expect("read");
        assert_eq!(read.role(), Role::Owner);
        assert_eq!(read.profile, "First.\n");
    }

    #[tokio::test]
    async fn reading_a_missing_account_reports_not_found_and_finding_one_does_not() {
        let (_directory, store) = store().await;

        assert!(matches!(
            store.read(&name("nobody")).await.unwrap_err(),
            UserStoreError::NotFound { .. }
        ));
        assert_eq!(store.find(&name("nobody")).await.expect("find"), None);
    }

    #[tokio::test]
    async fn deletes_an_account() {
        let (_directory, store) = store().await;

        store
            .create(&name("tim"), frontmatter(Role::Owner), "")
            .await
            .expect("create");
        store.delete(&name("tim")).await.expect("delete");

        assert!(store.is_empty().await.expect("is_empty"));
        assert!(matches!(
            store.delete(&name("tim")).await.unwrap_err(),
            UserStoreError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn listing_is_sorted_and_skips_what_is_not_an_account() {
        let (directory, store) = store().await;
        let users = directory.path().join(INTERNAL_DIR).join(USERS_DIR);

        for raw in ["tim", "alice", "bob"] {
            store
                .create(&name(raw), frontmatter(Role::Member), "")
                .await
                .expect("create");
        }
        tokio::fs::write(users.join("notes.txt"), "not an account")
            .await
            .expect("stray file");
        tokio::fs::write(users.join(".alice.md.tmp"), "interrupted write")
            .await
            .expect("stray temporary");

        let listed: Vec<String> = store
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|user| user.username.to_string())
            .collect();

        assert_eq!(listed, ["alice", "bob", "tim"]);
    }

    /// One unreadable file must not take the whole account list with it. That is
    /// exactly the moment somebody needs to see the other accounts.
    #[tokio::test]
    async fn a_corrupt_account_is_skipped_rather_than_failing_the_listing() {
        let (directory, store) = store().await;
        let users = directory.path().join(INTERNAL_DIR).join(USERS_DIR);

        store
            .create(&name("tim"), frontmatter(Role::Owner), "")
            .await
            .expect("create");
        tokio::fs::write(
            users.join("broken.md"),
            "---\nrole: never closed\n\nBody.\n",
        )
        .await
        .expect("write broken account");

        let listed = store.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].username, name("tim"));

        // It is still counted, though: the file is there, so the wiki is not
        // open, and pretending otherwise would unlock it.
        assert_eq!(store.count().await.expect("count"), 2);
    }

    /// The temporary file a write goes through must never be mistaken for an
    /// account — it holds a password hash and a leading dot is all that hides it.
    #[tokio::test]
    async fn an_atomic_write_leaves_no_visible_temporary() {
        let (directory, store) = store().await;
        store
            .write(&name("tim"), frontmatter(Role::Owner), "")
            .await
            .expect("write");

        let mut names: Vec<String> =
            std::fs::read_dir(directory.path().join(INTERNAL_DIR).join(USERS_DIR))
                .expect("read dir")
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
        names.sort();

        assert_eq!(names, ["tim.md"]);
    }

    #[tokio::test]
    async fn an_account_round_trips_through_its_file() {
        let (_directory, store) = store().await;
        let written = store
            .write(
                &name("tim"),
                UserFrontmatter {
                    display_name: Some("Tim Yuen".to_owned()),
                    ..frontmatter(Role::Owner)
                },
                "About me.\n",
            )
            .await
            .expect("write");

        let read = store.read(&name("tim")).await.expect("read");

        assert_eq!(read.frontmatter, written.frontmatter);
        assert_eq!(read.profile, "About me.\n");
        assert_eq!(read.display_name(), "Tim Yuen");
    }
}
