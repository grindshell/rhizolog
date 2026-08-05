# MVP plan

The finish line: **a wiki I can actually keep my notes in.** Markdown files on
disk, a full HTTP API over them, and an admin dashboard good enough to search,
read, and write pages without touching curl.

Design decisions live in [Architecture](architecture.md); the endpoint surface
lives in [API design](api-design.md).

## Scope

In: page CRUD, full-text search, tags, the link graph and backlinks,
meta-stats, OpenAPI + Swagger UI, live pickup of external file edits, and a
Solid dashboard with a working editor.

Out: page history and diffs, auth, multi-user anything, link rewriting on move,
attachments and image upload, a rich-text or CodeMirror editor, transclusion,
namespaces.

## Milestones

Each ends somewhere demoable, and each is a reasonable commit boundary.

### M0 — Skeleton
`axum` server, `/api/health`, tracing, env config, `utoipa` wired through
`OpenApiRouter` with Swagger UI mounted. Split `backend/` into `lib.rs` +
`main.rs` now, before there is anything to untangle, so integration tests can
build the router in-process.

*Done when:* `cargo run` serves a health check and `/swagger-ui` renders.

### M1 — Pages on disk
The `Slug` newtype and its validation, frontmatter parse/serialize, and the
filesystem store (read, write, list, delete, move). No HTTP yet — this is a
library with tests.

*Done when:* a page round-trips through parse → serialize unchanged, and the
slug traversal tests pass (including the Windows-specific cases: drive
prefixes, backslashes, reserved device names, trailing dots).

**Do not skimp here.** Every path that touches the filesystem funnels through
this module, and it is the only place in the MVP where a bug is a security bug
rather than a wrong number on a dashboard.

### M2 — Index and search
SQLite schema and migrations, page upsert/delete, the startup full scan with
`(mtime, size)` comparison, and FTS5 search with `snippet()` excerpts.

*Done when:* dropping 50 markdown files into the wiki dir and starting the
server makes them all searchable, and deleting `index.db` rebuilds identically.

### M3 — Page CRUD API
Wire the store and index together behind the `/api/pages` endpoints. Establish
the `AppError` → JSON error shape here, with `IntoResponse` and utoipa
responses, before there are twelve handlers to retrofit.

*Done when:* a page can be created, read, edited, moved, and deleted entirely
through Swagger UI, and each write is immediately visible to search.

### M4 — The graph
Link extraction during indexing, the `links` table, and the
`/links`, `/backlinks`, `/tags`, and `/stats` endpoints. Stats covers page and
link counts, orphans, wanted pages, the tag histogram, and most-linked pages.
Add the API-usage middleware here — a `MatchedPath`-keyed counter flushed to
SQLite periodically and on shutdown.

*Done when:* `/api/stats` describes the wiki accurately, and a `[[link]]` to a
page that does not exist shows up as a wanted page.

### M5 — Live external edits
The `notify` watcher feeding a debounced reindex queue.

*Done when:* editing a `.md` file in another editor updates search results
without restarting, and API writes do not fight the watcher.

At this point the backend is complete and dogfoodable via API alone.

### M6 — Frontend scaffold
Vite + SolidJS + TypeScript + Tailwind + DaisyUI in `frontend/`. Vite dev
server proxies `/api` to the backend; the backend serves `frontend/dist` via
`tower-http`'s `ServeDir` with an SPA fallback. Generate a typed API client
from the OpenAPI document rather than hand-writing fetch calls — the spec is
already the contract, so this is close to free and keeps the two halves honest.

*Done when:* `pnpm dev` browses and reads pages against a running backend, and
`pnpm build` + `cargo run` serves the same UI from the Rust binary alone.

Remember `--vcs none` / delete any `.git` a scaffolding tool leaves behind.
(`pnpm create vite` did not create one, but it does refuse a non-empty target
directory — scaffold elsewhere and move the files in.)

### M7 — Authoring and dashboard
Page editor (a `textarea` plus a debounced preview — no editor library in the
MVP), search UI, tag browsing, backlinks panel on the page view, and the stats
dashboard. What it became is described in [The dashboard](dashboard.md).

*Done when:* a page can be written start to finish in the browser. It can:
verified by writing one, following its wanted link, writing that, renaming it,
and deleting a page, all in the browser against the real build.

Two things this milestone assumed and got wrong, both now in
[API design](api-design.md):

- **The preview cannot use `?render=true`.** That renders what is *saved*, and a
  preview exists to show what is not. `POST /api/render` was added for it.
- **Rendered wikilinks were relative**, so every link in a rendered body pointed
  somewhere that did not exist. Rendering now rewrites them to `/pages/<slug>`.

And one bug it surfaced in the existing API: reading a page and writing it back
silently froze a derived title. `PageView` now carries `title_derived`.

### M8 — Polish
README with setup instructions, a seeded example wiki, `cargo clippy` clean,
consistent examples throughout the OpenAPI doc, and graceful shutdown that
flushes the usage counters.

*Done.* Notes on the parts that were not just typing:

**`example-wiki/`** shipped as six pages demonstrating nested slugs, both link
spellings, a wanted page, two orphans, and a wikilink inside a code fence that
is not a link. Its `index.md` states what the dashboard will report about it,
which makes the whole thing a check on the software rather than only a demo —
and it was wrong on the first pass, because the entry page is itself an orphan.
It has since grown to nine, for the slug path filters.

**The OpenAPI examples** started at 49 fields with no description, 42 with no
example, and 11 parameters with neither. The audit turned up something worse
than an absence: the `Slug` schema was publishing its Rust doc comment, which
talks about `Slug::parse` and carries a rustdoc link — a dead reference to a
reader who has no crate to resolve it against. Schemas whose doc comments are
aimed at maintainers now set `description` explicitly, and a test fails if a
rustdoc link reaches the document.

Examples now all describe one page, which exists in `example-wiki/`, so the
document can be followed against a running server. A unit test asserts the
documented HTML is what the renderer actually produces — it caught the example
being wrong immediately, because comrak marks wikilink anchors with
`data-wikilink="true"` and the hand-written example did not.

**Shutdown flushing** was already written but never tested. Usage counts are the
only thing in the index not derivable from the markdown, so a rebuild cannot
restore them; there is now a test that drives requests, flushes, reopens the
database, and asserts the counts survived.

## Dependencies

All verified to resolve together on the current index.

```toml
axum = "0.8"
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.7", features = ["fs", "trace", "cors"] }
utoipa = { version = "5", features = ["axum_extras", "chrono"] }
utoipa-axum = "0.2"
utoipa-swagger-ui = { version = "9", features = ["axum"] }
rusqlite = { version = "0.40", features = ["bundled"] }
comrak = "0.54"
notify = "8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml_ng = "0.10"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
walkdir = "2"

[dev-dependencies]
tempfile = "3"
tower = { version = "0.5", features = ["util"] }
```

Two notes. `notify` 9 is still a release candidate, so the MVP pins 8. And
`utoipa-swagger-ui` pulls in `zip`/`zopfli` to unpack the Swagger UI dist at
build time — a one-time hit on first compile, not a runtime dependency.

## Testing

- **Unit:** slug validation (the traversal corpus), frontmatter round-trips,
  link extraction, wikilink resolution including the ambiguous-basename case.
- **Integration:** build the router over a `tempfile` wiki directory and drive
  it with `tower::ServiceExt::oneshot`. No ports, no fixtures to clean up.
- **The consistency test that matters:** write a file externally, trigger a
  reindex, and assert the index matches a from-scratch rebuild. That is the
  invariant the whole storage design rests on, and it is the one that will
  break quietly.
- **Frontend:** Vitest over jsdom. Added after the MVP, which was a gap worth
  admitting: through M8 the dashboard's only safety net was the typechecker and
  a person clicking around. What it covers, and the two configuration traps it
  needed, are in [The dashboard](dashboard.md).

## Risks

**Index drift** is the structural risk of choosing files as the source of
truth. Mitigated by making a full rebuild cheap and always available, and by
testing rebuild-equivalence rather than trusting the incremental path.

**Watcher noise on Windows** — editors write via temp-file-and-rename, so a
single save can produce several events. Debounce, and treat reindex as
idempotent.

**Scope creep through the editor.** A markdown editor is a bottomless project.
The MVP is a textarea with a preview pane; anything more waits until the wiki
is in daily use and the real annoyances are known rather than guessed at.

## Housekeeping

The stale `app/` references `CLAUDE.md` carried after the `backend/` rename in
b2d2257 are corrected. The root `.gitignore` never needed changing — bare
`target/` matches at any depth.

Settled: the commit-subject scope for backend work was `app:`, which stopped
matching any directory at the `backend/` rename in b2d2257. It is now
`backend:`, and the history was rewritten to match rather than left as a
seam — the repository has no remote and nothing had been published, so there
was no hash out in the world for the rewrite to invalidate.

Only the subject lines changed; every tree is byte-identical to what it was.
The three commits preceding the first `app:` one were untouched and kept their
hashes, which is why the reference to b2d2257 above still resolves.
