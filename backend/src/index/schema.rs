//! The index schema, and the reason there are no migrations.
//!
//! Almost everything here is derived from the markdown files on disk. That
//! makes it disposable: when [`SCHEMA_VERSION`] changes, the derived tables are
//! dropped and rebuilt from the wiki rather than migrated. A schema change
//! costs one scan, so it never costs a migration script — which is most of the
//! payoff for keeping files as the source of truth.
//!
//! The exception is [`CREATE_DURABLE`]. API usage counts are not derived from
//! anything; there is nowhere to rebuild them from. They live in tables that a
//! version bump leaves alone, which also means a change to *those* would need a
//! real migration. Keep them boring.

/// Bump this whenever [`CREATE_DERIVED`] changes. The next startup will notice,
/// drop the derived tables, and rebuild them from disk.
pub const SCHEMA_VERSION: i64 = 2;

pub const KEY_SCHEMA_VERSION: &str = "schema_version";
pub const KEY_LAST_SYNC: &str = "last_sync";

/// Column index of `body` within `pages_fts`, for `snippet()`.
pub const FTS_BODY_COLUMN: i32 = 2;

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

create virtual table pages_fts using fts5(
    slug unindexed,
    title,
    body,
    tokenize = 'unicode61'
);
";

/// Dropped in dependency order so the foreign keys never block.
pub const DROP_DERIVED: &str = "
drop table if exists links;
drop table if exists page_tags;
drop table if exists pages_fts;
drop table if exists pages;
";
