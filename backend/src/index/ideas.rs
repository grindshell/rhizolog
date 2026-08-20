//! The index over Idea Inbox.
//!
//! Everything here is derived from `.rhizolog/ideas/`, so it is as disposable as
//! the rest of the index: drop the database and the next scan rebuilds it from
//! the files. Nothing an idea knows about itself is stored anywhere else.
//!
//! ## Three tables mirror files, six are folded
//!
//! `idea_captures`, `idea_threads` and `idea_events` are one row per authored
//! file, written when that file is read. The other six hold what the decisions
//! *add up to*: current membership, rejections, archive state, retirement,
//! promotion and last signal.
//!
//! The fold is written in SQL over `idea_seed_captures` and `idea_events`, and
//! that is the whole reason a rebuild and an incremental update agree. It is not
//! two algorithms that have to be kept in step; it is one query, run over
//! whichever rows are present, for whichever keys just changed. Reindexing one
//! event and rebuilding from an empty database run exactly the same statements.
//!
//! Every one of them ends in `order by id desc limit 1`, which is "the latest
//! applicable decision wins" spelled in SQL. Event ids sort chronologically as
//! text, so ordering by id is ordering by when the decision was taken.
//!
//! ## Deleted evidence is reported, not erased
//!
//! `idea_membership` and `idea_seed_captures` name captures without a foreign
//! key to them. That is deliberate and load-bearing: a capture can be deleted
//! while an idea still says it holds one, and a cascade would tidy away exactly
//! the symptom the user needs to see. Live membership is `idea_membership`
//! joined to `idea_captures`; whatever the join drops is the missing evidence,
//! and an idea left with none of it is the `evidence_missing` diagnostic.
//!
//! ## One owner clause, pasted everywhere
//!
//! Idea Inbox is personal working state, so every read takes an [`Owner`] and
//! filters in SQL before returning text, counts or ids. See [`OWNED`] for why
//! the comparison is `is` rather than `=`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, named_params, params};

use crate::ideas::{Capture, CaptureId, Event, EventId, Idea, IdeaId, Owner};
use crate::index::{Index, IndexError, Stamp, from_nanos, to_fts_query, to_nanos};
use crate::slug::Slug;
use crate::users::Username;

/// The clause every idea query pastes in.
///
/// `is` rather than `=`, and that is not a style choice. An open wiki's records
/// have a null owner, and in SQL `null = null` is null, which a `where` clause
/// reads as false: under `=` the open user would never match their own captures.
/// `is` compares nulls as equal, so the open user matches exactly the open
/// records and an account matches exactly its own. See
/// `knowledge-base/idea-inbox.md`.
pub const OWNED: &str = "owner is :owner";

/// A capture as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRecord {
    pub id: CaptureId,
    pub owner: Owner,
    pub created: DateTime<Utc>,
    /// The text, read back out of the full-text table that stores it.
    pub body: String,
    /// Whether it has been archived out of the inbox. Archived is "processed",
    /// not "this thought never happened": it still counts as evidence.
    pub archived: bool,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

/// What the fold decided about one idea.
///
/// No lifecycle label and no momentum score. Both are pure functions of this and
/// an explicit `at`, so the index would be storing an answer to a question
/// nobody had asked yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaState {
    pub id: IdeaId,
    pub owner: Owner,
    pub name: String,
    pub created: DateTime<Utc>,
    /// Currently connected captures whose files are still there, oldest first.
    pub members: Vec<CaptureId>,
    /// Currently connected captures whose files are gone. Evidence a receipt
    /// has to name rather than quietly attribute text to.
    pub missing: Vec<CaptureId>,
    /// Captures rejected as candidates for this idea, so they are not suggested
    /// again until somebody reconsiders.
    pub rejected: Vec<CaptureId>,
    pub retired: bool,
    /// The page this idea produced, if it has been promoted.
    pub promoted_to: Option<Slug>,
    pub last_signal: Option<DateTime<Utc>>,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

/// One idea as a listing shows it, without reading its captures back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaSummary {
    pub id: IdeaId,
    pub name: String,
    pub created: DateTime<Utc>,
    /// Connected captures whose files are still there.
    pub members: usize,
    /// Connected captures whose files are gone.
    pub missing: usize,
    pub retired: bool,
    pub promoted_to: Option<Slug>,
    pub last_signal: Option<DateTime<Utc>>,
    pub updated: DateTime<Utc>,
}

impl IdeaSummary {
    /// The same diagnostic [`IdeaState::evidence_missing`] reports.
    pub fn evidence_missing(&self) -> bool {
        !self.retired && self.members == 0
    }
}

/// An idea that currently holds a given capture, and how much else it holds.
///
/// What deleting a capture has to consult: taking the last live member away from
/// an idea that is not retired would leave it with nothing to derive a lifecycle
/// from, so that deletion is refused rather than performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaHold {
    pub id: IdeaId,
    pub name: String,
    pub retired: bool,
    /// Live connected captures, this one included.
    pub members: usize,
}

impl IdeaHold {
    /// Whether this capture is the only thing keeping the idea answerable.
    pub fn depends_on_it(&self) -> bool {
        !self.retired && self.members <= 1
    }
}

/// How to narrow the inbox.
///
/// `query` narrows the chronological listing rather than reordering it by
/// relevance, following the time log: an inbox is a thing you read in order, and
/// a search over it is one more filter rather than a different view.
#[derive(Debug, Clone)]
pub struct CaptureListOptions {
    pub query: Option<String>,
    /// `None` for both, `Some(false)` for the live inbox, `Some(true)` for what
    /// has been processed out of it.
    pub archived: Option<bool>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for CaptureListOptions {
    fn default() -> Self {
        Self {
            query: None,
            archived: None,
            from: None,
            to: None,
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureList {
    pub captures: Vec<CaptureRecord>,
    /// Total matching captures, not just the ones on this page of results.
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaList {
    pub ideas: Vec<IdeaSummary>,
    pub total: usize,
}

impl IdeaState {
    /// Whether this idea has lost the evidence it rests on.
    ///
    /// A non-retired idea with nothing live connected cannot be given a
    /// lifecycle label or a momentum score, because there is no authored text
    /// left to derive one from. Manufacturing an answer out of missing evidence
    /// is the one thing this feature must not do, so the dashboard groups these
    /// under Needs repair instead.
    pub fn evidence_missing(&self) -> bool {
        !self.retired && self.members.is_empty()
    }
}

impl Index {
    // Captures.

    /// Record a capture, replacing whatever was indexed under its id.
    pub async fn upsert_capture(&self, capture: &Capture) -> Result<(), IndexError> {
        let id = capture.id.to_string();
        let owner = owner_column(&capture.owner);
        let created = to_nanos(capture.created, "capture created")?;
        let updated = to_nanos(capture.updated, "capture updated")?;
        let size = capture.size as i64;
        let body = capture.body.clone();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            // An upsert rather than `insert or replace`, for the reason
            // [`Index::upsert`] gives at length: `replace` allocates a new
            // rowid, and the full-text row below is keyed by this one.
            transaction.execute(
                "insert into idea_captures (id, owner, created, updated, size)
                 values (?1, ?2, ?3, ?4, ?5)
                 on conflict(id) do update set
                     owner   = excluded.owner,
                     created = excluded.created,
                     updated = excluded.updated,
                     size    = excluded.size",
                params![&id, &owner, created, updated, size],
            )?;

            let rowid: i64 = transaction.query_row(
                "select rowid from idea_captures where id = ?1",
                params![&id],
                |row| row.get(0),
            )?;
            transaction.execute(
                "delete from idea_captures_fts where rowid = ?1",
                params![rowid],
            )?;
            transaction.execute(
                "insert into idea_captures_fts (rowid, id, body) values (?1, ?2, ?3)",
                params![rowid, &id, &body],
            )?;

            refold_capture_and_its_ideas(&transaction, &id)?;

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn remove_capture(&self, id: &CaptureId) -> Result<(), IndexError> {
        let id = id.to_string();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            // Read before deleting: the full-text row is keyed by the rowid of
            // the row that is about to go. A capture that was never indexed has
            // neither, which is an ordinary call rather than an error.
            let rowid: Option<i64> = transaction
                .query_row(
                    "select rowid from idea_captures where id = ?1",
                    params![&id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(rowid) = rowid {
                transaction.execute(
                    "delete from idea_captures_fts where rowid = ?1",
                    params![rowid],
                )?;
            }
            transaction.execute("delete from idea_captures where id = ?1", params![&id])?;

            // The folds run *after* the delete, so they see the capture as
            // gone: its own rows are cleaned up, and every idea that named it
            // recomputes a `last_signal` that no longer counts it.
            refold_capture_and_its_ideas(&transaction, &id)?;

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// One of this owner's captures, or `None` when there is no such capture or
    /// it is not theirs.
    pub async fn capture(
        &self,
        owner: &Owner,
        id: &CaptureId,
    ) -> Result<Option<CaptureRecord>, IndexError> {
        let wanted = id.clone();
        let key = id.to_string();
        let asking = owner.clone();
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let found = connection
                .query_row(
                    &format!(
                        "select idea_captures.created,
                                idea_captures.updated,
                                idea_captures.size,
                                idea_captures_fts.body,
                                coalesce(idea_capture_state.archived, 0)
                         from idea_captures
                         join idea_captures_fts
                              on idea_captures_fts.rowid = idea_captures.rowid
                         left join idea_capture_state
                              on idea_capture_state.capture_id = idea_captures.id
                         where idea_captures.id = :id and {OWNED}"
                    ),
                    named_params! { ":id": &key, ":owner": &owner },
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()?;

            let Some((created, updated, size, body, archived)) = found else {
                return Ok(None);
            };

            Ok(Some(CaptureRecord {
                id: wanted,
                // The row matched `OWNED`, so its owner is the caller's by
                // construction. Reading the column back and reparsing it would
                // add a failure case that cannot happen and would have to guess
                // at an answer if it did.
                owner: asking,
                created: from_nanos(created),
                body,
                archived: archived != 0,
                updated: from_nanos(updated),
                size: size as u64,
            }))
        })
        .await
    }

    /// This owner's inbox, newest first.
    pub async fn list_captures(
        &self,
        owner: &Owner,
        options: CaptureListOptions,
    ) -> Result<CaptureList, IndexError> {
        let asking = owner.clone();
        let owner = owner_column(owner);
        let CaptureListOptions {
            query,
            archived,
            from,
            to,
            limit,
            offset,
        } = options;

        // An empty search is no search rather than no results. The dashboard is
        // asked to omit `q` entirely, and a caller that sends `q=` anyway means
        // "show me everything" rather than "show me nothing".
        let query = query.as_deref().and_then(to_fts_query);
        let archived = archived.map(i64::from);
        let from = from.map(|at| to_nanos(at, "from")).transpose()?;
        let to = to.map(|at| to_nanos(at, "to")).transpose()?;

        self.with_connection(move |connection| {
            // One filter expression rather than conditional joins: every clause
            // short-circuits to "everything" when its parameter binds as NULL,
            // so there is one query to read instead of sixteen. The full-text
            // match is a rowid subquery because `idea_captures_fts` can only be
            // looked up by `match` or by rowid, and the listing needs the
            // capture row beside it either way.
            let filters = format!(
                "from idea_captures
                 left join idea_capture_state
                      on idea_capture_state.capture_id = idea_captures.id
                 where {OWNED}
                   and (:query is null or idea_captures.rowid in (
                       select rowid from idea_captures_fts
                       where idea_captures_fts match :query
                   ))
                   and (:archived is null
                        or coalesce(idea_capture_state.archived, 0) = :archived)
                   and (:from is null or idea_captures.created >= :from)
                   and (:to is null or idea_captures.created <= :to)"
            );

            let total: i64 = connection.query_row(
                &format!("select count(*) {filters}"),
                named_params! {
                    ":owner": &owner,
                    ":query": &query,
                    ":archived": &archived,
                    ":from": &from,
                    ":to": &to,
                },
                |row| row.get(0),
            )?;

            let limit = limit as i64;
            let offset = offset as i64;
            let mut statement = connection.prepare(&format!(
                "select idea_captures.id,
                        idea_captures.created,
                        idea_captures.updated,
                        idea_captures.size,
                        (select body from idea_captures_fts
                         where rowid = idea_captures.rowid),
                        coalesce(idea_capture_state.archived, 0)
                 {filters}
                 order by idea_captures.created desc, idea_captures.id desc
                 limit :limit offset :offset"
            ))?;

            let rows = statement.query_map(
                named_params! {
                    ":owner": &owner,
                    ":query": &query,
                    ":archived": &archived,
                    ":from": &from,
                    ":to": &to,
                    ":limit": &limit,
                    ":offset": &offset,
                },
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )?;

            let mut captures = Vec::new();
            for row in rows {
                let (id, created, updated, size, body, archived) = row?;
                let Ok(id) = CaptureId::parse(&id) else {
                    continue;
                };
                captures.push(CaptureRecord {
                    id,
                    owner: asking.clone(),
                    created: from_nanos(created),
                    body,
                    archived: archived != 0,
                    updated: from_nanos(updated),
                    size: size as u64,
                });
            }

            Ok(CaptureList {
                captures,
                total: total as usize,
            })
        })
        .await
    }

    /// Every indexed capture's mtime and size, for the startup scan to compare
    /// against the filesystem.
    pub async fn capture_stamps(&self) -> Result<HashMap<CaptureId, Stamp>, IndexError> {
        self.with_connection(|connection| {
            stamps_of(connection, "idea_captures", |raw| {
                CaptureId::parse(raw).ok()
            })
        })
        .await
    }

    pub async fn count_captures(&self, owner: &Owner) -> Result<usize, IndexError> {
        self.count_owned("idea_captures", owner).await
    }

    // Idea threads.

    pub async fn upsert_idea(&self, idea: &Idea) -> Result<(), IndexError> {
        let id = idea.id.to_string();
        let owner = owner_column(&idea.owner);
        let name = idea.name.clone();
        let created = to_nanos(idea.created, "idea created")?;
        let updated = to_nanos(idea.updated, "idea updated")?;
        let size = idea.size as i64;
        let seeds: Vec<String> = idea.seeds.iter().map(CaptureId::to_string).collect();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            transaction.execute(
                "insert into idea_threads (id, owner, name, created, updated, size)
                 values (?1, ?2, ?3, ?4, ?5, ?6)
                 on conflict(id) do update set
                     owner   = excluded.owner,
                     name    = excluded.name,
                     created = excluded.created,
                     updated = excluded.updated,
                     size    = excluded.size",
                params![&id, &owner, &name, created, updated, size],
            )?;

            // Rewritten wholesale: a seed taken out of the file has to leave
            // the index, and delete-then-insert is the only version of that
            // with no way to leave a row behind.
            transaction.execute(
                "delete from idea_seed_captures where idea_id = ?1",
                params![&id],
            )?;
            {
                let mut insert = transaction.prepare(
                    "insert or ignore into idea_seed_captures (idea_id, capture_id)
                     values (?1, ?2)",
                )?;
                for seed in &seeds {
                    insert.execute(params![&id, seed])?;
                }
            }

            fold_idea(&transaction, &id)?;

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Drop a thread and everything folded for it.
    ///
    /// The seeds, membership, rejections and state all carry a foreign key to
    /// `idea_threads` and cascade. Its *events* do not, and stay: they are the
    /// audit trail, and a thread file deleted by hand should not silently take
    /// the record of what was decided about it.
    pub async fn remove_idea(&self, id: &IdeaId) -> Result<(), IndexError> {
        let id = id.to_string();

        self.with_connection(move |connection| {
            connection.execute("delete from idea_threads where id = ?1", params![&id])?;
            Ok(())
        })
        .await
    }

    /// One of this owner's ideas, folded, or `None` when there is no such idea
    /// or it is not theirs.
    pub async fn idea_state(
        &self,
        owner: &Owner,
        id: &IdeaId,
    ) -> Result<Option<IdeaState>, IndexError> {
        let wanted = id.clone();
        let key = id.to_string();
        let asking = owner.clone();
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let found = connection
                .query_row(
                    &format!(
                        "select name, created, updated, size
                         from idea_threads
                         where id = :id and {OWNED}"
                    ),
                    named_params! { ":id": &key, ":owner": &owner },
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()?;

            let Some((name, created, updated, size)) = found else {
                return Ok(None);
            };

            // Ids sort chronologically as text, so this is oldest first without
            // having to join the captures for their timestamps.
            let mut members = Vec::new();
            let mut missing = Vec::new();
            {
                let mut statement = connection.prepare(
                    "select idea_membership.capture_id,
                            idea_captures.id is not null
                     from idea_membership
                     left join idea_captures
                          on idea_captures.id = idea_membership.capture_id
                     where idea_membership.idea_id = ?1
                     order by idea_membership.capture_id",
                )?;
                let rows = statement.query_map(params![&key], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })?;

                for row in rows {
                    let (capture, live) = row?;
                    let Ok(capture) = CaptureId::parse(&capture) else {
                        continue;
                    };
                    if live != 0 {
                        members.push(capture);
                    } else {
                        missing.push(capture);
                    }
                }
            }

            let rejected = {
                let mut statement = connection.prepare(
                    "select capture_id from idea_rejections
                     where idea_id = ?1 order by capture_id",
                )?;
                let rows = statement.query_map(params![&key], |row| row.get::<_, String>(0))?;
                let mut rejected = Vec::new();
                for row in rows {
                    if let Ok(capture) = CaptureId::parse(&row?) {
                        rejected.push(capture);
                    }
                }
                rejected
            };

            let state = connection
                .query_row(
                    "select retired, promoted_to, last_signal
                     from idea_thread_state where idea_id = ?1",
                    params![&key],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                        ))
                    },
                )
                .optional()?;
            let (retired, promoted_to, last_signal) = state.unwrap_or((0, None, None));

            Ok(Some(IdeaState {
                id: wanted,
                owner: asking,
                name,
                created: from_nanos(created),
                members,
                missing,
                rejected,
                retired: retired != 0,
                promoted_to: promoted_to.and_then(|slug| Slug::parse(&slug).ok()),
                last_signal: last_signal.map(from_nanos),
                updated: from_nanos(updated),
                size: size as u64,
            }))
        })
        .await
    }

    /// This owner's ideas, newest signal first.
    ///
    /// No lifecycle label and no momentum: both are pure functions of a state
    /// and an explicit `at`, and neither exists until the analyzer does. What is
    /// here is the folded fact each of them would be computed from.
    pub async fn list_ideas(
        &self,
        owner: &Owner,
        limit: usize,
        offset: usize,
    ) -> Result<IdeaList, IndexError> {
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let total: i64 = connection.query_row(
                &format!("select count(*) from idea_threads where {OWNED}"),
                named_params! { ":owner": &owner },
                |row| row.get(0),
            )?;

            let limit = limit as i64;
            let offset = offset as i64;
            let mut statement = connection.prepare(&format!(
                "select idea_threads.id,
                        idea_threads.name,
                        idea_threads.created,
                        idea_threads.updated,
                        (select count(*) from idea_membership
                         join idea_captures on idea_captures.id = idea_membership.capture_id
                         where idea_membership.idea_id = idea_threads.id),
                        (select count(*) from idea_membership
                         where idea_membership.idea_id = idea_threads.id),
                        coalesce(idea_thread_state.retired, 0),
                        idea_thread_state.promoted_to,
                        idea_thread_state.last_signal
                 from idea_threads
                 left join idea_thread_state
                      on idea_thread_state.idea_id = idea_threads.id
                 where {OWNED}
                 order by idea_thread_state.last_signal desc nulls last,
                          idea_threads.id desc
                 limit :limit offset :offset"
            ))?;

            let rows = statement.query_map(
                named_params! { ":owner": &owner, ":limit": &limit, ":offset": &offset },
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )?;

            let mut ideas = Vec::new();
            for row in rows {
                let (id, name, created, updated, live, connected, retired, promoted, signal) = row?;
                let Ok(id) = IdeaId::parse(&id) else {
                    continue;
                };
                ideas.push(IdeaSummary {
                    id,
                    name,
                    created: from_nanos(created),
                    members: live as usize,
                    // Connected minus live: the evidence the idea has lost.
                    missing: (connected - live).max(0) as usize,
                    retired: retired != 0,
                    promoted_to: promoted.and_then(|slug| Slug::parse(&slug).ok()),
                    last_signal: signal.map(from_nanos),
                    updated: from_nanos(updated),
                });
            }

            Ok(IdeaList {
                ideas,
                total: total as usize,
            })
        })
        .await
    }

    /// The ideas that currently hold this capture, and what else they hold.
    ///
    /// Consulted before a capture is deleted for good. Retired ideas are
    /// included so the caller can name every thread the deletion touches, not
    /// only the ones that would refuse it.
    pub async fn ideas_holding(
        &self,
        owner: &Owner,
        capture: &CaptureId,
    ) -> Result<Vec<IdeaHold>, IndexError> {
        let capture = capture.to_string();
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let mut statement = connection.prepare(&format!(
                "select idea_threads.id,
                        idea_threads.name,
                        coalesce(idea_thread_state.retired, 0),
                        (select count(*) from idea_membership as held
                         join idea_captures on idea_captures.id = held.capture_id
                         where held.idea_id = idea_threads.id)
                 from idea_membership
                 join idea_threads on idea_threads.id = idea_membership.idea_id
                 left join idea_thread_state
                      on idea_thread_state.idea_id = idea_threads.id
                 where idea_membership.capture_id = :capture and {OWNED}
                 order by idea_threads.id"
            ))?;

            let rows = statement.query_map(
                named_params! { ":capture": &capture, ":owner": &owner },
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )?;

            let mut holds = Vec::new();
            for row in rows {
                let (id, name, retired, members) = row?;
                if let Ok(id) = IdeaId::parse(&id) {
                    holds.push(IdeaHold {
                        id,
                        name,
                        retired: retired != 0,
                        members: members as usize,
                    });
                }
            }
            Ok(holds)
        })
        .await
    }

    /// The captures an idea currently holds, oldest first.
    ///
    /// Live ones only: a member whose file is gone has no text to return, and
    /// [`IdeaState::missing`] is where it is named instead. Ids sort
    /// chronologically as text, so ordering by id is ordering by when the
    /// thought was captured.
    pub async fn idea_captures(
        &self,
        owner: &Owner,
        idea: &IdeaId,
    ) -> Result<Vec<CaptureRecord>, IndexError> {
        let asking = owner.clone();
        let idea = idea.to_string();
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let mut statement = connection.prepare(&format!(
                "select idea_captures.id,
                        idea_captures.created,
                        idea_captures.updated,
                        idea_captures.size,
                        (select body from idea_captures_fts
                         where rowid = idea_captures.rowid),
                        coalesce(idea_capture_state.archived, 0)
                 from idea_membership
                 join idea_captures on idea_captures.id = idea_membership.capture_id
                 left join idea_capture_state
                      on idea_capture_state.capture_id = idea_captures.id
                 where idea_membership.idea_id = :idea and {OWNED}
                 order by idea_captures.id"
            ))?;

            let rows =
                statement.query_map(named_params! { ":idea": &idea, ":owner": &owner }, |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                })?;

            let mut captures = Vec::new();
            for row in rows {
                let (id, created, updated, size, body, archived) = row?;
                let Ok(id) = CaptureId::parse(&id) else {
                    continue;
                };
                captures.push(CaptureRecord {
                    id,
                    owner: asking.clone(),
                    created: from_nanos(created),
                    body,
                    archived: archived != 0,
                    updated: from_nanos(updated),
                    size: size as u64,
                });
            }
            Ok(captures)
        })
        .await
    }

    pub async fn idea_stamps(&self) -> Result<HashMap<IdeaId, Stamp>, IndexError> {
        self.with_connection(|connection| {
            stamps_of(connection, "idea_threads", |raw| IdeaId::parse(raw).ok())
        })
        .await
    }

    pub async fn count_ideas(&self, owner: &Owner) -> Result<usize, IndexError> {
        self.count_owned("idea_threads", owner).await
    }

    // Decision events.

    pub async fn upsert_idea_event(&self, event: &Event) -> Result<(), IndexError> {
        let id = event.id.to_string();
        let owner = owner_column(&event.actor);
        let kind = event.kind.as_str();
        let idea = event.subject.idea().map(IdeaId::to_string);
        let capture = event.subject.capture().map(CaptureId::to_string);
        let other = event.subject.other_capture().map(CaptureId::to_string);
        let page = event.subject.page().map(Slug::to_string);
        let created = to_nanos(event.created, "event created")?;
        let updated = to_nanos(event.updated, "event updated")?;
        let size = event.size as i64;

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            transaction.execute(
                "insert into idea_events
                     (id, owner, kind, idea_id, capture_id, other_capture_id,
                      page_slug, created, updated, size)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 on conflict(id) do update set
                     owner            = excluded.owner,
                     kind             = excluded.kind,
                     idea_id          = excluded.idea_id,
                     capture_id       = excluded.capture_id,
                     other_capture_id = excluded.other_capture_id,
                     page_slug        = excluded.page_slug,
                     created          = excluded.created,
                     updated          = excluded.updated,
                     size             = excluded.size",
                params![
                    &id, &owner, kind, &idea, &capture, &other, &page, created, updated, size
                ],
            )?;

            refold_for_event(
                &transaction,
                idea.as_deref(),
                capture.as_deref(),
                other.as_deref(),
            )?;

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn remove_idea_event(&self, id: &EventId) -> Result<(), IndexError> {
        let id = id.to_string();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            // What it named has to be read before it goes, or there is nothing
            // left to say which folds its absence changes.
            let named = transaction
                .query_row(
                    "select idea_id, capture_id, other_capture_id
                     from idea_events where id = ?1",
                    params![&id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?;

            transaction.execute("delete from idea_events where id = ?1", params![&id])?;

            if let Some((idea, capture, other)) = named {
                refold_for_event(
                    &transaction,
                    idea.as_deref(),
                    capture.as_deref(),
                    other.as_deref(),
                )?;
            }

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn idea_event_stamps(&self) -> Result<HashMap<EventId, Stamp>, IndexError> {
        self.with_connection(|connection| {
            stamps_of(connection, "idea_events", |raw| EventId::parse(raw).ok())
        })
        .await
    }

    pub async fn count_idea_events(&self, owner: &Owner) -> Result<usize, IndexError> {
        self.count_owned("idea_events", owner).await
    }

    /// Pairs of this owner's captures that were suggested for each other and
    /// turned down, in canonical order.
    ///
    /// Both halves are joined so that a pair naming a capture somebody else owns
    /// cannot come back, and so that a pair whose partner was deleted does not.
    pub async fn rejected_capture_pairs(
        &self,
        owner: &Owner,
    ) -> Result<Vec<(CaptureId, CaptureId)>, IndexError> {
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let mut statement = connection.prepare(
                "select rejections.capture_id, rejections.other_capture_id
                 from idea_capture_rejections as rejections
                 join idea_captures as first on first.id = rejections.capture_id
                 join idea_captures as second on second.id = rejections.other_capture_id
                 where first.owner is :owner and second.owner is :owner
                 order by rejections.capture_id, rejections.other_capture_id",
            )?;
            let rows = statement.query_map(named_params! { ":owner": &owner }, |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;

            let mut pairs = Vec::new();
            for row in rows {
                let (first, second) = row?;
                if let (Ok(first), Ok(second)) =
                    (CaptureId::parse(&first), CaptureId::parse(&second))
                {
                    pairs.push((first, second));
                }
            }
            Ok(pairs)
        })
        .await
    }

    async fn count_owned(&self, table: &'static str, owner: &Owner) -> Result<usize, IndexError> {
        let owner = owner_column(owner);

        self.with_connection(move |connection| {
            let count: i64 = connection.query_row(
                &format!("select count(*) from {table} where {OWNED}"),
                named_params! { ":owner": &owner },
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
        .await
    }
}

/// The `owner` column's value for an [`Owner`]: a name, or null for the open
/// user of a wiki with no accounts.
fn owner_column(owner: &Owner) -> Option<String> {
    owner.username().map(Username::to_string)
}

/// Every row's mtime and size in one of the three mirrored tables.
fn stamps_of<Id: std::hash::Hash + Eq>(
    connection: &Connection,
    table: &'static str,
    parse: fn(&str) -> Option<Id>,
) -> Result<HashMap<Id, Stamp>, IndexError> {
    let mut statement = connection.prepare(&format!("select id, updated, size from {table}"))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let mut stamps = HashMap::new();
    for row in rows {
        let (id, updated, size) = row?;
        // A row whose id no longer parses cannot correspond to a record we
        // would serve; leaving it out means the scan deletes it.
        if let Some(id) = parse(&id) {
            stamps.insert(
                id,
                Stamp {
                    updated: from_nanos(updated),
                    size: size as u64,
                },
            );
        }
    }
    Ok(stamps)
}

/// Recompute a capture's own folded rows, and every idea that names it.
///
/// The second half is what keeps `last_signal` honest. It is the one folded
/// value that reads outside the event log, so a capture arriving, changing or
/// disappearing moves it for every thread the capture belongs to.
fn refold_capture_and_its_ideas(
    transaction: &Transaction<'_>,
    capture: &str,
) -> rusqlite::Result<()> {
    fold_capture(transaction, capture)?;

    for idea in ideas_naming(transaction, capture)? {
        fold_idea(transaction, &idea)?;
    }
    Ok(())
}

/// Recompute everything one event's references bear on.
fn refold_for_event(
    transaction: &Transaction<'_>,
    idea: Option<&str>,
    capture: Option<&str>,
    other: Option<&str>,
) -> rusqlite::Result<()> {
    if let Some(idea) = idea {
        fold_idea(transaction, idea)?;
    }
    for capture in [capture, other].into_iter().flatten() {
        fold_capture(transaction, capture)?;
    }
    Ok(())
}

/// Every idea whose folded state could depend on this capture.
///
/// Membership is included as well as seeds and events, because a capture can be
/// a member through an event that has since been reindexed away, and the row is
/// what has to be recomputed.
fn ideas_naming(transaction: &Transaction<'_>, capture: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = transaction.prepare(
        "select idea_id from idea_seed_captures where capture_id = ?1
         union
         select idea_id from idea_events where capture_id = ?1 and idea_id is not null
         union
         select idea_id from idea_membership where capture_id = ?1",
    )?;
    let rows = statement.query_map(params![capture], |row| row.get::<_, String>(0))?;

    rows.collect()
}

/// Recompute everything folded for one idea.
///
/// Order matters at the bottom: `idea_thread_state.last_signal` reads the
/// membership this function has just written.
///
/// An idea whose file has not been indexed yet gets its rows cleared and nothing
/// written. That is not a failure case, it is the ordinary one: an event can be
/// read before the thread it names, and the thread's own upsert folds it again.
fn fold_idea(transaction: &Transaction<'_>, idea: &str) -> rusqlite::Result<()> {
    transaction.execute(
        "delete from idea_membership where idea_id = ?1",
        params![idea],
    )?;
    transaction.execute(
        "delete from idea_rejections where idea_id = ?1",
        params![idea],
    )?;
    transaction.execute(
        "delete from idea_thread_state where idea_id = ?1",
        params![idea],
    )?;

    let known: bool = transaction.query_row(
        "select exists (select 1 from idea_threads where id = ?1)",
        params![idea],
        |row| row.get(0),
    )?;
    if !known {
        return Ok(());
    }

    transaction.execute(FOLD_MEMBERSHIP, params![idea])?;
    transaction.execute(FOLD_REJECTIONS, params![idea])?;
    transaction.execute(FOLD_THREAD_STATE, params![idea])?;
    Ok(())
}

/// Recompute everything folded for one capture.
///
/// A capture that is not indexed gets its rows cleared and nothing written,
/// which is also how a deletion cleans up after itself: the row goes, this runs,
/// and the archive state and every pair rejection naming it go with it.
fn fold_capture(transaction: &Transaction<'_>, capture: &str) -> rusqlite::Result<()> {
    transaction.execute(
        "delete from idea_capture_state where capture_id = ?1",
        params![capture],
    )?;
    transaction.execute(
        "delete from idea_capture_rejections
         where capture_id = ?1 or other_capture_id = ?1",
        params![capture],
    )?;

    let known: bool = transaction.query_row(
        "select exists (select 1 from idea_captures where id = ?1)",
        params![capture],
        |row| row.get(0),
    )?;
    if !known {
        return Ok(());
    }

    transaction.execute(FOLD_CAPTURE_STATE, params![capture])?;
    transaction.execute(FOLD_CAPTURE_REJECTIONS, params![capture])?;
    Ok(())
}

// The fold, in SQL.
//
// Event kinds appear here as string literals, spelled exactly as
// `EventKind::as_str` spells them, which is also exactly how they appear in the
// authored file. Renaming one is an authored-format change and has to be made in
// three places at once; `the_fold_spells_event_kinds_the_way_the_files_do` below
// is the tripwire that says so.

/// A capture belongs to an idea when the latest connect-or-disconnect decision
/// about the pair says connected, and a seed with no decision at all counts as
/// connected: the thread's file is what put it there.
const FOLD_MEMBERSHIP: &str = "
insert into idea_membership (idea_id, capture_id)
select ?1, decided.capture_id
from (
    select capture_id from idea_seed_captures where idea_id = ?1
    union
    select capture_id from idea_events
    where idea_id = ?1
      and capture_id is not null
      and kind in ('capture_connected', 'capture_disconnected')
) as decided
where coalesce(
    (select latest.kind
     from idea_events as latest
     where latest.idea_id = ?1
       and latest.capture_id = decided.capture_id
       and latest.kind in ('capture_connected', 'capture_disconnected')
     order by latest.id desc
     limit 1),
    'capture_connected'
) = 'capture_connected'
";

/// A candidate stays rejected until it is reconsidered.
///
/// The capture has to still exist, for the same reason a pair does below: a
/// rejection is a standing instruction not to suggest something, and a capture
/// that is gone cannot be suggested. The decision itself is not lost, because
/// the event is still there to read.
const FOLD_REJECTIONS: &str = "
insert into idea_rejections (idea_id, capture_id)
select ?1, decided.capture_id
from (
    select distinct capture_id from idea_events
    where idea_id = ?1
      and capture_id is not null
      and other_capture_id is null
      and kind in ('candidate_rejected', 'candidate_reconsidered')
) as decided
where (
    select latest.kind
    from idea_events as latest
    where latest.idea_id = ?1
      and latest.capture_id = decided.capture_id
      and latest.other_capture_id is null
      and latest.kind in ('candidate_rejected', 'candidate_reconsidered')
    order by latest.id desc
    limit 1
) = 'candidate_rejected'
  and exists (select 1 from idea_captures where id = decided.capture_id)
";

/// Retirement, promotion, and the last time anything happened.
///
/// `last_signal` reads `idea_membership`, so this runs after it.
const FOLD_THREAD_STATE: &str = "
insert into idea_thread_state (idea_id, retired, promoted_to, last_signal)
values (
    ?1,
    case when (
        select kind from idea_events
        where idea_id = ?1 and kind in ('idea_retired', 'idea_reopened')
        order by id desc limit 1
    ) = 'idea_retired' then 1 else 0 end,
    (
        select page_slug from idea_events
        where idea_id = ?1 and kind = 'idea_promoted'
        order by id desc limit 1
    ),
    (
        select max(signal) from (
            select captures.created as signal
            from idea_membership as member
            join idea_captures as captures on captures.id = member.capture_id
            where member.idea_id = ?1
            union all
            select events.created
            from idea_events as events
            where events.idea_id = ?1
              and events.kind in ('interest_affirmed', 'capture_connected',
                                  'idea_reopened', 'idea_promoted')
        )
    )
)
";

/// Archived until restored. Nothing is written for a capture that has never been
/// either, so an absent row means "in the inbox".
const FOLD_CAPTURE_STATE: &str = "
insert into idea_capture_state (capture_id, archived)
select ?1, case when latest.kind = 'capture_archived' then 1 else 0 end
from (
    select kind from idea_events
    where capture_id = ?1
      and kind in ('capture_archived', 'capture_restored')
    order by id desc limit 1
) as latest
";

/// A pair rejection needs both captures to still exist. One of them being
/// deleted does not make the pair rejected forever; it makes the pair
/// unsuggestable, which is a different thing and needs no row.
const FOLD_CAPTURE_REJECTIONS: &str = "
insert into idea_capture_rejections (capture_id, other_capture_id)
select pairs.capture_id, pairs.other_capture_id
from (
    select distinct capture_id, other_capture_id
    from idea_events
    where other_capture_id is not null
      and (capture_id = ?1 or other_capture_id = ?1)
) as pairs
where (
    select latest.kind
    from idea_events as latest
    where latest.capture_id = pairs.capture_id
      and latest.other_capture_id = pairs.other_capture_id
    order by latest.id desc limit 1
) = 'candidate_rejected'
  and exists (select 1 from idea_captures where id = pairs.capture_id)
  and exists (select 1 from idea_captures where id = pairs.other_capture_id)
";

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ideas::{EventKind, Subject};

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn tim() -> Owner {
        Owner::of(Username::parse("tim").expect("valid username"))
    }

    fn capture_id(raw: &str) -> CaptureId {
        CaptureId::parse(raw).expect("valid capture id")
    }

    fn idea_id(raw: &str) -> IdeaId {
        IdeaId::parse(raw).expect("valid idea id")
    }

    async fn index() -> Index {
        Index::open(None).await.expect("open in-memory index")
    }

    fn capture(id: &str, owner: Owner, body: &str) -> Capture {
        let id = capture_id(id);
        Capture {
            created: id.instant(),
            id,
            owner,
            body: body.to_owned(),
            updated: at("2026-08-20T16:00:00Z"),
            size: body.len() as u64,
        }
    }

    fn idea(id: &str, owner: Owner, name: &str, seeds: &[&str]) -> Idea {
        let id = idea_id(id);
        Idea {
            created: id.instant(),
            id,
            owner,
            name: name.to_owned(),
            seeds: seeds.iter().map(|seed| capture_id(seed)).collect(),
            note: String::new(),
            updated: at("2026-08-20T16:00:00Z"),
            size: 64,
        }
    }

    fn event(id: &str, owner: Owner, kind: EventKind, subject: Subject) -> Event {
        let id = EventId::parse(id).expect("valid event id");
        Event {
            created: id.instant(),
            id,
            kind,
            subject,
            actor: owner,
            updated: at("2026-08-20T16:00:00Z"),
            size: 96,
        }
    }

    /// A capture, a thread seeded from it, and a second loose capture.
    async fn seeded() -> Index {
        let index = index().await;
        index
            .upsert_capture(&capture(
                "20260820T141530-000000000",
                tim(),
                "Dungeon seeds.",
            ))
            .await
            .unwrap();
        index
            .upsert_capture(&capture(
                "20260820T141600-000000000",
                tim(),
                "Seeded loot tables.",
            ))
            .await
            .unwrap();
        index
            .upsert_idea(&idea(
                "20260820T142000-000000000",
                tim(),
                "Dungeon seeds",
                &["20260820T141530-000000000"],
            ))
            .await
            .unwrap();
        index
    }

    async fn state(index: &Index) -> IdeaState {
        index
            .idea_state(&tim(), &idea_id("20260820T142000-000000000"))
            .await
            .unwrap()
            .expect("the idea is indexed")
    }

    #[tokio::test]
    async fn a_capture_round_trips_through_the_index() {
        let index = index().await;
        let written = capture("20260820T141530-000000000", tim(), "Dungeon seeds.");

        index.upsert_capture(&written).await.unwrap();
        let read = index
            .capture(&tim(), &written.id)
            .await
            .unwrap()
            .expect("indexed");

        assert_eq!(read.body, "Dungeon seeds.");
        assert_eq!(read.created, written.created);
        assert_eq!(read.owner, tim());
        assert!(!read.archived);
        assert_eq!(index.count_captures(&tim()).await.unwrap(), 1);
    }

    /// The privacy boundary, in the one place it is actually enforced.
    #[tokio::test]
    async fn another_owner_sees_none_of_it() {
        let index = seeded().await;
        let alice = Owner::of(Username::parse("alice").unwrap());

        assert_eq!(
            index
                .capture(&alice, &capture_id("20260820T141530-000000000"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            index
                .idea_state(&alice, &idea_id("20260820T142000-000000000"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(index.count_captures(&alice).await.unwrap(), 0);
        assert_eq!(index.count_ideas(&alice).await.unwrap(), 0);
    }

    /// An open wiki's records have a null owner, and `owner = null` is null in
    /// SQL, which a `where` clause reads as false. Without `is` the open user
    /// would be unable to see anything they had ever written.
    #[tokio::test]
    async fn the_open_user_can_see_their_own_records() {
        let index = index().await;
        index
            .upsert_capture(&capture(
                "20260820T141530-000000000",
                Owner::open(),
                "A thought.",
            ))
            .await
            .unwrap();

        assert!(
            index
                .capture(&Owner::open(), &capture_id("20260820T141530-000000000"))
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(index.count_captures(&Owner::open()).await.unwrap(), 1);
        // And an account sees none of them, which is the transition case.
        assert_eq!(index.count_captures(&tim()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn an_idea_starts_out_holding_its_seeds() {
        let index = seeded().await;
        let state = state(&index).await;

        assert_eq!(state.members, [capture_id("20260820T141530-000000000")]);
        assert!(state.missing.is_empty());
        assert!(!state.retired);
        assert_eq!(state.promoted_to, None);
        assert_eq!(
            state.last_signal,
            Some(capture_id("20260820T141530-000000000").instant())
        );
        assert!(!state.evidence_missing());
    }

    #[tokio::test]
    async fn connecting_and_disconnecting_fold_to_the_latest_decision() {
        let index = seeded().await;
        let loose = capture_id("20260820T141600-000000000");
        let thread = idea_id("20260820T142000-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CaptureConnected,
                Subject::IdeaCapture {
                    idea: thread.clone(),
                    capture: loose.clone(),
                },
            ))
            .await
            .unwrap();
        assert_eq!(state(&index).await.members.len(), 2);

        index
            .upsert_idea_event(&event(
                "20260820T142200-000000000",
                tim(),
                EventKind::CaptureDisconnected,
                Subject::IdeaCapture {
                    idea: thread.clone(),
                    capture: loose.clone(),
                },
            ))
            .await
            .unwrap();
        assert_eq!(
            state(&index).await.members,
            [capture_id("20260820T141530-000000000")]
        );

        // Reconnected later still: the latest decision is the one that counts.
        index
            .upsert_idea_event(&event(
                "20260820T142300-000000000",
                tim(),
                EventKind::CaptureConnected,
                Subject::IdeaCapture {
                    idea: thread,
                    capture: loose,
                },
            ))
            .await
            .unwrap();
        assert_eq!(state(&index).await.members.len(), 2);
    }

    /// The thread's file says which captures it was started from, and that is
    /// creation evidence rather than a claim about now.
    #[tokio::test]
    async fn a_seed_can_be_disconnected() {
        let index = seeded().await;

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CaptureDisconnected,
                Subject::IdeaCapture {
                    idea: idea_id("20260820T142000-000000000"),
                    capture: capture_id("20260820T141530-000000000"),
                },
            ))
            .await
            .unwrap();

        let state = state(&index).await;
        assert!(state.members.is_empty());
        assert!(state.evidence_missing());
    }

    #[tokio::test]
    async fn a_rejected_candidate_can_be_reconsidered() {
        let index = seeded().await;
        let thread = idea_id("20260820T142000-000000000");
        let loose = capture_id("20260820T141600-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CandidateRejected,
                Subject::IdeaCapture {
                    idea: thread.clone(),
                    capture: loose.clone(),
                },
            ))
            .await
            .unwrap();
        assert_eq!(state(&index).await.rejected, std::slice::from_ref(&loose));

        index
            .upsert_idea_event(&event(
                "20260820T142200-000000000",
                tim(),
                EventKind::CandidateReconsidered,
                Subject::IdeaCapture {
                    idea: thread,
                    capture: loose,
                },
            ))
            .await
            .unwrap();
        assert!(state(&index).await.rejected.is_empty());
    }

    #[tokio::test]
    async fn a_rejected_capture_pair_survives_until_reconsidered() {
        let index = seeded().await;
        let first = capture_id("20260820T141530-000000000");
        let second = capture_id("20260820T141600-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CandidateRejected,
                Subject::pair(first.clone(), second.clone()),
            ))
            .await
            .unwrap();
        assert_eq!(
            index.rejected_capture_pairs(&tim()).await.unwrap(),
            [(first.clone(), second.clone())]
        );

        index
            .upsert_idea_event(&event(
                "20260820T142200-000000000",
                tim(),
                EventKind::CandidateReconsidered,
                Subject::pair(second, first),
            ))
            .await
            .unwrap();
        assert!(
            index
                .rejected_capture_pairs(&tim())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn archiving_and_restoring_fold_to_the_latest_decision() {
        let index = seeded().await;
        let id = capture_id("20260820T141530-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CaptureArchived,
                Subject::Capture {
                    capture: id.clone(),
                },
            ))
            .await
            .unwrap();
        assert!(index.capture(&tim(), &id).await.unwrap().unwrap().archived);

        index
            .upsert_idea_event(&event(
                "20260820T142200-000000000",
                tim(),
                EventKind::CaptureRestored,
                Subject::Capture {
                    capture: id.clone(),
                },
            ))
            .await
            .unwrap();
        assert!(!index.capture(&tim(), &id).await.unwrap().unwrap().archived);

        // An archived capture is still connected: archive means processed, not
        // "this thought never happened".
        assert_eq!(state(&index).await.members, [id]);
    }

    #[tokio::test]
    async fn retiring_reopening_and_promoting_fold_onto_the_thread() {
        let index = seeded().await;
        let thread = idea_id("20260820T142000-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::IdeaRetired,
                Subject::Idea {
                    idea: thread.clone(),
                },
            ))
            .await
            .unwrap();
        assert!(state(&index).await.retired);

        index
            .upsert_idea_event(&event(
                "20260820T142200-000000000",
                tim(),
                EventKind::IdeaReopened,
                Subject::Idea {
                    idea: thread.clone(),
                },
            ))
            .await
            .unwrap();
        assert!(!state(&index).await.retired);

        index
            .upsert_idea_event(&event(
                "20260820T142300-000000000",
                tim(),
                EventKind::IdeaPromoted,
                Subject::Promotion {
                    idea: thread,
                    page: Slug::parse("notes/dungeon-seeds").unwrap(),
                },
            ))
            .await
            .unwrap();

        let state = state(&index).await;
        assert_eq!(
            state.promoted_to.as_ref().map(Slug::as_str),
            Some("notes/dungeon-seeds")
        );
        // Promotion is a signal, so it moves the clock forward.
        assert_eq!(
            state.last_signal,
            Some(
                EventId::parse("20260820T142300-000000000")
                    .unwrap()
                    .instant()
            )
        );
    }

    /// The property the `evidence_missing` diagnostic is made of: a deleted
    /// capture leaves the membership row that names it, so the idea can say what
    /// it has lost instead of quietly having lost nothing.
    #[tokio::test]
    async fn a_deleted_capture_leaves_its_membership_behind_as_missing_evidence() {
        let index = seeded().await;
        let id = capture_id("20260820T141530-000000000");

        index.remove_capture(&id).await.unwrap();

        let state = state(&index).await;
        assert!(state.members.is_empty());
        assert_eq!(state.missing, [id]);
        assert!(state.evidence_missing());
        // And with no live capture left, there is no signal to date it by.
        assert_eq!(state.last_signal, None);
    }

    /// A pair is about two captures, so losing one is not a rejection that
    /// outlives it.
    #[tokio::test]
    async fn deleting_a_capture_takes_its_pair_rejections_with_it() {
        let index = seeded().await;
        let first = capture_id("20260820T141530-000000000");
        let second = capture_id("20260820T141600-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CandidateRejected,
                Subject::pair(first.clone(), second.clone()),
            ))
            .await
            .unwrap();
        index.remove_capture(&second).await.unwrap();

        assert!(
            index
                .rejected_capture_pairs(&tim())
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// Reversing a decision by removing its event, which is what an external
    /// deletion of an event file does.
    #[tokio::test]
    async fn removing_an_event_recomputes_what_it_had_decided() {
        let index = seeded().await;
        let thread = idea_id("20260820T142000-000000000");
        let retirement = EventId::parse("20260820T142100-000000000").unwrap();

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::IdeaRetired,
                Subject::Idea {
                    idea: thread.clone(),
                },
            ))
            .await
            .unwrap();
        assert!(state(&index).await.retired);

        index.remove_idea_event(&retirement).await.unwrap();

        assert!(!state(&index).await.retired);
        assert_eq!(index.count_idea_events(&tim()).await.unwrap(), 0);
    }

    /// An event can be indexed before the thread it names, and a scan makes no
    /// promises about order. Folding has to converge either way.
    #[tokio::test]
    async fn an_event_read_before_its_thread_still_folds() {
        let index = index().await;
        let thread = idea_id("20260820T142000-000000000");
        let seed = "20260820T141530-000000000";
        let loose = "20260820T141600-000000000";

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::CaptureConnected,
                Subject::IdeaCapture {
                    idea: thread.clone(),
                    capture: capture_id(loose),
                },
            ))
            .await
            .unwrap();
        // Nothing to fold onto yet, and no error either.
        assert_eq!(index.idea_state(&tim(), &thread).await.unwrap(), None);

        index
            .upsert_idea(&idea(
                "20260820T142000-000000000",
                tim(),
                "Dungeon seeds",
                &[seed],
            ))
            .await
            .unwrap();
        index
            .upsert_capture(&capture(seed, tim(), "Dungeon seeds."))
            .await
            .unwrap();
        index
            .upsert_capture(&capture(loose, tim(), "Seeded loot tables."))
            .await
            .unwrap();

        let state = state(&index).await;
        assert_eq!(state.members, [capture_id(seed), capture_id(loose)]);
        // The connection is later than either capture, so it is the last signal.
        assert_eq!(
            state.last_signal,
            Some(
                EventId::parse("20260820T142100-000000000")
                    .unwrap()
                    .instant()
            )
        );
    }

    /// `last_signal` is the one folded value that reads outside the event log,
    /// so a capture arriving after the thread that names it has to move it. Until
    /// then the thread is holding evidence that is not there, and says so.
    #[tokio::test]
    async fn a_capture_indexed_after_its_thread_dates_it() {
        let index = index().await;
        let seed = "20260820T141530-000000000";
        index
            .upsert_idea(&idea(
                "20260820T142000-000000000",
                tim(),
                "Dungeon seeds",
                &[seed],
            ))
            .await
            .unwrap();

        let before = state(&index).await;
        assert!(before.members.is_empty());
        assert_eq!(before.missing, [capture_id(seed)]);
        assert_eq!(before.last_signal, None);
        assert!(before.evidence_missing());

        index
            .upsert_capture(&capture(seed, tim(), "Dungeon seeds."))
            .await
            .unwrap();

        let after = state(&index).await;
        assert_eq!(after.members, [capture_id(seed)]);
        assert!(after.missing.is_empty());
        assert_eq!(after.last_signal, Some(capture_id(seed).instant()));
        assert!(!after.evidence_missing());
    }

    /// Removing a thread takes everything folded for it and leaves the events,
    /// which are the audit trail and are not the thread's to delete.
    #[tokio::test]
    async fn removing_a_thread_keeps_the_decisions_taken_about_it() {
        let index = seeded().await;
        let thread = idea_id("20260820T142000-000000000");

        index
            .upsert_idea_event(&event(
                "20260820T142100-000000000",
                tim(),
                EventKind::IdeaRetired,
                Subject::Idea {
                    idea: thread.clone(),
                },
            ))
            .await
            .unwrap();
        index.remove_idea(&thread).await.unwrap();

        assert_eq!(index.idea_state(&tim(), &thread).await.unwrap(), None);
        assert_eq!(index.count_ideas(&tim()).await.unwrap(), 0);
        assert_eq!(index.count_idea_events(&tim()).await.unwrap(), 1);
    }

    /// The fold's SQL names event kinds as string literals, which are also the
    /// words in the authored files. This is the tripwire: rename one in
    /// `EventKind::as_str` and this fails, pointing at the SQL that has to move
    /// with it.
    #[test]
    fn the_fold_spells_event_kinds_the_way_the_files_do() {
        for (kind, spelling) in [
            (EventKind::CaptureConnected, "capture_connected"),
            (EventKind::CaptureDisconnected, "capture_disconnected"),
            (EventKind::CandidateRejected, "candidate_rejected"),
            (EventKind::CandidateReconsidered, "candidate_reconsidered"),
            (EventKind::InterestAffirmed, "interest_affirmed"),
            (EventKind::CaptureArchived, "capture_archived"),
            (EventKind::CaptureRestored, "capture_restored"),
            (EventKind::CaptureDeleted, "capture_deleted"),
            (EventKind::IdeaRetired, "idea_retired"),
            (EventKind::IdeaReopened, "idea_reopened"),
            (EventKind::IdeaPromoted, "idea_promoted"),
            (EventKind::RediscoveryDismissed, "rediscovery_dismissed"),
        ] {
            assert_eq!(kind.as_str(), spelling);
        }

        let fold = [
            FOLD_MEMBERSHIP,
            FOLD_REJECTIONS,
            FOLD_THREAD_STATE,
            FOLD_CAPTURE_STATE,
            FOLD_CAPTURE_REJECTIONS,
        ]
        .concat();
        for kind in [
            EventKind::CaptureConnected,
            EventKind::CaptureDisconnected,
            EventKind::CandidateRejected,
            EventKind::CandidateReconsidered,
            EventKind::InterestAffirmed,
            EventKind::CaptureArchived,
            EventKind::CaptureRestored,
            EventKind::IdeaRetired,
            EventKind::IdeaReopened,
            EventKind::IdeaPromoted,
        ] {
            assert!(
                fold.contains(kind.as_str()),
                "the fold never mentions {kind}"
            );
        }
    }
}
