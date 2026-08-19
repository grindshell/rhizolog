//! Sessions: the rows that say a request has already proved who it is.
//!
//! A session is stored as the **SHA-256 of its token**, never the token itself.
//! The index is a file on disk, and a row that could be lifted out and replayed
//! as a credential is a row worth not writing. The token exists in exactly two
//! places — the client that was handed it, and memory for the microseconds it
//! takes to hash an arriving one.
//!
//! Argon2 would be the wrong tool here, which is worth saying out loud because
//! the neighbouring [password](crate::users::password) module uses it. A
//! password is short, chosen by a human, and guessable, so verifying one is
//! deliberately slow. A session token is 256 bits from the OS's random source
//! with no preimage to guess, so there is nothing for a slow hash to buy — and
//! it would be paid on every single request rather than once per login.
//!
//! ## Sessions live in the durable half of the schema
//!
//! Not because they are precious — losing them logs everyone out, which is
//! survivable and is exactly what deleting `index.db` should do — but because a
//! [schema version bump](super::schema) is a routine consequence of changing how
//! pages are indexed, and that has nothing to do with who is signed in. See
//! `knowledge-base/accounts.md`.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

use crate::index::{Index, IndexError, from_nanos, to_nanos};
use crate::users::Username;

/// A session as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSession {
    pub username: Username,
    pub created: DateTime<Utc>,
    pub expires: DateTime<Utc>,
}

impl Index {
    /// Record a new session.
    pub async fn create_session(
        &self,
        token_hash: &str,
        username: &Username,
        created: DateTime<Utc>,
        expires: DateTime<Utc>,
    ) -> Result<(), IndexError> {
        let token_hash = token_hash.to_owned();
        let username = username.to_string();
        let created = to_nanos(created, "session created")?;
        let expires = to_nanos(expires, "session expiry")?;

        self.with_connection(move |connection| {
            connection.execute(
                "insert into sessions (token_hash, username, created, expires)
                 values (?1, ?2, ?3, ?4)
                 on conflict(token_hash) do update set
                     username = excluded.username,
                     created  = excluded.created,
                     expires  = excluded.expires",
                params![token_hash, username, created, expires],
            )?;
            Ok(())
        })
        .await
    }

    /// The session a token hash names, if it is still valid.
    ///
    /// An expired row comes back as `None` **and is deleted on the way**, so an
    /// instance that is used at all keeps its own session table trimmed without
    /// anything having to sweep it. The startup purge covers the rest.
    pub async fn session(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<StoredSession>, IndexError> {
        let token_hash = token_hash.to_owned();
        let now = to_nanos(now, "now")?;

        self.with_connection(move |connection| {
            let found = connection
                .query_row(
                    "select username, created, expires from sessions where token_hash = ?1",
                    params![token_hash],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()?;

            let Some((username, created, expires)) = found else {
                return Ok(None);
            };

            if expires <= now {
                connection.execute(
                    "delete from sessions where token_hash = ?1",
                    params![token_hash],
                )?;
                return Ok(None);
            }

            // A row naming an account whose name no longer parses cannot
            // identify anybody, and treating it as a valid session would be a
            // request authenticated as nothing in particular.
            let Ok(username) = Username::parse(&username) else {
                connection.execute(
                    "delete from sessions where token_hash = ?1",
                    params![token_hash],
                )?;
                return Ok(None);
            };

            Ok(Some(StoredSession {
                username,
                created: from_nanos(created),
                expires: from_nanos(expires),
            }))
        })
        .await
    }

    /// Push a session's expiry out.
    pub async fn renew_session(
        &self,
        token_hash: &str,
        expires: DateTime<Utc>,
    ) -> Result<(), IndexError> {
        let token_hash = token_hash.to_owned();
        let expires = to_nanos(expires, "session expiry")?;

        self.with_connection(move |connection| {
            connection.execute(
                "update sessions set expires = ?2 where token_hash = ?1",
                params![token_hash, expires],
            )?;
            Ok(())
        })
        .await
    }

    /// End one session. Idempotent: signing out twice is signing out.
    pub async fn delete_session(&self, token_hash: &str) -> Result<(), IndexError> {
        let token_hash = token_hash.to_owned();

        self.with_connection(move |connection| {
            connection.execute(
                "delete from sessions where token_hash = ?1",
                params![token_hash],
            )?;
            Ok(())
        })
        .await
    }

    /// End every session belonging to an account, and say how many there were.
    ///
    /// This is what makes a password change and a deleted account mean
    /// something. Without it, a token handed out before either one goes on
    /// working for its full lifetime — which is the difference between changing
    /// a password and revoking access.
    pub async fn delete_sessions_for(&self, username: &Username) -> Result<usize, IndexError> {
        let username = username.to_string();

        self.with_connection(move |connection| {
            let removed = connection.execute(
                "delete from sessions where username = ?1",
                params![username],
            )?;
            Ok(removed)
        })
        .await
    }

    /// Drop every session that has already expired.
    pub async fn purge_expired_sessions(&self, now: DateTime<Utc>) -> Result<usize, IndexError> {
        let now = to_nanos(now, "now")?;

        self.with_connection(move |connection| {
            let removed =
                connection.execute("delete from sessions where expires <= ?1", params![now])?;
            Ok(removed)
        })
        .await
    }

    /// How many sessions are on record, expired ones included. For tests and
    /// for `/api/auth/sessions`.
    pub async fn count_sessions(&self) -> Result<usize, IndexError> {
        self.with_connection(|connection| {
            let count: i64 =
                connection.query_row("select count(*) from sessions", [], |row| row.get(0))?;
            Ok(count as usize)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Duration;

    async fn index() -> Index {
        Index::open(None).await.expect("open in-memory index")
    }

    fn name(raw: &str) -> Username {
        Username::parse(raw).expect("valid username")
    }

    #[tokio::test]
    async fn a_session_round_trips_and_names_its_account() {
        let index = index().await;
        let now = Utc::now();

        index
            .create_session("hash", &name("tim"), now, now + Duration::days(30))
            .await
            .expect("create");

        let found = index.session("hash", now).await.expect("read");
        assert_eq!(found.map(|session| session.username), Some(name("tim")));
    }

    #[tokio::test]
    async fn an_unknown_token_is_nobody() {
        let index = index().await;
        assert_eq!(index.session("nope", Utc::now()).await.expect("read"), None);
    }

    /// The expiry has to be enforced on read, not merely recorded. A session
    /// that is only cleaned up by a sweep is a session that works until the
    /// sweep runs.
    #[tokio::test]
    async fn an_expired_session_is_refused_and_removed_as_it_is_read() {
        let index = index().await;
        let now = Utc::now();

        index
            .create_session(
                "hash",
                &name("tim"),
                now - Duration::days(31),
                now - Duration::hours(1),
            )
            .await
            .expect("create");

        assert_eq!(index.session("hash", now).await.expect("read"), None);
        assert_eq!(index.count_sessions().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn renewing_pushes_the_expiry_out() {
        let index = index().await;
        let now = Utc::now();

        index
            .create_session("hash", &name("tim"), now, now + Duration::minutes(1))
            .await
            .expect("create");
        index
            .renew_session("hash", now + Duration::days(30))
            .await
            .expect("renew");

        let later = now + Duration::hours(1);
        assert!(index.session("hash", later).await.expect("read").is_some());
    }

    #[tokio::test]
    async fn signing_out_ends_one_session_and_can_be_repeated() {
        let index = index().await;
        let now = Utc::now();

        index
            .create_session("hash", &name("tim"), now, now + Duration::days(30))
            .await
            .expect("create");
        index.delete_session("hash").await.expect("delete");

        assert_eq!(index.session("hash", now).await.expect("read"), None);
        index
            .delete_session("hash")
            .await
            .expect("deleting an absent session should be fine");
    }

    /// The difference between changing a password and revoking access: every
    /// token handed out before the change has to stop working.
    #[tokio::test]
    async fn every_session_for_an_account_can_be_ended_at_once() {
        let index = index().await;
        let now = Utc::now();
        let expires = now + Duration::days(30);

        for token in ["one", "two", "three"] {
            index
                .create_session(token, &name("tim"), now, expires)
                .await
                .expect("create");
        }
        index
            .create_session("other", &name("alice"), now, expires)
            .await
            .expect("create");

        assert_eq!(
            index
                .delete_sessions_for(&name("tim"))
                .await
                .expect("revoke"),
            3
        );
        assert_eq!(index.session("one", now).await.expect("read"), None);
        // Somebody else's session is untouched.
        assert!(index.session("other", now).await.expect("read").is_some());
    }

    #[tokio::test]
    async fn expired_sessions_can_be_purged_in_one_go() {
        let index = index().await;
        let now = Utc::now();

        index
            .create_session(
                "old",
                &name("tim"),
                now - Duration::days(60),
                now - Duration::days(30),
            )
            .await
            .expect("create");
        index
            .create_session("new", &name("tim"), now, now + Duration::days(30))
            .await
            .expect("create");

        assert_eq!(index.purge_expired_sessions(now).await.expect("purge"), 1);
        assert_eq!(index.count_sessions().await.expect("count"), 1);
    }

    /// Sessions are durable, so a schema bump — which is an ordinary consequence
    /// of changing how pages are indexed — must not sign everybody out.
    #[tokio::test]
    async fn a_session_survives_the_derived_tables_being_rebuilt() {
        let index = index().await;
        let now = Utc::now();

        index
            .create_session("hash", &name("tim"), now, now + Duration::days(30))
            .await
            .expect("create");
        index
            .clear()
            .await
            .expect("drop and rebuild the derived tables");

        assert!(index.session("hash", now).await.expect("read").is_some());
    }
}
