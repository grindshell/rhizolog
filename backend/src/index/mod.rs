//! The derived SQLite index: search, tags, and the stamps the startup scan
//! uses to tell what changed.
//!
//! Nothing here is authoritative. Delete the database and it rebuilds from the
//! markdown on disk, which is why [`schema`] has no migrations.
//!
//! All SQL lives in this module. `rusqlite` is blocking, so every method wraps
//! its work in `spawn_blocking` — keeping that boilerplate in one file is most
//! of why the SQL is confined here rather than spread across handlers.

pub mod graph;
pub mod pins;
pub mod schema;
pub mod sessions;
pub mod sync;
pub mod times;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::page::Page;
use crate::slug::Slug;
use schema::{FTS_BODY_COLUMN, KEY_LAST_SYNC, KEY_SCHEMA_VERSION, SCHEMA_VERSION};

pub use graph::{
    Graph, GraphEdge, GraphNode, GraphOptions, InboundLink, LinkTotals, LinkedPage, OutboundLink,
    PageLinks, PageRef, RouteUsage, Stats, TagCount, WantedPage,
};
pub use pins::Pin;
pub use sessions::StoredSession;
pub use sync::{SyncCounts, SyncReport, sync};
pub use times::{
    PageTimes, TimeGroup, TimeList, TimeListOptions, TimePageRef, TimeRecord, TimeRef, TimeSortBy,
    TimeTotals,
};

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("index database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("index task failed: {0}")]
    Task(String),

    #[error("{label} is out of the range the index can store")]
    TimestampOutOfRange { label: &'static str },
}

/// What the index recorded about a page's file, for change detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub updated: DateTime<Utc>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub slug: Slug,
    pub title: String,
    pub tags: Vec<String>,
    /// An excerpt of the body with the matched terms marked.
    pub snippet: String,
    /// Relevance, higher is better.
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Total matches, not just the ones on this page of results.
    pub total: usize,
}

/// A page's metadata, without its body. This is what listing returns — keeping
/// content out is what makes a whole-wiki listing a reasonable first call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRecord {
    pub slug: Slug,
    pub title: String,
    pub tags: Vec<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct PageList {
    pub pages: Vec<PageRecord>,
    /// Total matching pages, not just the ones on this page of results.
    pub total: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortBy {
    #[default]
    Slug,
    Title,
    Created,
    Updated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

impl SortBy {
    /// The column to sort on.
    ///
    /// A column name cannot be a bound parameter, so it is interpolated into
    /// the SQL. Going through this enum is what keeps that safe: the only
    /// strings that can reach the query are the four below.
    fn column(self) -> &'static str {
        match self {
            Self::Slug => "slug",
            Self::Title => "title",
            Self::Created => "created",
            Self::Updated => "updated",
        }
    }
}

impl SortOrder {
    fn keyword(self) -> &'static str {
        match self {
            Self::Ascending => "asc",
            Self::Descending => "desc",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListOptions {
    /// Restrict to pages carrying this tag.
    pub tag: Option<String>,
    /// Restrict to pages at or under this slug path.
    ///
    /// The hierarchical reading of a slug: `notes/rust` matches the page
    /// `notes/rust` and everything beneath it, and stops at the separator, so
    /// `notes/rustlings` is a different directory and does not match.
    pub prefix: Option<String>,
    /// Restrict to pages sitting in a directory of this name, wherever it is.
    ///
    /// The flat reading of the same slug, and the one that behaves like a tag:
    /// `rust` matches `notes/rust/async` and `code/rust/traits` alike.
    pub segment: Option<String>,
    pub sort: SortBy,
    pub order: SortOrder,
    pub limit: usize,
    pub offset: usize,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            tag: None,
            prefix: None,
            segment: None,
            sort: SortBy::default(),
            order: SortOrder::default(),
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Clone)]
pub struct Index {
    connection: Arc<Mutex<Connection>>,
}

impl Index {
    /// Open the index at `path`, creating or rebuilding it as needed.
    ///
    /// Pass `None` for an in-memory index, which is what the tests use.
    pub async fn open(path: Option<&Path>) -> Result<Self, IndexError> {
        let path = path.map(Path::to_path_buf);

        let connection = tokio::task::spawn_blocking(move || -> Result<Connection, IndexError> {
            let connection = match &path {
                Some(path) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            rusqlite::Error::SqliteFailure(
                                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                                Some(error.to_string()),
                            )
                        })?;
                    }
                    Connection::open(path)?
                }
                None => Connection::open_in_memory()?,
            };

            connection.execute_batch(
                "pragma journal_mode = wal;
                 pragma synchronous = normal;
                 pragma foreign_keys = on;",
            )?;

            ensure_schema(&connection)?;
            Ok(connection)
        })
        .await
        .map_err(|error| IndexError::Task(error.to_string()))??;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    async fn with_connection<T, F>(&self, work: F) -> Result<T, IndexError>
    where
        F: FnOnce(&mut Connection) -> Result<T, IndexError> + Send + 'static,
        T: Send + 'static,
    {
        let connection = Arc::clone(&self.connection);

        tokio::task::spawn_blocking(move || {
            // A panic inside one of these closures leaves the Rust-side mutex
            // poisoned but the SQLite connection perfectly usable, so recover
            // rather than taking the whole index down with it.
            let mut guard = connection.lock().unwrap_or_else(|error| error.into_inner());
            work(&mut guard)
        })
        .await
        .map_err(|error| IndexError::Task(error.to_string()))?
    }

    /// Record a page, replacing whatever was indexed under its slug.
    pub async fn upsert(&self, page: &Page) -> Result<(), IndexError> {
        let slug = page.slug.to_string();
        let title = page.title();
        let tags = page.tags().to_vec();
        let directories: Vec<String> = page.slug.directories().map(str::to_owned).collect();
        let body = page.body.clone();
        let links = crate::markdown::extract_links(&page.slug, &page.body);
        let created = to_nanos(page.created(), "created")?;
        let updated = to_nanos(page.updated, "updated")?;
        let size = page.size as i64;

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            // An upsert rather than `insert or replace`, because the two differ
            // in a way the FTS row below depends on: `replace` deletes the
            // conflicting row and inserts a new one, which allocates a new
            // rowid, while this updates in place and keeps it. A page's rowid
            // is therefore stable for as long as the page exists.
            transaction.execute(
                "insert into pages (slug, title, created, updated, size)
                 values (?1, ?2, ?3, ?4, ?5)
                 on conflict(slug) do update set
                     title   = excluded.title,
                     created = excluded.created,
                     updated = excluded.updated,
                     size    = excluded.size",
                params![&slug, &title, created, updated, size],
            )?;

            transaction.execute("delete from page_tags where slug = ?1", params![&slug])?;
            {
                let mut insert = transaction
                    .prepare("insert or ignore into page_tags (slug, tag) values (?1, ?2)")?;
                for tag in &tags {
                    insert.execute(params![&slug, tag])?;
                }
            }

            // Derived from the slug, so it is rewritten here rather than
            // anywhere a page is written: a move is an upsert under the new
            // slug and a remove of the old, and this follows for free.
            transaction.execute("delete from page_segments where slug = ?1", params![&slug])?;
            {
                let mut insert = transaction.prepare(
                    "insert or replace into page_segments (slug, segment, depth)
                     values (?1, ?2, ?3)",
                )?;
                for (depth, directory) in directories.iter().enumerate() {
                    insert.execute(params![&slug, directory, depth as i64])?;
                }
            }

            transaction.execute("delete from links where src_slug = ?1", params![&slug])?;
            {
                let mut insert = transaction.prepare(
                    "insert or ignore into links (src_slug, target, display, kind)
                     values (?1, ?2, ?3, ?4)",
                )?;
                for link in &links {
                    insert.execute(params![
                        &slug,
                        &link.target,
                        &link.display,
                        link.kind.as_str()
                    ])?;
                }
            }

            // FTS5 has no upsert, so the old row goes first — and it goes by
            // **rowid**, which with `MATCH` is the only way an FTS5 table can be
            // looked up at all. `slug` is an unindexed column, so
            // `where slug = ?` has no index to use and scans the entire table:
            // one full scan per page indexed, which is quadratic across a
            // rebuild and was eight minutes on a wiki of twenty thousand pages.
            // The rowid is the page's own, which is why the insert above had to
            // stop being a `replace`.
            let rowid: i64 = transaction.query_row(
                "select rowid from pages where slug = ?1",
                params![&slug],
                |row| row.get(0),
            )?;
            transaction.execute("delete from pages_fts where rowid = ?1", params![rowid])?;
            transaction.execute(
                "insert into pages_fts (rowid, slug, title, body) values (?1, ?2, ?3, ?4)",
                params![rowid, &slug, &title, &body],
            )?;

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn remove(&self, slug: &Slug) -> Result<(), IndexError> {
        let slug = slug.to_string();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;

            // Read before deleting: the FTS row is keyed by the page's rowid,
            // and the row that holds it is about to go. A page that was never
            // indexed has neither, which is an ordinary call rather than an
            // error — `sync` removes a page it failed to read.
            let rowid: Option<i64> = transaction
                .query_row(
                    "select rowid from pages where slug = ?1",
                    params![&slug],
                    |row| row.get(0),
                )
                .optional()?;

            transaction.execute("delete from pages where slug = ?1", params![&slug])?;
            transaction.execute("delete from page_tags where slug = ?1", params![&slug])?;
            transaction.execute("delete from page_segments where slug = ?1", params![&slug])?;
            // Outbound links go with the page. Inbound ones do not: they belong
            // to the pages that wrote them, and they become wanted links.
            transaction.execute("delete from links where src_slug = ?1", params![&slug])?;
            if let Some(rowid) = rowid {
                transaction.execute("delete from pages_fts where rowid = ?1", params![rowid])?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Every indexed page's mtime and size, for the startup scan to compare
    /// against the filesystem.
    pub async fn stamps(&self) -> Result<HashMap<Slug, Stamp>, IndexError> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare("select slug, updated, size from pages")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;

            let mut stamps = HashMap::new();
            for row in rows {
                let (slug, updated, size) = row?;
                // A row whose slug no longer parses cannot correspond to a page
                // we would serve; leaving it out means the scan deletes it.
                if let Ok(slug) = Slug::parse(&slug) {
                    stamps.insert(
                        slug,
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

    pub async fn count(&self) -> Result<usize, IndexError> {
        self.with_connection(|connection| {
            let count: i64 =
                connection.query_row("select count(*) from pages", [], |row| row.get(0))?;
            Ok(count as usize)
        })
        .await
    }

    /// Full-text search over page titles and bodies.
    ///
    /// The time log has its own search, on [`Index::list_times`], rather than
    /// being folded in here: a time entry and a page are not the same shape, so
    /// one result set holding both would have to be lossy or a union type. See
    /// [`times`] for what that search is instead.
    ///
    /// Terms are matched literally — see [`to_fts_query`] — and results come
    /// back with a marked-up excerpt so a caller can tell which hits are worth
    /// fetching without pulling every body.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<SearchResults, IndexError> {
        let Some(fts_query) = to_fts_query(query) else {
            return Ok(SearchResults {
                hits: Vec::new(),
                total: 0,
            });
        };

        self.with_connection(move |connection| {
            let total: i64 = connection.query_row(
                "select count(*) from pages_fts where pages_fts match ?1",
                params![&fts_query],
                |row| row.get(0),
            )?;

            // FTS5's auxiliary functions take the table's real name, not a
            // query alias — `snippet(f, ...)` is rejected as an unknown column
            // — so `pages_fts` is spelled out. The column index must likewise
            // be a literal rather than a bound parameter; it is a compile-time
            // constant here, not anything a caller supplies.
            let sql = format!(
                "select pages_fts.slug,
                        pages.title,
                        snippet(pages_fts, {FTS_BODY_COLUMN}, '<mark>', '</mark>', '...', 12),
                        -bm25(pages_fts)
                 from pages_fts
                 join pages on pages.slug = pages_fts.slug
                 where pages_fts match ?1
                 order by bm25(pages_fts)
                 limit ?2 offset ?3"
            );
            let mut statement = connection.prepare(&sql)?;

            let rows =
                statement.query_map(params![&fts_query, limit as i64, offset as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                    ))
                })?;

            let mut pending = Vec::new();
            for row in rows {
                let (slug, title, snippet, score) = row?;
                let Ok(slug) = Slug::parse(&slug) else {
                    continue;
                };
                pending.push((slug, title, snippet, score));
            }

            let mut tags_of = connection.prepare("select tag from page_tags where slug = ?1")?;
            let mut hits = Vec::with_capacity(pending.len());
            for (slug, title, snippet, score) in pending {
                let tags = tags_of
                    .query_map(params![slug.as_str()], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                hits.push(SearchHit {
                    slug,
                    title,
                    tags,
                    snippet,
                    score,
                });
            }

            Ok(SearchResults {
                hits,
                total: total as usize,
            })
        })
        .await
    }

    /// List pages, newest-first by default, without their bodies.
    pub async fn list(&self, options: ListOptions) -> Result<PageList, IndexError> {
        let ListOptions {
            tag,
            prefix,
            segment,
            sort,
            order,
            limit,
            offset,
        } = options;

        self.with_connection(move |connection| {
            // One filter expression rather than conditional joins: each clause
            // short-circuits to "everything" when its parameter binds as NULL,
            // so there is one query to read instead of eight. The filters
            // intersect — asking for a tag and a path asks for both.
            //
            // The prefix clause compares with `substr` rather than `like`.
            // SQLite's `like` is case-insensitive over ASCII, and slugs are
            // case-sensitive (`Notes/x` and `notes/x` are two files on Linux);
            // `substr` also spares the caller's prefix from having to escape
            // `%` and `_`. Comparing against `prefix || '/'` is what stops
            // `notes/rust` from matching `notes/rustlings`, and the equality
            // beside it is what keeps the page `notes/rust` itself in its own
            // listing — a page that names a directory is that directory's
            // index, and hiding it there would be a surprise.
            const FILTERS: &str = "where (?1 is null or exists (
                     select 1 from page_tags
                     where page_tags.slug = pages.slug and page_tags.tag = ?1
                 ))
                 and (?2 is null
                      or slug = ?2
                      or substr(slug, 1, length(?2) + 1) = ?2 || '/')
                 and (?3 is null or exists (
                     select 1 from page_segments
                     where page_segments.slug = pages.slug
                       and page_segments.segment = ?3
                 ))";

            let total: i64 = connection.query_row(
                &format!("select count(*) from pages {FILTERS}"),
                params![&tag, &prefix, &segment],
                |row| row.get(0),
            )?;

            let sql = format!(
                "select slug, title, created, updated, size
                 from pages
                 {FILTERS}
                 order by {} {}, slug asc
                 limit ?4 offset ?5",
                sort.column(),
                order.keyword(),
            );
            let mut statement = connection.prepare(&sql)?;

            let rows = statement.query_map(
                params![&tag, &prefix, &segment, limit as i64, offset as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )?;

            let mut pending = Vec::new();
            for row in rows {
                let (slug, title, created, updated, size) = row?;
                let Ok(slug) = Slug::parse(&slug) else {
                    continue;
                };
                pending.push((slug, title, created, updated, size));
            }

            let mut tags_of =
                connection.prepare("select tag from page_tags where slug = ?1 order by tag")?;
            let mut pages = Vec::with_capacity(pending.len());
            for (slug, title, created, updated, size) in pending {
                let tags = tags_of
                    .query_map(params![slug.as_str()], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                pages.push(PageRecord {
                    slug,
                    title,
                    tags,
                    created: from_nanos(created),
                    updated: from_nanos(updated),
                    size: size as u64,
                });
            }

            Ok(PageList {
                pages,
                total: total as usize,
            })
        })
        .await
    }

    /// Drop everything derived from the wiki, so the next scan rebuilds it.
    ///
    /// Times go too. Unlike pins, they are derived — from the files in
    /// `.rhizolog/times/` rather than from the markdown, but derived all the
    /// same, so the scan has somewhere to get them back from.
    pub async fn clear(&self) -> Result<(), IndexError> {
        self.with_connection(|connection| {
            let transaction = connection.transaction()?;
            transaction.execute("delete from pages", [])?;
            transaction.execute("delete from page_tags", [])?;
            transaction.execute("delete from page_segments", [])?;
            transaction.execute("delete from links", [])?;
            transaction.execute("delete from pages_fts", [])?;
            transaction.execute("delete from time_pages", [])?;
            transaction.execute("delete from times_fts", [])?;
            transaction.execute("delete from times", [])?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn set_last_sync(&self, at: DateTime<Utc>) -> Result<(), IndexError> {
        let nanos = to_nanos(at, "last sync")?;

        self.with_connection(move |connection| {
            connection.execute(
                "insert or replace into meta (key, value) values (?1, ?2)",
                params![KEY_LAST_SYNC, nanos.to_string()],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn last_sync(&self) -> Result<Option<DateTime<Utc>>, IndexError> {
        self.with_connection(|connection| {
            let stored: Option<String> = connection
                .query_row(
                    "select value from meta where key = ?1",
                    params![KEY_LAST_SYNC],
                    |row| row.get(0),
                )
                .optional()?;

            Ok(stored
                .and_then(|value| value.parse::<i64>().ok())
                .map(from_nanos))
        })
        .await
    }
}

/// Create the schema, dropping and rebuilding the derived half of it if it was
/// written by a different version of Rhizolog.
///
/// The durable tables are created first, because the version number this
/// decision rests on lives in one of them.
fn ensure_schema(connection: &Connection) -> Result<(), IndexError> {
    connection.execute_batch(schema::CREATE_DURABLE)?;

    let existing: Option<String> = connection
        .query_row(
            "select value from meta where key = ?1",
            params![KEY_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .optional()?;

    let version = existing.and_then(|value| value.parse::<i64>().ok());
    if version == Some(SCHEMA_VERSION) {
        return Ok(());
    }

    if version.is_some() {
        tracing::info!(
            from = ?version,
            to = SCHEMA_VERSION,
            "index schema changed; rebuilding from the wiki"
        );
    }

    connection.execute_batch(schema::DROP_DERIVED)?;
    connection.execute_batch(schema::CREATE_DERIVED)?;
    connection.execute(
        "insert or replace into meta (key, value) values (?1, ?2)",
        params![KEY_SCHEMA_VERSION, SCHEMA_VERSION.to_string()],
    )?;

    Ok(())
}

/// Turn a user's query into an FTS5 expression that matches literally.
///
/// Each term is quoted, so punctuation a user typed cannot become FTS5 syntax
/// and a stray `"` cannot turn a search into a 500. Terms combine with an
/// implicit AND; a trailing `*` still means prefix search, which is what makes
/// search-as-you-type usable.
///
/// The cost is that FTS5's own operators (`OR`, `NEAR`) are not reachable. For
/// a single-user wiki, a search box that never errors is the better trade.
fn to_fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .filter_map(|term| {
            let (word, prefix) = match term.strip_suffix('*') {
                Some(stem) => (stem, true),
                None => (term, false),
            };
            if word.is_empty() {
                return None;
            }

            // Inside an FTS5 string, `""` is an escaped quote.
            let escaped = word.replace('"', "\"\"");
            Some(if prefix {
                format!("\"{escaped}\"*")
            } else {
                format!("\"{escaped}\"")
            })
        })
        .collect();

    (!terms.is_empty()).then(|| terms.join(" "))
}

fn to_nanos(at: DateTime<Utc>, label: &'static str) -> Result<i64, IndexError> {
    at.timestamp_nanos_opt()
        .ok_or(IndexError::TimestampOutOfRange { label })
}

fn from_nanos(nanos: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::page::Frontmatter;

    async fn index() -> Index {
        Index::open(None).await.expect("open in-memory index")
    }

    fn page(slug: &str, title: &str, tags: &[&str], body: &str) -> Page {
        Page {
            slug: Slug::parse(slug).expect("valid slug"),
            frontmatter: Frontmatter {
                title: Some(title.to_owned()),
                tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                created: None,
            },
            body: body.to_owned(),
            updated: DateTime::from_timestamp_nanos(1_700_000_000_000_000_000),
            size: body.len() as u64,
        }
    }

    #[tokio::test]
    async fn upserts_and_counts_pages() {
        let index = index().await;
        assert_eq!(index.count().await.unwrap(), 0);

        index
            .upsert(&page("notes/rhizome", "Rhizome", &[], "Branches off."))
            .await
            .unwrap();
        assert_eq!(index.count().await.unwrap(), 1);

        // Upserting the same slug replaces rather than duplicates.
        index
            .upsert(&page("notes/rhizome", "Rhizome", &[], "Rewritten."))
            .await
            .unwrap();
        assert_eq!(index.count().await.unwrap(), 1);

        let results = index.search("rewritten", 10, 0).await.unwrap();
        assert_eq!(results.total, 1);
        // The replaced body is gone from the index, not merely shadowed.
        assert_eq!(index.search("branches", 10, 0).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn removing_a_page_clears_all_of_its_rows() {
        let index = index().await;
        let page = page("notes/rhizome", "Rhizome", &["theory"], "Branches off.");

        index.upsert(&page).await.unwrap();
        index.remove(&page.slug).await.unwrap();

        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.search("branches", 10, 0).await.unwrap().total, 0);
        assert!(index.stamps().await.unwrap().is_empty());
    }

    /// Full-text rows are keyed by the rowid of the page they describe, so a
    /// page's rowid has to survive being reindexed and has to stay its own.
    /// Get either wrong and a rewrite silently takes somebody else's text out
    /// of the search index — which nothing else here would notice, because the
    /// rewritten page itself would look perfectly correct.
    #[tokio::test]
    async fn rewriting_one_page_leaves_the_others_searchable() {
        let index = index().await;
        for n in 0..5 {
            index
                .upsert(&page(
                    &format!("notes/page-{n}"),
                    "T",
                    &[],
                    &format!("distinctive-{n}"),
                ))
                .await
                .unwrap();
        }

        // Rewrite one, twice, which is what an edit and a watcher echo do.
        for body in ["rewritten once", "rewritten twice"] {
            index
                .upsert(&page("notes/page-2", "T", &[], body))
                .await
                .unwrap();
        }

        for n in [0, 1, 3, 4] {
            assert_eq!(
                index
                    .search(&format!("distinctive-{n}"), 10, 0)
                    .await
                    .unwrap()
                    .total,
                1,
                "rewriting page-2 lost page-{n} from the search index"
            );
        }

        assert_eq!(index.count().await.unwrap(), 5);
        assert_eq!(index.search("rewritten", 10, 0).await.unwrap().total, 1);
        assert_eq!(index.search("distinctive-2", 10, 0).await.unwrap().total, 0);
    }

    /// The same property one level up: removing a page must take its own text
    /// and nothing else.
    #[tokio::test]
    async fn removing_one_page_leaves_the_others_searchable() {
        let index = index().await;
        for n in 0..3 {
            index
                .upsert(&page(
                    &format!("notes/page-{n}"),
                    "T",
                    &[],
                    &format!("distinctive-{n}"),
                ))
                .await
                .unwrap();
        }

        index
            .remove(&Slug::parse("notes/page-1").unwrap())
            .await
            .unwrap();

        assert_eq!(index.search("distinctive-1", 10, 0).await.unwrap().total, 0);
        assert_eq!(index.search("distinctive-0", 10, 0).await.unwrap().total, 1);
        assert_eq!(index.search("distinctive-2", 10, 0).await.unwrap().total, 1);
    }

    #[tokio::test]
    async fn searches_titles_and_bodies_with_an_excerpt() {
        let index = index().await;
        index
            .upsert(&page(
                "notes/rhizome",
                "Rhizome",
                &["theory", "deleuze"],
                "Knowledge branches off chaotically and that is the point.",
            ))
            .await
            .unwrap();

        let results = index.search("chaotically", 10, 0).await.unwrap();

        assert_eq!(results.total, 1);
        let hit = &results.hits[0];
        assert_eq!(hit.slug.as_str(), "notes/rhizome");
        assert_eq!(hit.title, "Rhizome");
        assert_eq!(hit.tags, ["deleuze", "theory"]);
        assert!(
            hit.snippet.contains("<mark>chaotically</mark>"),
            "expected a marked excerpt, got {:?}",
            hit.snippet
        );

        // Titles are searchable too.
        assert_eq!(index.search("rhizome", 10, 0).await.unwrap().total, 1);
    }

    #[tokio::test]
    async fn multiple_terms_are_combined_with_and() {
        let index = index().await;
        index
            .upsert(&page("a", "A", &[], "alpha beta"))
            .await
            .unwrap();
        index
            .upsert(&page("b", "B", &[], "alpha gamma"))
            .await
            .unwrap();

        assert_eq!(index.search("alpha", 10, 0).await.unwrap().total, 2);
        assert_eq!(index.search("alpha beta", 10, 0).await.unwrap().total, 1);
        assert_eq!(index.search("alpha delta", 10, 0).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn a_trailing_star_searches_by_prefix() {
        let index = index().await;
        index
            .upsert(&page("a", "A", &[], "chaotically"))
            .await
            .unwrap();

        assert_eq!(index.search("chaot*", 10, 0).await.unwrap().total, 1);
        assert_eq!(index.search("chaot", 10, 0).await.unwrap().total, 0);
    }

    /// Punctuation a user typed must not reach FTS5 as syntax. Every one of
    /// these is a query that would otherwise be a 500.
    #[tokio::test]
    async fn punctuation_in_a_query_never_errors() {
        let index = index().await;
        index
            .upsert(&page("a", "A", &[], "rust-lang and \"quoted\" text"))
            .await
            .unwrap();

        for query in [
            "\"",
            "\"\"",
            "*",
            "(",
            ")",
            "AND",
            "OR",
            "NOT",
            "NEAR",
            "^",
            ":",
            "-",
            "rust-lang",
            "a OR b",
            "\"unclosed",
            "x AND (y",
            "",
            "   ",
        ] {
            let result = index.search(query, 10, 0).await;
            assert!(result.is_ok(), "query {query:?} failed: {result:?}");
        }

        // A hyphenated term still finds the page rather than being read as NOT.
        assert_eq!(index.search("rust-lang", 10, 0).await.unwrap().total, 1);
    }

    #[tokio::test]
    async fn an_empty_query_matches_nothing_rather_than_everything() {
        let index = index().await;
        index.upsert(&page("a", "A", &[], "alpha")).await.unwrap();

        let results = index.search("   ", 10, 0).await.unwrap();
        assert_eq!(results.total, 0);
        assert!(results.hits.is_empty());
    }

    #[tokio::test]
    async fn search_paginates_and_reports_the_full_total() {
        let index = index().await;
        for n in 0..5 {
            index
                .upsert(&page(&format!("page-{n}"), "T", &[], "alpha"))
                .await
                .unwrap();
        }

        let first = index.search("alpha", 2, 0).await.unwrap();
        assert_eq!(first.hits.len(), 2);
        assert_eq!(first.total, 5, "total must count all matches, not the page");

        let last = index.search("alpha", 2, 4).await.unwrap();
        assert_eq!(last.hits.len(), 1);
        assert_eq!(last.total, 5);
    }

    #[tokio::test]
    async fn stamps_round_trip_exactly() {
        let index = index().await;
        let mut page = page("notes/rhizome", "Rhizome", &[], "Body.");
        // Nanosecond precision, to catch a lossy timestamp round trip.
        page.updated = DateTime::from_timestamp_nanos(1_700_000_000_123_456_789);
        page.size = 4242;

        index.upsert(&page).await.unwrap();
        let stamps = index.stamps().await.unwrap();

        let stamp = stamps.get(&page.slug).expect("page is stamped");
        assert_eq!(stamp.updated, page.updated);
        assert_eq!(stamp.size, 4242);
    }

    #[tokio::test]
    async fn lists_pages_without_their_bodies() {
        let index = index().await;
        index
            .upsert(&page("notes/rhizome", "Rhizome", &["theory"], "Long body."))
            .await
            .unwrap();

        let list = index.list(ListOptions::default()).await.unwrap();

        assert_eq!(list.total, 1);
        let record = &list.pages[0];
        assert_eq!(record.slug.as_str(), "notes/rhizome");
        assert_eq!(record.title, "Rhizome");
        assert_eq!(record.tags, ["theory"]);
    }

    #[tokio::test]
    async fn lists_filtered_by_tag() {
        let index = index().await;
        index
            .upsert(&page("a", "A", &["theory", "shared"], "body"))
            .await
            .unwrap();
        index
            .upsert(&page("b", "B", &["shared"], "body"))
            .await
            .unwrap();

        let theory = index
            .list(ListOptions {
                tag: Some("theory".to_owned()),
                ..ListOptions::default()
            })
            .await
            .unwrap();
        assert_eq!(theory.total, 1);
        assert_eq!(theory.pages[0].slug.as_str(), "a");

        let shared = index
            .list(ListOptions {
                tag: Some("shared".to_owned()),
                ..ListOptions::default()
            })
            .await
            .unwrap();
        assert_eq!(shared.total, 2);

        let missing = index
            .list(ListOptions {
                tag: Some("nonexistent".to_owned()),
                ..ListOptions::default()
            })
            .await
            .unwrap();
        assert_eq!(missing.total, 0);
    }

    /// A wiki where the same directory name occurs in two places, which is the
    /// only interesting case: it is what separates the two path filters.
    async fn branching_index() -> Index {
        let index = index().await;
        for slug in [
            "notes/rust",
            "notes/rust/async",
            "notes/rust/pinning",
            "notes/rustlings",
            "code/rust/traits",
            "index",
        ] {
            index.upsert(&page(slug, slug, &[], "body")).await.unwrap();
        }
        index
    }

    async fn slugs_under(index: &Index, options: ListOptions) -> Vec<String> {
        let list = index.list(options).await.unwrap();
        assert_eq!(list.total, list.pages.len(), "nothing was paginated away");
        list.pages
            .into_iter()
            .map(|page| page.slug.as_str().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn lists_filtered_by_slug_prefix() {
        let index = branching_index().await;

        // The page that names the directory is the directory's index, and
        // belongs in its own listing. `notes/rustlings` does not: the prefix
        // has to stop at the separator, not at the characters.
        let under = slugs_under(
            &index,
            ListOptions {
                prefix: Some("notes/rust".to_owned()),
                ..ListOptions::default()
            },
        )
        .await;
        assert_eq!(
            under,
            ["notes/rust", "notes/rust/async", "notes/rust/pinning"]
        );

        // The prefix is hierarchical, so the other `rust` directory is a
        // different place entirely.
        assert!(!under.contains(&"code/rust/traits".to_owned()));

        // Case-sensitively: `like` would have matched here, and slugs are two
        // different files on a case-sensitive filesystem.
        let shouted = slugs_under(
            &index,
            ListOptions {
                prefix: Some("NOTES/RUST".to_owned()),
                ..ListOptions::default()
            },
        )
        .await;
        assert!(shouted.is_empty());
    }

    #[tokio::test]
    async fn lists_filtered_by_slug_segment() {
        let index = branching_index().await;

        // Flat, like a tag: both `rust` directories answer, wherever they sit.
        let rust = slugs_under(
            &index,
            ListOptions {
                segment: Some("rust".to_owned()),
                ..ListOptions::default()
            },
        )
        .await;
        assert_eq!(
            rust,
            ["code/rust/traits", "notes/rust/async", "notes/rust/pinning"]
        );

        // `notes/rust` is a page named `rust`, not a page inside one, so it is
        // absent — and `notes/rustlings` was never a match to begin with.
        assert!(!rust.contains(&"notes/rust".to_owned()));

        // A page's own name is not a directory it sits in.
        let basename = slugs_under(
            &index,
            ListOptions {
                segment: Some("async".to_owned()),
                ..ListOptions::default()
            },
        )
        .await;
        assert!(basename.is_empty());
    }

    #[tokio::test]
    async fn path_filters_intersect_with_tags() {
        let index = index().await;
        index
            .upsert(&page("notes/rust/async", "A", &["theory"], "body"))
            .await
            .unwrap();
        index
            .upsert(&page("notes/rust/pinning", "P", &[], "body"))
            .await
            .unwrap();
        index
            .upsert(&page("code/rust/traits", "T", &["theory"], "body"))
            .await
            .unwrap();

        let both = slugs_under(
            &index,
            ListOptions {
                tag: Some("theory".to_owned()),
                segment: Some("rust".to_owned()),
                prefix: Some("notes".to_owned()),
                ..ListOptions::default()
            },
        )
        .await;
        assert_eq!(both, ["notes/rust/async"]);
    }

    #[tokio::test]
    async fn a_moved_page_leaves_its_old_directories_behind() {
        let index = index().await;
        index
            .upsert(&page("notes/rust/async", "Async", &[], "body"))
            .await
            .unwrap();

        // A move is an upsert at the new slug and a remove of the old one.
        index
            .upsert(&page("code/rust/async", "Async", &[], "body"))
            .await
            .unwrap();
        index
            .remove(&Slug::parse("notes/rust/async").unwrap())
            .await
            .unwrap();

        let notes = slugs_under(
            &index,
            ListOptions {
                segment: Some("notes".to_owned()),
                ..ListOptions::default()
            },
        )
        .await;
        assert!(notes.is_empty(), "the old directory row outlived the page");
    }

    #[tokio::test]
    async fn lists_sorted_and_paginated() {
        let index = index().await;
        for (slug, title) in [("c", "Gamma"), ("a", "Alpha"), ("b", "Beta")] {
            index.upsert(&page(slug, title, &[], "body")).await.unwrap();
        }

        let by_slug = index.list(ListOptions::default()).await.unwrap();
        let slugs: Vec<&str> = by_slug.pages.iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(slugs, ["a", "b", "c"]);

        let by_title_desc = index
            .list(ListOptions {
                sort: SortBy::Title,
                order: SortOrder::Descending,
                ..ListOptions::default()
            })
            .await
            .unwrap();
        let titles: Vec<&str> = by_title_desc
            .pages
            .iter()
            .map(|p| p.title.as_str())
            .collect();
        assert_eq!(titles, ["Gamma", "Beta", "Alpha"]);

        let page_two = index
            .list(ListOptions {
                limit: 2,
                offset: 2,
                ..ListOptions::default()
            })
            .await
            .unwrap();
        assert_eq!(page_two.pages.len(), 1);
        assert_eq!(page_two.total, 3, "total counts every page, not the slice");
    }

    #[tokio::test]
    async fn clearing_empties_the_index() {
        let index = index().await;
        index
            .upsert(&page("a", "A", &["t"], "alpha"))
            .await
            .unwrap();

        index.clear().await.unwrap();

        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.search("alpha", 10, 0).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn last_sync_round_trips() {
        let index = index().await;
        assert_eq!(index.last_sync().await.unwrap(), None);

        let at = DateTime::from_timestamp_nanos(1_700_000_000_000_000_000);
        index.set_last_sync(at).await.unwrap();

        assert_eq!(index.last_sync().await.unwrap(), Some(at));
    }

    #[test]
    fn fts_queries_are_escaped() {
        assert_eq!(to_fts_query("alpha"), Some("\"alpha\"".to_owned()));
        assert_eq!(
            to_fts_query("alpha beta"),
            Some("\"alpha\" \"beta\"".to_owned())
        );
        assert_eq!(to_fts_query("alpha*"), Some("\"alpha\"*".to_owned()));
        assert_eq!(
            to_fts_query("say \"hi\""),
            Some("\"say\" \"\"\"hi\"\"\"".to_owned())
        );
        assert_eq!(to_fts_query(""), None);
        assert_eq!(to_fts_query("   "), None);
        assert_eq!(to_fts_query("*"), None);
    }
}
