//! Idea Inbox: capturing thoughts, threading them, and deciding about them.
//!
//! ## Everything here is somebody's
//!
//! Idea Inbox is personal working state, so every endpoint acts for one
//! [`Owner`] and a record belonging to anybody else is reported **missing**
//! rather than forbidden. A caller able to tell those apart would have an
//! existence oracle for another person's notes, and once candidates and receipts
//! exist a similarity score would be a far better one.
//!
//! None of these routes answers an unauthenticated caller, including under
//! `RHIZOLOG_ANONYMOUS_READ`, which lists no idea path. [`Viewer::owner`] is the
//! second lock on that door.
//!
//! ## Saving a capture is one field and one action
//!
//! `POST /api/captures` takes text and nothing else. No title, no slug, no tag,
//! no interpretation, and no candidates in the response: analysis is a separate
//! request against a capture that has already been written, so analysis being
//! slow or broken can never lose the words somebody just typed.
//!
//! ## Deciding is `PUT` and `DELETE`, not `POST /connect`
//!
//! Connecting a capture to an idea is `PUT /api/ideas/{id}/captures/{capture}`
//! and disconnecting it is the `DELETE`. The state being asked for is in the
//! URL, so a client that repeats itself changes nothing and no second event is
//! written. The four verbs that are genuinely acts rather than states, and that
//! mean something every time they happen, are `POST`: affirm, retire, reopen and
//! dismiss.
//!
//! Retiring what is already retired is a `409` rather than a no-op, and that is
//! not inconsistent with the above: a caller retiring twice believes the state
//! is something it is not, and saying so is more use than an event nobody asked
//! for. Affirming twice is two affirmations at two times, and both are real.
//!
//! ## Ids are not slugs, so these routes are not wildcards
//!
//! A [`CaptureId`] contains no `/`, which is what lets
//! `/api/ideas/{id}/captures/{capture_id}` exist at all: `matchit` requires a
//! catch-all to be the last segment, which is the restriction that sent a page's
//! move to `/api/move`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::api::extract::Json as JsonBody;
use crate::auth::Viewer;
use crate::error::{AppError, AppResult};
use crate::ideas::analysis::{self, Target};
use crate::ideas::lifecycle::{
    self, Boundaries, Components, CountedAffirmation, CountedCapture, Integrity, Lifecycle, Receipt,
};
use crate::ideas::{
    Capture, CaptureId, Event, EventKind, Idea, IdeaId, Owner, RecordKind, Subject,
};
use crate::index::{
    CaptureListOptions, CaptureRecord, IdeaListOptions, IdeaStanding, IdeaState, IdeaSummary,
};
use crate::slug::Slug;
use crate::store::StoreError;

const DEFAULT_LIMIT: usize = 50;

/// How many decimal places a weight, a score or a contribution keeps on the
/// wire.
///
/// Enough that adding the listed contributions reproduces the similarity to
/// within a millionth, and few enough that a receipt reads as arithmetic rather
/// than as floating-point noise.
const PLACES: f64 = 1e6;

/// Round a score for the wire. Every float this module returns goes through it,
/// so a client comparing two numbers is comparing them at the same precision.
fn rounded(value: f64) -> f64 {
    (value * PLACES).round() / PLACES
}

/// Caps how much one listing can return.
///
/// The same ceiling `MAX_SEEDS` uses, because it is the same question from the
/// other side: a seed list is a page of captures somebody selected.
const MAX_LIMIT: usize = 200;

// ---------------------------------------------------------------- responses

fn example_text() -> &'static str {
    "Maybe dungeon quests should require finding particular seeds.\n"
}

/// One captured thought.
#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureView {
    pub id: CaptureId,
    /// The text exactly as it was supplied. Never summarised, never rewritten.
    #[schema(example = example_text)]
    pub text: String,
    pub created: DateTime<Utc>,
    /// Whether it has been archived out of the inbox.
    ///
    /// Archived means processed, not "this never happened": an archived capture
    /// still belongs to whatever ideas hold it and still counts as evidence.
    pub archived: bool,
    /// The file's modification time.
    pub updated: DateTime<Utc>,
    /// The file's size in bytes.
    #[schema(example = 92)]
    pub size: u64,
}

impl From<CaptureRecord> for CaptureView {
    fn from(record: CaptureRecord) -> Self {
        Self {
            id: record.id,
            text: record.body,
            created: record.created,
            archived: record.archived,
            updated: record.updated,
            size: record.size,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CaptureListResponse {
    /// Newest first.
    pub captures: Vec<CaptureView>,
    /// Total matching captures, not the number returned.
    #[schema(example = 143)]
    pub total: usize,
    /// The limit that was applied, after clamping.
    #[schema(example = 50)]
    pub limit: usize,
    #[schema(example = 0)]
    pub offset: usize,
}

/// One idea thread, as a listing shows it.
///
/// `state` and `momentum` are worked out for the instant the listing was asked
/// about, which the response repeats. Neither is stored: the same files answer
/// differently tomorrow, and that is the feature rather than staleness.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdeaSummaryView {
    pub id: IdeaId,
    /// What the person called it. Never generated.
    #[schema(example = "Dungeon seeds")]
    pub name: String,
    pub created: DateTime<Utc>,
    /// How many captures it currently holds and can still read.
    #[schema(example = 3)]
    pub captures: usize,
    /// How many it holds whose files are gone.
    #[schema(example = 0)]
    pub missing: usize,
    pub retired: bool,
    /// The page this idea produced, if it has been promoted.
    pub promoted_to: Option<Slug>,
    /// The last time anything happened to it: a capture connected, or an
    /// affirmation, reopening or promotion.
    pub last_signal: Option<DateTime<Utc>>,
    /// When rediscovery was last dismissed for it. Absent if it never was.
    ///
    /// Not a lifecycle input, and deliberately so: dismissing a card says
    /// something about the card rather than about the idea. It is here because
    /// choosing today's rediscovery is the client's to do, and this is the one
    /// thing that choice needs which nothing else on this view says.
    pub dismissed: Option<DateTime<Utc>>,
    /// Whether the idea has lost the evidence it rests on.
    ///
    /// A thread with nothing live connected cannot be given a lifecycle state,
    /// because there is no authored text left to derive one from. It is reported
    /// rather than papered over.
    pub needs_repair: bool,
    /// Whether every connected capture is still readable.
    pub integrity: Integrity,
    /// Where the rules put it. Absent exactly when `needs_repair` is true.
    pub state: Option<Lifecycle>,
    /// How much is going on. Absent for the same reason. Never shown without a
    /// way to open the receipt that explains it.
    #[schema(example = 5)]
    pub momentum: Option<u32>,
    pub updated: DateTime<Utc>,
}

impl From<IdeaStanding> for IdeaSummaryView {
    fn from(standing: IdeaStanding) -> Self {
        let IdeaStanding {
            summary,
            integrity,
            state,
            momentum,
        } = standing;

        Self {
            needs_repair: summary.evidence_missing(),
            id: summary.id,
            name: summary.name,
            created: summary.created,
            captures: summary.members,
            missing: summary.missing,
            retired: summary.retired,
            promoted_to: summary.promoted_to,
            last_signal: summary.last_signal,
            dismissed: summary.dismissed,
            integrity,
            state,
            momentum,
            updated: summary.updated,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IdeaListResponse {
    /// Most recently active first.
    pub ideas: Vec<IdeaSummaryView>,
    /// Matching ideas, not the number returned. Counted after the `state` and
    /// `integrity` filters, so paging through it reaches every one of them.
    #[schema(example = 7)]
    pub total: usize,
    #[schema(example = 50)]
    pub limit: usize,
    #[schema(example = 0)]
    pub offset: usize,
    /// The instant every state and momentum here was worked out for.
    pub at: DateTime<Utc>,
    /// The rules that produced them.
    #[schema(example = "idea-momentum/v1")]
    pub ruleset: String,
}

/// One idea thread, with what it holds and where the rules put it.
///
/// The state and momentum here are worked out for the moment of the request.
/// `GET /api/ideas/{id}/receipt` is the same answer with every number and every
/// piece of evidence behind it, and takes an `at` for any other moment.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdeaView {
    pub id: IdeaId,
    #[schema(example = "Dungeon seeds")]
    pub name: String,
    /// Working notes from the thread's own file. Usually empty.
    pub note: String,
    pub created: DateTime<Utc>,
    /// The captures it currently holds, oldest first.
    pub captures: Vec<CaptureView>,
    /// Captures it holds whose files are gone. Named rather than dropped, so a
    /// reader can see what the idea has lost instead of quietly losing nothing.
    pub missing: Vec<CaptureId>,
    /// Captures turned down as candidates, so they are not suggested again.
    pub rejected: Vec<CaptureId>,
    pub retired: bool,
    pub promoted_to: Option<Slug>,
    pub last_signal: Option<DateTime<Utc>>,
    /// When rediscovery was last dismissed for it. Absent if it never was.
    pub dismissed: Option<DateTime<Utc>>,
    pub needs_repair: bool,
    pub integrity: Integrity,
    /// Absent exactly when `needs_repair` is true.
    pub state: Option<Lifecycle>,
    /// Absent for the same reason. See the receipt for how it was arrived at.
    #[schema(example = 5)]
    pub momentum: Option<u32>,
    /// The instant the two above were worked out for.
    pub computed_at: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// What a capture's deletion cost.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeletedCapture {
    pub id: CaptureId,
    /// The ideas that held it, so a caller can say which threads changed
    /// before refreshing them. Deliberately no other capture's text.
    pub ideas: Vec<AffectedIdea>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AffectedIdea {
    pub id: IdeaId,
    #[schema(example = "Dungeon seeds")]
    pub name: String,
    /// Whether it is now short of the evidence it rests on.
    pub needs_repair: bool,
}

/// What one capture might belong with, and why the analyzer thinks so.
///
/// Advisory and only advisory. Nothing here has created a connection, and
/// accepting one is a separate request the user makes.
#[derive(Debug, Serialize, ToSchema)]
pub struct CandidateResponse {
    pub capture: CaptureId,
    /// The analyzer that produced these. Changing tokenization, weighting, the
    /// threshold or how a centroid is built changes this string.
    #[schema(example = "tfidf/v1")]
    pub analyzer: String,
    /// How many of your captures the weights were computed over. This is the
    /// `N` in the idf, and it is your corpus alone.
    #[schema(example = 143)]
    pub corpus: usize,
    /// How many distinct terms this capture has. Zero means there was nothing
    /// to match on, which is not the same answer as nothing matched.
    #[schema(example = 11)]
    pub terms: usize,
    /// The similarity a candidate has to reach to be suggested at all.
    #[schema(example = 0.35)]
    pub threshold: f64,
    /// At most three, highest first.
    pub candidates: Vec<CandidateView>,
}

/// Whether a candidate suggests a thread that exists or another loose capture.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// Accepting appends one membership event.
    Idea,
    /// Accepting means naming a new idea holding both, which is how a thread
    /// comes to exist before there is a thread.
    Capture,
}

/// The thread a candidate points at.
#[derive(Debug, Serialize, ToSchema)]
pub struct IdeaTargetView {
    pub id: IdeaId,
    #[schema(example = "Dungeon seeds")]
    pub name: String,
    /// How many captures it currently holds.
    #[schema(example = 3)]
    pub captures: usize,
}

/// One suggestion.
#[derive(Debug, Serialize, ToSchema)]
pub struct CandidateView {
    pub kind: TargetKind,
    /// Present when `kind` is `idea`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idea: Option<IdeaTargetView>,
    /// Present when `kind` is `capture`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureView>,
    /// Lexical similarity between 0 and 1, rounded to six decimal places. This
    /// is how alike the words are, not a probability that the thoughts are
    /// related, and the interface has to say so.
    #[schema(example = 0.482913)]
    pub similarity: f64,
    /// The shared terms that produced it, biggest contribution first, at most
    /// five of them.
    pub signals: Vec<SignalView>,
    /// What the listed signals add up to. Below `similarity` when more than five
    /// terms were shared, which is the honest way to show five of them.
    #[schema(example = 0.44021)]
    pub explained: f64,
}

/// One term both records carry, and what it was worth.
#[derive(Debug, Serialize, ToSchema)]
pub struct SignalView {
    /// A word, or two adjacent words, appearing literally in both. Never a stem
    /// and never a synonym: every signal shown is text you wrote.
    #[schema(example = "dungeon seeds")]
    pub term: String,
    /// How many of your captures contain it. The input to its idf, and the
    /// reason a word you use constantly counts for less.
    #[schema(example = 4)]
    pub documents: usize,
    /// Its weight in this capture, as a component of a unit vector.
    #[schema(example = 0.51203)]
    pub capture_weight: f64,
    /// Its weight in the target.
    #[schema(example = 0.44107)]
    pub target_weight: f64,
    /// `capture_weight * target_weight`. These sum to the similarity.
    #[schema(example = 0.225837)]
    pub contribution: f64,
}

/// Why an idea is in the state it is in.
///
/// Everything needed to recompute `momentum` is here: the components, the
/// boundaries they were measured against, and every capture and event that was
/// counted with a flag saying which window it fell in. A reader should never
/// have to take the number on trust.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReceiptResponse {
    pub idea: IdeaId,
    #[schema(example = "Dungeon seeds")]
    pub name: String,
    /// The rules that produced this, and the only thing that changes it.
    #[schema(example = "idea-momentum/v1")]
    pub ruleset: String,
    /// The instant this is an answer about: your `at`, or the server's now.
    pub computed_at: DateTime<Utc>,
    pub boundaries: Boundaries,
    pub integrity: Integrity,
    /// Absent when the evidence is missing. There is then nothing to derive it
    /// from, and inventing one is the thing this feature must never do.
    pub state: Option<Lifecycle>,
    #[schema(example = 5)]
    pub momentum: Option<u32>,
    pub components: Option<Components>,
    pub last_signal: Option<DateTime<Utc>>,
    /// Every connected capture still readable, oldest first.
    pub captures: Vec<CountedCapture>,
    /// Every affirmation and reopening, in decision order. One that fell outside
    /// the window is listed too, flagged as not counted.
    pub affirmations: Vec<CountedAffirmation>,
    /// Connected captures whose files are gone.
    pub missing: Vec<CaptureId>,
    /// One sentence per line, each from a fixed template.
    pub explanation: Vec<String>,
}

/// The page an idea would make, assembled and not written.
///
/// Nothing here creates anything. Promotion is three steps and this is the
/// first: take the markdown, create an ordinary page with the ordinary page
/// API, then record the association with `PUT /api/ideas/{id}/promotion`.
#[derive(Debug, Serialize, ToSchema)]
pub struct DraftResponse {
    pub idea: IdeaId,
    /// What the page is proposed to be called: the idea's name, unchanged.
    /// Rhizolog does not generate one here any more than it does anywhere else.
    #[schema(example = "Dungeon seeds")]
    pub title: String,
    /// A heading, the idea's note, and every capture it still holds, oldest
    /// first, with the text exactly as it was typed.
    #[schema(example = "# Dungeon seeds\n\nMaybe dungeon quests should require seeds.\n")]
    pub markdown: String,
    /// The captures that went into it, in the order they appear. A connected
    /// capture whose text is blank is not one of them: it put no paragraph in
    /// the markdown, so there is nothing here for it to be the source of.
    pub sources: Vec<DraftSource>,
    /// Connected captures whose files are gone, and which therefore contributed
    /// nothing. Named rather than quietly left out, so a draft that is short of
    /// something says so.
    pub missing: Vec<CaptureId>,
    /// The page this idea has already produced, if it has been promoted before.
    pub promoted_to: Option<Slug>,
}

/// One capture the draft was assembled from.
#[derive(Debug, Serialize, ToSchema)]
pub struct DraftSource {
    pub id: CaptureId,
    pub created: DateTime<Utc>,
    /// Whether it has been archived out of the inbox. It is still evidence and
    /// still in the draft; this is here so a caller can say where a paragraph
    /// came from.
    pub archived: bool,
}

// ----------------------------------------------------------------- requests

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCapture {
    /// The thought, exactly as you want it kept. The only required field there
    /// is, and the only one refused when it is blank.
    #[schema(example = example_text)]
    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchCapture {
    /// Corrected text. `created` and the owner do not move.
    #[schema(example = example_text)]
    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateIdea {
    /// What to call it. Rhizolog never generates one.
    #[schema(example = "Dungeon seeds")]
    pub name: String,
    /// The captures it starts from. At least one, and all yours.
    ///
    /// This list is the thread's creation evidence and is immutable: what it
    /// holds later is a fold over the decisions taken about it.
    pub captures: Vec<CaptureId>,
    /// Optional working notes.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchIdea {
    /// A new name. Absent leaves it alone; blank is refused.
    #[serde(default)]
    #[schema(example = "Seeded dungeons")]
    pub name: Option<String>,
    /// New working notes. Absent leaves them alone.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CaptureListQuery {
    /// Narrow the inbox to captures matching these terms.
    ///
    /// A filter rather than a different view: results stay newest-first rather
    /// than being reordered by relevance, because an inbox is a thing you read
    /// in order. Terms are matched literally and combined with AND; a trailing
    /// `*` searches by prefix. Omit it entirely rather than sending `q=`.
    #[param(example = "dungeon seeds")]
    pub q: Option<String>,
    /// `false` for the live inbox, `true` for what has been archived out of it,
    /// absent for both.
    pub archived: Option<bool>,
    /// Only captures made at or after this instant.
    pub from: Option<DateTime<Utc>>,
    /// Only captures made at or before this instant.
    pub to: Option<DateTime<Utc>>,
    /// Defaults to 50, capped at 200.
    #[param(example = 50)]
    pub limit: Option<usize>,
    #[param(example = 0)]
    pub offset: Option<usize>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IdeaListQuery {
    /// Only ideas in this state: `retired`, `dormant`, `new`, `active` or
    /// `recurring`. Absent means every state.
    #[param(example = "active")]
    pub state: Option<String>,
    /// Only ideas whose evidence is `sound`, or only those with
    /// `evidence_missing`. The second is the Needs repair group.
    #[param(example = "sound")]
    pub integrity: Option<String>,
    /// Work the states out for this instant instead of now. The files do not
    /// change; what they add up to does.
    pub at: Option<DateTime<Utc>>,
    /// Defaults to 50, capped at 200.
    #[param(example = 50)]
    pub limit: Option<usize>,
    #[param(example = 0)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecordPromotion {
    /// The page this idea produced.
    ///
    /// It has to exist already and be one you can read. This records what
    /// happened; it does not write a page, and it does not move or copy one.
    #[schema(example = "notes/dungeon-seeds")]
    pub page: Slug,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReceiptQuery {
    /// Work the answer out for this instant instead of now. Nothing about the
    /// state is stored, so this is inspection rather than history.
    pub at: Option<DateTime<Utc>>,
}

// ------------------------------------------------------------------ helpers

fn capture_id(raw: &str) -> AppResult<CaptureId> {
    CaptureId::parse(raw).map_err(|source| AppError::InvalidRecordId {
        record: RecordKind::Capture,
        raw: raw.to_owned(),
        source,
    })
}

fn idea_id(raw: &str) -> AppResult<IdeaId> {
    IdeaId::parse(raw).map_err(|source| AppError::InvalidRecordId {
        record: RecordKind::Idea,
        raw: raw.to_owned(),
        source,
    })
}

fn page_of(limit: Option<usize>, offset: Option<usize>) -> (usize, usize) {
    (
        limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT),
        offset.unwrap_or(0),
    )
}

/// Read a `state=` filter, naming every value that would have worked.
fn lifecycle_filter(raw: Option<&str>) -> AppResult<Option<Lifecycle>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            Lifecycle::parse(value)
                .map(Some)
                .ok_or_else(|| AppError::InvalidParameter {
                    parameter: "state",
                    value: value.to_owned(),
                    allowed: Lifecycle::NAMES,
                })
        }
    }
}

fn integrity_filter(raw: Option<&str>) -> AppResult<Option<Integrity>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            Integrity::parse(value)
                .map(Some)
                .ok_or_else(|| AppError::InvalidParameter {
                    parameter: "integrity",
                    value: value.to_owned(),
                    allowed: Integrity::NAMES,
                })
        }
    }
}

/// Put a freshly written capture into the index, and say so plainly if it will
/// not go.
///
/// The authored file is already on disk at this point, which is why this is not
/// simply `?` on an index error: a caller told only "internal error" would retry
/// and write the capture a second time.
async fn index_capture(state: &AppState, capture: &Capture) -> AppResult<()> {
    state
        .index
        .upsert_capture(capture)
        .await
        .map_err(|source| AppError::WrittenButNotIndexed {
            what: "the capture",
            source,
        })
}

async fn index_idea(state: &AppState, idea: &Idea) -> AppResult<()> {
    state
        .index
        .upsert_idea(idea)
        .await
        .map_err(|source| AppError::WrittenButNotIndexed {
            what: "the idea",
            source,
        })
}

async fn index_event(state: &AppState, event: &Event) -> AppResult<()> {
    state
        .index
        .upsert_idea_event(event)
        .await
        .map_err(|source| AppError::WrittenButNotIndexed {
            what: "the decision",
            source,
        })
}

/// Append one decision and put it in the index.
async fn decide(
    state: &AppState,
    owner: &Owner,
    kind: EventKind,
    subject: Subject,
) -> AppResult<()> {
    let event = state.ideas.record(owner, kind, subject, Utc::now()).await?;
    index_event(state, &event).await
}

/// This owner's idea, folded, or a 404 that says nothing about whose it was.
async fn state_of(state: &AppState, owner: &Owner, id: &IdeaId) -> AppResult<IdeaState> {
    state.index.idea_state(owner, id).await?.ok_or_else(|| {
        AppError::Ideas(crate::ideas::IdeaServiceError::IdeaNotFound { id: id.clone() })
    })
}

/// This owner's capture, or the same 404.
async fn capture_of(state: &AppState, owner: &Owner, id: &CaptureId) -> AppResult<CaptureRecord> {
    state.index.capture(owner, id).await?.ok_or_else(|| {
        AppError::Ideas(crate::ideas::IdeaServiceError::CaptureNotFound { id: id.clone() })
    })
}

/// Work out where one of this owner's ideas stands at a given instant.
///
/// The index gathers the evidence and the pure rules read it. There is one
/// implementation of those rules and this is how everything reaches it, so the
/// number a listing shows and the number a receipt explains cannot drift apart.
async fn assess(
    state: &AppState,
    owner: &Owner,
    id: &IdeaId,
    at: DateTime<Utc>,
) -> AppResult<(IdeaSummary, Receipt)> {
    let found = state
        .index
        .idea_evidence(owner, Some(id))
        .await?
        .pop()
        .ok_or_else(|| {
            AppError::Ideas(crate::ideas::IdeaServiceError::IdeaNotFound { id: id.clone() })
        })?;

    let receipt = lifecycle::assess(&found.evidence, at);
    Ok((found.summary, receipt))
}

/// The working note from a thread's own file, or nothing if it will not read.
///
/// The one thing about an idea the index does not hold, so the two responses
/// that carry it pay for a file read. A file that cannot be read at this instant
/// costs the note rather than the request: everything else in the response came
/// from the index and is still true.
async fn note_of(state: &AppState, owner: &Owner, id: &IdeaId) -> String {
    match state.ideas.read_idea(owner, id).await {
        Ok(idea) => idea.note,
        Err(error) => {
            tracing::debug!(%id, %error, "could not read an idea's note");
            String::new()
        }
    }
}

/// Assemble the full view of one idea.
///
/// The folded state comes from the index, which is the read path everywhere in
/// Rhizolog. The note comes from the thread's own file, for the reason
/// [`note_of`] gives.
async fn idea_view(
    state: &AppState,
    owner: &Owner,
    folded: IdeaState,
    at: DateTime<Utc>,
) -> AppResult<IdeaView> {
    let captures = state.index.idea_captures(owner, &folded.id).await?;
    let (summary, receipt) = assess(state, owner, &folded.id, at).await?;
    let note = note_of(state, owner, &folded.id).await;

    Ok(IdeaView {
        needs_repair: receipt.needs_repair(),
        integrity: receipt.integrity,
        state: receipt.state,
        momentum: receipt.momentum,
        computed_at: receipt.computed_at,
        id: folded.id,
        name: folded.name,
        note,
        created: folded.created,
        captures: captures.into_iter().map(CaptureView::from).collect(),
        missing: folded.missing,
        rejected: folded.rejected,
        retired: folded.retired,
        promoted_to: folded.promoted_to,
        last_signal: folded.last_signal,
        dismissed: summary.dismissed,
        updated: folded.updated,
    })
}

/// Read the idea back and answer with it.
async fn answer_with_idea(
    state: &AppState,
    owner: &Owner,
    id: &IdeaId,
) -> AppResult<Json<IdeaView>> {
    let folded = state_of(state, owner, id).await?;
    Ok(Json(idea_view(state, owner, folded, Utc::now()).await?))
}

fn created<T: Serialize>(location: String, body: T) -> Response {
    let mut response = (StatusCode::CREATED, Json(body)).into_response();
    if let Ok(value) = HeaderValue::from_str(&location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

// ----------------------------------------------------------------- captures

/// The inbox, newest first.
#[utoipa::path(
    get,
    path = "/api/captures",
    tag = "ideas",
    params(CaptureListQuery),
    responses(
        (status = 200, description = "Matching captures", body = CaptureListResponse),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_captures(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<CaptureListQuery>,
) -> AppResult<Json<CaptureListResponse>> {
    let owner = viewer.owner()?;
    let (limit, offset) = page_of(query.limit, query.offset);

    let list = state
        .index
        .list_captures(
            &owner,
            CaptureListOptions {
                query: query.q,
                archived: query.archived,
                from: query.from,
                to: query.to,
                limit,
                offset,
            },
        )
        .await?;

    Ok(Json(CaptureListResponse {
        captures: list.captures.into_iter().map(CaptureView::from).collect(),
        total: list.total,
        limit,
        offset,
    }))
}

/// Save a thought.
///
/// The response carries no suggestions. Candidates are a separate request
/// against a capture that is already on disk, so nothing about analysis can lose
/// or reject the words somebody just typed.
#[utoipa::path(
    post,
    path = "/api/captures",
    tag = "ideas",
    request_body = CreateCapture,
    responses(
        (status = 201, description = "The capture as written", body = CaptureView),
        (status = 400, description = "There was no text", body = crate::error::ErrorResponse),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    JsonBody(request): JsonBody<CreateCapture>,
) -> AppResult<Response> {
    let owner = viewer.owner()?;

    let capture = state
        .ideas
        .capture(&owner, &request.text, Utc::now())
        .await?;
    index_capture(&state, &capture).await?;

    let location = format!("/api/captures/{}", capture.id);
    let view = CaptureView {
        id: capture.id,
        text: capture.body,
        created: capture.created,
        archived: false,
        updated: capture.updated,
        size: capture.size,
    };
    Ok(created(location, view))
}

/// Read one capture.
#[utoipa::path(
    get,
    path = "/api/captures/{id}",
    tag = "ideas",
    params(("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789")),
    responses(
        (status = 200, description = "The capture", body = CaptureView),
        (status = 400, description = "The id is not valid", body = crate::error::ErrorResponse),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<CaptureView>> {
    let owner = viewer.owner()?;
    let id = capture_id(&raw)?;

    Ok(Json(capture_of(&state, &owner, &id).await?.into()))
}

/// Correct a capture's text.
///
/// Its timestamp and its owner do not move: the first is what the lifecycle
/// rules count from, and git is the history of the edit.
#[utoipa::path(
    patch,
    path = "/api/captures/{id}",
    tag = "ideas",
    params(("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789")),
    request_body = PatchCapture,
    responses(
        (status = 200, description = "The corrected capture", body = CaptureView),
        (status = 400, description = "There was no text", body = crate::error::ErrorResponse),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn patch_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
    JsonBody(request): JsonBody<PatchCapture>,
) -> AppResult<Json<CaptureView>> {
    let owner = viewer.owner()?;
    let id = capture_id(&raw)?;

    let capture = state.ideas.edit_capture(&owner, &id, &request.text).await?;
    index_capture(&state, &capture).await?;

    Ok(Json(capture_of(&state, &owner, &id).await?.into()))
}

/// Delete a capture for good.
///
/// Refused with `409 capture_required_by_idea` while it is the only thing a
/// live idea still holds: the alternative is a thread with no authored evidence,
/// which cannot be given a lifecycle state at all. Connect something else or
/// retire the idea first.
///
/// The decision is recorded before the file goes, so an interrupted delete
/// leaves an event saying what was meant rather than a gap saying nothing. The
/// response names the threads that changed and carries no other capture's text.
#[utoipa::path(
    delete,
    path = "/api/captures/{id}",
    tag = "ideas",
    params(("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789")),
    responses(
        (status = 200, description = "Deleted, and what it cost", body = DeletedCapture),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
        (status = 409, description = "An idea has nothing else to stand on", body = crate::error::ErrorResponse),
    ),
)]
pub async fn delete_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<DeletedCapture>> {
    let owner = viewer.owner()?;
    let id = capture_id(&raw)?;
    capture_of(&state, &owner, &id).await?;

    let holds = state.index.ideas_holding(&owner, &id).await?;
    if let Some(hold) = holds.iter().find(|hold| hold.depends_on_it()) {
        return Err(AppError::CaptureRequiredByIdea {
            id,
            idea: hold.id.clone(),
            name: hold.name.clone(),
        });
    }

    decide(
        &state,
        &owner,
        EventKind::CaptureDeleted,
        Subject::Capture {
            capture: id.clone(),
        },
    )
    .await?;

    state.ideas.delete_capture(&owner, &id).await?;
    state
        .index
        .remove_capture(&id)
        .await
        .map_err(|source| AppError::WrittenButNotIndexed {
            what: "the deletion",
            source,
        })?;

    let mut ideas = Vec::new();
    for hold in holds {
        let folded = state.index.idea_state(&owner, &hold.id).await?;
        ideas.push(AffectedIdea {
            needs_repair: folded.is_some_and(|state| state.evidence_missing()),
            id: hold.id,
            name: hold.name,
        });
    }

    Ok(Json(DeletedCapture { id, ideas }))
}

/// Archive a capture out of the inbox.
///
/// Processed, not erased: it stays connected to whatever ideas hold it and
/// stays available as evidence. Archiving one that is already archived writes no
/// second event.
#[utoipa::path(
    post,
    path = "/api/captures/{id}/archive",
    tag = "ideas",
    params(("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789")),
    responses(
        (status = 200, description = "The archived capture", body = CaptureView),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn archive_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<CaptureView>> {
    set_archived(state, viewer, raw, true).await
}

/// Put an archived capture back in the inbox.
#[utoipa::path(
    post,
    path = "/api/captures/{id}/restore",
    tag = "ideas",
    params(("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789")),
    responses(
        (status = 200, description = "The restored capture", body = CaptureView),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn restore_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<CaptureView>> {
    set_archived(state, viewer, raw, false).await
}

async fn set_archived(
    state: AppState,
    viewer: Viewer,
    raw: String,
    archived: bool,
) -> AppResult<Json<CaptureView>> {
    let owner = viewer.owner()?;
    let id = capture_id(&raw)?;
    let capture = capture_of(&state, &owner, &id).await?;

    if capture.archived != archived {
        let kind = if archived {
            EventKind::CaptureArchived
        } else {
            EventKind::CaptureRestored
        };
        decide(
            &state,
            &owner,
            kind,
            Subject::Capture {
                capture: id.clone(),
            },
        )
        .await?;
    }

    Ok(Json(capture_of(&state, &owner, &id).await?.into()))
}

/// Turn down a suggestion that two loose captures belong together.
///
/// The pair is held in one canonical order, so rejecting `a` against `b` and
/// `b` against `a` are the same decision and the second writes no event.
#[utoipa::path(
    put,
    path = "/api/captures/{id}/rejections/{other_id}",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789"),
        ("other_id" = String, Path, description = "The capture it was suggested with", example = "20260820T142000-234567890"),
    ),
    responses(
        (status = 204, description = "Rejected"),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reject_capture_pair(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((raw, other_raw)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    set_pair_rejected(state, viewer, raw, other_raw, true).await
}

/// Let a rejected pair be suggested again.
#[utoipa::path(
    delete,
    path = "/api/captures/{id}/rejections/{other_id}",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789"),
        ("other_id" = String, Path, description = "The capture it was suggested with", example = "20260820T142000-234567890"),
    ),
    responses(
        (status = 204, description = "Reconsidered"),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reconsider_capture_pair(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((raw, other_raw)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    set_pair_rejected(state, viewer, raw, other_raw, false).await
}

async fn set_pair_rejected(
    state: AppState,
    viewer: Viewer,
    raw: String,
    other_raw: String,
    rejected: bool,
) -> AppResult<StatusCode> {
    let owner = viewer.owner()?;
    let first = capture_id(&raw)?;
    let second = capture_id(&other_raw)?;

    let pair = Subject::pair(first, second);
    let already = state
        .index
        .rejected_capture_pairs(&owner)
        .await?
        .into_iter()
        .any(|(a, b)| Subject::pair(a, b) == pair);

    if already != rejected {
        let kind = if rejected {
            EventKind::CandidateRejected
        } else {
            EventKind::CandidateReconsidered
        };
        decide(&state, &owner, kind, pair).await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// What this capture might belong with, and why.
///
/// Scored against every non-retired idea and every capture of yours that
/// belongs to none, over a corpus that is yours alone. A candidate carries the
/// shared terms that produced it and what each was worth, so the number can be
/// checked rather than believed.
///
/// This creates nothing. Every candidate is a question, and the answer is
/// `PUT .../captures/...` to connect, `POST /api/ideas` to name a new thread
/// from two loose captures, or `PUT .../rejections/...` to say no and not be
/// asked again.
///
/// A capture with no terms at all, such as one whose text is punctuation, comes
/// back with `terms: 0` and no candidates. That is a different answer from
/// nothing having matched and the response says which it is.
#[utoipa::path(
    get,
    path = "/api/captures/{id}/candidates",
    tag = "ideas",
    params(("id" = String, Path, description = "Capture id", example = "20260820T141530-123456789")),
    responses(
        (status = 200, description = "What it might belong with", body = CandidateResponse),
        (status = 404, description = "No such capture", body = crate::error::ErrorResponse),
        (status = 503, description = "The analyzer has no terms for it; reindex", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_capture_candidates(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<CandidateResponse>> {
    let owner = viewer.owner()?;
    let id = capture_id(&raw)?;
    capture_of(&state, &owner, &id).await?;

    let field = state.index.candidate_field(&owner).await?;
    // The capture is indexed and the corpus is not: the two reads happened at
    // different moments and something moved between them. Derived and
    // retryable, and saying so is more use than a generic failure.
    if !field.corpus.contains(&id) {
        return Err(AppError::IdeaAnalysisUnavailable { id });
    }

    let found = analysis::candidates(&field, &id);
    let mut candidates = Vec::with_capacity(found.len());
    for candidate in found {
        let signals: Vec<SignalView> = candidate
            .signals
            .into_iter()
            .map(|signal| SignalView {
                documents: field.corpus.frequency(&signal.term),
                term: signal.term,
                capture_weight: rounded(signal.capture_weight),
                target_weight: rounded(signal.target_weight),
                contribution: rounded(signal.contribution),
            })
            .collect();
        // Summed from the rounded contributions rather than the exact ones, so
        // that a reader adding up the numbers in front of them arrives at this
        // number and not one two millionths away from it.
        let explained = rounded(signals.iter().map(|signal| signal.contribution).sum());

        let (kind, idea, capture) = match candidate.target {
            Target::Idea { id, name, members } => (
                TargetKind::Idea,
                Some(IdeaTargetView {
                    id,
                    name,
                    captures: members,
                }),
                None,
            ),
            // Read back for its text, which the interface has to show before it
            // can ask whether the two belong together. It is the caller's own
            // capture, and it was reached through the owner-scoped read.
            Target::Capture { id } => (
                TargetKind::Capture,
                None,
                state
                    .index
                    .capture(&owner, &id)
                    .await?
                    .map(CaptureView::from),
            ),
        };

        candidates.push(CandidateView {
            kind,
            idea,
            capture,
            similarity: rounded(candidate.similarity),
            signals,
            explained,
        });
    }

    Ok(Json(CandidateResponse {
        analyzer: analysis::ANALYZER.to_owned(),
        corpus: field.corpus.len(),
        terms: field.corpus.distinct_terms(&id),
        threshold: analysis::THRESHOLD,
        capture: id,
        candidates,
    }))
}

// -------------------------------------------------------------------- ideas

/// Idea threads, most recently active first.
///
/// `state` and `integrity` narrow the list by values that are computed rather
/// than stored, so `total` counts what matched and paging through it reaches
/// every one of them.
#[utoipa::path(
    get,
    path = "/api/ideas",
    tag = "ideas",
    params(IdeaListQuery),
    responses(
        (status = 200, description = "The ideas", body = IdeaListResponse),
        (status = 400, description = "No such state or integrity", body = crate::error::ErrorResponse),
        (status = 401, description = "This wiki requires authentication", body = crate::error::ErrorResponse),
    ),
)]
pub async fn list_ideas(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<IdeaListQuery>,
) -> AppResult<Json<IdeaListResponse>> {
    let owner = viewer.owner()?;
    let (limit, offset) = page_of(query.limit, query.offset);
    let at = query.at.unwrap_or_else(Utc::now);

    let list = state
        .index
        .list_ideas(
            &owner,
            IdeaListOptions {
                state: lifecycle_filter(query.state.as_deref())?,
                integrity: integrity_filter(query.integrity.as_deref())?,
                at,
                limit,
                offset,
            },
        )
        .await?;

    Ok(Json(IdeaListResponse {
        ideas: list.ideas.into_iter().map(IdeaSummaryView::from).collect(),
        total: list.total,
        limit,
        offset,
        at,
        ruleset: lifecycle::RULESET.to_owned(),
    }))
}

/// Start a named thread from captures you already have.
///
/// One authored file write, seeds and all, so an idea either exists with the
/// grouping you made or does not exist at all.
#[utoipa::path(
    post,
    path = "/api/ideas",
    tag = "ideas",
    request_body = CreateIdea,
    responses(
        (status = 201, description = "The idea as created", body = IdeaView),
        (status = 400, description = "No name, or no captures", body = crate::error::ErrorResponse),
        (status = 404, description = "One of the captures does not exist", body = crate::error::ErrorResponse),
    ),
)]
pub async fn create_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    JsonBody(request): JsonBody<CreateIdea>,
) -> AppResult<Response> {
    let owner = viewer.owner()?;

    let idea = state
        .ideas
        .start_idea(
            &owner,
            &request.name,
            &request.captures,
            request.note.as_deref().unwrap_or_default(),
            Utc::now(),
        )
        .await?;
    index_idea(&state, &idea).await?;

    let location = format!("/api/ideas/{}", idea.id);
    let folded = state_of(&state, &owner, &idea.id).await?;
    Ok(created(
        location,
        idea_view(&state, &owner, folded, Utc::now()).await?,
    ))
}

/// One idea, with what it holds.
#[utoipa::path(
    get,
    path = "/api/ideas/{id}",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    responses(
        (status = 200, description = "The idea", body = IdeaView),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    answer_with_idea(&state, &owner, &idea_id(&raw)?).await
}

/// Rename an idea or edit its note.
///
/// Its seeds and its creation time do not move: they are what the thread was
/// started from, and that already happened.
#[utoipa::path(
    patch,
    path = "/api/ideas/{id}",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    request_body = PatchIdea,
    responses(
        (status = 200, description = "The idea", body = IdeaView),
        (status = 400, description = "The name was blank", body = crate::error::ErrorResponse),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
    ),
)]
pub async fn patch_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
    JsonBody(request): JsonBody<PatchIdea>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let id = idea_id(&raw)?;

    let idea = state
        .ideas
        .edit_idea(
            &owner,
            &id,
            request.name.as_deref(),
            request.note.as_deref(),
        )
        .await?;
    index_idea(&state, &idea).await?;

    answer_with_idea(&state, &owner, &id).await
}

/// Connect a capture to an idea.
#[utoipa::path(
    put,
    path = "/api/ideas/{id}/captures/{capture_id}",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890"),
        ("capture_id" = String, Path, description = "Capture id", example = "20260820T141530-123456789"),
    ),
    responses(
        (status = 200, description = "The idea, with the capture connected", body = IdeaView),
        (status = 404, description = "No such idea or capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn connect_idea_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((raw, capture_raw)): Path<(String, String)>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;
    let capture = capture_id(&capture_raw)?;

    let folded = state_of(&state, &owner, &idea).await?;
    let connected = folded.members.contains(&capture) || folded.missing.contains(&capture);

    if !connected {
        decide(
            &state,
            &owner,
            EventKind::CaptureConnected,
            Subject::IdeaCapture {
                idea: idea.clone(),
                capture,
            },
        )
        .await?;
    }

    answer_with_idea(&state, &owner, &idea).await
}

/// Disconnect a capture from an idea.
///
/// Refused with `409 idea_would_be_empty` when it is the last one. Retiring the
/// idea is the reversible way to set it aside.
#[utoipa::path(
    delete,
    path = "/api/ideas/{id}/captures/{capture_id}",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890"),
        ("capture_id" = String, Path, description = "Capture id", example = "20260820T141530-123456789"),
    ),
    responses(
        (status = 200, description = "The idea, with the capture disconnected", body = IdeaView),
        (status = 404, description = "No such idea or capture", body = crate::error::ErrorResponse),
        (status = 409, description = "It was the idea's last capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn disconnect_idea_capture(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((raw, capture_raw)): Path<(String, String)>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;
    let capture = capture_id(&capture_raw)?;

    let folded = state_of(&state, &owner, &idea).await?;
    let connected = folded.members.contains(&capture) || folded.missing.contains(&capture);

    if connected {
        if folded.members.len() + folded.missing.len() <= 1 {
            return Err(AppError::IdeaWouldBeEmpty { id: idea });
        }
        decide(
            &state,
            &owner,
            EventKind::CaptureDisconnected,
            Subject::IdeaCapture {
                idea: idea.clone(),
                capture,
            },
        )
        .await?;
    }

    answer_with_idea(&state, &owner, &idea).await
}

/// Turn down a suggestion that a capture belongs to an idea.
#[utoipa::path(
    put,
    path = "/api/ideas/{id}/rejections/{capture_id}",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890"),
        ("capture_id" = String, Path, description = "Capture id", example = "20260820T141530-123456789"),
    ),
    responses(
        (status = 200, description = "The idea", body = IdeaView),
        (status = 404, description = "No such idea or capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reject_idea_candidate(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((raw, capture_raw)): Path<(String, String)>,
) -> AppResult<Json<IdeaView>> {
    set_candidate_rejected(state, viewer, raw, capture_raw, true).await
}

/// Let a rejected candidate be suggested again.
#[utoipa::path(
    delete,
    path = "/api/ideas/{id}/rejections/{capture_id}",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890"),
        ("capture_id" = String, Path, description = "Capture id", example = "20260820T141530-123456789"),
    ),
    responses(
        (status = 200, description = "The idea", body = IdeaView),
        (status = 404, description = "No such idea or capture", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reconsider_idea_candidate(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((raw, capture_raw)): Path<(String, String)>,
) -> AppResult<Json<IdeaView>> {
    set_candidate_rejected(state, viewer, raw, capture_raw, false).await
}

async fn set_candidate_rejected(
    state: AppState,
    viewer: Viewer,
    raw: String,
    capture_raw: String,
    rejected: bool,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;
    let capture = capture_id(&capture_raw)?;

    let folded = state_of(&state, &owner, &idea).await?;
    if folded.rejected.contains(&capture) != rejected {
        let kind = if rejected {
            EventKind::CandidateRejected
        } else {
            EventKind::CandidateReconsidered
        };
        decide(
            &state,
            &owner,
            kind,
            Subject::IdeaCapture {
                idea: idea.clone(),
                capture,
            },
        )
        .await?;
    }

    answer_with_idea(&state, &owner, &idea).await
}

/// Say that an idea still interests you.
///
/// An act rather than a state, so it is meaningful every time and writes an
/// event every time. It is what a dormant idea's rediscovery card asks for.
#[utoipa::path(
    post,
    path = "/api/ideas/{id}/affirm",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    responses(
        (status = 200, description = "The idea", body = IdeaView),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
    ),
)]
pub async fn affirm_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;
    state_of(&state, &owner, &idea).await?;

    decide(
        &state,
        &owner,
        EventKind::InterestAffirmed,
        Subject::Idea { idea: idea.clone() },
    )
    .await?;

    answer_with_idea(&state, &owner, &idea).await
}

/// Set an idea aside without deleting anything.
#[utoipa::path(
    post,
    path = "/api/ideas/{id}/retire",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    responses(
        (status = 200, description = "The retired idea", body = IdeaView),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
        (status = 409, description = "It is already retired", body = crate::error::ErrorResponse),
    ),
)]
pub async fn retire_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;

    if state_of(&state, &owner, &idea).await?.retired {
        return Err(AppError::IdeaAlreadyRetired { id: idea });
    }

    decide(
        &state,
        &owner,
        EventKind::IdeaRetired,
        Subject::Idea { idea: idea.clone() },
    )
    .await?;

    answer_with_idea(&state, &owner, &idea).await
}

/// Bring a retired idea back.
#[utoipa::path(
    post,
    path = "/api/ideas/{id}/reopen",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    responses(
        (status = 200, description = "The reopened idea", body = IdeaView),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
        (status = 409, description = "It is not retired", body = crate::error::ErrorResponse),
    ),
)]
pub async fn reopen_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;

    if !state_of(&state, &owner, &idea).await?.retired {
        return Err(AppError::IdeaNotRetired { id: idea });
    }

    decide(
        &state,
        &owner,
        EventKind::IdeaReopened,
        Subject::Idea { idea: idea.clone() },
    )
    .await?;

    answer_with_idea(&state, &owner, &idea).await
}

/// Stop an idea being resurfaced for a while.
///
/// Dismissing a rediscovery card is not the same as saying the idea is over,
/// which is what retiring says. This one only quietens it.
#[utoipa::path(
    post,
    path = "/api/ideas/{id}/dismiss",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    responses(
        (status = 200, description = "The idea", body = IdeaView),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
    ),
)]
pub async fn dismiss_idea(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;
    state_of(&state, &owner, &idea).await?;

    decide(
        &state,
        &owner,
        EventKind::RediscoveryDismissed,
        Subject::Idea { idea: idea.clone() },
    )
    .await?;

    answer_with_idea(&state, &owner, &idea).await
}

/// Why an idea is in the state it is in.
///
/// The components, the window boundaries they were measured against, and every
/// capture and event that was counted with a flag saying which window it fell
/// in. Nothing here is stored: the same files answer differently tomorrow, which
/// is what `at` exists to demonstrate.
///
/// An idea whose captures have been deleted from underneath it comes back with
/// `integrity: evidence_missing`, the ids it lost, and no state and no momentum
/// at all. That is deliberate. There is nothing left to derive them from, and
/// deriving them anyway is the one thing this feature must not do.
#[utoipa::path(
    get,
    path = "/api/ideas/{id}/receipt",
    tag = "ideas",
    params(
        ("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890"),
        ReceiptQuery,
    ),
    responses(
        (status = 200, description = "The receipt", body = ReceiptResponse),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_idea_receipt(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
    Query(query): Query<ReceiptQuery>,
) -> AppResult<Json<ReceiptResponse>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;

    let (summary, receipt) =
        assess(&state, &owner, &idea, query.at.unwrap_or_else(Utc::now)).await?;

    Ok(Json(ReceiptResponse {
        idea: summary.id,
        name: summary.name,
        ruleset: receipt.ruleset.to_owned(),
        computed_at: receipt.computed_at,
        boundaries: receipt.boundaries,
        integrity: receipt.integrity,
        state: receipt.state,
        momentum: receipt.momentum,
        components: receipt.components,
        last_signal: receipt.last_signal,
        captures: receipt.captures,
        affirmations: receipt.affirmations,
        missing: receipt.missing,
        explanation: receipt.explanation,
    }))
}

// ---------------------------------------------------------------- promotion

/// Whether a capture has anything to put in a draft.
///
/// A capture whose text is entirely whitespace contributes no paragraph, and it
/// must not be named as the source of one either. `sources` says what the
/// markdown was assembled from, in the order it appears, so a client walking the
/// two together to find out where a paragraph came from would be off by one for
/// every blank left in the list. One predicate, asked by both, rather than two
/// places that happen to agree today.
///
/// Only a hand-written file gets here: creating and correcting a capture both
/// refuse blank text. That is why a blank is quietly left out rather than
/// reported. `missing` is for evidence that cannot be read at all, which is a
/// different thing and worth saying out loud; this one is readable and simply
/// says nothing.
fn contributes(capture: &CaptureRecord) -> bool {
    !capture.body.trim().is_empty()
}

/// Assemble the page an idea would make.
///
/// A heading, the thread's note, then every capture it holds, oldest first,
/// separated by blank lines. The text is copied and never rewritten,
/// summarised, reordered or interpreted: what comes out is what somebody wrote,
/// and a draft that improved on it would be the first place that stopped being
/// true.
///
/// Nothing is added to say where a paragraph came from. Provenance belongs in
/// the response, which carries the ids and the timestamps, rather than in prose
/// somebody would have to delete out of their own page.
fn draft_markdown(name: &str, note: &str, captures: &[CaptureRecord]) -> String {
    let mut blocks: Vec<&str> = Vec::with_capacity(captures.len() + 1);

    let note = note.trim();
    if !note.is_empty() {
        blocks.push(note);
    }
    for capture in captures {
        if contributes(capture) {
            blocks.push(capture.body.trim());
        }
    }

    let heading = format!("# {}\n", name.trim());
    if blocks.is_empty() {
        heading
    } else {
        format!("{heading}\n{}\n", blocks.join("\n\n"))
    }
}

/// The page this idea would make, assembled and not written.
///
/// Promotion is deliberately three steps and this is the first: read the draft,
/// create an ordinary page with `POST /api/pages`, then record the association
/// with `PUT /api/ideas/{id}/promotion`. Two authored writes and two index
/// updates are not one transaction and this API does not pretend otherwise. The
/// good news is what that buys: if the third step fails, the page still exists
/// and the association can be recorded again without losing anything.
///
/// Every capture the idea still holds is here, and the ones whose files are gone
/// are named in `missing` so that a short draft says it is short. A capture that
/// is readable and blank is in neither list: it put nothing in the markdown, and
/// `sources` names what the markdown was made of.
#[utoipa::path(
    get,
    path = "/api/ideas/{id}/draft",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    responses(
        (status = 200, description = "The draft", body = DraftResponse),
        (status = 404, description = "No such idea", body = crate::error::ErrorResponse),
    ),
)]
pub async fn read_idea_draft(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<DraftResponse>> {
    let owner = viewer.owner()?;
    let id = idea_id(&raw)?;

    let folded = state_of(&state, &owner, &id).await?;
    // Filtered once and used for both, so the markdown and the list of what it
    // was made from cannot disagree about what a capture contributed.
    let captures: Vec<CaptureRecord> = state
        .index
        .idea_captures(&owner, &id)
        .await?
        .into_iter()
        .filter(contributes)
        .collect();
    let note = note_of(&state, &owner, &id).await;

    Ok(Json(DraftResponse {
        markdown: draft_markdown(&folded.name, &note, &captures),
        title: folded.name,
        sources: captures
            .into_iter()
            .map(|capture| DraftSource {
                id: capture.id,
                created: capture.created,
                archived: capture.archived,
            })
            .collect(),
        missing: folded.missing,
        promoted_to: folded.promoted_to,
        idea: folded.id,
    }))
}

/// Record the page an idea produced.
///
/// The page has to exist and be one you can read. A slug that is neither gets
/// `404 idea_promotion_page_not_found`, and it is the same answer either way:
/// telling a caller that a page exists but is not theirs would answer questions
/// about somebody else's wiki for the price of guessing a slug.
///
/// Idempotent. Recording the slug an idea is already promoted to writes no
/// second event, which is what makes this safe to retry when the page was
/// created and the association was not. Recording a *different* slug writes a
/// new event and makes that the current answer, while the earlier association
/// stays in the event log: an idea that became one page and then another has
/// done both of those things.
///
/// Nothing about the captures changes. Promotion is a thing that happened to an
/// idea, not a way of consuming one, and the thread keeps every capture it held
/// so that the page's sources can still be read.
#[utoipa::path(
    put,
    path = "/api/ideas/{id}/promotion",
    tag = "ideas",
    params(("id" = String, Path, description = "Idea id", example = "20260820T142000-234567890")),
    request_body = RecordPromotion,
    responses(
        (status = 200, description = "The idea, with the page recorded", body = IdeaView),
        (status = 400, description = "That is not a slug", body = crate::error::ErrorResponse),
        (status = 404, description = "No such idea, or no such page", body = crate::error::ErrorResponse),
    ),
)]
pub async fn record_idea_promotion(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(raw): Path<String>,
    JsonBody(request): JsonBody<RecordPromotion>,
) -> AppResult<Json<IdeaView>> {
    let owner = viewer.owner()?;
    let idea = idea_id(&raw)?;
    let folded = state_of(&state, &owner, &idea).await?;

    // Read from the file rather than asked of the index, which is what
    // `GET /api/pages/{slug}` does and for the same reason: a page whose
    // frontmatter changed a moment ago must not be judged by what it used to
    // say. A page that is missing and a page that is not this caller's are one
    // answer here.
    let page = match state.store.read(&request.page).await {
        Ok(page) => page,
        Err(StoreError::NotFound { .. }) => {
            return Err(AppError::IdeaPromotionPageNotFound { slug: request.page });
        }
        // A page that will not parse is a different problem and says so, which
        // discloses nothing `GET /api/pages/{slug}` does not already: that read
        // reports a malformed page before it ever consults its visibility.
        Err(error) => return Err(error.into()),
    };
    if !crate::api::pages::readable(&page, &viewer) {
        return Err(AppError::IdeaPromotionPageNotFound { slug: request.page });
    }

    if folded.promoted_to.as_ref() != Some(&request.page) {
        decide(
            &state,
            &owner,
            EventKind::IdeaPromoted,
            Subject::Promotion {
                idea: idea.clone(),
                page: request.page,
            },
        )
        .await?;
    }

    answer_with_idea(&state, &owner, &idea).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(body: &str) -> CaptureRecord {
        CaptureRecord {
            id: CaptureId::parse("20260820T141530-123456789").expect("valid id"),
            owner: Owner::open(),
            body: body.to_owned(),
            archived: false,
            created: DateTime::UNIX_EPOCH,
            updated: DateTime::UNIX_EPOCH,
            size: body.len() as u64,
        }
    }

    /// A capture's body arrives with the blank line the frontmatter left behind
    /// and whatever trailing newline the file had, and neither belongs in a page.
    /// What is between them is untouched.
    #[test]
    fn a_draft_is_a_heading_and_the_captures_verbatim() {
        let markdown = draft_markdown(
            "Dungeon seeds",
            "",
            &[
                capture("\nSeeds should decide the loot.\n"),
                capture("\nAnd the corridors.\n\nEven the doors.\n"),
            ],
        );

        assert_eq!(
            markdown,
            "# Dungeon seeds\n\nSeeds should decide the loot.\n\nAnd the corridors.\n\nEven the \
             doors.\n"
        );
    }

    #[test]
    fn a_note_goes_above_the_captures_and_an_empty_one_leaves_no_gap() {
        assert_eq!(
            draft_markdown(
                "Dungeon seeds",
                "\nWhat this is about.\n",
                &[capture("A thought.\n")]
            ),
            "# Dungeon seeds\n\nWhat this is about.\n\nA thought.\n"
        );
        assert_eq!(
            draft_markdown("Dungeon seeds", "   \n", &[capture("A thought.\n")]),
            "# Dungeon seeds\n\nA thought.\n"
        );
    }

    /// An idea whose captures have all been deleted still drafts, because the
    /// name is authored too. It comes back as a heading and nothing else rather
    /// than as prose apologising for itself.
    #[test]
    fn a_draft_with_nothing_left_to_say_is_just_the_heading() {
        assert_eq!(
            draft_markdown("Dungeon seeds", "", &[]),
            "# Dungeon seeds\n"
        );
    }

    /// The predicate the markdown and `sources` are both filtered by. A blank
    /// capture leaves no paragraph and no gap where one would have been, and
    /// the handler drops it from the list of what the draft was made of for
    /// the same reason and by the same test.
    #[test]
    fn a_capture_that_says_nothing_contributes_nothing() {
        assert!(contributes(&capture("A thought.\n")));
        assert!(!contributes(&capture("\n   \n")));
        assert!(!contributes(&capture("")));

        assert_eq!(
            draft_markdown(
                "Dungeon seeds",
                "",
                &[
                    capture("Seeds should decide the loot.\n"),
                    capture("\n   \n"),
                    capture("And the corridors.\n"),
                ]
            ),
            "# Dungeon seeds\n\nSeeds should decide the loot.\n\nAnd the corridors.\n"
        );
    }
}
