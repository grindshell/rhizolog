# Architecture

The decisions that shape the MVP. See [MVP plan](mvp-plan.md) for the build
order and [API design](api-design.md) for the endpoint surface.

## Storage: files are the truth, SQLite is a cache

Page content lives as `.md` files on disk. SQLite holds a **derived index**
(search, tags, links, stats) that is fully rebuildable by deleting the DB and
restarting.

Why: Rhizolog is a developer tool. Developers expect to `grep` the wiki, edit
it in their own editor, and commit it to git. Agents are far better at reading
and writing markdown files than at driving a CRUD API. Making files
authoritative means neither has to go through us.

The cost is that the index can drift from disk. We accept that and make
reconciliation cheap:

- Full scan on startup, comparing `(mtime, size)` per file against the index.
- A `notify` file watcher for live external edits.
- API writes reindex the touched page synchronously, so a read after a write is
  always consistent.
- `POST /api/reindex` forces a full rebuild.

Reindexing a page is idempotent, so the watcher echo from our own writes is
harmless and needs no suppression logic.

## Layout on disk

```
<wiki root>/
  index.md
  notes/
    rust/async.md
  .rhizolog/
    index.db                              # derived; safe to delete
    server.json                           # volatile; where the server is
    times/
      2026-08/
        20260806T142530-123456789.md      # NOT derived; the only copy
    ideas/
      captures/2026-08/                   # NOT derived; the only copy
      threads/
      events/2026-08/
    words/
      2026-08.log                         # NOT derived; the only copy
    prose.toml                            # NOT derived; authored configuration
    users/
      tim.md                              # NOT derived, and secret
```

The walker skips any directory beginning with `.`, which keeps `.rhizolog/`
and `.git/` out of the wiki.

`.rhizolog/` is therefore **not all disposable**, despite what its name
suggests. The database is; the time log, the idea inbox and the word log beside
it are authored data with no other copy. See [Time tracking](time-tracking.md)
for why they sit under a dot-directory rather than in plain sight, and ignore the
derived files by name rather than the whole directory in a wiki kept in git.

Four kinds of thing live there, and they want different treatment:

| | |
|---|---|
| `index.db` | **derived** — rebuilt from the wiki; deleting it costs one scan |
| `times/` | **authored** — the only copy; back it up, commit it |
| `ideas/` | **authored** as well: captures, threads and decisions, and the only copy of them |
| `words/` | **authored** too: what was written, when, and by which tool. See [Long-form writing](long-form.md) |
| `prose.toml` | **authored configuration**, and absent on most wikis, which is the ordinary case rather than a fault |
| `users/` | **authored, and secret** — the only copy; back it up, do *not* commit it |
| `server.json` | **volatile** — where a running server is; meaningless once it stops |

`prose.toml` is the one thing here that is configuration rather than a record. It
sits with `times/` and `ideas/` rather than with `users/` because it is worth
committing and holds no secrets, and it is read on every request rather than
cached, so tuning a rule is a matter of saving the file and asking again.

`words/` is the one that departs from the shape the others use: a file per month
holding a line per observation, rather than a file per record. The reason is
frequency. A time entry is a document somebody may open and correct; a word
observation is a machine's reading, never edited, arriving every time a file is
saved, and a file per save would be thousands of files a month.

`ideas/` appears on the first capture rather than when the store is opened, and
that is not tidiness: `server::start` opens the stores and then watches the wiki,
so a store that created directories would be the server writing into the tree it
is about to watch. See [Idea Inbox](idea-inbox.md).

`users/` is the odd one: authored data, so it belongs with `times/`, and every
file in it carries an Argon2 hash of a real password, so it is the one authored
thing here that is gitignored. See [Accounts](accounts.md).

The last one is the [published endpoint](desktop-app.md). A server that may not
get the port it asked for has to say where it ended up, and beside the wiki is
where a caller can find it without being told anything it does not already
know.

## Page identity

A page's slug is its path relative to the wiki root with `.md` removed, always
with `/` separators: `notes/rust/async.md` → `notes/rust/async`.

Directories are for humans. The link graph is what actually gives the wiki its
shape, which is the whole point of the "rhizome" framing. A flat wiki is just
the case where no slug contains a `/`.

### A slug has two readings, and both are navigable

The index used to treat a slug as an opaque string. It no longer quite does:
the directories a page sits in are indexed, because a directory name is a
claim about a page and the wiki already has a word for that — a tag.

`notes/rust/async` can be read two ways, and both are worth following:

- **Hierarchically.** The page is *under* `notes/rust`. Following that stays
  inside this branch and finds its siblings. This is `?prefix=`.
- **Flatly.** The page is *in a `rust` directory*. Following that leaves the
  branch entirely and finds `code/rust/traits` too. This is `?segment=`, and it
  behaves exactly like `?tag=` because it is the same kind of question.

The second one is the one that fits the premise. A tree says `notes/rust` and
`code/rust` are unrelated places that happen to share a name; the flat reading
says a directory name means something wherever it is written. Keeping both is
the honest answer — the hierarchy is real, it is just not the only structure
present — and they stay separate controls in the UI rather than one control
that guesses.

A page's own name is not one of these. `async` in `notes/rust/async` names the
page, not a container, so it is not indexed as a directory and does not answer
`?segment=async`. The prefix filter does include the page that *names* a
directory: `?prefix=notes/rust` returns `notes/rust` itself, because a page
sitting where a directory sits is that directory's index and hiding it from its
own listing would be a surprise.

The prefix comparison stops at the separator, so `notes/rustlings` is a
different directory from `notes/rust`. It is done with `substr` rather than
`like`: SQLite's `like` is case-insensitive over ASCII, and slugs are
case-sensitive.

### Slug validation is security-critical

Slugs arrive from HTTP and become filesystem paths. Every slug is parsed
through one `Slug` newtype that rejects anything unsafe, and no other code path
is allowed to build a page path. Rejected: `..` segments, leading `/`, Windows
drive prefixes (`C:`), backslashes, NUL and control characters, reserved
Windows device names (`CON`, `NUL`, `PRN`, `AUX`, `COM1`…), trailing dots or
spaces, and empty segments. After joining, the resolved path is asserted to
still be inside the wiki root.

Development is on Windows, where several of these are silently accepted by the
OS in ways Linux would reject — so the tests must cover them explicitly rather
than relying on the platform to complain.

## File format

YAML frontmatter plus a markdown body:

```markdown
---
title: Rhizome
tags: [theory, deleuze]
created: 2026-08-05T10:00:00Z
---

Knowledge branches off chaotically. See [[notes/rust/async]].
```

- **`title`** — optional. Falls back to the first `# H1`, then to the slug's
  humanized basename.
- **`tags`** — optional list.
- **`created`** — set once, on creation. Written as a full timestamp and read as
  either that or a bare `2026-08-19`, which means midnight UTC.
- **`visibility`** — optional, one of `public`, `internal`, `restricted`,
  `private`. Absent means `internal`, and an unrecognised word means `private`.
- **`owner`** and **`readers`** — optional; who a `private` or `restricted` page
  belongs to and who else may read it. All three do nothing on a wiki with no
  accounts. See [Page visibility](visibility.md).
- **There is no `updated` field.** It is read from the file's mtime instead.

That last one matters: if `updated` lived in frontmatter, every hand-edit and
every `git checkout` would leave it lying. Deriving it from the filesystem
keeps it honest and is consistent with files being the source of truth.

`serde_yaml` is unmaintained (it is published as `0.9.34+deprecated`), so
frontmatter uses `serde_yaml_ng`.

### A date somebody typed must not cost them the page

Every other field here is a string, so YAML's reserved words survive being
written and read back — a title of `123`, a tag of `no`, an owner called `null`.
`created` is the exception: it is parsed into a `DateTime<Utc>`, and a value it
cannot read is not a missing field but a **malformed page**, which drops out of
every listing taking its title and tags with it.

That is a heavy price for `created: 2026-08-19`, which is how a person writes a
date. So `frontmatter::timestamp` reads a bare date as midnight UTC. There is no
time in a bare date to lose, so filling one in invents nothing.

A wall-clock time with no zone — `2026-08-19T10:00:00` — is still refused, and
the difference is the point: that value carries a real time whose meaning depends
on where it was written, and reading it as UTC would move it silently by up to
fourteen hours. The refusal names both forms that would have worked.

Rewriting normalises a bare date to the full timestamp. This module otherwise
keeps frontmatter as written, but `created` was never kept as written — it is a
parsed value, so an offset was already being normalised away.

### Two ways to be refused, and they are not the same

A block that will not parse and a block that parses into the wrong shape used to
be one error, reported as *"frontmatter is not valid YAML"* whichever it was.
That is a bad thing to be told about YAML that is fine and merely holds a word
where a date belongs: it sends a reader to their quoting rather than to their
value.

They are separate now, and the split has to happen at the call site, because
`serde_yaml_ng::Error` covers deserialisation as well as parsing and does not say
which it was — the enum behind it is private and `location()` is all it exposes.
So `frontmatter::parse` reads the block **twice**: once into a
`serde_yaml_ng::Value`, which answers "is this YAML at all", and then again into
the target type. The second read goes back to the string rather than through the
`Value`, at the cost of one more parse of a few hundred bytes, because a `Value`
carries no spans and the category is only worth having if the line number comes
with it:

```
frontmatter is not valid YAML: did not find expected ',' or ']' at line 2 column 1
frontmatter has a value that could not be read: title: invalid type: sequence, expected a string at line 2 column 3
frontmatter has a value that could not be read: created: invalid value: string "tomorrow", expected a timestamp like 2026-08-19T10:00:00Z or a date like 2026-08-19 at line 3 column 10
```

That last one is also why `frontmatter::timestamp` raises its refusal from inside
a serde `Visitor` rather than after deserialising a `String` and checking it:
`serde_yaml_ng` attaches the position of the node it is standing on, and once a
value has been read out it is no longer standing on one.

`PageError` and `UserError` wrap `FrontmatterError` rather than restating its
variants, which is what `TimeError` already did. Restating meant three copies of
each message — and the message was the thing being got wrong.

### A UTF-8 BOM is stripped before parsing

`read_to_string` keeps a byte order mark — U+FEFF is a valid character — so a
file that starts with one does not start with `---`, and its entire frontmatter
block gets read as body. Title, tags, and `created` vanish, and the title
silently falls back to the humanized slug.

This is not a corner case on Windows: Notepad, PowerShell's
`Set-Content -Encoding utf8`, and any editor set to "UTF-8 with BOM" all write
one by default. It was found by hand-editing a page against a running server —
every test until then had built its input as a Rust string literal, where the
problem cannot occur.

Two details worth keeping:

- The recorded `size` counts the BOM, because it is part of the file. Measuring
  the stripped text instead would make every scan see a size mismatch and
  reindex that page forever.
- The BOM is not written back. A page that round-trips through the API comes
  out normalised without one.

## The link graph

Two link forms are extracted at index time:

- `[[slug]]` and `[[slug|display text]]` — wiki links
- Standard markdown `[text](target)` — internal if it resolves to a page,
  external otherwise

Extraction goes through comrak's AST, not a regex over the source. comrak parses
`[[slug]]` natively, which means a wikilink inside a code fence stays inside the
code fence — the parser has already decided what is code and what is prose, and
a regex would have to relitigate that and get it wrong. A page documenting
wikilink syntax should not acquire links by describing them.

Markdown link targets are resolved relative to the page's own directory, so
`[rhizome](../rhizome.md)` from `notes/rust/async` reaches `notes/rhizome`. A
target that climbs out of the wiki is **dropped**, not recorded as broken: a
wanted page should be something you could go and create, and
`../../etc/passwd` is not.

### Resolution is exact, and it is a query

A wikilink target is a slug, matched exactly. There is no basename fallback —
an earlier draft of this page promised that `[[async]]` would find
`notes/rust/async` when nothing else was named `async`, and that is now
withdrawn for two reasons.

The first is that it makes a link's meaning depend on wiki state in a way that
changes silently. Write a second page called `async` and every existing
`[[async]]` quietly retargets or goes ambiguous. For a tool whose whole subject
is how knowledge branches, links that move on their own are the wrong kind of
surprise.

The second is that exact matching keeps resolution a **pure join** against
`pages`, evaluated at query time and never cached. That is what makes the graph
self-healing: create a page that three others already link to, and those three
links resolve immediately, with nothing to reindex. A basename fallback would
need either a re-resolve pass on every page create or a materially hairier
query.

The cost is verbosity — you write `[[notes/rust/async]]`, not `[[async]]`. The
editor can offer completion for that; it cannot un-break a link that retargeted
itself.

**Unresolved links are a feature, not an error.** They are "wanted pages" —
branches someone gestured at but has not written yet — and they surface in
`/api/stats`. Combined with orphans (pages nothing links to), that is the main
meta-stat the dashboard exists to show.

**A `contents:` entry is not a link, and it is still an edge.** It lives in
`page_parts` rather than in `links`, because that table is keyed
`(src_slug, target, kind)` and cannot hold the same child twice under one parent,
which is exactly the case an appendix listed under two parts is. But a page has
one parent where it has hundreds of time entries, so the graph unions the spine
in where it keeps time out: a chapter named by its book is not an orphan, the
drawing marks that line `part` rather than giving it a sixth `kind`, and a walk
crosses it, so a chapter's neighbourhood holds the book it belongs to. A
`contents:` entry that is not a valid slug is reported in the manifest and never
drawn: it is a mistake somebody made, not a page worth writing. See
[Long-form writing](long-form.md).

This is also why page moves do not rewrite backlinks in the MVP: a move turns
inbound links into wanted pages, which shows up in the stats rather than
silently rotting. Link-rewriting on move is a post-MVP convenience.

## Index schema

Derived from the wiki, and therefore disposable:

```sql
pages(slug PK, title, created, updated, size, visibility, owner)
page_tags(slug, tag)
page_readers(slug, username)            -- who a restricted page admits
page_segments(slug, segment, depth)     -- the directories a page sits in
links(src_slug, target, display, kind)  -- kind: wiki | internal | external
pages_fts                               -- FTS5 over (slug unindexed, title, body)
times(id PK, name, started, ended, has_note, updated, size)
time_pages(time_id, target)             -- the pages an entry was spent on
times_fts                               -- FTS5 over (id unindexed, name, note)
idea_captures(id PK, owner, created, updated, size)
idea_captures_fts                       -- FTS5 over (id unindexed, body)
idea_terms(capture_id, term, occurrences)      -- the analyzer's unigrams and bigrams
idea_threads(id PK, owner, name, created, updated, size)
idea_seed_captures(idea_id, capture_id)        -- what a thread was started from
idea_events(id PK, owner, kind, idea_id, capture_id, other_capture_id,
            page_slug, created, updated, size)
idea_membership(idea_id, capture_id)           -- folded from the events
idea_rejections(idea_id, capture_id)           -- folded
idea_capture_rejections(capture_id, other_capture_id)   -- folded
idea_capture_state(capture_id, archived)       -- folded
idea_thread_state(idea_id, retired, promoted_to, last_signal)  -- folded
```

`times` is derived from the files under `.rhizolog/times/`, exactly as `pages`
is derived from the markdown beside them, so the startup scan has two trees to
reconcile rather than one and a version bump rebuilds both.

`time_pages` is deliberately **not** rows in `links`. A page collects one of
these every time a timer starts, so hundreds is ordinary, and mixing them in
would drown its backlinks and make it the most-linked page in the wiki. See
[Time tracking](time-tracking.md).

`ended` and `started`, not `end` and `start`: `end` closes a `case` in SQLite,
and a column that must be quoted in every query it appears in is a column that
eventually will not be.

Five of the `idea_*` tables are **folds**: membership, the two kinds of
rejection, capture state and thread state. Every one of them is recomputed by a
single statement over `idea_seed_captures` and `idea_events`,
keyed on whatever just changed, and each ends in `order by id desc limit 1`,
which is latest-decision-wins spelled in SQL. There is no separate rebuild path,
so a rebuild and an incremental update cannot disagree about what an idea holds.
What is deliberately *not* here is any lifecycle state or momentum: those are
computed when read, from these rows and an instant, so the same files answer
differently tomorrow. `idea_membership` carries no foreign key to
`idea_captures` on purpose, which is what makes a thread able to say what
evidence it has lost. See [Idea Inbox](idea-inbox.md).

`visibility`, `owner` and `page_readers` are what every page-returning query
filters on, and they are derived like everything else here — the frontmatter is
the truth, and a page's visibility on the wire comes from the file that was just
read rather than from whatever the last scan wrote down. Storing them anyway is
what lets the listing be filtered in SQL instead of by reading ten thousand files
off disk. The schema version was bumped when they landed, because an index built
before them has `visibility` nowhere and would serve every page to everybody.
See [Page visibility](visibility.md).

`page_segments` is redundant with the slug in `pages` and exists only to make
"every page in a `rust` directory" an indexed lookup instead of a scan. `depth`
is in its key rather than `segment`, because a slug may pass through the same
name twice (`notes/rust/notes/pinning`) and position is what tells the two rows
apart.

Not derived from anything, and therefore kept:

```sql
meta(key, value)               -- schema_version, last_sync
api_usage(route, method, count)
pins(slug PK, pinned_at)       -- pages kept within reach
sessions(token_hash PK, username, created, expires)
```

`sessions` is durable for a different reason from the other three. Losing them
is survivable — it signs everybody out, which is exactly what deleting the
database should do — but a version bump is an ordinary consequence of changing
how *pages* are indexed, and that has nothing to do with who is signed in. The
key is the SHA-256 of the token rather than the token, and there is no foreign
key to an account because accounts are files. See [Accounts](accounts.md).

`pins` deliberately has no `references pages(slug)`: a foreign key from a
durable table into a derived one would either block the rebuild or cascade the
pins away with it, so a pin is resolved by joining `pages` at read time. See
[Pins](pins.md).

Page bodies live only in the FTS5 table, never duplicated in `pages`; full
content always comes from disk. FTS5 is confirmed available in the bundled
SQLite (3.53.2) that `rusqlite`'s `bundled` feature compiles, including the
`unicode61` and `trigram` tokenizers and the `snippet()` function used to build
search result excerpts.

`links.target` holds the target **as written** — a slug for wiki and internal
links, a URL for external ones. It is never resolved once and stored; see
"Resolution is exact, and it is a query" above.

### A full-text row is found by rowid, and nothing else will do

`pages_fts` carries `slug` so a hit can name its page without a join, and
`times_fts` carries `id` for the same reason. **Neither can be searched on.**
Both are `unindexed` columns, and an FTS5 table has exactly two ways in: a
`MATCH` against the text, and its `rowid`. Anything else is a full scan of the
table.

That is easy to write by accident, because the SQL looks ordinary. FTS5 has no
upsert, so reindexing a page means deleting its old row first, and
`delete from pages_fts where slug = ?` is the obvious spelling. It also scans
every row in the table — so indexing the *n*th page reads *n* rows, and a full
index is quadratic. On a wiki of 20,000 pages that was **eight minutes and
fifty-five seconds** of blank screen before the server was ready, against
eighteen seconds once the delete went by rowid. It was invisible for as long as
it was because the example wiki has nine pages, where the difference is a
millisecond.

It was not only a startup cost. `upsert` runs on every API write and every file
the watcher notices, so before the fix each save scanned the whole full-text
table too, and a wiki got slower to edit as it grew.

The rowid used is the `pages` (or `times`) row's own. That is what makes
`insert or replace into pages` unusable: `replace` deletes the conflicting row
and inserts a new one, which allocates a **new** rowid and orphans the full-text
row keyed to the old one. Both tables are written with
`on conflict(...) do update` instead, which updates in place and keeps the
rowid — so the identity a page's searchable text hangs from lives exactly as
long as the page does.

Two tests in `index/mod.rs` and one in `index/times.rs` guard it, and they are
written the way the failure actually presents: rewrite one page, then assert the
*other* pages are still searchable. A mismatched rowid deletes somebody else's
text, and the page being rewritten looks perfectly correct afterwards.

### There are no migrations

The schema carries a version number. When it changes, the derived tables are
**dropped and rebuilt from the wiki** rather than migrated. This is the real
payoff of keeping files authoritative: a schema change costs one scan and no
migration script, forever.

The split matters here. API usage counts are the one thing in the index with no
source to rebuild from, so they live in tables a version bump leaves alone —
which also means a change to *those* would need a real migration. Keep them
boring.

### API usage

Counted in memory and flushed to SQLite every 60 seconds and on shutdown. One
database write per request would be absurd for a number nobody reads in real
time. `/api/stats` adds the unflushed tally to the persisted counts, so the
figure it reports is current without the read having to write.

The counter keys on axum's `MatchedPath` — the route template
(`/api/pages/{slug}`), not the URL that arrived. Keying on the raw URI would
grow a row per page ever fetched, which is unbounded and useless. The template
is spelled the way the OpenAPI document spells it, so the stats and the docs
agree on what an endpoint is called.

### Timestamps are integers, and there is no content hash

Timestamps are stored as nanoseconds since the Unix epoch, not as RFC 3339
text. The scan decides whether a page changed by comparing its recorded mtime
against the filesystem's, and integers compare exactly where a round-tripped
string invites precision bugs.

An earlier draft of this page had a `body_hash` column. It is gone: mtime and
size already answer the only question the scan asks, and a column nothing reads
is worse than no column. The gap — an edit that preserves both mtime and size —
needs a tool that restores mtime plus a replacement of exactly equal length, and
a full rebuild fixes it. Hashing every file on every startup is the worse trade.

## Search behaviour

Query terms are matched **literally**. Each whitespace-separated term is quoted
before it reaches FTS5, so punctuation a user typed cannot become query syntax
and a stray `"` cannot turn a search box into a 500. Terms combine with an
implicit AND, and a trailing `*` still means prefix search so search-as-you-type
stays usable.

The cost is that FTS5's own operators (`OR`, `NEAR`) are unreachable. For a
single-user wiki, a search that never errors is worth more than a query
language; passthrough can be a flag later if it is ever missed.

One implementation note worth keeping, because it is not obvious from the SQLite
docs: FTS5's auxiliary functions (`snippet`, `bm25`) take the table's **real
name**, not a query alias. `snippet(f, ...)` after `from pages_fts f` fails with
"no such column: f".

## Concurrency

Deliberately boring: one writer at a time behind a `tokio::sync::Mutex`, and one
`rusqlite::Connection` behind a mutex accessed through `spawn_blocking`. All SQL
lives in the `index` module, so the blocking boilerplate is contained to one file
rather than spread across handlers.

[Accounts](accounts.md) put more than one person on the other end of that, which
does not change the design but does change how much of it is load-bearing.
The store's own comment — "the only way to lose that race is to race yourself" —
is no longer strictly true of a networked instance. What protects it is that a
page write is a rename of a fully-written temporary file, so two writers produce
one of the two pages rather than half of each; last write wins, and there is no
optimistic concurrency to say so. Worth revisiting if a networked instance ever
gets busy.

Password hashing is the one genuinely CPU-bound thing in the process. Argon2 is
slow on purpose — tens of milliseconds — so every hash and verify goes through
`spawn_blocking`; one on an executor thread would stall every other request in
the process for as long as it ran.

`rusqlite` over `sqlx` because the `bundled` feature vendors SQLite (no system
library to install on Windows) and there is no async story worth paying for
when concurrent writes are structurally impossible.

## No history in the MVP

Pages carry `created` and an mtime-derived `updated`, nothing more. The wiki
directory is very likely to be a git repo already, which covers history for the
one user who exists. Revisions, diffs, and restore are post-MVP.

## Module layout

`backend/` becomes a lib plus a thin bin, so integration tests can build the
router in-process:

```
src/
  main.rs        the headless binary: tracing, config, wait for Ctrl-C
  server.rs      start / shutdown: the whole boot sequence, and stopping it
  lib.rs         re-exports; builds the axum Router
  config.rs      env-driven config
  error.rs       AppError -> one JSON error shape
  slug.rs        Slug newtype + validation
  frontmatter.rs splitting a YAML block from the markdown after it
  page.rs        Page, Frontmatter, parse/serialize round-trip
  markdown.rs    comrak render, link extraction, wikilink rewriting
  store.rs       filesystem read/write/list/delete/move
  times/         TimeId, TimeEntry, the time log on disk, statistics
  ideas/         captures, threads and decisions: the store, the rules, and the
                 two pure halves (analysis.rs, lifecycle.rs) that explain them
  words/         the word log on disk, the churn diff, the series
  prose/         prose/v1: the rules file, the five rule kinds, tokenizing
  compile.rs     assembling a tree of pages into one document and a manifest
  users/         Username, User, the accounts on disk, password hashing
  auth.rs        who a request is: sessions, the Viewer, the gate in front of /api
  index/         SQLite: schema, upsert, search, links, tags, pins, times, ideas,
                 words, sessions, stats
                 audience.rs: the one visibility predicate every page query pastes in
  watcher.rs     notify -> reindex queue
  assets.rs      the built dashboard: Dir | Embedded | None
  endpoint.rs    .rhizolog/server.json: publish, withdraw, confirm
  api/           route handlers + OpenApi assembly
```

`frontmatter.rs` exists because a page, a time entry and an account are all "a
small YAML header, then prose". The answers to what counts as a fence, what a
BOM does, and how the two halves go back together belong in one place — the
second copy is where they quietly diverge, and the third would be the one
holding password hashes.

`server.rs` holds everything between "here is a config" and "it is serving":
opening the four stores, reconciling the index, binding, starting the watcher
and the usage flusher, and stopping all of them again afterwards. It is not in
`main.rs` because that sequence has more than one driver, and they differ only
in the last step — a console binary stops on Ctrl-C, a desktop shell stops when
its window closes, and a test stops when it has finished asserting.
`server::start` returns once the server is **ready** rather than once it has
begun, which is what lets a caller hand out the address it bound. See
[The desktop app](desktop-app.md).

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `RHIZOLOG_ROOT` | `./wiki` | Wiki directory |
| `RHIZOLOG_DB` | `<root>/.rhizolog/index.db` | Derived index |
| `RHIZOLOG_ADDR` | `127.0.0.1:3000`, or any free port | Listen address |
| `RHIZOLOG_ASSETS` | `../frontend/dist` | Built dashboard; missing is fine |
| `RHIZOLOG_LOG` | `rhizolog=info,tower_http=info` | `tracing` filter |
| `RHIZOLOG_SECURE_COOKIES` | off | Mark the session cookie `Secure`; on behind TLS |
| `RHIZOLOG_ANONYMOUS_READ` | off | Serve `public` pages to callers who have not signed in |

Binding to loopback by default is intentional, and [accounts](accounts.md) did
not change it: a wiki with no accounts is open, and its API can write files
anywhere under the wiki root, so a public default would be exactly as wrong as
it ever was. The fallback keeps the same host for that reason — a loopback
default cannot become a public bind by giving way. What is new is that binding
wider is now a supported thing to *choose*, and `server::start` warns at startup
when an instance is bound off loopback with no accounts to sign in to.

`RHIZOLOG_ANONYMOUS_READ` is the other half of marking a page `public`, and it is
a variable rather than something derived from the wiki because it answers a
question nothing on disk knows: not "does this wiki have accounts", which is a
fact about a directory, but "should strangers be able to read this instance",
which is a statement of deployment intent. Publishing therefore takes two
deliberate acts in two places — a line in a file and a variable in a deployment —
and neither is much use without the other. See
[Page visibility](visibility.md).

`RHIZOLOG_SECURE_COOKIES` has to default off, because the server speaks HTTP and
a browser discards a `Secure` cookie that arrives over one — which presents as a
sign-in that returns `200` and leaves you signed out. It is a switch rather than
something read from `X-Forwarded-Proto`, since inferring it means trusting a
header anybody who can reach the port can send.

### A default is a preference; a variable is a requirement

`config::Listen` carries the difference. `RHIZOLOG_ADDR` is `Exactly`: it fails
if the address is taken, because somebody who wrote a port down wrote it down
somewhere else too, and quietly serving elsewhere would point that somewhere
else at nothing. The default is `Preferably`: 3000 if it is free, otherwise
whatever the OS hands out, because a second copy finding 3000 busy is ordinary
and refusing to start would be a poor answer to it.

Giving way is only survivable because the result gets published — see
`.rhizolog/server.json` above.
