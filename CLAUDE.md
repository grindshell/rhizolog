# Rhizolog

Rhizolog ("rhizome" + "log") is a wiki backend and server in the vein of
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

This is a **git monorepo with a single `.git` at the root**, and a **cargo
workspace** whose members are `backend/` and `desktop/`.

| Path | Purpose |
|------|---------|
| `Cargo.toml` | The workspace. Build artifacts go to `target/` at the root, not `backend/target/` |
| `backend/` | The Rust library and the headless `rhizolog` server (crate `rhizolog`) |
| `desktop/` | The Tauri app (crate and binary `rhizolog-desktop`; the *product* is Rhizolog) |
| `frontend/` | The TypeScript frontend, served by the backend |
| `example-wiki/` | A small committed wiki *and time log* to run against; its `index.md` states what the dashboard should report about both |
| `knowledge-base/` | Markdown knowledge base tracking Rhizolog's design and implementation |
| `README.md` | Setup and usage, for people who are not this file |
| `CLAUDE.md` | This file |

**The desktop crate may only use `Config` and `Server`** from the library. It
can see `Store`, `Index` and `TimeStore` too, and using them would be the end of
the property the whole design protects: the app must not be able to do anything
a browser pointed at a remote Rhizolog cannot do over HTTP. When the shell needs
wiki data it makes an HTTP request to itself. See
`knowledge-base/desktop-app.md`.

`backend/wiki/` is the default `RHIZOLOG_ROOT` and is gitignored, as is
`.rhizolog/index.db` anywhere. Do not develop against `example-wiki/` — it is a
fixture, and changing it changes what the docs claim.

That now includes `example-wiki/.rhizolog/times/`: 18 committed entries whose
totals `example-wiki/index.md` states exactly. Pointing `RHIZOLOG_ROOT` at the
example wiki to *look* at it is fine and is what the README tells people to do;
starting a timer while it is pointed there writes a new file into the fixture
and breaks those numbers. Check `git status example-wiki` afterwards.

**`.rhizolog/` is not all disposable.** `index.db` is derived and rebuilds on
startup; `.rhizolog/times/` beside it is the time log, which is authored data
with no other copy. That is why the gitignore names the database rather than
the directory. See `knowledge-base/time-tracking.md`.

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
- **Main branch is `master`.** Commit directly to it or branch off it for
  larger work.
- **Scope commit subjects** by the area touched: `backend:`, `ui:` for
  frontend, `kb:` for knowledge base, `repo:` for root-level/tooling changes.
  A commit may touch several areas; pick the dominant one.
- **The knowledge base is first-class.** When a design decision is made or an
  implementation approach changes, record it as a page in `knowledge-base/`
  in the same commit as the code where practical. `knowledge-base/index.md`
  is the entry point — keep it linking to every page.
- Build artifacts never get committed: `target/`, `node_modules/`, and
  frontend `dist/` output are gitignored at the root.
- **One `Cargo.lock`, at the root.** It is the workspace's. A `Cargo.lock`
  inside `backend/` or `desktop/` is a leftover and should be deleted.

## Commands

Backend (run from `backend/`):

```
cargo run        # start the headless server
cargo test       # run tests
cargo fmt        # format
cargo clippy     # lint
```

`cargo test --features embed-assets` runs six more, covering the dashboard
compiled into the binary. That feature reads `frontend\dist` at compile time,
so `pnpm build` has to have run first.

**Cargo unifies features across a workspace build**, so `cargo test --workspace`
and `cargo build --workspace` turn `embed-assets` on whether you asked or not —
`desktop/` depends on `rhizolog` with it enabled, and there is only one build of
the library. Which means those two also need `pnpm build` behind them.
`cargo test -p rhizolog` is the one that tests the library as the headless
server actually ships it.

Desktop app (run from `desktop\`, or anywhere with `-p rhizolog-desktop`):

```
cargo run                 # a window onto a server it starts itself
cargo build --release     # the portable exe, target\release\rhizolog-desktop.exe
```

It depends on `rhizolog` with `embed-assets` on, so **`pnpm build` is a
prerequisite of building it at all**. `desktop\icons\` are placeholders,
generated so the crate would build; replace them with real artwork.

The app remembers which wiki it opened in `rhizolog.settings.json`, written
beside the executable — so in a checkout that is `target\debug\`. Delete it to
get the first-run folder picker back. `RHIZOLOG_ROOT` overrides it and is not
remembered, which is how to point the app at a scratch wiki.

Frontend (run from `frontend/`):

```
pnpm install     # install dependencies
pnpm dev         # dev server with HMR, proxying /api to the backend
pnpm build       # production build (output served by the backend)
pnpm test        # vitest run (jsdom); `pnpm test:watch` to iterate
pnpm typecheck   # tsc --noEmit
pnpm gen:api     # regenerate API types from openapi.json
```

`frontend/openapi.json` is the input to `pnpm gen:api`. Refresh it from
`backend/` with:

```
cargo run --example dump-openapi
```

That writes the file straight from the compiled routes — no server, no HTTP,
and none of the encoding hazards below. Do **not** refresh it with `curl`; see
the note under Environment notes for what that does. If you fetch it over HTTP
anyway, download it as **bytes**:

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
  catches it. Use `cargo run --example dump-openapi`, which sidesteps HTTP
  entirely; failing that, download bytes and write them verbatim
  (`WebClient.DownloadData` + `[System.IO.File]::WriteAllBytes`), then check
  the first non-ASCII bytes are `E2 80 94`. The same applies to `>` and
  `Out-File` generally, which re-encode and add a BOM — `git show HEAD:f > tmp`
  does not give you the committed bytes; `git checkout HEAD -- f` does.
- **Never pass a quoted string straight to a native command.** 5.1 re-parses an
  argument on its way to a native executable and strips the double quotes it
  takes for delimiters, so `git commit -m @'...'@` commits the message with
  every `"` silently removed — a single-quoted here-string stops `$` expansion
  but not this, because the mangling happens at the native-command boundary,
  after the here-string has already been resolved. The commit succeeds and
  nothing warns you; it shows up only if you read the message back with
  `git log -1 --format=%B`. Write the message to a file and use
  `git commit -F <file>`, which carries quotes and em-dashes through intact.
- Stop the server before `cargo build`: a running `rhizolog.exe` is locked,
  and the build fails with "Access is denied" rather than anything informative.
- **Two cargo binaries whose names differ only in case are one file here.**
  Windows filenames are case-insensitive, so a `[[bin]]` called `Rhizolog`
  writes `target\debug\Rhizolog.exe` over the server's `rhizolog.exe` — no
  warning, no error, just whichever cargo happened to link last. The symptom is
  bizarre: `cargo run -p rhizolog` opens a window, or the desktop app starts a
  console server against `.\wiki`. The desktop binary is therefore
  `rhizolog-desktop`, and the pretty name lives in `productName` and
  `mainBinaryName` in `tauri.conf.json` where it cannot collide with anything.
  Check `Get-ChildItem target\debug -Filter *.exe` if two binaries ever seem to
  be the same program.
- **Moving or renaming the repository breaks Swagger UI until you
  `cargo clean -p utoipa-swagger-ui`.** That crate's build script writes the
  *absolute* path of its downloaded asset directory into a generated
  `embed.rs`, and cargo caches the result — so after a move it points at a
  directory that no longer exists. Nothing fails loudly: the crate compiles,
  the route is registered, and `/swagger-ui` still redirects to
  `/swagger-ui/`, which then 404s with every asset behind it gone. Renaming
  this project from `rhizowiki` to `rhizolog` did exactly that and it went
  unnoticed for several commits. `swagger_ui_serves_its_own_assets` in
  `backend/tests/frontend.rs` now fails when it happens.
