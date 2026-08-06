//! The index over the time log.
//!
//! Everything here is derived from `.rhizolog/times/`, so it is as disposable
//! as the rest of the index: drop the database and the next scan rebuilds it
//! from the files.
//!
//! ## A time link is not a link
//!
//! A time entry names the pages it was spent on, and those names are edges into
//! the wiki — but they are kept in `time_pages`, well away from `links`.
//!
//! The reason is arithmetic. A page you actually work on collects one of these
//! every time you start a timer, so hundreds is normal and thousands is not
//! absurd. Put them in `links` and every one of them is a backlink: the page's
//! backlink panel becomes a scrolling list of `Deep work`, its referrer count
//! makes it the most-linked page in the wiki by an order of magnitude, and the
//! `most_linked` table in `/api/stats` stops meaning anything. Nothing about
//! the link graph would be wrong, exactly; it would just be swamped, and the
//! signal it exists to carry — which pages the *writing* points at — would be
//! gone.
//!
//! So they are a separate kind of edge, reported separately, and a page shows
//! them the way a hundred of anything should be shown: as one line with a total
//! on it. See [`Index::page_times`].
//!
//! What they share with links is resolution. `time_pages.target` is a slug as
//! written, joined against `pages` at read time and never resolved once and
//! stored — so time can be tracked against a page before it is written, and it
//! attaches itself the moment somebody writes it.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

use crate::index::{Index, IndexError, Stamp, from_nanos, to_nanos};
use crate::index::{SortOrder, schema::TOP_N};
use crate::slug::Slug;
use crate::times::stats::{Sample, SamplePage};
use crate::times::{TimeEntry, TimeId};

/// A page a time entry is attached to, with whatever the index knows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePageRef {
    pub slug: Slug,
    /// The page's title, or `None` when nothing is written at that slug yet.
    pub title: Option<String>,
}

/// A time entry as the index holds it: everything but the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRecord {
    pub id: TimeId,
    pub name: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub pages: Vec<TimePageRef>,
    /// Whether the file carries a note. The note itself comes from disk.
    pub has_note: bool,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

impl TimeRecord {
    pub fn is_running(&self) -> bool {
        self.end.is_none()
    }

    /// Seconds elapsed, counting up to `now` while the timer runs.
    pub fn seconds(&self, now: DateTime<Utc>) -> u64 {
        (self.end.unwrap_or(now) - self.start)
            .num_seconds()
            .max(0)
            .unsigned_abs()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeList {
    pub times: Vec<TimeRecord>,
    /// Total matching entries, not just the ones on this page of results.
    pub total: usize,
}

/// Everything recorded under one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeGroup {
    pub name: String,
    pub entries: usize,
    /// Total tracked, with running entries counted up to the moment asked.
    pub seconds: u64,
    pub running: usize,
    pub first_start: DateTime<Utc>,
    pub last_start: DateTime<Utc>,
}

/// The whole log at a glance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeTotals {
    pub entries: usize,
    /// Distinct names.
    pub groups: usize,
    pub seconds: u64,
    pub running: usize,
    pub first_start: Option<DateTime<Utc>>,
    pub last_start: Option<DateTime<Utc>>,
}

/// A time entry named from somewhere else — a page, say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRef {
    pub id: TimeId,
    pub name: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub seconds: u64,
}

/// How much time is attached to one page.
///
/// A summary rather than a list, because the list is the thing that does not
/// scale — see the note at the top of this module. `recent` is a sample so the
/// page can show *something*, capped at [`TOP_N`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageTimes {
    pub entries: usize,
    pub seconds: u64,
    pub running: usize,
    /// Distinct activity names tracked against this page.
    pub groups: usize,
    pub recent: Vec<TimeRef>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimeSortBy {
    #[default]
    Start,
    Name,
    Duration,
}

impl TimeSortBy {
    /// The expression to sort on.
    ///
    /// Interpolated into the SQL because a column cannot be a bound parameter.
    /// Going through this enum is what keeps that safe: the only strings that
    /// can reach a query are the three below. `?1` in the duration case is the
    /// caller's "now", which every query here binds first.
    fn expression(self) -> &'static str {
        match self {
            Self::Start => "started",
            Self::Name => "name",
            Self::Duration => "max(0, coalesce(ended, ?1) - started)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimeListOptions {
    /// Restrict to one group, matched exactly.
    pub name: Option<String>,
    /// Restrict to entries attached to this page.
    pub page: Option<String>,
    /// `Some(true)` for running entries only, `Some(false)` for finished ones.
    pub running: Option<bool>,
    /// Restrict to entries overlapping `[from, to)`. Either end may be open.
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub sort: TimeSortBy,
    pub order: SortOrder,
    pub limit: usize,
    pub offset: usize,
}

impl Default for TimeListOptions {
    fn default() -> Self {
        Self {
            name: None,
            page: None,
            running: None,
            from: None,
            to: None,
            sort: TimeSortBy::default(),
            // A log reads newest first. This is the one listing in Rhizolog
            // where the default order is descending, because "what did I just
            // do" is the question being asked.
            order: SortOrder::Descending,
            limit: 50,
            offset: 0,
        }
    }
}

/// Which entries a listing's window admits.
///
/// `?2` is the caller's `from` and `?3` its `to`; a NULL in either binds to
/// "no bound on that side". An entry counts when it *overlaps* the window
/// rather than when it starts inside it, so a session that began yesterday and
/// is still running shows up in today's list — it is time being spent today.
const WINDOW: &str = "(?2 is null or coalesce(ended, ?1) > ?2)
     and (?3 is null or started < ?3)";

impl Index {
    /// Record a time entry, replacing whatever was indexed under its id.
    pub async fn upsert_time(&self, entry: &TimeEntry) -> Result<(), IndexError> {
        let id = entry.id.to_string();
        let name = entry.name.clone();
        let started = to_nanos(entry.start, "time start")?;
        let ended = entry.end.map(|end| to_nanos(end, "time end")).transpose()?;
        let has_note = i64::from(entry.has_note());
        let updated = to_nanos(entry.updated, "time updated")?;
        let size = entry.size as i64;
        let pages: Vec<String> = entry.pages.iter().map(Slug::to_string).collect();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            transaction.execute(
                "insert or replace into times (id, name, started, ended, has_note, updated, size)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![&id, &name, started, ended, has_note, updated, size],
            )?;

            transaction.execute("delete from time_pages where time_id = ?1", params![&id])?;
            {
                let mut insert = transaction.prepare(
                    "insert or ignore into time_pages (time_id, target) values (?1, ?2)",
                )?;
                for page in &pages {
                    insert.execute(params![&id, page])?;
                }
            }

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn remove_time(&self, id: &TimeId) -> Result<(), IndexError> {
        let id = id.to_string();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute("delete from time_pages where time_id = ?1", params![&id])?;
            transaction.execute("delete from times where id = ?1", params![&id])?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Every indexed entry's mtime and size, for the startup scan to compare
    /// against the filesystem.
    pub async fn time_stamps(&self) -> Result<HashMap<TimeId, Stamp>, IndexError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare("select id, updated, size from times")?;
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
                // A row whose id no longer parses cannot correspond to an entry
                // we would serve; leaving it out means the scan deletes it.
                if let Ok(id) = TimeId::parse(&id) {
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
        })
        .await
    }

    pub async fn count_times(&self) -> Result<usize, IndexError> {
        self.with_connection(|connection| {
            let count: i64 =
                connection.query_row("select count(*) from times", [], |row| row.get(0))?;
            Ok(count as usize)
        })
        .await
    }

    /// List entries, without their notes.
    pub async fn list_times(
        &self,
        options: TimeListOptions,
        now: DateTime<Utc>,
    ) -> Result<TimeList, IndexError> {
        let TimeListOptions {
            name,
            page,
            running,
            from,
            to,
            sort,
            order,
            limit,
            offset,
        } = options;

        let now = to_nanos(now, "now")?;
        let from = from.map(|at| to_nanos(at, "from")).transpose()?;
        let to = to.map(|at| to_nanos(at, "to")).transpose()?;
        let running = running.map(i64::from);

        self.with_connection(move |connection| {
            // One filter expression rather than conditional clauses, the same
            // shape `list` uses for pages: each condition short-circuits to
            // "everything" when its parameter binds as NULL.
            let filters = format!(
                "where {WINDOW}
                 and (?4 is null or name = ?4)
                 and (?5 is null or exists (
                     select 1 from time_pages
                     where time_pages.time_id = times.id and time_pages.target = ?5
                 ))
                 and (?6 is null or (ended is null) = (?6 = 1))"
            );

            let total: i64 = connection.query_row(
                &format!("select count(*) from times {filters}"),
                params![now, from, to, &name, &page, running],
                |row| row.get(0),
            )?;

            let sql = format!(
                "select id, name, started, ended, has_note, updated, size
                 from times
                 {filters}
                 order by {} {}, id desc
                 limit ?7 offset ?8",
                sort.expression(),
                order.keyword(),
            );
            // Scoped so the prepared statement is dropped before the borrow
            // `attach_pages` needs.
            let mut times = Vec::new();
            {
                let mut statement = connection.prepare(&sql)?;
                let rows = statement.query_map(
                    params![
                        now,
                        from,
                        to,
                        &name,
                        &page,
                        running,
                        limit as i64,
                        offset as i64
                    ],
                    read_record,
                )?;

                for row in rows {
                    if let Some(record) = row? {
                        times.push(record);
                    }
                }
            }

            attach_pages(connection, &mut times)?;

            Ok(TimeList {
                times,
                total: total as usize,
            })
        })
        .await
    }

    /// Every group, most time first.
    pub async fn time_groups(&self, now: DateTime<Utc>) -> Result<Vec<TimeGroup>, IndexError> {
        let now = to_nanos(now, "now")?;

        self.with_connection(move |connection| {
            let mut query = connection.prepare(
                "select name,
                        count(*),
                        sum(max(0, coalesce(ended, ?1) - started)),
                        sum(case when ended is null then 1 else 0 end),
                        min(started),
                        max(started)
                 from times
                 group by name
                 order by 3 desc, name asc",
            )?;

            let groups = query
                .query_map(params![now], |row| {
                    Ok(TimeGroup {
                        name: row.get(0)?,
                        entries: row.get::<_, i64>(1)? as usize,
                        seconds: nanos_to_seconds(row.get::<_, i64>(2)?),
                        running: row.get::<_, i64>(3)? as usize,
                        first_start: from_nanos(row.get::<_, i64>(4)?),
                        last_start: from_nanos(row.get::<_, i64>(5)?),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(groups)
        })
        .await
    }

    pub async fn time_totals(&self, now: DateTime<Utc>) -> Result<TimeTotals, IndexError> {
        let now = to_nanos(now, "now")?;

        self.with_connection(move |connection| {
            let totals = connection.query_row(
                "select count(*),
                        count(distinct name),
                        coalesce(sum(max(0, coalesce(ended, ?1) - started)), 0),
                        coalesce(sum(case when ended is null then 1 else 0 end), 0),
                        min(started),
                        max(started)
                 from times",
                params![now],
                |row| {
                    Ok(TimeTotals {
                        entries: row.get::<_, i64>(0)? as usize,
                        groups: row.get::<_, i64>(1)? as usize,
                        seconds: nanos_to_seconds(row.get::<_, i64>(2)?),
                        running: row.get::<_, i64>(3)? as usize,
                        first_start: row.get::<_, Option<i64>>(4)?.map(from_nanos),
                        last_start: row.get::<_, Option<i64>>(5)?.map(from_nanos),
                    })
                },
            )?;

            Ok(totals)
        })
        .await
    }

    /// Every entry overlapping `[from, to)`, reduced to what the statistics
    /// arithmetic needs.
    ///
    /// The bucketing happens in Rust rather than here — see
    /// [`crate::times::stats`] for why — so this hands back rows rather than
    /// totals. Page titles come along because the statistics rank pages, and
    /// looking each one up afterwards would be a query per page.
    pub async fn time_samples(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<Sample>, IndexError> {
        let from = to_nanos(from, "from")?;
        let to = to_nanos(to, "to")?;

        self.with_connection(move |connection| {
            // A running entry has no `ended`, so it overlaps anything that has
            // not finished before it began. `?1` here is `from`, not `now`:
            // there is no clock in this query, and closing running entries is
            // the caller's business.
            let mut query = connection.prepare(
                "select id, name, started, ended
                 from times
                 where (ended is null or ended > ?1) and started < ?2
                 order by started",
            )?;

            let mut samples: Vec<(String, Sample)> = query
                .query_map(params![from, to], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        Sample {
                            name: row.get(1)?,
                            start: from_nanos(row.get::<_, i64>(2)?),
                            end: row.get::<_, Option<i64>>(3)?.map(from_nanos),
                            pages: Vec::new(),
                        },
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut pages_of = connection.prepare(
                "select time_pages.target, pages.title
                 from time_pages
                 left join pages on pages.slug = time_pages.target
                 where time_pages.time_id = ?1
                 order by time_pages.target",
            )?;

            for (id, sample) in &mut samples {
                sample.pages = pages_of
                    .query_map(params![id.as_str()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter_map(|(target, title)| {
                        let slug = Slug::parse(&target).ok()?;
                        Some(SamplePage {
                            // A page that does not exist yet still deserves a
                            // label, and its slug is the honest one.
                            title: title.unwrap_or_else(|| slug.to_string()),
                            slug,
                        })
                    })
                    .collect();
            }

            Ok(samples.into_iter().map(|(_, sample)| sample).collect())
        })
        .await
    }

    /// How much time is attached to one page.
    pub async fn page_times(
        &self,
        slug: &Slug,
        now: DateTime<Utc>,
    ) -> Result<PageTimes, IndexError> {
        let slug = slug.to_string();
        let now_nanos = to_nanos(now, "now")?;

        self.with_connection(move |connection| {
            const ATTACHED: &str = "from times
                 join time_pages on time_pages.time_id = times.id
                 where time_pages.target = ?1";

            let (entries, seconds, running, groups) = connection.query_row(
                &format!(
                    "select count(*),
                            coalesce(sum(max(0, coalesce(ended, ?2) - started)), 0),
                            coalesce(sum(case when ended is null then 1 else 0 end), 0),
                            count(distinct name)
                     {ATTACHED}"
                ),
                params![&slug, now_nanos],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as usize,
                        nanos_to_seconds(row.get::<_, i64>(1)?),
                        row.get::<_, i64>(2)? as usize,
                        row.get::<_, i64>(3)? as usize,
                    ))
                },
            )?;

            let mut recent_query = connection.prepare(&format!(
                "select times.id, times.name, times.started, times.ended
                 {ATTACHED}
                 order by times.started desc, times.id desc
                 limit ?3"
            ))?;

            let recent = recent_query
                .query_map(params![&slug, now_nanos, TOP_N as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter_map(|(id, name, started, ended)| {
                    let start = from_nanos(started);
                    let end = ended.map(from_nanos);
                    Some(TimeRef {
                        id: TimeId::parse(&id).ok()?,
                        name,
                        seconds: (end.unwrap_or(now) - start)
                            .num_seconds()
                            .max(0)
                            .unsigned_abs(),
                        start,
                        end,
                    })
                })
                .collect();

            Ok(PageTimes {
                entries,
                seconds,
                running,
                groups,
                recent,
            })
        })
        .await
    }

    /// One entry as the index holds it, or `None` if it is not indexed.
    pub async fn time_record(&self, id: &TimeId) -> Result<Option<TimeRecord>, IndexError> {
        let id = id.to_string();

        self.with_connection(move |connection| {
            let record = connection
                .query_row(
                    "select id, name, started, ended, has_note, updated, size
                     from times where id = ?1",
                    params![&id],
                    read_record,
                )
                .optional()?
                .flatten();

            let mut records: Vec<TimeRecord> = record.into_iter().collect();
            attach_pages(connection, &mut records)?;
            Ok(records.pop())
        })
        .await
    }
}

/// Read one row of the seven columns every entry query selects.
///
/// Returns `None` for a row whose id no longer parses: it names an entry that
/// could never be served, so there is nothing useful to hand back for it.
fn read_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<TimeRecord>> {
    let raw: String = row.get(0)?;
    let Ok(id) = TimeId::parse(&raw) else {
        return Ok(None);
    };

    Ok(Some(TimeRecord {
        id,
        name: row.get(1)?,
        start: from_nanos(row.get::<_, i64>(2)?),
        end: row.get::<_, Option<i64>>(3)?.map(from_nanos),
        pages: Vec::new(),
        has_note: row.get::<_, i64>(4)? != 0,
        updated: from_nanos(row.get::<_, i64>(5)?),
        size: row.get::<_, i64>(6)? as u64,
    }))
}

/// Fill in each record's attached pages, resolving titles by joining `pages`.
fn attach_pages(
    connection: &rusqlite::Connection,
    records: &mut [TimeRecord],
) -> Result<(), IndexError> {
    let mut pages_of = connection.prepare(
        "select time_pages.target, pages.title
         from time_pages
         left join pages on pages.slug = time_pages.target
         where time_pages.time_id = ?1
         order by time_pages.target",
    )?;

    for record in records {
        record.pages = pages_of
            .query_map(params![record.id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(target, title)| {
                Some(TimePageRef {
                    slug: Slug::parse(&target).ok()?,
                    title,
                })
            })
            .collect();
    }

    Ok(())
}

/// Durations are summed in nanoseconds and reported in seconds.
///
/// Summing already-divided values would round every entry independently and
/// lose up to a second per entry, which over a year of them is visible.
fn nanos_to_seconds(nanos: i64) -> u64 {
    (nanos.max(0) / 1_000_000_000).unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::page::{Frontmatter, Page};

    async fn index() -> Index {
        Index::open(None).await.expect("open in-memory index")
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    /// An indexed entry, built without going near the filesystem.
    fn entry(name: &str, start: &str, end: Option<&str>, pages: &[&str]) -> TimeEntry {
        let start = at(start);
        TimeEntry {
            id: TimeId::mint(start, 0),
            name: name.to_owned(),
            start,
            end: end.map(at),
            pages: pages
                .iter()
                .map(|page| Slug::parse(page).expect("valid slug"))
                .collect(),
            note: String::new(),
            updated: at("2026-08-06T20:00:00Z"),
            size: 120,
        }
    }

    async fn seed_page(index: &Index, slug: &str, title: &str) {
        index
            .upsert(&Page {
                slug: Slug::parse(slug).expect("valid slug"),
                frontmatter: Frontmatter {
                    title: Some(title.to_owned()),
                    ..Frontmatter::default()
                },
                body: "Body.\n".to_owned(),
                updated: at("2026-08-06T20:00:00Z"),
                size: 6,
            })
            .await
            .expect("upsert page");
    }

    fn now() -> DateTime<Utc> {
        at("2026-08-06T20:00:00Z")
    }

    #[tokio::test]
    async fn entries_round_trip_with_their_pages() {
        let index = index().await;
        seed_page(&index, "notes/rust/async", "Async in Rust").await;
        let written = entry(
            "Deep work",
            "2026-08-06T14:00:00Z",
            Some("2026-08-06T15:00:00Z"),
            &["notes/rust/async", "notes/unwritten"],
        );

        index.upsert_time(&written).await.unwrap();
        let record = index.time_record(&written.id).await.unwrap().unwrap();

        assert_eq!(record.name, "Deep work");
        assert_eq!(record.seconds(now()), 3600);
        assert!(!record.is_running());
        assert_eq!(record.pages.len(), 2);
        assert_eq!(record.pages[0].title.as_deref(), Some("Async in Rust"));
        // Tracked against a page nobody has written: resolved by joining, so it
        // simply has no title yet.
        assert_eq!(record.pages[1].slug.as_str(), "notes/unwritten");
        assert_eq!(record.pages[1].title, None);
    }

    /// The property that makes attachment a query rather than a cache, the same
    /// one the link graph has.
    #[tokio::test]
    async fn writing_a_page_resolves_the_time_already_tracked_against_it() {
        let index = index().await;
        let written = entry(
            "Deep work",
            "2026-08-06T14:00:00Z",
            Some("2026-08-06T15:00:00Z"),
            &["notes/later"],
        );
        index.upsert_time(&written).await.unwrap();

        assert_eq!(
            index.time_record(&written.id).await.unwrap().unwrap().pages[0].title,
            None
        );

        // Only the page is written. Nothing touches the entry.
        seed_page(&index, "notes/later", "Later").await;

        assert_eq!(
            index.time_record(&written.id).await.unwrap().unwrap().pages[0]
                .title
                .as_deref(),
            Some("Later")
        );
    }

    #[tokio::test]
    async fn upserting_replaces_rather_than_duplicating() {
        let index = index().await;
        let mut written = entry("Deep work", "2026-08-06T14:00:00Z", None, &["notes/a"]);
        index.upsert_time(&written).await.unwrap();

        written.name = "Shallow work".to_owned();
        written.pages = vec![Slug::parse("notes/b").unwrap()];
        index.upsert_time(&written).await.unwrap();

        assert_eq!(index.count_times().await.unwrap(), 1);
        let record = index.time_record(&written.id).await.unwrap().unwrap();
        assert_eq!(record.name, "Shallow work");
        assert_eq!(record.pages.len(), 1, "the old attachment survived");
        assert_eq!(record.pages[0].slug.as_str(), "notes/b");
    }

    #[tokio::test]
    async fn removing_an_entry_clears_its_attachments() {
        let index = index().await;
        let written = entry("Deep work", "2026-08-06T14:00:00Z", None, &["notes/a"]);
        index.upsert_time(&written).await.unwrap();

        index.remove_time(&written.id).await.unwrap();

        assert_eq!(index.count_times().await.unwrap(), 0);
        assert!(index.time_record(&written.id).await.unwrap().is_none());
        assert_eq!(
            index
                .page_times(&Slug::parse("notes/a").unwrap(), now())
                .await
                .unwrap()
                .entries,
            0
        );
        assert!(index.time_stamps().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_running_entry_counts_up_and_is_filterable() {
        let index = index().await;
        index
            .upsert_time(&entry("Running", "2026-08-06T19:30:00Z", None, &[]))
            .await
            .unwrap();
        index
            .upsert_time(&entry(
                "Finished",
                "2026-08-06T14:00:00Z",
                Some("2026-08-06T15:00:00Z"),
                &[],
            ))
            .await
            .unwrap();

        let running = index
            .list_times(
                TimeListOptions {
                    running: Some(true),
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();

        assert_eq!(running.total, 1);
        assert_eq!(running.times[0].name, "Running");
        assert_eq!(running.times[0].seconds(now()), 30 * 60);

        let finished = index
            .list_times(
                TimeListOptions {
                    running: Some(false),
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();
        assert_eq!(finished.total, 1);
        assert_eq!(finished.times[0].name, "Finished");
    }

    /// Several timers at once is the point, so nothing may refuse an overlap.
    #[tokio::test]
    async fn overlapping_entries_are_all_counted() {
        let index = index().await;
        for (name, start, end) in [
            ("Pairing", "2026-08-06T14:00:00Z", "2026-08-06T15:00:00Z"),
            ("Listening", "2026-08-06T14:30:00Z", "2026-08-06T15:30:00Z"),
        ] {
            index
                .upsert_time(&entry(name, start, Some(end), &[]))
                .await
                .unwrap();
        }

        let totals = index.time_totals(now()).await.unwrap();
        assert_eq!(totals.entries, 2);
        assert_eq!(totals.groups, 2);
        assert_eq!(totals.seconds, 2 * 3600, "overlap is not deduplicated");
    }

    #[tokio::test]
    async fn lists_newest_first_by_default_and_filters_by_name() {
        let index = index().await;
        for (name, start) in [
            ("Deep work", "2026-08-06T09:00:00Z"),
            ("Email", "2026-08-06T11:00:00Z"),
            ("Deep work", "2026-08-06T14:00:00Z"),
        ] {
            index
                .upsert_time(&entry(name, start, None, &[]))
                .await
                .unwrap();
        }

        let all = index
            .list_times(TimeListOptions::default(), now())
            .await
            .unwrap();
        let starts: Vec<String> = all
            .times
            .iter()
            .map(|record| record.start.to_rfc3339())
            .collect();
        assert_eq!(starts[0], at("2026-08-06T14:00:00Z").to_rfc3339());

        let deep = index
            .list_times(
                TimeListOptions {
                    name: Some("Deep work".to_owned()),
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();
        assert_eq!(deep.total, 2);
        // Exactly, not case-insensitively: a group is its name as written.
        let shouted = index
            .list_times(
                TimeListOptions {
                    name: Some("DEEP WORK".to_owned()),
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();
        assert_eq!(shouted.total, 0);
    }

    /// A window admits what overlaps it, not only what starts inside it —
    /// otherwise a session that began yesterday and is still going is missing
    /// from today.
    #[tokio::test]
    async fn a_window_admits_overlapping_entries() {
        let index = index().await;
        index
            .upsert_time(&entry(
                "Overnight",
                "2026-08-05T22:00:00Z",
                Some("2026-08-06T02:00:00Z"),
                &[],
            ))
            .await
            .unwrap();
        index
            .upsert_time(&entry(
                "Yesterday",
                "2026-08-05T09:00:00Z",
                Some("2026-08-05T10:00:00Z"),
                &[],
            ))
            .await
            .unwrap();

        let today = index
            .list_times(
                TimeListOptions {
                    from: Some(at("2026-08-06T00:00:00Z")),
                    to: Some(at("2026-08-07T00:00:00Z")),
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();

        assert_eq!(today.total, 1);
        assert_eq!(today.times[0].name, "Overnight");
    }

    #[tokio::test]
    async fn lists_filtered_by_the_page_the_time_was_spent_on() {
        let index = index().await;
        index
            .upsert_time(&entry(
                "Deep work",
                "2026-08-06T09:00:00Z",
                None,
                &["notes/a"],
            ))
            .await
            .unwrap();
        index
            .upsert_time(&entry("Email", "2026-08-06T11:00:00Z", None, &["notes/b"]))
            .await
            .unwrap();

        let list = index
            .list_times(
                TimeListOptions {
                    page: Some("notes/a".to_owned()),
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();

        assert_eq!(list.total, 1);
        assert_eq!(list.times[0].name, "Deep work");
    }

    #[tokio::test]
    async fn paginates_and_reports_the_full_total() {
        let index = index().await;
        for hour in 0..5 {
            index
                .upsert_time(&entry(
                    "Deep work",
                    &format!("2026-08-06T0{hour}:00:00Z"),
                    None,
                    &[],
                ))
                .await
                .unwrap();
        }

        let page = index
            .list_times(
                TimeListOptions {
                    limit: 2,
                    offset: 2,
                    ..TimeListOptions::default()
                },
                now(),
            )
            .await
            .unwrap();

        assert_eq!(page.times.len(), 2);
        assert_eq!(page.total, 5, "total counts every entry, not the slice");
    }

    #[tokio::test]
    async fn groups_entries_by_name_most_time_first() {
        let index = index().await;
        for (name, start, end) in [
            ("Email", "2026-08-06T11:00:00Z", "2026-08-06T11:30:00Z"),
            ("Deep work", "2026-08-06T09:00:00Z", "2026-08-06T11:00:00Z"),
            ("Deep work", "2026-08-06T14:00:00Z", "2026-08-06T15:00:00Z"),
        ] {
            index
                .upsert_time(&entry(name, start, Some(end), &[]))
                .await
                .unwrap();
        }
        index
            .upsert_time(&entry("Deep work", "2026-08-06T19:00:00Z", None, &[]))
            .await
            .unwrap();

        let groups = index.time_groups(now()).await.unwrap();

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "Deep work");
        assert_eq!(groups[0].entries, 3);
        assert_eq!(groups[0].running, 1);
        // Two finished hours plus an hour still running.
        assert_eq!(groups[0].seconds, 4 * 3600);
        assert_eq!(groups[0].first_start, at("2026-08-06T09:00:00Z"));
        assert_eq!(groups[0].last_start, at("2026-08-06T19:00:00Z"));
        assert_eq!(groups[1].name, "Email");
    }

    #[tokio::test]
    async fn summarises_the_time_attached_to_a_page() {
        let index = index().await;
        seed_page(&index, "notes/a", "A").await;
        for (name, start, end) in [
            (
                "Deep work",
                "2026-08-06T09:00:00Z",
                Some("2026-08-06T11:00:00Z"),
            ),
            (
                "Reading",
                "2026-08-06T14:00:00Z",
                Some("2026-08-06T15:00:00Z"),
            ),
        ] {
            index
                .upsert_time(&entry(name, start, end, &["notes/a"]))
                .await
                .unwrap();
        }
        index
            .upsert_time(&entry(
                "Deep work",
                "2026-08-06T19:00:00Z",
                None,
                &["notes/a"],
            ))
            .await
            .unwrap();
        index
            .upsert_time(&entry(
                "Elsewhere",
                "2026-08-06T08:00:00Z",
                None,
                &["notes/b"],
            ))
            .await
            .unwrap();

        let times = index
            .page_times(&Slug::parse("notes/a").unwrap(), now())
            .await
            .unwrap();

        assert_eq!(times.entries, 3);
        assert_eq!(times.running, 1);
        assert_eq!(times.groups, 2);
        assert_eq!(times.seconds, 4 * 3600);
        assert_eq!(times.recent[0].name, "Deep work");
        assert_eq!(times.recent[0].start, at("2026-08-06T19:00:00Z"));
        assert_eq!(times.recent.len(), 3);
    }

    #[tokio::test]
    async fn a_page_with_no_time_reports_zeroes_rather_than_nothing() {
        let index = index().await;
        seed_page(&index, "notes/a", "A").await;

        let times = index
            .page_times(&Slug::parse("notes/a").unwrap(), now())
            .await
            .unwrap();

        assert_eq!(times, PageTimes::default());
    }

    #[tokio::test]
    async fn samples_carry_the_pages_and_titles_the_statistics_rank_by() {
        let index = index().await;
        seed_page(&index, "notes/a", "A").await;
        index
            .upsert_time(&entry(
                "Deep work",
                "2026-08-06T09:00:00Z",
                Some("2026-08-06T11:00:00Z"),
                &["notes/a"],
            ))
            .await
            .unwrap();
        index
            .upsert_time(&entry(
                "Last year",
                "2025-01-01T09:00:00Z",
                Some("2025-01-01T10:00:00Z"),
                &[],
            ))
            .await
            .unwrap();

        let samples = index
            .time_samples(at("2026-01-01T00:00:00Z"), at("2027-01-01T00:00:00Z"))
            .await
            .unwrap();

        assert_eq!(samples.len(), 1, "the window should exclude last year");
        assert_eq!(samples[0].name, "Deep work");
        assert_eq!(samples[0].pages[0].title, "A");
    }

    /// Times are derived from the files in `.rhizolog/times/`, so unlike pins
    /// a rebuild has somewhere to get them back from — and clearing must
    /// therefore take them with it.
    #[tokio::test]
    async fn clearing_the_index_drops_times_too() {
        let index = index().await;
        index
            .upsert_time(&entry(
                "Deep work",
                "2026-08-06T09:00:00Z",
                None,
                &["notes/a"],
            ))
            .await
            .unwrap();

        index.clear().await.unwrap();

        assert_eq!(index.count_times().await.unwrap(), 0);
        assert!(index.time_groups(now()).await.unwrap().is_empty());
    }
}
