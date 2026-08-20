//! Handing the open user's records to the first account.
//!
//! Owner comparison is equality, which is the whole privacy design and is also
//! a trap on exactly one day: a capture written while the wiki had no accounts
//! has no owner, so the moment an account exists it belongs to nobody and its
//! author cannot reach it through the API any more. That would cost somebody
//! their entire inbox for doing the thing the documentation told them to do.
//!
//! So creating the first account stamps `owner:` onto every unowned capture and
//! thread, and `actor:` onto every unowned event. The open user and the first
//! account are the same person on a single-user wiki that has just been pointed
//! at a network, and the alternatives are worse: leaving the files unowned is
//! honest and unhelpful, and treating an unowned record as readable by every
//! account leaks working notes silently the first time a second account is
//! added. See `knowledge-base/idea-inbox.md`.
//!
//! ## It runs in two places, and that is on purpose
//!
//! There is no single moment the API controls. `POST /api/users` is one way to
//! create the first account; dropping a file into `.rhizolog/users/` is the
//! other, and [`crate::users::store`] documents it as a supported one, answered
//! from the directory on the very next request with no handler involved.
//!
//! So the account-creating handler runs this, which is what keeps the inbox from
//! disappearing even for a moment, and startup runs it too, which is what
//! catches the file somebody dropped in. [`adopt`] is idempotent: a second run
//! is three counting queries that find nothing.
//!
//! ## It refuses to guess
//!
//! Two accounts and a pile of unowned records is a question only a person can
//! answer, so it is reported rather than resolved. That state is reachable by
//! writing two account files before the server next starts.
//!
//! ## It is a migration, so it is written to be interrupted
//!
//! Every record is rewritten on its own and the work is "set the owner where
//! there is none", so stopping half way leaves a wiki that finishes the job the
//! next time this runs. A record that cannot be read is logged and skipped
//! rather than taking the rest of the inbox down with it.

use thiserror::Error;

use crate::ideas::{IdeaStore, IdeaStoreError, Owner};
use crate::index::{Index, IndexError};
use crate::users::{UserStore, UserStoreError, Username};

#[derive(Debug, Error)]
pub enum AdoptionError {
    #[error(transparent)]
    Store(#[from] IdeaStoreError),

    #[error(transparent)]
    Users(#[from] UserStoreError),

    #[error(transparent)]
    Index(#[from] IndexError),
}

/// What one run of [`adopt`] found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adoption {
    /// No accounts, so the wiki is open and its records already belong to the
    /// one user there is. Nothing to do, and nothing wrong.
    NotNeeded,

    /// Every record already has an owner. The ordinary answer on every start
    /// after the first, and three counting queries to reach.
    NothingToAdopt,

    /// Records were handed to the only account there is.
    Adopted {
        owner: Username,
        captures: usize,
        ideas: usize,
        events: usize,
    },

    /// More than one account, and records belonging to none of them.
    ///
    /// Not resolved, because there is no answer here that is not a guess about
    /// whose thoughts those were.
    Ambiguous { accounts: usize, records: usize },
}

impl Adoption {
    pub fn adopted_anything(&self) -> bool {
        matches!(self, Self::Adopted { .. })
    }
}

/// Give the open user's records to the first account, if that is what this wiki
/// needs.
///
/// The decision is made from the index, which is three counts rather than a walk
/// of every file, so the common case of nothing to do is cheap enough to run on
/// every start. **That means the index has to be in step first**: at startup
/// this belongs after reconciliation, not before it, or a deleted database would
/// report an empty inbox and adopt nothing.
///
/// The work itself goes to the files and then to the index, record by record, in
/// the same order every other write in Rhizolog uses.
pub async fn adopt(
    ideas: &IdeaStore,
    users: &UserStore,
    index: &Index,
) -> Result<Adoption, AdoptionError> {
    let accounts = users.names().await?;
    if accounts.is_empty() {
        return Ok(Adoption::NotNeeded);
    }

    let open = Owner::open();
    let records = index.count_captures(&open).await?
        + index.count_ideas(&open).await?
        + index.count_idea_events(&open).await?;
    if records == 0 {
        return Ok(Adoption::NothingToAdopt);
    }

    let [only] = accounts.as_slice() else {
        return Ok(Adoption::Ambiguous {
            accounts: accounts.len(),
            records,
        });
    };
    let owner = Owner::of(only.clone());

    let mut captures = 0;
    for entry in ideas.walk_captures() {
        let capture = match ideas.read_capture(&entry.id).await {
            Ok(capture) => capture,
            Err(error) => {
                tracing::warn!(id = %entry.id, %error, "skipping a capture that could not be read");
                continue;
            }
        };
        if !capture.owner.is_open() {
            continue;
        }

        let adopted = ideas
            .write_capture(
                &capture.id,
                crate::ideas::CaptureDraft {
                    created: capture.created,
                    owner: owner.clone(),
                    body: capture.body,
                },
            )
            .await?;
        index.upsert_capture(&adopted).await?;
        captures += 1;
    }

    let mut threads = 0;
    for entry in ideas.walk_ideas() {
        let idea = match ideas.read_idea(&entry.id).await {
            Ok(idea) => idea,
            Err(error) => {
                tracing::warn!(id = %entry.id, %error, "skipping an idea that could not be read");
                continue;
            }
        };
        if !idea.owner.is_open() {
            continue;
        }

        let adopted = ideas
            .write_idea(
                &idea.id,
                crate::ideas::IdeaDraft {
                    name: idea.name,
                    created: idea.created,
                    owner: owner.clone(),
                    seeds: idea.seeds,
                    note: idea.note,
                },
            )
            .await?;
        index.upsert_idea(&adopted).await?;
        threads += 1;
    }

    let mut events = 0;
    for entry in ideas.walk_events() {
        match ideas.adopt_event(&entry.id, &owner).await {
            Ok(Some(event)) => {
                index.upsert_idea_event(&event).await?;
                events += 1;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(id = %entry.id, %error, "skipping an event that could not be read");
            }
        }
    }

    Ok(Adoption::Adopted {
        owner: only.clone(),
        captures,
        ideas: threads,
        events,
    })
}

/// Run adoption and say what happened, without letting it stop anything.
///
/// Both callers want the same handling: a wiki whose idea records could not be
/// adopted is still a wiki worth serving and an account still worth creating,
/// and the records are exactly where they were. Startup would otherwise refuse
/// to boot over it, and the account-creating handler would report a failure for
/// an account it had successfully created.
pub async fn adopt_and_report(ideas: &IdeaStore, users: &UserStore, index: &Index) {
    match adopt(ideas, users, index).await {
        Ok(Adoption::Adopted {
            owner,
            captures,
            ideas,
            events,
        }) => tracing::info!(
            %owner,
            captures,
            ideas,
            events,
            "the first account has adopted the idea records this wiki had before it"
        ),
        Ok(Adoption::Ambiguous { accounts, records }) => tracing::warn!(
            accounts,
            records,
            "there are idea records belonging to no account, and more than one account to \
             give them to. Nothing has been changed: write `owner:` into the files yourself, \
             or leave them for whoever should have them."
        ),
        Ok(Adoption::NotNeeded | Adoption::NothingToAdopt) => {}
        Err(error) => tracing::warn!(
            %error,
            "could not adopt the idea records this wiki had before its first account; they are \
             unchanged, and the next start will try again"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{DateTime, Utc};
    use tempfile::TempDir;

    use crate::ideas::{CaptureDraft, EventDraft, EventKind, IdeaDraft, Subject};
    use crate::users::{Role, UserFrontmatter};

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn name(raw: &str) -> Username {
        Username::parse(raw).expect("valid username")
    }

    async fn fixture() -> (TempDir, IdeaStore, UserStore, Index) {
        let directory = TempDir::new().expect("temp dir");
        let ideas = IdeaStore::open(directory.path()).await.expect("open ideas");
        let users = UserStore::open(directory.path()).await.expect("open users");
        let index = Index::open(None).await.expect("open index");
        (directory, ideas, users, index)
    }

    /// A capture, a thread seeded from it, and a decision, all written while the
    /// wiki had no accounts.
    async fn open_records(ideas: &IdeaStore, index: &Index) {
        let capture = ideas
            .create_capture(CaptureDraft {
                created: at("2026-08-20T14:15:30Z"),
                owner: Owner::open(),
                body: "Dungeon seeds.\n".to_owned(),
            })
            .await
            .expect("capture");
        let idea = ideas
            .create_idea(IdeaDraft {
                name: "Dungeon seeds".to_owned(),
                created: at("2026-08-20T14:20:00Z"),
                owner: Owner::open(),
                seeds: vec![capture.id.clone()],
                note: String::new(),
            })
            .await
            .expect("idea");
        let event = ideas
            .append_event(
                EventDraft::new(
                    EventKind::InterestAffirmed,
                    Subject::Idea {
                        idea: idea.id.clone(),
                    },
                    at("2026-08-20T14:21:00Z"),
                    Owner::open(),
                )
                .expect("draft"),
            )
            .await
            .expect("event");

        index.upsert_capture(&capture).await.expect("index capture");
        index.upsert_idea(&idea).await.expect("index idea");
        index.upsert_idea_event(&event).await.expect("index event");
    }

    async fn add_account(users: &UserStore, raw: &str) {
        users
            .create(
                &name(raw),
                UserFrontmatter {
                    role: Role::Owner,
                    ..UserFrontmatter::default()
                },
                "",
            )
            .await
            .expect("create account");
    }

    /// The day this exists for: a wiki full of captures gains its first account,
    /// and the person who wrote them keeps them.
    #[tokio::test]
    async fn the_first_account_takes_the_open_users_records() {
        let (_directory, ideas, users, index) = fixture().await;
        open_records(&ideas, &index).await;
        add_account(&users, "tim").await;

        let outcome = adopt(&ideas, &users, &index).await.expect("adopt");

        assert_eq!(
            outcome,
            Adoption::Adopted {
                owner: name("tim"),
                captures: 1,
                ideas: 1,
                events: 1,
            }
        );

        let tim = Owner::of(name("tim"));
        assert_eq!(index.count_captures(&tim).await.unwrap(), 1);
        assert_eq!(index.count_ideas(&tim).await.unwrap(), 1);
        assert_eq!(index.count_idea_events(&tim).await.unwrap(), 1);
        assert_eq!(index.count_captures(&Owner::open()).await.unwrap(), 0);

        // And it reached the files, not only the index.
        let reopened = IdeaStore::open(ideas.root().parent().unwrap().parent().unwrap())
            .await
            .expect("reopen");
        let walked = reopened.walk_captures();
        let capture = reopened
            .read_capture(&walked[0].id)
            .await
            .expect("read capture");
        assert_eq!(capture.owner, tim);
    }

    /// The fold is derived from the decisions, and adoption changes who they are
    /// attributed to rather than what they say. It must not move.
    #[tokio::test]
    async fn adoption_does_not_disturb_what_was_decided() {
        let (_directory, ideas, users, index) = fixture().await;
        open_records(&ideas, &index).await;
        let thread = ideas.walk_ideas()[0].id.clone();
        let before = index
            .idea_state(&Owner::open(), &thread)
            .await
            .unwrap()
            .expect("state");

        add_account(&users, "tim").await;
        adopt(&ideas, &users, &index).await.expect("adopt");

        let after = index
            .idea_state(&Owner::of(name("tim")), &thread)
            .await
            .unwrap()
            .expect("state");

        assert_eq!(after.members, before.members);
        assert_eq!(after.last_signal, before.last_signal);
        assert_eq!(after.retired, before.retired);
        assert_eq!(after.created, before.created);
    }

    /// Both callers run this, and startup runs it on every boot, so the second
    /// run has to be free and harmless.
    #[tokio::test]
    async fn running_it_again_finds_nothing_to_do() {
        let (_directory, ideas, users, index) = fixture().await;
        open_records(&ideas, &index).await;
        add_account(&users, "tim").await;

        assert!(
            adopt(&ideas, &users, &index)
                .await
                .unwrap()
                .adopted_anything()
        );
        assert_eq!(
            adopt(&ideas, &users, &index).await.unwrap(),
            Adoption::NothingToAdopt
        );
    }

    #[tokio::test]
    async fn an_open_wiki_needs_no_adoption() {
        let (_directory, ideas, users, index) = fixture().await;
        open_records(&ideas, &index).await;

        assert_eq!(
            adopt(&ideas, &users, &index).await.unwrap(),
            Adoption::NotNeeded
        );
        assert_eq!(index.count_captures(&Owner::open()).await.unwrap(), 1);
    }

    /// Two accounts and a pile of unowned captures is a question only a person
    /// can answer, and guessing at it would hand one person's thoughts to
    /// another.
    #[tokio::test]
    async fn more_than_one_account_is_reported_rather_than_guessed_at() {
        let (_directory, ideas, users, index) = fixture().await;
        open_records(&ideas, &index).await;
        add_account(&users, "tim").await;
        add_account(&users, "alice").await;

        assert_eq!(
            adopt(&ideas, &users, &index).await.unwrap(),
            Adoption::Ambiguous {
                accounts: 2,
                records: 3,
            }
        );
        // Nothing moved.
        assert_eq!(index.count_captures(&Owner::open()).await.unwrap(), 1);
        assert_eq!(
            index.count_captures(&Owner::of(name("tim"))).await.unwrap(),
            0
        );
    }

    /// A record that already belongs to somebody is not the open user's to give
    /// away, whatever else is going on.
    #[tokio::test]
    async fn a_record_that_already_has_an_owner_is_left_alone() {
        let (_directory, ideas, users, index) = fixture().await;
        let alice = Owner::of(name("alice"));
        let hers = ideas
            .create_capture(CaptureDraft {
                created: at("2026-08-20T14:30:00Z"),
                owner: alice.clone(),
                body: "Not yours.\n".to_owned(),
            })
            .await
            .expect("capture");
        index.upsert_capture(&hers).await.expect("index");
        open_records(&ideas, &index).await;
        add_account(&users, "tim").await;

        let outcome = adopt(&ideas, &users, &index).await.expect("adopt");

        assert!(matches!(outcome, Adoption::Adopted { captures: 1, .. }));
        assert_eq!(index.count_captures(&alice).await.unwrap(), 1);
        assert_eq!(
            index.capture(&alice, &hers.id).await.unwrap().unwrap().body,
            "Not yours.\n"
        );
    }
}
