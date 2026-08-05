# Architecture

The decisions that shape the MVP. See [MVP plan](mvp-plan.md) for the build
order and [API design](api-design.md) for the endpoint surface.

## Storage: files are the truth, SQLite is a cache

Page content lives as `.md` files on disk. SQLite holds a **derived index**
(search, tags, links, stats) that is fully rebuildable by deleting the DB and
restarting.

Why: Rhizowiki is a developer tool. Developers expect to `grep` the wiki, edit
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
  .rhizowiki/
    index.db        # derived; safe to delete
```

The walker skips any directory beginning with `.`, which keeps `.rhizowiki/`
and `.git/` out of the wiki.

## Page identity

A page's slug is its path relative to the wiki root with `.md` removed, always
with `/` separators: `notes/rust/async.md` → `notes/rust/async`.

Directories are for humans. The index treats a slug as an opaque string — the
link graph is what actually gives the wiki its shape, which is the whole point
of the "rhizome" framing. A flat wiki is just the case where no slug contains
a `/`.

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
- **`created`** — set once, on creation.
- **There is no `updated` field.** It is read from the file's mtime instead.

That last one matters: if `updated` lived in frontmatter, every hand-edit and
every `git checkout` would leave it lying. Deriving it from the filesystem
keeps it honest and is consistent with files being the source of truth.

`serde_yaml` is unmaintained (it is published as `0.9.34+deprecated`), so
frontmatter uses `serde_yaml_ng`.

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

This is also why page moves do not rewrite backlinks in the MVP: a move turns
inbound links into wanted pages, which shows up in the stats rather than
silently rotting. Link-rewriting on move is a post-MVP convenience.

## Index schema

Derived from the wiki, and therefore disposable:

```sql
pages(slug PK, title, created, updated, size)
page_tags(slug, tag)
links(src_slug, target, display, kind)  -- kind: wiki | internal | external
pages_fts                               -- FTS5 over (slug unindexed, title, body)
```

Not derived from anything, and therefore kept:

```sql
meta(key, value)               -- schema_version, last_sync
api_usage(route, method, count)
```

Page bodies live only in the FTS5 table, never duplicated in `pages`; full
content always comes from disk. FTS5 is confirmed available in the bundled
SQLite (3.53.2) that `rusqlite`'s `bundled` feature compiles, including the
`unicode61` and `trigram` tokenizers and the `snippet()` function used to build
search result excerpts.

`links.target` holds the target **as written** — a slug for wiki and internal
links, a URL for external ones. It is never resolved once and stored; see
"Resolution is exact, and it is a query" above.

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

Single-user, so this stays deliberately boring: one writer at a time behind a
`tokio::sync::Mutex`, and one `rusqlite::Connection` behind a mutex accessed
through `spawn_blocking`. All SQL lives in the `index` module, so the blocking
boilerplate is contained to one file rather than spread across handlers.

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
  main.rs        bootstrap: config, tracing, open index, spawn watcher, serve
  lib.rs         re-exports; builds the axum Router
  config.rs      env-driven config
  error.rs       AppError -> one JSON error shape
  slug.rs        Slug newtype + validation
  page.rs        Page, Frontmatter, parse/serialize round-trip
  markdown.rs    comrak render, link extraction, wikilink rewriting
  store.rs       filesystem read/write/list/delete/move
  index/         SQLite: schema, migrations, upsert, search, links, tags, stats
  watcher.rs     notify -> reindex queue
  api/           route handlers + OpenApi assembly
```

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `RHIZOWIKI_ROOT` | `./wiki` | Wiki directory |
| `RHIZOWIKI_DB` | `<root>/.rhizowiki/index.db` | Derived index |
| `RHIZOWIKI_ADDR` | `127.0.0.1:3000` | Listen address |

Binding to loopback by default is intentional: single-user, no auth, and the
API can write files anywhere under the wiki root.
