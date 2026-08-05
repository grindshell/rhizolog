# Rhizowiki

Rhizowiki ("rhizome" + "wiki") is a wiki backend and server in the vein of
MediaWiki and TiddlyWiki, built on the idea that knowledge branches off
chaotically — and that's exactly what it should track.

What sets it apart:

- **A developer tool for managing knowledge bases**, not a public wiki engine.
- **API-first**: a rich HTTP API (with OpenAPI definitions) designed to be
  friendly to AI agents as well as humans.
- **Single-user**: no accounts, roles, or multi-tenancy. The UI is an admin
  dashboard for searching and authoring pages and checking meta-stats
  (links between pages, tags, API usage, etc.).

## Repository layout

This is a **git monorepo with a single `.git` at the root**.

| Path | Purpose |
|------|---------|
| `backend/` | The Rust backend (cargo project, crate name `rhizowiki`) |
| `frontend/` | The TypeScript frontend, served by the backend |
| `example-wiki/` | A small committed wiki to run against; its `index.md` states what the dashboard should report about it |
| `knowledge-base/` | Markdown knowledge base tracking Rhizowiki's design and implementation |
| `README.md` | Setup and usage, for people who are not this file |
| `CLAUDE.md` | This file |

`backend/wiki/` is the default `RHIZOWIKI_ROOT` and is gitignored, as is
`.rhizowiki/` anywhere. Do not develop against `example-wiki/` — it is a
fixture, and changing it changes what the docs claim.

## Tech stack

**Backend** (`backend/`) — Rust, edition 2024:

- `tokio` — async executor
- `axum` — HTTP server
- `utoipa-axum` — OpenAPI definitions kept in sync with the axum routes

**Frontend** (`frontend/`, not yet scaffolded) — TypeScript:

- `pnpm` — package manager
- SolidJS — frontend framework
- TailwindCSS — CSS framework
- DaisyUI — component library

The backend serves the built frontend assets; there is no separate frontend
deployment.

## Managing the monorepo

- **One repo, one `.git`.** Scaffolding tools like to create nested git repos
  (`cargo new` does; `pnpm create` templates sometimes do). If a generator
  creates a `.git` inside `backend/` or elsewhere, delete it — otherwise the root
  repo treats that directory as an opaque embedded repo and stops tracking
  its files. Pass `--vcs none` to `cargo new`/`cargo init` to avoid this.
- **Main branch is `main`.** Commit directly to it or branch off it for
  larger work.
- **Scope commit subjects** by the area touched: `backend:`, `ui:` for
  frontend, `kb:` for knowledge base, `repo:` for root-level/tooling changes.
  A commit may touch several areas; pick the dominant one.
- **The knowledge base is first-class.** When a design decision is made or an
  implementation approach changes, record it as a page in `knowledge-base/`
  in the same commit as the code where practical. `knowledge-base/index.md`
  is the entry point — keep it linking to every page.
- Build artifacts never get committed: `backend/target/`, `node_modules/`, and
  frontend `dist/` output are gitignored at the root.

## Commands

Backend (run from `backend/`):

```
cargo run        # start the server
cargo test       # run tests
cargo fmt        # format
cargo clippy     # lint
```

Frontend (run from `frontend/`):

```
pnpm install     # install dependencies
pnpm dev         # dev server with HMR, proxying /api to the backend
pnpm build       # production build (output served by the backend)
pnpm test        # vitest run (jsdom); `pnpm test:watch` to iterate
pnpm typecheck   # tsc --noEmit
pnpm gen:api     # regenerate API types from openapi.json
```

`frontend/openapi.json` is dumped from a running backend and is the input to
`pnpm gen:api`. Refresh it when the API changes, and download it as **bytes** —
see the note under Environment notes for why `curl` will not do:

```
$data = (New-Object System.Net.WebClient).DownloadData("http://127.0.0.1:3000/api-docs/openapi.json")
[System.IO.File]::WriteAllBytes("$PWD\frontend\openapi.json", $data)
```

## Environment notes

- Development happens on Windows; the shell is PowerShell. Avoid bash-isms
  in any scripts or documented commands (`&&` chaining doesn't work in
  Windows PowerShell 5.1 — use `;`).
- **Never edit a source file by round-tripping it through the shell.**
  `(Get-Content f -Raw) -replace ... | Set-Content f -Encoding utf8` looks
  harmless and corrupts the file twice over: 5.1's `Get-Content` decodes as the
  system ANSI codepage, so every non-ASCII character comes back as mojibake
  (`—` becomes `â€”`), and `-Encoding utf8` writes a BOM. This codebase uses
  em-dashes in prose throughout, and a BOM has already caused one real bug —
  it hid a page's frontmatter, since the text no longer started with `---`.
  Use the editing tools. If a bulk edit is genuinely necessary, go through
  `[System.IO.File]::ReadAllText` / `WriteAllText` with
  `UTF8Encoding($false)`, and check the result for a BOM and for `â€`.
- **Never save a downloaded file through a PowerShell string.** `curl` in 5.1
  is an alias for `Invoke-WebRequest`, which decodes a response body as
  Latin-1 when its `Content-Type` carries no charset — and the backend's
  `application/json` does not. So `curl .../openapi.json > openapi.json`,
  the obvious way to refresh the spec, turns every em-dash in it from
  `E2 80 94` into `C3 A2 C2 80 C2 94`. The result is still valid JSON and
  still one line, so the diff looks like a normal regeneration and nothing
  catches it. Download bytes and write them verbatim
  (`WebClient.DownloadData` + `[System.IO.File]::WriteAllBytes`), then check
  the first non-ASCII bytes are `E2 80 94`. The same applies to `>` and
  `Out-File` generally, which re-encode and add a BOM — `git show HEAD:f > tmp`
  does not give you the committed bytes; `git checkout HEAD -- f` does.
- Stop the server before `cargo build`: a running `rhizowiki.exe` is locked,
  and the build fails with "Access is denied" rather than anything informative.
