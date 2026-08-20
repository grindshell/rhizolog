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
use crate::ideas::{
    Capture, CaptureId, Event, EventKind, Idea, IdeaId, Owner, RecordKind, Subject,
};
use crate::index::{CaptureListOptions, CaptureRecord, IdeaState, IdeaSummary};
use crate::slug::Slug;

const DEFAULT_LIMIT: usize = 50;

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
    /// Whether the idea has lost the evidence it rests on.
    ///
    /// A thread with nothing live connected cannot be given a lifecycle state,
    /// because there is no authored text left to derive one from. It is reported
    /// rather than papered over.
    pub needs_repair: bool,
    pub updated: DateTime<Utc>,
}

impl From<IdeaSummary> for IdeaSummaryView {
    fn from(summary: IdeaSummary) -> Self {
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
            updated: summary.updated,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IdeaListResponse {
    /// Most recently active first.
    pub ideas: Vec<IdeaSummaryView>,
    #[schema(example = 7)]
    pub total: usize,
    #[schema(example = 50)]
    pub limit: usize,
    #[schema(example = 0)]
    pub offset: usize,
}

/// One idea thread, with what it holds.
///
/// No lifecycle label and no momentum score: both are pure functions of this and
/// an explicit moment, and neither exists until the analyzer does. What is here
/// is the folded evidence either would be computed from.
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
    pub needs_repair: bool,
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
    /// Defaults to 50, capped at 200.
    #[param(example = 50)]
    pub limit: Option<usize>,
    #[param(example = 0)]
    pub offset: Option<usize>,
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

/// Assemble the full view of one idea.
///
/// The folded state comes from the index, which is the read path everywhere in
/// Rhizolog. The note comes from the thread's own file, because it is the one
/// thing about an idea the index does not hold; a file that cannot be read at
/// this instant costs the note rather than the request.
async fn idea_view(state: &AppState, owner: &Owner, folded: IdeaState) -> AppResult<IdeaView> {
    let captures = state.index.idea_captures(owner, &folded.id).await?;

    let note = match state.ideas.read_idea(owner, &folded.id).await {
        Ok(idea) => idea.note,
        Err(error) => {
            tracing::debug!(id = %folded.id, %error, "could not read an idea's note");
            String::new()
        }
    };

    Ok(IdeaView {
        needs_repair: folded.evidence_missing(),
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
    Ok(Json(idea_view(state, owner, folded).await?))
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

// -------------------------------------------------------------------- ideas

/// Idea threads, most recently active first.
#[utoipa::path(
    get,
    path = "/api/ideas",
    tag = "ideas",
    params(IdeaListQuery),
    responses(
        (status = 200, description = "The ideas", body = IdeaListResponse),
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

    let list = state.index.list_ideas(&owner, limit, offset).await?;

    Ok(Json(IdeaListResponse {
        ideas: list.ideas.into_iter().map(IdeaSummaryView::from).collect(),
        total: list.total,
        limit,
        offset,
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
    Ok(created(location, idea_view(&state, &owner, folded).await?))
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
