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
///
/// Version 7 adds `pages.visibility`, `pages.owner` and `page_readers`. An index
/// written before it has neither, and every page in it would read as visible to
/// everybody — so this is one of the bumps where *not* rebuilding is a leak
/// rather than a stale row. The rebuild is the same one scan it always is.
///
/// Version 8 adds Idea Inbox: the three authored trees under `.rhizolog/ideas/`
/// and the tables folded from them. `idea_terms` was deliberately absent, since
/// a table nothing writes yet is worse than a second version number.
///
/// Version 9 is that second version number, and it is the analyzer arriving:
/// `idea_terms` holds the unigrams and bigrams `tfidf/v1` counts. An index
/// written before it has no terms at all, so candidates would come back empty
/// for every capture rather than wrong, and rebuilding is what fills them in.
/// **Changing how a capture is tokenized is a bump too**, because every row here
/// is the output of that one function.
///
/// Version 11 adds `page_parts`, the ordered spine a `contents:` list names. An
/// index written before it has none, so every chapter in a manuscript would be
/// reported as an orphan and nothing could be compiled.
///
/// Version 10 adds `pages.words`. An index written before it has none, and a
/// column defaulting to zero would report every page in an existing wiki as
/// empty and every prefix total as nothing, which is worse than absent because
/// it looks like an answer. **Changing how a body is counted is a bump too**,
/// for `idea_terms`' reason: every row is the output of that one function. See
/// [`crate::markdown::count_words`] and `knowledge-base/long-form.md`.
///
/// Version 12 adds `page_words`, folded from `.rhizolog/words/`. That directory
/// is a **fourth authored tree**, so this is the first derived table whose
/// source is neither the markdown nor one of the three trees that came before
/// it. An index written before this has none of it, and the series would come
/// back empty rather than wrong. Rebuilding reads the log, which is a few
/// hundred kilobytes and is the only reason it was worth putting on disk.
///
/// Version 13 adds `page_parts.is_slug`, which is [`crate::slug::Slug::parse`]
/// answered once at index time instead of by every reader. An index written
/// before it defaults the column to nothing, and a contents entry that is not a
/// slug would then be advertised as a page worth writing. That is the bump
/// where not rebuilding puts `../etc/passwd` on the dashboard, so it is closer
/// to version 7 than to a stale row.
pub const SCHEMA_VERSION: i64 = 13;

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

-- Sessions: proof that a request has already said who it is.
--
-- Durable, and the reason is not that they are precious. Losing them signs
-- everybody out, which is survivable and is exactly what deleting the database
-- should do. But a version bump is an ordinary consequence of changing how
-- *pages* are indexed, and that has nothing to do with who is signed in.
--
-- The key is the SHA-256 of the token, never the token: this file sits on a
-- disk, and a row that could be lifted out and replayed as a credential is a
-- row worth not writing. No `references` to an account either — accounts are
-- files, not rows, and nothing in SQLite can point at one.
create table if not exists sessions (
    token_hash text    primary key,
    username   text    not null,
    created    integer not null,
    expires    integer not null
) strict;

-- Signing an account out everywhere is a `delete ... where username = ?`, which
-- happens on every password change and every account deletion.
create index if not exists sessions_by_username on sessions(username);
";

/// Everything rebuildable from the markdown on disk.
///
/// Timestamps are nanoseconds since the Unix epoch rather than text: the
/// startup scan decides whether a page changed by comparing its recorded mtime
/// against the filesystem's, and integers compare exactly where round-tripped
/// RFC 3339 invites precision bugs.
pub const CREATE_DERIVED: &str = "
-- `visibility` and `owner` are here rather than in a table of their own because
-- every query that returns a page has to consult them, and a join that is never
-- optional is a column. `not null` on `visibility` matters: a null would make
-- the audience predicate's comparisons null rather than false, and a `where`
-- clause that is null excludes the row — which fails safe, but by accident and
-- in a way nobody reading the SQL would predict. The default is written by the
-- indexer from `Page::visibility`, so there is exactly one place that decides
-- what an unmarked page means.
--
-- `owner` is nullable, and a nullable owner is why the predicate guards every
-- comparison with `:viewer is not null`: in SQL `null = null` is null, so an
-- anonymous caller must never be allowed to reach a comparison against an
-- ownerless page and have it read as a match.
-- `words` is the body's word count, and it is a column for the reason
-- `visibility` is one: it is asked for by every listing and summed over every
-- prefix, and the alternative is reading ten thousand files off disk to add
-- them up. It counts prose rather than bytes, so it is not derivable from
-- `size` -- a page that is mostly a code fence is large and nearly wordless.
create table pages (
    slug       text    primary key,
    title      text    not null,
    created    integer not null,
    updated    integer not null,
    size       integer not null,
    words      integer not null default 0,
    visibility text    not null default 'internal',
    owner      text
) strict;

create index pages_by_visibility on pages(visibility);
create index pages_by_owner on pages(owner);

-- The `readers:` list of a restricted page, one row each. The same shape as
-- `page_tags`, because it is the same kind of question: which pages carry this
-- name.
--
-- Rows are kept for pages that are not restricted, exactly as the file keeps the
-- list — widening a page and narrowing it again should not lose who could read
-- it. The predicate only consults this table when `visibility = 'restricted'`,
-- so a stale row grants nothing.
create table page_readers (
    slug     text not null references pages(slug) on delete cascade,
    username text not null,
    primary key (slug, username)
) strict;

create index page_readers_by_username on page_readers(username);

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

-- The ordered spine: what a page's `contents:` list names, in the order it
-- names it. Not rows in `links`, and the reason is the primary key rather than
-- volume. `links` is keyed (src_slug, target, kind), so one parent cannot list
-- the same child twice -- and an appendix under two parts is exactly the case
-- the manifest reports as `duplicate`. Adding an ordinal to that key would make
-- it representable and would also change what a row *is* for every other kind:
-- `[[a]]` written twice in a page is one row today, and collapsing repeats is
-- what makes a backlink panel readable.
--
-- So position is the identity here. That is the opposite trade from
-- `time_pages`, which is kept out of the graph because a page collects hundreds
-- of them; a page has exactly one parent, so these are edges the graph wants.
--
-- `target` is a slug as written, resolved by joining `pages` at read time like
-- every other target in this schema, so a chapter written later fills its gap
-- with nothing to reindex. It is not a `references`, for the same reason: a
-- contents list may name a page that does not exist yet, and that is a gap in
-- the manuscript rather than an error.
--
-- Which means a mistyped entry is in this table: `../etc/passwd` is stored as
-- written, because the manifest has to report it as `invalid` in its own
-- position. `is_slug` is whether it parses, decided once by `Slug::parse` when
-- the row is written. The rule is security-critical and belongs in exactly one
-- place, so no reader spells it again -- the graph filtered these rows in Rust
-- until this column existed, and a query that forgot to would have advertised a
-- path as a page somebody should write.
create table page_parts (
    src_slug text    not null references pages(slug) on delete cascade,
    ordinal  integer not null,
    target   text    not null,
    is_slug  integer not null,
    primary key (src_slug, ordinal)
) strict;

create index page_parts_by_target on page_parts(target);

-- The word log, folded from `.rhizolog/words/`. The files are the log; this is
-- only an index over them, and deleting the database loses nothing. That
-- sentence is the whole reason the log is not a durable table here: a writing
-- history is unreconstructable, and every document in this project tells the
-- reader that deleting this database costs one scan.
--
-- Rebuilt wholesale rather than compared file by file, because the log is small
-- and a partial rebuild has states a whole one cannot get into.
--
-- `id` is insertion order, which is the order the files were read in, and it is
-- what the last thing that happened at a slug is decided by. Two observations
-- can share an instant; they cannot share an id.
--
-- `src` is the slug a page arrived from, for a `moved` record and nothing else.
-- It is what closes the series at the slug that was vacated, so a new page
-- written there later starts its own rather than inheriting a total.
--
-- No `references pages(slug)`: the history outlives the page. The words were
-- written and deleting the file does not unwrite them.
create table page_words (
    id      integer primary key,
    at      integer not null,
    slug    text    not null,
    actor   text    not null,
    account text    not null,
    kind    text    not null,
    added   integer not null,
    removed integer not null,
    total   integer not null,
    src     text
) strict;

create index page_words_at on page_words(at);
create index page_words_by_slug on page_words(slug, id);
create index page_words_by_src on page_words(src);

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

-- Idea Inbox, derived from `.rhizolog/ideas/` exactly as `times` is derived from
-- the log beside it. Three tables mirror the three authored trees, and the rest
-- are *folded*: they hold what the decision events add up to, and every one of
-- them is thrown away and recomputed whenever anything they depend on changes.
--
-- `owner` is nullable and means the open user of a wiki with no accounts, which
-- is why every idea query compares it with `is` rather than `=`. Under `=` a
-- null owner would never match anything, including itself.
create table idea_captures (
    id      text primary key,
    owner   text,
    created integer not null,
    updated integer not null,
    size    integer not null
) strict;

create index idea_captures_by_owner on idea_captures(owner);
create index idea_captures_by_created on idea_captures(created);

-- The capture's text lives here and nowhere else. It is not a second column on
-- `idea_captures` because an fts5 table stores its content anyway, so a column
-- beside it would be the same string written twice with two chances to disagree.
-- Keyed by the rowid of its `idea_captures` row, exactly as `pages_fts` is.
create virtual table idea_captures_fts using fts5(
    id unindexed,
    body,
    tokenize = 'unicode61'
);

create table idea_threads (
    id      text primary key,
    owner   text,
    name    text not null,
    created integer not null,
    updated integer not null,
    size    integer not null
) strict;

create index idea_threads_by_owner on idea_threads(owner);

-- The `captures:` list from a thread's own file: what it was started from, and
-- never what it currently holds. Deliberately no `references idea_captures`: a
-- capture can be deleted and the grouping somebody made still happened, so this
-- keeps naming it. `idea_membership` below is where still-true is answered.
create table idea_seed_captures (
    idea_id    text not null references idea_threads(id) on delete cascade,
    capture_id text not null,
    primary key (idea_id, capture_id)
) strict;

-- Every decision, one row each. No foreign keys at all: an event may name a
-- record whose file has since been deleted, and the audit trail is supposed to
-- outlive the evidence rather than cascade away with it.
--
-- `id` is the fold order. Every `order by id desc limit 1` below spells the same
-- rule: the latest applicable decision wins. That is the whole of how the folded
-- tables are derived.
create table idea_events (
    id               text    primary key,
    owner            text,
    kind             text    not null,
    idea_id          text,
    capture_id       text,
    other_capture_id text,
    page_slug        text,
    created          integer not null,
    updated          integer not null,
    size             integer not null
) strict;

create index idea_events_by_idea on idea_events(idea_id);
create index idea_events_by_capture on idea_events(capture_id);
create index idea_events_by_other_capture on idea_events(other_capture_id);
create index idea_events_by_owner on idea_events(owner);

-- Folded: which captures an idea currently holds, seeds plus connections minus
-- disconnections. No `references idea_captures` here either, and this is the
-- load-bearing one: a row naming a deleted capture is exactly what an
-- `evidence_missing` idea is made of, and a cascade would erase the symptom
-- rather than report it. Live membership is this table joined to
-- `idea_captures`; the difference is the missing evidence.
create table idea_membership (
    idea_id    text not null references idea_threads(id) on delete cascade,
    capture_id text not null,
    primary key (idea_id, capture_id)
) strict;

create index idea_membership_by_capture on idea_membership(capture_id);

-- Folded: candidates the user said no to, so they are not suggested again.
-- A reconsider event takes the row away.
create table idea_rejections (
    idea_id    text not null references idea_threads(id) on delete cascade,
    capture_id text not null,
    primary key (idea_id, capture_id)
) strict;

-- Folded: the same, for two loose captures suggested for each other before any
-- thread exists. The pair is held in lexical order, so it has one identity
-- whichever capture produced the suggestion, and the fold only writes a row
-- while both captures are still there.
create table idea_capture_rejections (
    capture_id       text not null,
    other_capture_id text not null,
    primary key (capture_id, other_capture_id)
) strict;

create index idea_capture_rejections_by_other on idea_capture_rejections(other_capture_id);

-- Folded: whether a capture has been archived out of the inbox. Sparse, because
-- most captures have never been archived or restored, and an absent row is not
-- archived.
create table idea_capture_state (
    capture_id text primary key references idea_captures(id) on delete cascade,
    archived   integer not null
) strict;

-- Folded: everything about a thread that its own file does not say.
--
-- `last_signal` is the latest of a connected capture's creation time and an
-- affirm, connect, reopen or promote event, and it is the one folded value that
-- reaches outside the event log: it reads `idea_captures.created`. That is why
-- indexing or removing a capture has to recompute every idea that names it.
--
-- Neither the lifecycle label nor the momentum score is here. Both are pure
-- functions of this row and an explicit `at`, so storing them would be storing
-- an answer to a question nobody had asked yet.
create table idea_thread_state (
    idea_id     text    primary key references idea_threads(id) on delete cascade,
    retired     integer not null,
    promoted_to text,
    last_signal integer
) strict;

-- The analyzer's view of a capture: every unigram and every adjacent bigram in
-- its text, counted. Derived from the body by `ideas::analysis::counts` and by
-- nothing else, which is why changing that function is a schema-version change
-- even though it changes no DDL.
--
-- Separate from `idea_captures_fts`, which tokenizes for a different purpose.
-- Search wants to find a capture from a word somebody typed into a box; this
-- wants weights, occurrence counts and bigrams, and fts5 offers none of the
-- three without reaching into its internals.
--
-- No `owner` column. Every read joins `idea_captures` for it, so there is one
-- place a capture's owner is recorded and no way for the two to disagree.
create table idea_terms (
    capture_id  text    not null references idea_captures(id) on delete cascade,
    term        text    not null,
    occurrences integer not null,
    primary key (capture_id, term)
) strict;

create index idea_terms_by_term on idea_terms(term);
";

/// Dropped in dependency order so the foreign keys never block.
pub const DROP_DERIVED: &str = "
drop table if exists links;
drop table if exists page_tags;
drop table if exists page_segments;
drop table if exists page_readers;
drop table if exists pages_fts;
drop table if exists pages;
drop table if exists time_pages;
drop table if exists times_fts;
drop table if exists times;
drop table if exists idea_membership;
drop table if exists idea_rejections;
drop table if exists idea_thread_state;
drop table if exists idea_seed_captures;
drop table if exists idea_capture_state;
drop table if exists idea_capture_rejections;
drop table if exists idea_events;
drop table if exists idea_terms;
drop table if exists idea_captures_fts;
drop table if exists idea_captures;
drop table if exists idea_threads;
";
