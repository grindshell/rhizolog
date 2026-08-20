//! The rules a capture, an idea and a decision have to satisfy.
//!
//! [`IdeaStore`] reads and writes files and asks no questions. This is the layer
//! that asks them, and it sits here rather than in the request handlers because
//! there will be more than one caller: the HTTP API, and anything later that
//! writes idea data without going through it. A rule that lives in a handler is
//! a rule the next caller does not get.
//!
//! ## Every operation is asked on somebody's behalf
//!
//! Idea Inbox is personal working state. Each call takes the [`Owner`] making
//! it, and a record whose owner is not equal to that one is reported **missing**
//! rather than forbidden. That is deliberate and it is the whole privacy design
//! in one line: a caller who could tell the difference between "no such capture"
//! and "not yours" would have an existence oracle for somebody else's notes,
//! and later, once candidates and receipts exist, a similarity score would be a
//! far better one. See `knowledge-base/idea-inbox.md`.
//!
//! On a wiki with no accounts the owner is [`Owner::open`] and every record is
//! the one user's, which is the same behaviour the wiki has always had.
//!
//! Because every record an operation touches is checked against the caller,
//! "a capture may only connect to an idea with the same owner" holds by
//! transitivity and is not written a second time here. It is a real invariant
//! all the same, and reconciliation is where a hand-written file that breaks it
//! gets caught: see the integrity diagnostics in the plan.
//!
//! ## What this layer still cannot decide
//!
//! Anything that needs the folded current state, because that is derived and
//! does not exist yet. Refusing to disconnect an idea's last capture, refusing
//! to delete a capture an idea still needs, and refusing to retire an already
//! retired idea are all questions for the derived index. Promotion's page
//! reference is not checked here either: the page store is not this service's,
//! and the API layer holds both.

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::ideas::{
    Capture, CaptureDraft, CaptureId, Event, EventDraft, EventKind, Idea, IdeaDraft, IdeaId,
    IdeaStore, IdeaStoreError, MAX_SEEDS, Owner, RecordError, Subject,
};

#[derive(Debug, Error)]
pub enum IdeaServiceError {
    #[error("a capture needs some text")]
    EmptyCapture,

    #[error("an idea needs a name")]
    EmptyName,

    #[error("an idea has to be started from at least one capture")]
    NoSeeds,

    #[error("an idea may be started from at most {MAX_SEEDS} captures, not {count}")]
    TooManySeeds { count: usize },

    #[error("no capture {id}")]
    CaptureNotFound { id: CaptureId },

    #[error("no idea {id}")]
    IdeaNotFound { id: IdeaId },

    #[error(transparent)]
    Record(#[from] RecordError),

    #[error(transparent)]
    Store(#[from] IdeaStoreError),
}

impl IdeaServiceError {
    /// A stable, machine-readable identifier for what went wrong.
    ///
    /// A record that is not this caller's reports `*_not_found`, exactly as a
    /// record that never existed does. That is the disclosure rule, and it has
    /// to hold here as much as in the message: a code a client could branch on
    /// to tell the two apart would be the oracle the prose was careful not to
    /// be.
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyCapture => "capture_empty",
            Self::EmptyName => "idea_name_empty",
            Self::NoSeeds => "idea_no_seeds",
            Self::TooManySeeds { .. } => "idea_too_many_seeds",
            Self::CaptureNotFound { .. } => "capture_not_found",
            Self::IdeaNotFound { .. } => "idea_not_found",
            Self::Record(_) => "idea_event_invalid",
            Self::Store(error) => error.code(),
        }
    }
}

/// Idea Inbox, with its rules attached.
#[derive(Debug, Clone)]
pub struct IdeaService {
    store: IdeaStore,
}

impl IdeaService {
    pub fn new(store: IdeaStore) -> Self {
        Self { store }
    }

    /// The unscoped file layer underneath.
    ///
    /// For reconciliation and the indexer, which have to see every record on the
    /// wiki and are not acting for anybody. Request handling should not reach
    /// through here.
    pub fn store(&self) -> &IdeaStore {
        &self.store
    }

    // Captures.

    /// Save one captured thought.
    ///
    /// The only thing refused is text that is not there. A capture takes one
    /// field and one action, so there is nothing else to get wrong: no title, no
    /// slug, no tag, no interpretation.
    pub async fn capture(
        &self,
        owner: &Owner,
        text: &str,
        at: DateTime<Utc>,
    ) -> Result<Capture, IdeaServiceError> {
        if text.trim().is_empty() {
            return Err(IdeaServiceError::EmptyCapture);
        }

        Ok(self
            .store
            .create_capture(CaptureDraft {
                created: at,
                owner: owner.clone(),
                body: text.to_owned(),
            })
            .await?)
    }

    /// One of this owner's captures.
    pub async fn read_capture(
        &self,
        owner: &Owner,
        id: &CaptureId,
    ) -> Result<Capture, IdeaServiceError> {
        let capture = match self.store.read_capture(id).await {
            Ok(capture) => capture,
            Err(IdeaStoreError::NotFound { .. }) => {
                return Err(IdeaServiceError::CaptureNotFound { id: id.clone() });
            }
            Err(error) => return Err(error.into()),
        };

        if capture.owner != *owner {
            return Err(IdeaServiceError::CaptureNotFound { id: id.clone() });
        }

        Ok(capture)
    }

    /// Correct a capture's text. Its timestamp and owner do not move.
    pub async fn edit_capture(
        &self,
        owner: &Owner,
        id: &CaptureId,
        text: &str,
    ) -> Result<Capture, IdeaServiceError> {
        if text.trim().is_empty() {
            return Err(IdeaServiceError::EmptyCapture);
        }
        self.read_capture(owner, id).await?;

        Ok(self.store.patch_capture(id, text).await?)
    }

    /// Remove a capture's file for good.
    ///
    /// The caller is expected to have appended `capture_deleted` first, so that
    /// a failure here leaves an event that says what was meant rather than a
    /// gap that says nothing. Whether the capture is still the last live member
    /// of an idea is a question for the folded state and is checked above this
    /// layer.
    pub async fn delete_capture(
        &self,
        owner: &Owner,
        id: &CaptureId,
    ) -> Result<(), IdeaServiceError> {
        self.read_capture(owner, id).await?;
        Ok(self.store.delete_capture(id).await?)
    }

    // Idea threads.

    /// Start a named thread from captures this owner holds.
    ///
    /// Every seed is read before anything is written, so an idea never comes
    /// into existence naming a capture that is not there.
    pub async fn start_idea(
        &self,
        owner: &Owner,
        name: &str,
        seeds: &[CaptureId],
        note: &str,
        at: DateTime<Utc>,
    ) -> Result<Idea, IdeaServiceError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(IdeaServiceError::EmptyName);
        }

        let seeds = dedupe(seeds);
        if seeds.is_empty() {
            return Err(IdeaServiceError::NoSeeds);
        }
        if seeds.len() > MAX_SEEDS {
            return Err(IdeaServiceError::TooManySeeds { count: seeds.len() });
        }

        for seed in &seeds {
            self.read_capture(owner, seed).await?;
        }

        Ok(self
            .store
            .create_idea(IdeaDraft {
                name: name.to_owned(),
                created: at,
                owner: owner.clone(),
                seeds,
                note: note.to_owned(),
            })
            .await?)
    }

    /// One of this owner's ideas.
    pub async fn read_idea(&self, owner: &Owner, id: &IdeaId) -> Result<Idea, IdeaServiceError> {
        let idea = match self.store.read_idea(id).await {
            Ok(idea) => idea,
            Err(IdeaStoreError::NotFound { .. }) => {
                return Err(IdeaServiceError::IdeaNotFound { id: id.clone() });
            }
            Err(error) => return Err(error.into()),
        };

        if idea.owner != *owner {
            return Err(IdeaServiceError::IdeaNotFound { id: id.clone() });
        }

        Ok(idea)
    }

    /// Rename a thread or edit its note. Its seeds and origin do not move.
    pub async fn edit_idea(
        &self,
        owner: &Owner,
        id: &IdeaId,
        name: Option<&str>,
        note: Option<&str>,
    ) -> Result<Idea, IdeaServiceError> {
        let name = name.map(str::trim);
        if name.is_some_and(str::is_empty) {
            return Err(IdeaServiceError::EmptyName);
        }
        self.read_idea(owner, id).await?;

        Ok(self.store.patch_idea(id, name, note).await?)
    }

    // Decisions.

    /// Append one decision, having confirmed that everything it names exists
    /// and belongs to the actor.
    ///
    /// An event is a claim about records, and an event naming a capture that was
    /// never there is not a decision anybody took. The page a promotion names is
    /// the one reference this layer cannot check, because pages are not its
    /// store.
    pub async fn record(
        &self,
        actor: &Owner,
        kind: EventKind,
        subject: Subject,
        at: DateTime<Utc>,
    ) -> Result<Event, IdeaServiceError> {
        let draft = EventDraft::new(kind, subject, at, actor.clone())?;

        if let Some(idea) = draft.subject().idea() {
            self.read_idea(actor, idea).await?;
        }
        if let Some(capture) = draft.subject().capture() {
            self.read_capture(actor, capture).await?;
        }
        if let Some(other) = draft.subject().other_capture() {
            self.read_capture(actor, other).await?;
        }

        Ok(self.store.append_event(draft).await?)
    }
}

/// Keep the first mention of each capture and drop the rest.
fn dedupe(seeds: &[CaptureId]) -> Vec<CaptureId> {
    let mut seen = std::collections::HashSet::new();
    seeds
        .iter()
        .filter(|seed| seen.insert((*seed).clone()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::slug::Slug;
    use crate::users::Username;

    async fn service() -> (TempDir, IdeaService) {
        let directory = TempDir::new().expect("temp dir");
        let store = IdeaStore::open(directory.path()).await.expect("open");
        (directory, IdeaService::new(store))
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn owner(name: &str) -> Owner {
        Owner::of(Username::parse(name).expect("valid username"))
    }

    async fn a_capture(service: &IdeaService, owner: &Owner, text: &str, created: &str) -> Capture {
        service
            .capture(owner, text, at(created))
            .await
            .expect("capture")
    }

    #[tokio::test]
    async fn captures_text_and_reads_it_back() {
        let (_directory, service) = service().await;
        let tim = owner("tim");

        let written = a_capture(
            &service,
            &tim,
            "Dungeon quests should require seeds.\n",
            "2026-08-20T14:15:30Z",
        )
        .await;

        let read = service.read_capture(&tim, &written.id).await.expect("read");
        assert_eq!(read.body, "Dungeon quests should require seeds.\n");
        assert_eq!(read.owner, tim);
    }

    /// The one thing a capture is refused over.
    #[tokio::test]
    async fn a_capture_with_no_text_is_refused() {
        let (_directory, service) = service().await;

        for text in ["", "   ", "\n\t\n"] {
            let error = service
                .capture(&Owner::open(), text, at("2026-08-20T14:15:30Z"))
                .await
                .expect_err("refused");
            assert!(
                matches!(error, IdeaServiceError::EmptyCapture),
                "{text:?}: {error}"
            );
        }

        assert_eq!(service.store().walk_captures().len(), 0);
    }

    /// A wiki with no accounts is open, and its one user owns everything, which
    /// is exactly how the wiki behaved before accounts existed.
    #[tokio::test]
    async fn an_accountless_wiki_is_one_open_user() {
        let (_directory, service) = service().await;

        let written = a_capture(
            &service,
            &Owner::open(),
            "A thought.\n",
            "2026-08-20T14:15:30Z",
        )
        .await;

        assert!(written.owner.is_open());
        assert!(
            service
                .read_capture(&Owner::open(), &written.id)
                .await
                .is_ok()
        );
    }

    /// The privacy design in one assertion: somebody else's capture is missing,
    /// not forbidden, because the difference would be an existence oracle.
    #[tokio::test]
    async fn another_owners_record_is_reported_missing_rather_than_refused() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let alice = owner("alice");

        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;
        let idea = service
            .start_idea(
                &tim,
                "Dungeon seeds",
                std::slice::from_ref(&written.id),
                "",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect("idea");

        let error = service
            .read_capture(&alice, &written.id)
            .await
            .expect_err("refused");
        assert!(
            matches!(error, IdeaServiceError::CaptureNotFound { .. }),
            "{error}"
        );

        let error = service
            .read_idea(&alice, &idea.id)
            .await
            .expect_err("refused");
        assert!(
            matches!(error, IdeaServiceError::IdeaNotFound { .. }),
            "{error}"
        );

        // Word for word the answer an id that never existed would get, which is
        // the point: there is nothing in it to tell the two apart.
        let never = CaptureId::parse("20200101T000000-000000000").unwrap();
        assert_eq!(
            service
                .read_capture(&alice, &never)
                .await
                .unwrap_err()
                .to_string(),
            format!("no capture {never}")
        );
        assert_eq!(
            service
                .read_capture(&alice, &written.id)
                .await
                .unwrap_err()
                .to_string(),
            format!("no capture {}", written.id)
        );
    }

    /// An open capture is not an unclaimed one. Adopting it means writing
    /// `owner:` into its file, and nothing here guesses.
    #[tokio::test]
    async fn an_open_capture_does_not_belong_to_an_account() {
        let (_directory, service) = service().await;
        let written = a_capture(
            &service,
            &Owner::open(),
            "A thought.\n",
            "2026-08-20T14:15:30Z",
        )
        .await;

        let error = service
            .read_capture(&owner("tim"), &written.id)
            .await
            .expect_err("refused");

        assert!(
            matches!(error, IdeaServiceError::CaptureNotFound { .. }),
            "{error}"
        );
    }

    #[tokio::test]
    async fn editing_a_capture_keeps_its_timestamp_and_refuses_emptying_it() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let written = a_capture(&service, &tim, "First draft.\n", "2026-08-20T14:15:30Z").await;

        let edited = service
            .edit_capture(&tim, &written.id, "Second draft.\n")
            .await
            .expect("edit");
        assert_eq!(edited.body, "Second draft.\n");
        assert_eq!(edited.created, written.created);

        let error = service
            .edit_capture(&tim, &written.id, "  ")
            .await
            .expect_err("refused");
        assert!(matches!(error, IdeaServiceError::EmptyCapture), "{error}");
        assert_eq!(
            service.read_capture(&tim, &written.id).await.unwrap().body,
            "Second draft.\n"
        );
    }

    #[tokio::test]
    async fn an_idea_needs_a_name_and_a_seed_that_exists() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;
        let created = at("2026-08-20T14:20:00Z");

        let error = service
            .start_idea(&tim, "   ", std::slice::from_ref(&written.id), "", created)
            .await
            .expect_err("refused");
        assert!(matches!(error, IdeaServiceError::EmptyName), "{error}");

        let error = service
            .start_idea(&tim, "Dungeon seeds", &[], "", created)
            .await
            .expect_err("refused");
        assert!(matches!(error, IdeaServiceError::NoSeeds), "{error}");

        let missing = CaptureId::parse("20200101T000000-000000000").unwrap();
        let error = service
            .start_idea(&tim, "Dungeon seeds", &[missing], "", created)
            .await
            .expect_err("refused");
        assert!(
            matches!(error, IdeaServiceError::CaptureNotFound { .. }),
            "{error}"
        );

        // Nothing was written by any of the three.
        assert_eq!(service.store().walk_ideas().len(), 0);
    }

    /// An idea cannot be started from somebody else's capture, and it is
    /// refused with the same answer as a capture that does not exist.
    #[tokio::test]
    async fn an_idea_cannot_be_seeded_from_another_owners_capture() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let alice = owner("alice");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;

        let error = service
            .start_idea(
                &alice,
                "Dungeon seeds",
                &[written.id],
                "",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect_err("refused");

        assert!(
            matches!(error, IdeaServiceError::CaptureNotFound { .. }),
            "{error}"
        );
        assert_eq!(service.store().walk_ideas().len(), 0);
    }

    #[tokio::test]
    async fn a_repeated_seed_is_recorded_once() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;

        let idea = service
            .start_idea(
                &tim,
                "Dungeon seeds",
                &[written.id.clone(), written.id.clone()],
                "",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect("idea");

        assert_eq!(idea.seeds, [written.id]);
    }

    #[tokio::test]
    async fn renaming_an_idea_leaves_everything_else_alone() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;
        let idea = service
            .start_idea(
                &tim,
                "Dungeon seeds",
                std::slice::from_ref(&written.id),
                "Notes.\n",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect("idea");

        let renamed = service
            .edit_idea(&tim, &idea.id, Some("Seeded dungeons"), None)
            .await
            .expect("rename");

        assert_eq!(renamed.name, "Seeded dungeons");
        assert_eq!(renamed.note, "Notes.\n");
        assert_eq!(renamed.seeds, [written.id]);

        let error = service
            .edit_idea(&tim, &idea.id, Some(" "), None)
            .await
            .expect_err("refused");
        assert!(matches!(error, IdeaServiceError::EmptyName), "{error}");
    }

    #[tokio::test]
    async fn a_decision_names_records_that_exist_and_belong_to_the_actor() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let alice = owner("alice");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;
        let idea = service
            .start_idea(
                &tim,
                "Dungeon seeds",
                std::slice::from_ref(&written.id),
                "",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect("idea");
        let taken = at("2026-08-20T14:20:30Z");

        let event = service
            .record(
                &tim,
                EventKind::CaptureConnected,
                Subject::IdeaCapture {
                    idea: idea.id.clone(),
                    capture: written.id.clone(),
                },
                taken,
            )
            .await
            .expect("record");
        assert_eq!(event.actor, tim);

        // Somebody else's idea is missing, so the decision is not recorded.
        let error = service
            .record(
                &alice,
                EventKind::IdeaRetired,
                Subject::Idea {
                    idea: idea.id.clone(),
                },
                taken,
            )
            .await
            .expect_err("refused");
        assert!(
            matches!(error, IdeaServiceError::IdeaNotFound { .. }),
            "{error}"
        );

        // A capture that never existed is refused the same way.
        let missing = CaptureId::parse("20200101T000000-000000000").unwrap();
        let error = service
            .record(
                &tim,
                EventKind::CaptureArchived,
                Subject::Capture { capture: missing },
                taken,
            )
            .await
            .expect_err("refused");
        assert!(
            matches!(error, IdeaServiceError::CaptureNotFound { .. }),
            "{error}"
        );

        assert_eq!(service.store().walk_events().len(), 1);
    }

    /// A subject that does not match its kind never reaches the disk, and the
    /// message names the reference that would have made it one.
    #[tokio::test]
    async fn a_decision_whose_references_do_not_match_its_kind_is_refused() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;
        let idea = service
            .start_idea(
                &tim,
                "Dungeon seeds",
                &[written.id],
                "",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect("idea");

        let error = service
            .record(
                &tim,
                EventKind::CaptureConnected,
                Subject::Idea { idea: idea.id },
                at("2026-08-20T14:20:30Z"),
            )
            .await
            .expect_err("refused");

        assert!(matches!(error, IdeaServiceError::Record(_)), "{error}");
        assert_eq!(service.store().walk_events().len(), 0);
    }

    /// The one reference this layer cannot check. Pages are not its store, so a
    /// promotion is recorded and the page is somebody else's problem.
    #[tokio::test]
    async fn a_promotion_records_a_page_this_layer_does_not_verify() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;
        let idea = service
            .start_idea(
                &tim,
                "Dungeon seeds",
                &[written.id],
                "",
                at("2026-08-20T14:20:00Z"),
            )
            .await
            .expect("idea");

        let event = service
            .record(
                &tim,
                EventKind::IdeaPromoted,
                Subject::Promotion {
                    idea: idea.id,
                    page: Slug::parse("notes/dungeon-seeds").expect("valid slug"),
                },
                at("2026-08-20T14:20:30Z"),
            )
            .await
            .expect("record");

        assert_eq!(
            event.subject.page().map(Slug::as_str),
            Some("notes/dungeon-seeds")
        );
    }

    #[tokio::test]
    async fn deleting_a_capture_needs_it_to_be_this_owners() {
        let (_directory, service) = service().await;
        let tim = owner("tim");
        let alice = owner("alice");
        let written = a_capture(&service, &tim, "A thought.\n", "2026-08-20T14:15:30Z").await;

        let error = service
            .delete_capture(&alice, &written.id)
            .await
            .expect_err("refused");
        assert!(
            matches!(error, IdeaServiceError::CaptureNotFound { .. }),
            "{error}"
        );
        assert_eq!(service.store().walk_captures().len(), 1);

        service
            .delete_capture(&tim, &written.id)
            .await
            .expect("delete");
        assert_eq!(service.store().walk_captures().len(), 0);
    }
}
