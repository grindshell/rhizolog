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

### The seam is `Config`, `Server`, and the public client path

`desktop/` can see everything `rhizolog` exports, which includes `Store`,
`Index` and `TimeStore`. It must use none of them. The rule above — no
capability in the app that a browser pointed at a remote instance lacks — is
only enforceable if the shell cannot reach wiki data except the way everything
else does.

A Tauri command that reads a page straight off disk would be a two-line
convenience and the end of the property this design exists to protect. When the
shell needs wiki data, it makes an HTTP request to itself, like every other
client.

`endpoint::live` is not an exception to that, despite being a library call: it
reads a published file and makes an HTTP request, which is exactly the discovery
path an agent uses and involves no privileged access to anything. The rule is
about the wiki, not about the crate boundary — **`Store`, `Index` and
`TimeStore` are the names that must not appear in `desktop/`.**

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

### A windowed binary has no stdout

`tracing_subscriber::fmt` to stdout is the console server's whole account of
itself, and a `windows_subsystem = "windows"` build has nowhere to put it. The
app adds a daily rolling file in Tauri's app log dir —
`%LOCALAPPDATA%\dev.rhizolog.app\logs\` on Windows — and keeps the stdout layer
as well, which costs nothing and is what makes `cargo run` on a debug build
behave the way anyone would expect.

`RHIZOLOG_LOG` still filters both. This is not a nicety: a launch that fails
before there is a window has to leave something behind, or the only symptom is
an icon that bounced once.

## Configuration stops being environment-only

[`Config::from_env`](../backend/src/config.rs) is the only constructor, and
there is no shell environment behind a double-clicked icon. Precedence becomes
**environment variable, then settings file, then default**, so the documented
`RHIZOLOG_*` variables keep working and keep winning.

The settings file lives **beside the executable**, which is what makes the app
portable — copy the directory to a USB stick and the wiki it points at comes
with it. When that directory is not writable, it falls back to the OS config
directory rather than failing.

The precedence lives in `Config::resolve(Fallbacks)`; what the fallbacks *are*
is the caller's business, because each binary is right about a different set.
`Config::from_env` is now that call with the server's answers, so the headless
path is unchanged.

`Fallbacks::root` is an **`Option`**, and that is the interesting part. The
desktop app has nothing to put there when `RHIZOLOG_ROOT` is set — it never
asked the user, because it did not need to — and `None` with no variable either
is `ConfigError::NoWikiRoot` rather than a guess. Which matters more than it
sounds:

### A wrong root is silent, so first run must ask

[`Store::open`](../backend/src/store.rs) calls `create_dir_all` on the root
before canonicalising it. That is right for a server told where to look, and
wrong for a GUI guessing: a bad default does not fail, it quietly creates an
empty wiki somewhere nobody will look for it again.

So the app has **no default wiki at all**. First run opens a folder picker;
declining it exits without a dialog, because being asked and saying no is not an
error. The choice is written to the settings file, and the same reasoning
applies on every later run: a remembered root that is no longer a directory gets
the picker again rather than being handed to `Store::open`, which would greet
somebody whose wiki had moved with an empty dashboard where their notes used to
be.

This dissolves an earlier worry on this page — that `./wiki` resolves against a
working directory a double-clicked executable does not control. There is no
`./wiki` in the desktop app to resolve. The concern survives for anything else
it carries, so `Fallbacks::assets` is `<exe dir>/dist`: drop a `dist` folder
beside a portable copy and it overrides the built-in dashboard, from any working
directory.

The picker must never be pointed at `example-wiki/` — it is a fixture whose
totals `example-wiki/index.md` states exactly, and starting one timer against it
rewrites what the documentation claims.

### Changing wikis is File → Open Wiki…, and it restarts

`Store`, `TimeStore`, `Index` and the watcher are each bound to one root at
startup, so the menu item saves the choice, stops the server properly and calls
`restart`. Stopping first is not optional: a restart that skipped it would lose
the usage counts and leave an endpoint file describing a server about to stop
existing.

The item is **disabled when `RHIZOLOG_ROOT` is set**, because the app is not the
thing deciding — a restart would come straight back to the same wiki, and
offering a choice that cannot be honoured is worse than not offering it.

### The settings file will be hand-edited, so it tolerates a BOM

It is a small JSON file in a folder people are invited to carry around on a
stick. Notepad and PowerShell's `Set-Content -Encoding utf8` both write a UTF-8
BOM without being asked, and `serde_json` rejects the document outright — so
without stripping it, fixing a typo in the file makes the app forget which wiki
it opens and ask again.

This is the same hazard, for the same reason, that page parsing already handles;
see the BOM section of [Architecture](architecture.md). It was found the way the
first one was: by writing the file from PowerShell and watching the app ask a
question it should have known the answer to.

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

`start` writes it as its last act, once the server is bound, reconciled and
watching, and `shutdown` withdraws it first. So **the file appearing is the
readiness signal** — a caller that finds one does not need to poll for a healthy
index, and no `503 index_syncing` state has to be invented to cover the gap.

Publishing belongs inside `start` rather than to whoever called it, precisely
because that guarantee is easy to break from outside: a caller that writes the
file when it feels ready rather than when the server is has reintroduced the
gap, and nothing would catch it.

The gap is real, though: on a large wiki the window has nothing to show while
the scan runs. A splash window swapped for the real one when `start` returns is
the answer, and it is not built yet — see [`TODO.md`](../TODO.md).

### The file is a hint; `/api/health` is the proof

A process killed hard leaves the file behind, and a pid can be reused. Rather
than trust it, a reader confirms with `GET /api/health` and checks the
`wiki_root` it reports ([`meta.rs:53`](../backend/src/api/meta.rs)) against the
wiki it meant. That endpoint already returns exactly the right fields, and a
handshake beats a liveness heuristic.

Both halves are `endpoint::live`, which is the whole protocol in one function:
read the hint, then confirm it. The comparison is against the root actually
asked about rather than the one the file names, because a copied wiki directory
brings its `server.json` along and that file describes somebody else's live
server. All three cases have tests.

### One instance per wiki, not per application

That handshake is also the **single-instance lock**, at the granularity that
matches the damage. Two windows on one wiki means two writers on one index, two
file watchers, and an endpoint file that can only describe the newer of them —
so the older window's published address becomes a lie. Two windows on two
different wikis costs nothing and is a reasonable thing to want, which is why
`tauri-plugin-single-instance` is not the answer here: it locks the
application, and would forbid the harmless case along with the harmful one.

A second launch on a wiki that is already open therefore says so, names the URL
the running one is serving, and offers **Open a different wiki…** or **Quit**.
The offer is the useful part: the case where somebody wants two windows is
usually the case where they want two *wikis*, and the alternative — refusing and
exiting — would leave them with no way to say so.

**It does not raise the other window.** That needs platform code to find another
process's window and ask for the foreground, Windows may decline the request and
flash the taskbar instead, and the dialog already says which URL to look for.
Worth revisiting only if the dialog turns out to annoy.

**It is a courtesy, not a mutex.** Two launches close enough together both check
before either publishes, and both start. Closing that properly needs an OS-level
lock taken before the bind; the check as it stands covers the case that actually
happens, which is launching while a window is already open.

### `.rhizolog/` now holds three kinds of thing

[Time tracking](time-tracking.md) already had to say out loud that the
directory is not all disposable — `index.db` is derived, `times/` is authored
data with no other copy. `server.json` is a third category: **volatile**,
meaningless the moment the process that wrote it stops, and never worth
preserving or restoring.

It is gitignored by name, beside the database, for the same reason the database
is: a wiki kept in git must not have the whole directory ignored.

## The frontend ships inside the binary

A portable single file cannot point at a directory that travels separately, so
`AppState.assets` is no longer an `Option<PathBuf>`:

```rust
pub enum Assets { Dir(PathBuf), Embedded, None }
```

`Embedded` is `rust-embed` over `frontend/dist`, behind an `embed-assets`
feature on the library that `desktop/` turns on. That is a narrow feature — one
small dependency and a build-time requirement that `frontend/dist` exists — not
a gate on an entire GUI stack, which is the crate split paying for itself a
second time. It is off by default, so `cargo build` does not quietly acquire
`pnpm build` as a prerequisite.

`Dir` stays exactly as it was, so `pnpm dev`'s proxy loop and `RHIZOLOG_ASSETS`
are untouched, and `None` keeps its meaning and its message. A headless
`rhizolog` therefore still serves a built frontend from disk if pointed at one —
the split is about what must compile, not about withdrawing the UI from the
server.

**A directory that exists wins over the embedded copy.** Somebody who has
pointed `RHIZOLOG_ASSETS` at a fresh build wants that build, not the one
compiled in weeks ago, and the startup log says which was chosen. It also means
the portable binary needs no special configuration: run it anywhere there is no
`frontend/dist` and the compiled-in copy is simply what is there.

**`rust-embed`'s `debug-embed` feature is on.** Without it a debug build reads
the files from the absolute path baked in at compile time — which is exactly the
trap `utoipa-swagger-ui` has already sprung on this project once, and it is
worse here because a debug desktop build would look fine on the machine that
made it. An embedded build should be embedded in both profiles.

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

The server is started from a task rather than awaited in `setup`, so the event
loop is already running while the wiki is reconciled. The window appears when
there is something behind it. On a large wiki that is a gap with nothing on
screen, and a splash window is the eventual answer; for now the gap is the same
one the console server has, and it is measured in the same scan.

Closing the window is not the end of the process's obligations, so
`RunEvent::ExitRequested` calls `prevent_exit`, awaits `Server::shutdown`, and
only then exits — otherwise the last minute of usage counts goes with it. The
`exit` at the end of that work comes back round as another `ExitRequested`,
which an `AtomicBool` absorbs rather than letting it recurse.

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

### A webview has no tabs, so `target="_blank"` is the shell's problem

The dashboard's "API docs" link is a plain anchor to `/swagger-ui` with
`target="_blank"`, and in the app it did nothing at all — no window, no error,
no log line. WebView2 raises `NewWindowRequested` for a `_blank` anchor as it
does for `window.open`, and wry's default when no handler is registered is to
mark the event handled and drop it. Tauri only registers one when the window was
built with `on_new_window`, so the request died inside the webview and the click
had no effect anybody could see.

It was never only the API docs. Every external link in a rendered page body
carries `target="_blank"` — the frontend puts it there so nothing in a page can
navigate the dashboard's own tab away — and so does every external link in the
"Links out" panel. The whole class of "open this elsewhere" was dead in the app
and fine in a browser, which is this page's divergence arriving from the other
direction: not the app doing something a browser cannot, but failing at
something a browser does.

So the shell answers, since there is nowhere else to answer from and the page
must not learn it is inside Tauri. What the answer is depends on where the link
points:

- **A page of this server gets a second window** onto the same origin, with the
  same handler attached to it — otherwise Swagger UI's own links out would be
  dead one level down. The window's label is derived from the URL's path, which
  is what makes a second click on the same link raise the window the first one
  opened rather than stack another behind it. It carries no menu: File → Open
  Wiki… restarts the application, which is not a thing to offer from a window
  looking at one page.
- **Everything else goes to the real browser.** A Tauri window has no address
  bar, no back button and nothing that says whose site is in it, which is not
  something to point at the open web. Only `http`, `https`, `mailto` and `tel`
  are handed over: that call ends at `ShellExecute` on Windows, and a `file:`
  URL in a wiki page should not be a way to start a program.

`tauri-plugin-opener` opens the browser, **with its JavaScript half switched
off**. The plugin's own answer to `target="_blank"` is a script injected into the
page that cancels the click and calls Tauri IPC — which is both the Tauri
JavaScript in the SPA that the section above forbids and, on a remote origin
with no capability, a second way for the link to do nothing.

The plugin also opens folders, which is what the log-folder menu item in
[`TODO.md`](../TODO.md) was waiting for.

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
  src/server.rs     # start / shutdown, the extracted boot sequence
  src/endpoint.rs   # .rhizolog/server.json
  src/assets.rs     # Dir | Embedded | None
desktop/
  Cargo.toml        # rhizolog = { path = "../backend", features = ["embed-assets"] }
  build.rs          # tauri_build::build()
  tauri.conf.json   # no frontendDist, no windows: both are made at runtime
  icons/            # placeholders; replace with real artwork
  src/main.rs       # windows_subsystem = "windows"; window, logging, lifecycle
  src/settings.rs   # which wiki, remembered between runs
```

### The binary cannot be called `Rhizolog`

It was, briefly, on the reasoning that a portable download should be named for
the product. Windows filenames are case-insensitive, so `Rhizolog.exe` and the
server's `rhizolog.exe` are **one file** in `target\debug\` — building both
leaves whichever cargo linked last, with no warning and no error. The symptom is
that `cargo run -p rhizolog` opens a window, or the app starts a console server
against `.\wiki`; it cost a confusing half hour before the directory listing gave
it away.

So the binary is `rhizolog-desktop` and the product name lives in
`productName` and `mainBinaryName` in `tauri.conf.json`, where the installer and
the window can use it and nothing can collide with it.

Nothing inside `backend/` moves, and `backend/` goes on being only a backend —
which was the other thing one binary would have cost, since a crate carrying
`tauri.conf.json` and an icon set is not a backend whatever the directory is
called.

The workspace exists so `desktop/` can depend on `backend/` by a path without a
second lockfile and a second build of everything they share. It has one visible
consequence: **`backend/target/` becomes `target/` at the repository root.**
The root `.gitignore` ignores `target/` at any depth so nothing leaks, but
`CLAUDE.md`'s layout table and its note about stopping the server before
`cargo build` both named the old path, and the README's commands are run from
`backend/`. Both were corrected in the same commit as the split.

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
  fallback and the `/api` catch-all. These only compile under
  `--features embed-assets`, so a plain `cargo test` does not run them and CI
  needs the second invocation — a feature nothing exercises is a feature that
  breaks quietly.
- `start` then `shutdown` round-tripping, with the usage tally flushed — the
  regression that would otherwise only show up as slowly wrong numbers in
  `/api/stats`.
- `server.json` written on ready, removed on clean shutdown, and treated as
  absent when the server it names does not answer `/api/health` for that root —
  including the copied-directory case, where it answers for a different one.
- Port fallback: a second instance on a busy 3000 lands somewhere else and says
  where.
- `swagger_ui_serves_its_own_assets` against a release build.
- `cargo build -p rhizolog` on a container with no GUI toolkit installed. The
  crate graph is what guarantees the server needs no display libraries, and a
  path dependency added in the wrong direction would revoke that silently — the
  build is the only thing that would notice.

## Deliberately out of scope

These, and the packaging work still outstanding, are tracked in
[`TODO.md`](../TODO.md) at the repository root. The reasoning stays here; what
is left to do is listed there.

- **Authentication.** Loopback is the whole security boundary today and that is
  written into `config.rs`. A remote instance needs a token, and the config
  should have somewhere to put one before that day, but shipping a desktop app
  does not make that day arrive.
- **CORS.** Everything is same-origin, including the webview, so nothing needs
  it. `tower-http`'s `cors` feature was enabled in `Cargo.toml` and used by
  nothing; it is now off, because a feature switched on in advance of a decision
  is how the decision gets made by accident. An agent reaching a remote instance
  from a browser context is what would change this, and turning the feature back
  on is one line at that point.
- **Auto-update.** A portable executable that rewrites itself is a different
  product decision; for now, replacing the file is the update.
- **macOS and Linux bundles.** The architecture is portable, the packaging work
  is not, and development is on Windows.
