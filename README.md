# Rhizowiki

A wiki over a directory of markdown files, with an HTTP API that is meant to be
used — by you, and by whatever agents you point at it.

"Rhizome" plus "wiki". A rhizome is a root system with no trunk: any point
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

## Quick start

You need [Rust](https://rustup.rs/) (edition 2024) and, for the dashboard,
[pnpm](https://pnpm.io/).

Build the dashboard once, then run the server against the example wiki:

```powershell
cd frontend; pnpm install; pnpm build
cd ../backend; $env:RHIZOWIKI_ROOT = "../example-wiki"; cargo run
```

On macOS or Linux the last line is `RHIZOWIKI_ROOT=../example-wiki cargo run`.

Then open:

| | |
|---|---|
| http://127.0.0.1:3000 | the dashboard |
| http://127.0.0.1:3000/swagger-ui | the API, browsable |
| http://127.0.0.1:3000/api-docs/openapi.json | the OpenAPI document |

The first compile takes a while: SQLite is built from source, and Swagger UI is
unpacked at build time.

`example-wiki/` is six pages arranged to show the features off — nested slugs,
wikilinks, a page that is linked but not written, and two orphans. Read
[its index](example-wiki/index.md) first; it explains what the dashboard will
say about it and why.

To use your own notes instead, point `RHIZOWIKI_ROOT` at any directory of
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

## Configuration

All optional, all environment variables.

| Variable | Default | What it is |
|---|---|---|
| `RHIZOWIKI_ROOT` | `./wiki` | The wiki directory. Created if missing. |
| `RHIZOWIKI_DB` | `<root>/.rhizowiki/index.db` | The derived index. Safe to delete. |
| `RHIZOWIKI_ADDR` | `127.0.0.1:3000` | Where to listen. |
| `RHIZOWIKI_ASSETS` | `../frontend/dist` | The built dashboard. Missing is fine. |
| `RHIZOWIKI_LOG` | `rhizowiki=info,tower_http=info` | `tracing` filter. |

Defaults are relative to the working directory, which is assumed to be
`backend/`.

Think before changing `RHIZOWIKI_ADDR`. There is no authentication, and the API
writes files.

## Development

This is one git repository. Scaffolding tools like to create nested ones — if a
generator leaves a `.git` inside `backend/` or `frontend/`, delete it, or the
root repository will treat that directory as opaque and stop tracking what is
inside it.

| Path | |
|---|---|
| `backend/` | The Rust server (crate `rhizowiki`) |
| `frontend/` | The dashboard: Vite, SolidJS, Tailwind, daisyUI |
| `example-wiki/` | A small wiki to run against |
| `knowledge-base/` | Why the thing is built the way it is |

Backend, from `backend/`:

```
cargo run        # start the server
cargo test       # 193 tests
cargo fmt
cargo clippy
```

Frontend, from `frontend/`:

```
pnpm dev         # dev server with HMR, proxying /api to the backend
pnpm build       # production build, which the backend serves
pnpm test        # 48 tests
pnpm typecheck
```

`pnpm dev` expects a backend already running on port 3000 and proxies `/api`,
`/api-docs`, and `/swagger-ui` to it.

The frontend's API types are generated from the OpenAPI document rather than
written by hand, so a backend change that breaks a caller becomes a type error
instead of a runtime surprise. After changing the API, with the server running:

```
curl http://127.0.0.1:3000/api-docs/openapi.json -o frontend/openapi.json
pnpm gen:api
```

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
pickup of outside edits, and a dashboard you can write in.

Not implemented, on purpose: page history and diffs, authentication, anything
multi-user, link rewriting on move, file attachments, and transclusion.
