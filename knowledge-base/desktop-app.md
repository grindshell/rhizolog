# The desktop app

Rhizolog ships as a portable [Tauri](https://v2.tauri.app/) app: an executable
that opens a window onto a wiki, built on the same library as the headless
server and serving the same HTTP API from inside the window. Nothing about the
API changes when it is wrapped in one.

This page records what that costs and what it decides. See
[Architecture](architecture.md) for the storage model underneath and
[Tech stack](tech-stack.md) for the pieces already in place.

## The window is over the server, not instead of it

The Tauri binary **is** the backend. It starts axum on loopback in its own
process and opens a webview at `http://127.0.0.1:<port>`. The app's own UI
talks to the HTTP API over the same origin an agent would use.

Two alternatives were considered and rejected.

**A bundled sidecar** — Tauri spawning `rhizolog.exe` as an external binary —
buys nothing here. Both halves are Rust, so there is no language boundary to
justify a process boundary, and it costs an orphaned server whenever the shell
dies badly, plus a second thing to version.

**Native `#[tauri::command]` IPC** is the idiomatic Tauri answer and it throws
away the premise. The moment page reads go through IPC in the desktop app, the
local build and a remote instance are different applications that happen to
share a repository. The whole point of
[the API being primary](product-vision.md) is that it is the *only* interface;
a desktop shell is not a licence to grow a second one.

So the rule is: **the desktop app may not do anything through Tauri that a
browser pointed at a remote Rhizolog could not do through HTTP.** Native
conveniences live in the shell around the webview, never in the page.

## Two binaries, one library

`backend/` keeps producing `rhizolog`, the headless server, exactly as it is
today and with no Tauri anywhere in its dependency graph. A new `desktop/`
crate produces the app, depending on `rhizolog` by path and adding the window.

The tempting version of this is two `[[bin]]` targets in one crate, and it does
not work. Cargo resolves dependencies per crate, not per binary, so `tauri`
would be in the graph either way — and building the headless server on Linux
would then need webkit2gtk installed **to compile a binary that never links
it**. Remote instances are the reason the API stays uniform, and most of them
will be boxes with no display. A crate boundary is the only boundary cargo
actually enforces; a cargo feature would express the same intent and still leave
the GUI stack one careless `default = [...]` away from the server build.

Splitting also settles a Windows question that one binary could not answer. A
PE picks its subsystem at link time, so `rhizolog` stays a console application
— synchronous output, working Ctrl-C, the honest exit code at
[`main.rs:13`](../backend/src/main.rs) — while the app is built with
`windows_subsystem = "windows"` and never flashes a console. As one binary
those are the same field, and one of the two has to lose: either a console
window blinks behind every launch from Explorer, or `AttachConsole` gives the
server mode output that arrives after the shell prompt has already come back.

`main.rs` also stops needing to know which mode it is in. No argument dispatch,
no `serve` subcommand, no mode flag — `rhizolog` serves, the app opens a
window, and what they share is a library rather than a branch.

### The seam is `Config` and `Server`, and nothing else

`desktop/` can see everything `rhizolog` exports, which includes `Store`,
`Index` and `TimeStore`. It must use none of them. The rule above — no
capability in the app that a browser pointed at a remote instance lacks — is
only enforceable if the shell's entire vocabulary is `Config`, `Server::start`,
`Server::address` and `Server::shutdown`.

A Tauri command that reads a page straight off disk would be a two-line
convenience and the end of the property this design exists to protect. When the
shell needs wiki data, it makes an HTTP request to itself, like every other
client.

## The boot sequence has to become a library

Everything in [`main.rs:25-111`](../backend/src/main.rs) — open the store, the
time log and the index, reconcile, bind, log the address, spawn the watcher,
resolve the assets, spawn the usage flusher, serve, flush again on the way out —
is the sequence the desktop shell needs too, and it is currently welded to
`Config::from_env`, stdout tracing, and Ctrl-C.

It moves to `backend/src/server.rs`, where it stops being a private convenience
and becomes the library's public lifecycle API — with the crate split, it is the
only door `desktop/` has:

```rust
pub struct Server { /* address, state, join handle, shutdown token */ }

pub async fn start(config: &Config) -> anyhow::Result<Server>;
impl Server {
    pub fn address(&self) -> SocketAddr;
    pub async fn shutdown(self) -> anyhow::Result<()>;
}
```

`start` returns once the server is *ready* — bound, reconciled, watching — so
the caller learns the real port before it has anywhere to put it. `main.rs`
shrinks to config, `start`, wait for Ctrl-C, `shutdown`. The Tauri `setup` hook
calls the same two functions.

### Shutdown has three more doors than it used to

[`main.rs:104-107`](../backend/src/main.rs) aborts the flusher and flushes the
usage tally one last time, deliberately in that order so the two cannot split
the final batch. A desktop app exits through a closed window, a tray quit, or
an OS logoff, and none of them are Ctrl-C.

`RunEvent::ExitRequested` therefore has to prevent the default exit, await
`Server::shutdown()`, and exit afterwards. Skip it and every session silently
discards up to sixty seconds of API usage counts — the one thing in the index
[with no source to rebuild from](architecture.md) — while leaving SQLite to be
killed mid-write.

The desktop `main` also cannot be `#[tokio::main]`: Tauri owns the main thread.
`tauri::async_runtime` is tokio, so `spawn_blocking` still has the thread pool
that [the index's blocking boilerplate](architecture.md) depends on.

## Configuration stops being environment-only

[`Config::from_env`](../backend/src/config.rs) is the only constructor, and
there is no shell environment behind a double-clicked icon. Precedence becomes
**environment variable, then settings file, then default**, so the documented
`RHIZOLOG_*` variables keep working and keep winning.

The settings file lives **beside the executable**, which is what makes the app
portable — copy the directory to a USB stick and the wiki it points at comes
with it. When that directory is not writable, it falls back to the OS config
directory rather than failing.

Two defaults do not survive the move.

**`./wiki` is relative to the working directory**, which for a shell is the
checkout and for a double-clicked executable is *usually* its own directory —
but for a Start-menu shortcut it is whatever "Start in" says, and for a file
association it is the opened file's folder. The desktop default has to resolve
against `std::env::current_exe()`. Nothing would report this as an error, which
is the problem.

**`../frontend/dist`** is meaningless in a bundle; see below.

### A wrong root is silent, so first run must ask

[`Store::open`](../backend/src/store.rs) calls `create_dir_all` on the root
before canonicalising it. That is right for a server told where to look, and
wrong for a GUI guessing: a bad default does not fail, it quietly creates an
empty wiki somewhere nobody will look for it again.

First run opens a folder picker rather than materialising a default. The picker
must also never be pointed at `example-wiki/` — it is a fixture whose totals
`example-wiki/index.md` states exactly, and starting one timer against it
rewrites what the documentation claims.

## The port is negotiated, and the result is written down

Hardcoding `127.0.0.1:3000` ([`config.rs:59`](../backend/src/config.rs)) is
fine for a dev server and wrong for an app that gets launched twice, or once on
a machine where something else already holds 3000.

The policy:

1. `RHIZOLOG_ADDR`, if set, is binding. An explicit address that is taken is an
   error, not a hint — someone asked for that port for a reason.
2. Otherwise try `127.0.0.1:3000`, because a predictable URL is worth having.
3. On `AddrInUse`, bind `127.0.0.1:0` and take what the OS gives.

Which means the port is no longer knowable in advance, so the server publishes
it: `<wiki root>/.rhizolog/server.json`.

```json
{
  "url": "http://127.0.0.1:3000",
  "wiki_root": "C:\\Users\\tim\\wiki",
  "pid": 24601,
  "version": "0.1.0",
  "started": "2026-08-06T14:25:30Z"
}
```

It sits beside the wiki because that is the handle a caller already has. An
agent working in a wiki checkout knows the directory; it should not also have to
be told a port. Discovery order, for anything that wants to reach a running
instance: `RHIZOLOG_ADDR`, then `.rhizolog/server.json`, then
`http://127.0.0.1:3000`.

### Written on ready, so its existence means something

The file is written after `start` returns and removed on clean shutdown. Since
`start` only returns once the initial reconciliation is done, **the file
appearing is the readiness signal** — a caller that finds it does not need to
poll for a healthy index, and no `503 index_syncing` state has to be invented to
cover the gap.

The gap is real, though: on a large wiki the window has nothing to show while
the scan runs, so the shell opens a splash window and swaps it for the real one
when `start` returns.

### The file is a hint; `/api/health` is the proof

A process killed hard leaves the file behind, and a pid can be reused. Rather
than trust it, a reader confirms with `GET /api/health` and checks the
`wiki_root` it reports ([`meta.rs:53`](../backend/src/api/meta.rs)) against the
wiki it meant. That endpoint already returns exactly the right fields, and a
handshake beats a liveness heuristic.

This doubles as the **single-instance lock**, and per wiki root rather than per
application — which is the correct granularity. Two windows on one wiki means
two SQLite writers, two file watchers, and a usage tally split across
processes; two windows on two different wikis is fine and should stay fine. A
second launch that finds a live server for the same root focuses the existing
window instead of starting anything.

### `.rhizolog/` now holds three kinds of thing

[Time tracking](time-tracking.md) already had to say out loud that the
directory is not all disposable — `index.db` is derived, `times/` is authored
data with no other copy. `server.json` is a third category: **volatile**,
meaningless the moment the process that wrote it stops, and never worth
preserving or restoring.

It is gitignored by name, beside the database, for the same reason the database
is: a wiki kept in git must not have the whole directory ignored.

## The frontend ships inside the binary

`AppState.assets` is an `Option<PathBuf>` served by `ServeDir`
([`api/mod.rs:173`](../backend/src/api/mod.rs)). A portable single file cannot
point at a directory that travels separately, so it becomes:

```rust
pub enum Assets { Dir(PathBuf), Embedded, None }
```

`Embedded` is `rust-embed` over `frontend/dist`, behind an `embed-assets`
feature on the library that `desktop/` turns on. That is a narrow feature — one
small dependency and a build-time requirement that `frontend/dist` exists — not
a gate on an entire GUI stack, which is the crate split paying for itself a
second time.

`Dir` stays exactly as it is, so `pnpm dev`'s proxy loop and `RHIZOLOG_ASSETS`
are untouched, and `None` keeps its current meaning and its message in
`missing_route`. A headless `rhizolog` therefore still serves a built frontend
from disk if pointed at one — the split is about what must compile, not about
withdrawing the UI from the server.

The alternative — let Tauri serve the SPA from its own asset protocol and leave
axum with `/api` — is where local and remote quietly diverge. A remote instance
serves its UI over HTTP; if the local one does not, then the SPA fallback, the
`/api` catch-all that stops a mistyped endpoint returning HTML, and every
same-origin assumption in the client are all exercised in only one of the two
configurations. Embedding keeps one code path and one set of tests.

### Swagger UI is already embedded, and its path trap gets sharper

`CLAUDE.md` records that `utoipa-swagger-ui`'s build script bakes the
**absolute** path of its asset directory into a generated `embed.rs`, which is
why moving the repository breaks `/swagger-ui` until `cargo clean -p`.

Shipping raises the stakes: `rust-embed` compiles the files into the binary in
release builds but reads them from disk in debug ones, so a debug build handed
to anyone else is a Swagger UI that 404s every asset on a path that never
existed on their machine. `swagger_ui_serves_its_own_assets` in
`backend/tests/frontend.rs` is the existing guard, and it needs to run against
the bundled release artifact rather than only under `cargo test`.

Keeping Swagger UI in the desktop app is deliberate. For an agent
[the spec is the manual](api-design.md), and a local instance is exactly where
someone is most likely to go looking for it.

## The window

`app.windows` is `[]` in `tauri.conf.json` and the window is built at runtime
with `WebviewUrl::External(http://127.0.0.1:<port>)`, because the port is not
known until `start` returns. `build.frontendDist` is omitted entirely — Tauri
treats it as optional, and axum owns every byte the webview will load.

That leaves nothing to display if `start` fails, so a failed launch reports
through a native dialog and exits rather than opening a window onto nothing.

### Native conveniences live in the shell

A folder picker for "open wiki", "reveal in Explorer", a tray icon, a global
hotkey to start a timer — all of them belong to the Tauri shell, calling the
same HTTP API, with **no Tauri JavaScript in the SPA at all**.

Besides keeping the promise at the top of this page, there is a mechanical
reason. The webview loads an `http://127.0.0.1:<port>` origin, which Tauri v2
treats as *remote*: JavaScript access to Tauri APIs would need a capability
listing that URL. The documented wildcard support is for subdomains, and the
port here is negotiated at startup — so the one thing that would have to be
wildcarded is the one thing that varies.

## Portable, on Windows

**Tauri has no portable bundle target.** NSIS and MSI are the Windows bundles;
portable means shipping the `.exe` from `target/release` and nothing else. Four
consequences:

- **WebView2 must already be present.** Tauri's docs state the runtime ships
  with the OS on Windows 10 (April 2018 or later) and Windows 11, so in practice
  it is there. But `webviewInstallMode` — `skip`, `downloadBootstrapper`,
  `embedBootstrapper`, `offlineInstaller`, `fixedRuntime` — only governs the
  *installers*, so a raw executable has to detect a missing runtime itself and
  say so. A blank window is the failure mode otherwise.
- **`WEBVIEW2_USER_DATA_FOLDER` should be set deliberately**, next to the
  executable. Otherwise "portable" leaks webview state into a directory the user
  did not choose and will not think to clean up.
- **An unsigned executable gets a SmartScreen warning.** Signing is a cost to
  plan for, not a detail.
- **Loopback does not trip the firewall.** Binding `127.0.0.1` raises no
  Windows Defender prompt; binding `0.0.0.0` does, on first launch, which would
  be an alarming thing for a wiki to do. The loopback default in
  [`config.rs:57-59`](../backend/src/config.rs) is now load-bearing for a second
  reason.

## Where it lives in the repo

```
Cargo.toml          # workspace: members = ["backend", "desktop"]
backend/            # the rhizolog library, and the headless `rhizolog` binary
  src/server.rs     # new: start / shutdown, the extracted boot sequence
desktop/
  Cargo.toml        # rhizolog = { path = "../backend" }
  build.rs          # tauri_build::build()
  tauri.conf.json
  icons/
  src/main.rs       # windows_subsystem = "windows"; splash, window, lifecycle
```

Nothing inside `backend/` moves, and `backend/` goes on being only a backend —
which was the other thing one binary would have cost, since a crate carrying
`tauri.conf.json` and an icon set is not a backend whatever the directory is
called.

The workspace exists so `desktop/` can depend on `backend/` by a path without a
second lockfile and a second build of everything they share. It has one visible
consequence: **`backend/target/` becomes `target/` at the repository root.**
The root `.gitignore` ignores `target/` at any depth so nothing leaks, but
`CLAUDE.md`'s layout table and its note about stopping the server before
`cargo build` both name the old path, and the README's commands are run from
`backend/`. Those want updating in the same commit as the split.

The two artifacts are named for what they are: the portable app is
`Rhizolog.exe` — via `[[bin]] name` and Tauri's `productName` — and the server
stays `rhizolog`, because one is something a person double-clicks and the other
is something a person types.

One warning that applies literally here: **`cargo tauri init` and
`pnpm create tauri-app` both create a nested `.git`**, which is the exact trap
`CLAUDE.md` describes — the root repository would stop tracking `backend/`
entirely. Scaffold by hand, or delete it immediately.

The development loop does not change. Because the SPA carries no Tauri code, a
browser at `http://127.0.0.1:3000` is a faithful environment:
`cargo run -p rhizolog` plus `pnpm dev` stays the way the UI is built, and
`cargo tauri dev` is only for working on the shell. Someone who never touches
the desktop app never builds it.

## Changing wikis means restarting, for v1

`Store`, `TimeStore`, `Index` and the watcher are each bound to one root at
startup. Switching wikis live means tearing all four down and rebuilding them,
which is only safe once `start`/`shutdown` are genuinely re-entrant — and it
also means moving `server.json`, re-pointing the webview, and deciding what
happens to a timer running in the wiki being left.

For v1, picking a different wiki writes the settings file and restarts the app.
The design above does not preclude the better version: `Server` owning the whole
lifecycle is what makes it reachable later.

## What this needs in tests

- `Assets::Embedded` served through the same assertions
  `backend/tests/frontend.rs` already makes about a directory, including the SPA
  fallback and the `/api` catch-all.
- `start` then `shutdown` round-tripping, with the usage tally flushed — the
  regression that would otherwise only show up as slowly wrong numbers in
  `/api/stats`.
- `server.json` written on ready, removed on clean shutdown, and treated as
  absent when the server it names does not answer `/api/health` for that root.
- Port fallback: a second instance on a busy 3000 lands somewhere else and says
  where.
- `swagger_ui_serves_its_own_assets` against a release build.
- `cargo build -p rhizolog` on a container with no GUI toolkit installed. The
  crate graph is what guarantees the server needs no display libraries, and a
  path dependency added in the wrong direction would revoke that silently — the
  build is the only thing that would notice.

## Deliberately out of scope

- **Authentication.** Loopback is the whole security boundary today and that is
  written into `config.rs`. A remote instance needs a token, and the config
  should have somewhere to put one before that day, but shipping a desktop app
  does not make that day arrive.
- **CORS.** `tower-http`'s `cors` feature is enabled in `Cargo.toml` and nothing
  in the crate uses it — everything is same-origin, including the webview. An
  agent reaching a remote instance from a browser context is what would change
  that, and it should be a deliberate decision rather than a default that was
  already switched on.
- **Auto-update.** A portable executable that rewrites itself is a different
  product decision; for now, replacing the file is the update.
- **macOS and Linux bundles.** The architecture is portable, the packaging work
  is not, and development is on Windows.
