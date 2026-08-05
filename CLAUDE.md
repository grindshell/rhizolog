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
| `frontend/` | *Planned* — the TypeScript frontend, served by the backend |
| `knowledge-base/` | Markdown knowledge base tracking Rhizowiki's design and implementation |
| `CLAUDE.md` | This file |

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
- **Scope commit subjects** by the area touched: `app:` for backend, `ui:`
  for frontend, `kb:` for knowledge base, `repo:` for root-level/tooling
  changes. A commit may touch several areas; pick the dominant one.
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
pnpm typecheck   # tsc --noEmit
pnpm gen:api     # regenerate API types from openapi.json
```

`frontend/openapi.json` is dumped from a running backend
(`curl http://127.0.0.1:3000/api-docs/openapi.json`) and is the input to
`pnpm gen:api`. Refresh it when the API changes.

## Environment notes

- Development happens on Windows; the shell is PowerShell. Avoid bash-isms
  in any scripts or documented commands (`&&` chaining doesn't work in
  Windows PowerShell 5.1 — use `;`).
