# Rhizolog

A wiki over a directory of markdown files, with an HTTP API that is meant to be
used — by you, and by whatever agents you point at it.

"Rhizome" plus "log". A rhizome is a root system with no trunk: any point
connects to any other and there is no privileged centre. That is the bet this
project makes about notes — that knowledge branches off chaotically, and that
filing it as though it were a tree loses the connections worth keeping.

What makes it different from the wikis you already know:

- **Markdown files on disk are the source of truth.** Not a database with an
  export button. Edit them in your editor, move them with `mv`, keep them in
  git. The search index is derived and can be deleted at any time; it rebuilds
  on startup, and a file watcher picks up outside edits while the server runs.
- **API first.** A full HTTP API with an OpenAPI document that is generated from
  the routes, so it cannot drift from them. Errors are uniform and carry stable
  machine-readable codes. The dashboard is the API's first client, not its
  privileged one.
- **Single user.** No accounts, no roles, no tenancy. It binds to loopback and
  it has no authentication, deliberately — this is a developer tool for
  managing a knowledge base, not a public wiki engine.
- **It tracks time, too.** Timers you can start and stop, entries you can type
  in after the fact, and a note on any of them. Attach an entry to the pages it
  was spent on and the dashboard will tell you where the hours went. Entries
  are files as well, so `git log` gives you a history of your time nobody had
  to build.

## Quick start

You need [Rust](https://rustup.rs/) (edition 2024) and, for the dashboard,
[pnpm](https://pnpm.io/).

Build the dashboard once, then run the server against the example wiki:

```powershell
cd frontend; pnpm install; pnpm build
cd ../backend; $env:RHIZOLOG_ROOT = "../example-wiki"; cargo run
```

On macOS or Linux the last line is `RHIZOLOG_ROOT=../example-wiki cargo run`.

Then open:

| | |
|---|---|
| http://127.0.0.1:3000 | the dashboard |
| http://127.0.0.1:3000/swagger-ui | the API, browsable |
| http://127.0.0.1:3000/api-docs/openapi.json | the OpenAPI document |

The first compile takes a while: SQLite is built from source, and Swagger UI is
unpacked at build time.

`example-wiki/` is nine pages arranged to show the features off — nested slugs,
wikilinks, a page that is linked but not written, two orphans, and the same
directory name in two places, which is what makes the two path filters differ.
It also carries a week of tracked time: eighteen entries, two overlapping
timers, a session that runs past midnight, and hours logged against the page
nobody has written. Read [its index](example-wiki/index.md) first; it explains
what the dashboard will say about it and why — including why Today is empty.

To use your own notes instead, point `RHIZOLOG_ROOT` at any directory of
markdown files. Nothing needs importing.

The dashboard is optional. Skip `pnpm build` and the API works exactly the same;
the server just says there is no frontend to serve.

## Pages

A page is a markdown file with optional YAML frontmatter:

```markdown
---
title: Async in Rust
tags:
  - rust
  - async
---

# Async in Rust

Futures are lazy. See [[notes/rust/pinning]].
```

Everything in the frontmatter is optional, including the whole block. Without a
`title` the page takes one from its first heading, and failing that from its
slug — and it keeps following that heading as you edit it.

A page's **slug** is its path under the wiki root without the `.md`:
`notes/rust/async.md` is `notes/rust/async`. Slugs are validated against the
stricter of the Windows and POSIX rules on every platform, so a wiki written on
one stays valid on the other.

Links come in two spellings and both are tracked:

- `[[notes/rust/pinning]]` — a wikilink, always absolute from the wiki root.
- `[pinning](pinning.md)` — an ordinary markdown link, resolved relative to the
  page it appears in. The `.md` is optional.

A link to a page that does not exist is not an error. It is a **wanted page**,
it shows up on the dashboard, and it starts working the moment somebody writes
it — no reindex. Links inside code fences are not links, because they are pulled
out of the parsed document rather than scanned for.

## The graph

`/graph` draws the pages and the links between them, and `GET /api/graph`
returns the same thing as nodes and edges. Wanted pages are drawn too, as dashed
rings — they are branches the wiki has reached for, and leaving them out would
make it look tidier than it is.

Narrow it with `?root=` for one page's neighbourhood (a walk of `?depth=` hops,
following links in both directions), or with the same `?prefix=` and `?tag=`
filters the page listing takes. Every page links to its own neighbourhood from
its Graph button.

The layout is deterministic: node positions come from a hash of the slug rather
than a random seed, so the same wiki draws the same picture every time and a
shape that changed means the wiki changed.

## Time

A time entry has a name, a start, usually an end, and optionally a markdown
note and a list of pages it was spent on. Entries are grouped by name — there
is nothing to create or delete, a group exists because entries carry its name.

```markdown
---
name: Deep work
start: 2026-08-06T14:25:30Z
end: 2026-08-06T15:40:00Z
pages:
  - notes/rust/async
---

Chased down a lifetime error in the poll loop.
```

They live in `<root>/.rhizolog/times/<YYYY-MM>/`, beside the derived index but
**not** derived: that directory is the only copy, so ignore
`.rhizolog/index.db` in git rather than the whole directory. They are not
pages — they will not appear in a listing or in search.

Start one from the top bar, from any page, or with
`POST /api/times {"name": "Deep work"}`. Several can run at once and they are
allowed to overlap, because attention is not exclusive and a tracker that
insisted otherwise would be asking you to lie to it.

The dashboard's time section splits day, week, month and year, ranks the
activities and the pages the hours went to, and draws a heat map of every hour
of the week. A session that ran past midnight is split across both days and
lights every hour it touched, rather than being filed under the hour it started
in.

Names and notes are searchable with `GET /api/times?q=`, or the box on the Time
screen. It is a filter rather than a mode: it narrows the log alongside the
group, page and date filters instead of replacing them, and it leaves the log in
order — so "what did I write about the poll loop last week" is one request.
Entries are deliberately absent from `/api/search`, which is about pages.

Time attached to a page shows on that page as one line with a total on it, not
as backlinks. That is deliberate: a page you actually work on collects an entry
every time you start a timer, and folding hundreds of them into the link graph
would bury the links.

## Configuration

All optional, all environment variables.

| Variable | Default | What it is |
|---|---|---|
| `RHIZOLOG_ROOT` | `./wiki` | The wiki directory. Created if missing. |
| `RHIZOLOG_DB` | `<root>/.rhizolog/index.db` | The derived index. Safe to delete. |
| — | `<root>/.rhizolog/times/` | The time log. **Not** derived; back it up. |
| — | `<root>/.rhizolog/server.json` | Where the running server is. Gone when it stops. |
| `RHIZOLOG_ADDR` | `127.0.0.1:3000`, or any free port | Where to listen. |
| `RHIZOLOG_ASSETS` | `../frontend/dist` | The built dashboard. Missing is fine. |
| `RHIZOLOG_LOG` | `rhizolog=info,tower_http=info` | `tracing` filter. |

Defaults are relative to the working directory, which is assumed to be
`backend/`.

Think before changing `RHIZOLOG_ADDR`. There is no authentication, and the API
writes files.

### Finding a running server

By default the server takes port 3000 if it can and **any free port if it
cannot**, so a second copy — or a machine where something else got there first
— still starts. Setting `RHIZOLOG_ADDR` turns that off: an address you asked
for by name is used or the server refuses to start, because you have probably
written that port down somewhere else too.

Which means the port is not always knowable in advance, so a running server
writes it down:

```json
{
  "url": "http://127.0.0.1:3000",
  "wiki_root": "C:\\Users\\tim\\wiki",
  "pid": 24601,
  "version": "0.1.0",
  "started": "2026-08-06T14:25:30Z"
}
```

It appears at `<root>/.rhizolog/server.json` only once the server is **ready** —
listening, with its index reconciled — so finding one means you can use it
immediately. A clean shutdown removes it.

For a script or an agent, the order to try is `RHIZOLOG_ADDR`, then
`server.json` beside the wiki, then `http://127.0.0.1:3000`. Treat the file as a
hint rather than proof: a server killed hard leaves it behind, and process ids
get reused, so confirm with `GET /api/health` and check the `wiki_root` it
reports is the wiki you meant. That is one request and it cannot be fooled by a
stale file.

## Development

This is one git repository. Scaffolding tools like to create nested ones — if a
generator leaves a `.git` inside `backend/` or `frontend/`, delete it, or the
root repository will treat that directory as opaque and stop tracking what is
inside it.

| Path | |
|---|---|
| `backend/` | The Rust server (crate `rhizolog`) |
| `frontend/` | The dashboard: Vite, SolidJS, Tailwind, daisyUI |
| `example-wiki/` | A small wiki, and a week of time, to run against |
| `knowledge-base/` | Why the thing is built the way it is |

Backend, from `backend/`:

```
cargo run        # start the server
cargo test       # 336 tests
cargo fmt
cargo clippy
```

Frontend, from `frontend/`:

```
pnpm dev         # dev server with HMR, proxying /api to the backend
pnpm build       # production build, which the backend serves
pnpm test        # 99 tests
pnpm typecheck
```

`pnpm dev` expects a backend already running on port 3000 and proxies `/api`,
`/api-docs`, and `/swagger-ui` to it.

The frontend's API types are generated from the OpenAPI document rather than
written by hand, so a backend change that breaks a caller becomes a type error
instead of a runtime surprise. After changing the API:

```
cd backend; cargo run --example dump-openapi
cd ../frontend; pnpm gen:api
```

No server needs to be running: the example writes the spec straight from the
compiled routes.

That exists because downloading it is a trap on Windows, and the obvious way is
the one that does not work. `curl` in PowerShell 5.1 is an alias for
`Invoke-WebRequest`, which decodes a body as Latin-1 when its `Content-Type`
carries no charset — and `application/json` from here carries none. Every
em-dash in the spec turns from `E2 80 94` into `C3 A2 C2 80 C2 94`. The file
stays valid JSON, stays one line, and the diff still reads like an ordinary
regeneration, so nothing catches it. `>` and `Out-File` are no better; they
re-encode too, and add a BOM.

If you do fetch it over HTTP, download bytes and write them verbatim:

```powershell
$data = (New-Object System.Net.WebClient).DownloadData("http://127.0.0.1:3000/api-docs/openapi.json")
[System.IO.File]::WriteAllBytes("$PWD\frontend\openapi.json", $data)
```

Worth checking after a refresh either way: the file should have no BOM, and its
first non-ASCII bytes should be `E2 80 94`.

Windows PowerShell 5.1 has no `&&`; use `;` to chain. And do not round-trip a
source file through `Get-Content` and `Set-Content` — 5.1 reads as ANSI and
writes UTF-8 with a BOM, which mangles every non-ASCII character in the file and
adds a byte-order mark that has, in this project, already hidden a page's
frontmatter once.

## Why it is built this way

[`knowledge-base/`](knowledge-base/index.md) is the long answer, kept as a wiki
because that is the obvious thing to do here. Start with
[the architecture](knowledge-base/architecture.md) for the storage model, or
[the API design](knowledge-base/api-design.md) for what "friendly to agents"
was taken to mean concretely.

## Status

The MVP is complete: pages, search, tags, the link graph, meta-stats, live
pickup of outside edits, and a dashboard you can write in. Time tracking is in
too: timers, manual entries, notes, groups, search over the log, and the
statistics section.

Not implemented, on purpose: page history and diffs, authentication, anything
multi-user, link rewriting on move, file attachments, and transclusion.
