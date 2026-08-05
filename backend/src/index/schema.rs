//! The index schema, and the reason there are no migrations.
//!
//! Everything here is derived from the markdown files on disk. That makes the
//! schema disposable: when [`SCHEMA_VERSION`] changes, the index is dropped and
//! rebuilt from the wiki rather than migrated. A schema change costs one scan,
//! so it never costs a migration script — which is most of the payoff for
//! keeping files as the source of truth.

/// Bump this whenever the statements below change. The next startup will
/// notice, drop the index, and rebuild it from disk.
pub const SCHEMA_VERSION: i64 = 1;

pub const KEY_SCHEMA_VERSION: &str = "schema_version";
pub const KEY_LAST_SYNC: &str = "last_sync";

/// Column index of `body` within `pages_fts`, for `snippet()`.
pub const FTS_BODY_COLUMN: i32 = 2;

/// Timestamps are stored as nanoseconds since the Unix epoch rather than as
/// text. The startup scan decides whether a page changed by comparing its
/// recorded mtime against the filesystem's, and integers compare exactly where
/// round-tripped RFC 3339 invites precision bugs.
pub const CREATE: &str = "
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

create virtual table pages_fts using fts5(
    slug unindexed,
    title,
    body,
    tokenize = 'unicode61'
);

create table meta (
    key   text primary key,
    value text not null
) strict;
";

/// Dropped in dependency order so `page_tags`' foreign key never blocks.
pub const DROP: &str = "
drop table if exists page_tags;
drop table if exists pages_fts;
drop table if exists pages;
drop table if exists meta;
";
