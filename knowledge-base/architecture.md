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

Resolution: exact slug match first, then a unique basename match (`[[async]]`
finds `notes/rust/async` if nothing else is named `async`). Ambiguous or
missing targets stay unresolved.

**Unresolved links are a feature, not an error.** They are "wanted pages" —
branches someone gestured at but has not written yet — and they surface in
`/api/stats`. Combined with orphans (pages nothing links to), that is the main
meta-stat the dashboard exists to show.

This is also why page moves do not rewrite backlinks in the MVP: a move turns
inbound links into wanted pages, which shows up in the stats rather than
silently rotting. Link-rewriting on move is a post-MVP convenience.

## Index schema

```sql
pages(slug PK, title, path, created, updated, size, body_hash)
page_tags(slug, tag)
links(src_slug, dst_slug, kind, display, resolved)
pages_fts  -- FTS5 over (slug, title, body)
api_usage(route, method, count)
meta(key, value)  -- schema_version, last_full_reindex
```

Page bodies are stored only in the FTS5 table, not duplicated in `pages`; full
content always comes from disk. FTS5 is confirmed available in the bundled
SQLite (3.53.2) that `rusqlite`'s `bundled` feature compiles, including the
`unicode61` and `trigram` tokenizers and the `snippet()` function used to build
search result excerpts.

`api_usage` keys on axum's `MatchedPath` (the route template, e.g.
`/api/pages/{slug}`) rather than the raw URI, so cardinality stays bounded.

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
