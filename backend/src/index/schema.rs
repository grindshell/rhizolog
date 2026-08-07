//! The index schema, and the reason there are no migrations.
//!
//! Almost everything here is derived from the markdown files on disk. That
//! makes it disposable: when [`SCHEMA_VERSION`] changes, the derived tables are
//! dropped and rebuilt from the wiki rather than migrated. A schema change
//! costs one scan, so it never costs a migration script — which is most of the
//! payoff for keeping files as the source of truth.
//!
//! The exception is [`CREATE_DURABLE`]. API usage counts and pins are not
//! derived from anything; there is nowhere to rebuild them from. They live in
//! tables that a version bump leaves alone, which also means a change to *those*
//! would need a real migration. Keep them boring.

/// Bump this whenever [`CREATE_DERIVED`] changes. The next startup will notice,
/// drop the derived tables, and rebuild them from disk.
///
/// Version 6 changed no DDL. The full-text rows are now keyed by the rowid of
/// the `pages` or `times` row they describe, and an index written before that
/// has arbitrary rowids in its FTS tables — so a delete would miss, or hit
/// somebody else's row. Rebuilding is what puts them back in step, and it costs
/// one scan, which is the whole reason this mechanism exists.
pub const SCHEMA_VERSION: i64 = 6;

pub const KEY_SCHEMA_VERSION: &str = "schema_version";
pub const KEY_LAST_SYNC: &str = "last_sync";

/// Column index of `body` within `pages_fts`, for `snippet()`.
pub const FTS_BODY_COLUMN: i32 = 2;

/// Column index of `note` within `times_fts`, for `snippet()`.
pub const FTS_NOTE_COLUMN: i32 = 2;

/// How many entries the top-N lists in `/api/stats` carry.
pub const TOP_N: usize = 10;

/// Tables that are not derived from the wiki, and so are never dropped.
///
/// Created with `if not exists` because they have to survive the version check
/// that reads `meta` in the first place.
pub const CREATE_DURABLE: &str = "
create table if not exists meta (
    key   text primary key,
    value text not null
) strict;

create table if not exists api_usage (
    route  text    not null,
    method text    not null,
    count  integer not null default 0,
    primary key (route, method)
) strict;

-- Pages the user keeps within reach. Nothing derives them, so like `api_usage`
-- they survive a schema bump.
--
-- Deliberately no `references pages(slug)`: `pages` is derived and gets dropped
-- and rebuilt, which a foreign key from a durable table would either block or
-- silently cascade away. A pin is resolved by joining at read time instead, and
-- a pin whose page is not in the index is reported as missing rather than
-- quietly dropped.
create table if not exists pins (
    slug      text    primary key,
    pinned_at integer not null
) strict;
";

/// Everything rebuildable from the markdown on disk.
///
/// Timestamps are nanoseconds since the Unix epoch rather than text: the
/// startup scan decides whether a page changed by comparing its recorded mtime
/// against the filesystem's, and integers compare exactly where round-tripped
/// RFC 3339 invites precision bugs.
pub const CREATE_DERIVED: &str = "
create table pages (
    slug    text    primary key,
    title   text    not null,
    created integer not null,
    updated integer not null,
    size    integer not null
) strict;

create table page_tags (
    slug text not null references pages(slug) on delete cascade,
    tag  text not null,
    primary key (slug, tag)
) strict;

create index page_tags_by_tag on page_tags(tag);

-- The directories a page sits in, one row each: `notes/rust/async` records
-- `notes` at depth 0 and `rust` at depth 1. It is redundant with the slug in
-- `pages` and exists only to make -- every page in a `rust` directory, wherever
-- that directory sits -- an indexed lookup rather than a scan. That is the same
-- shape as `page_tags` because it is the same question. The page's own name is
-- not a directory and is not recorded.
--
-- `depth` is in the key rather than `segment`: a slug may pass through the same
-- name twice (`notes/rust/notes/pinning`), and position is what distinguishes
-- the two rows.
create table page_segments (
    slug    text    not null references pages(slug) on delete cascade,
    segment text    not null,
    depth   integer not null,
    primary key (slug, depth)
) strict;

create index page_segments_by_segment on page_segments(segment);

-- `target` is a slug for wiki and internal links, and a URL for external ones.
-- It is stored as written and resolved by joining against `pages` at query
-- time, never resolved once and cached: that is what lets a wanted page become
-- a real link the moment someone creates it, with nothing to reindex.
create table links (
    src_slug text not null references pages(slug) on delete cascade,
    target   text not null,
    display  text,
    kind     text not null,
    primary key (src_slug, target, kind)
) strict;

create index links_by_target on links(target);

-- Each row's rowid is the rowid of the `pages` row it describes, because that
-- is the only handle an FTS5 table can be looked up by other than `MATCH`.
-- `slug` is stored so a hit can name its page without a join, but it is
-- `unindexed` and therefore useless to search on: `where slug = ?` scans every
-- row, which is what made rebuilding an index quadratic.
create virtual table pages_fts using fts5(
    slug unindexed,
    title,
    body,
    tokenize = 'unicode61'
);

-- Time entries, derived from `.rhizolog/times/` exactly as `pages` is derived
-- from the markdown beside it. The files are the log; this is only an index
-- over them, and deleting the database loses nothing.
--
-- `started` and `ended` rather than `start` and `end`: `end` is a keyword in
-- SQLite (it closes a `case`), and a column that has to be quoted in every
-- query it appears in is a column that will eventually not be.
--
-- `has_note` is a flag rather than the note itself. A listing wants to show
-- that an entry has something to say without carrying every note in the wiki,
-- and the note comes from disk when one is actually read.
create table times (
    id       text    primary key,
    name     text    not null,
    started  integer not null,
    ended    integer,
    has_note integer not null,
    updated  integer not null,
    size     integer not null
) strict;

create index times_by_start on times(started);

-- The grouping. Entries are grouped by name spelled exactly as written, the
-- same way tags are not normalised.
create index times_by_name on times(name);

-- The link between a time entry and the pages it was spent on.
--
-- Deliberately not a row in `links`. A page may collect hundreds of these, and
-- mixing them into the link graph would drown its backlinks, inflate its
-- referrer count, and make a page with a busy timer look like the most
-- important page in the wiki. They are a different kind of edge and they live
-- in a different table, which is what lets a page report them as one summary
-- line instead of a hundred rows.
--
-- `target` is a slug as written and is resolved by joining `pages` at query
-- time, the same as `links.target`: time can be tracked against a page that
-- does not exist yet, and it starts resolving the moment someone writes it.
create table time_pages (
    time_id text not null references times(id) on delete cascade,
    target  text not null,
    primary key (time_id, target)
) strict;

create index time_pages_by_target on time_pages(target);

-- Search over the log. Both indexed columns are deliberate: `note` is the
-- obvious one, and `name` is what makes the search useful on a log where most
-- entries have no note at all -- `deep` should find every `Deep work` entry.
--
-- That is a different question from `times.name = ?`, which is the group filter
-- and is exact by design. This one is fuzzy and they intersect, so asking for
-- `poll loop` inside `Deep work` is one request.
-- Keyed by the rowid of its `times` row, exactly as `pages_fts` is.
create virtual table times_fts using fts5(
    id unindexed,
    name,
    note,
    tokenize = 'unicode61'
);
";

/// Dropped in dependency order so the foreign keys never block.
pub const DROP_DERIVED: &str = "
drop table if exists links;
drop table if exists page_tags;
drop table if exists page_segments;
drop table if exists pages_fts;
drop table if exists pages;
drop table if exists time_pages;
drop table if exists times_fts;
drop table if exists times;
";
