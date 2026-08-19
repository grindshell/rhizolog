# TODO

Work that is known about and not done. Each entry says why it is not done,
because "we forgot" and "we decided not to yet" are different things and a list
that cannot tell them apart stops being read.

The reasoning behind most of this lives in
[`knowledge-base/`](knowledge-base/index.md); this file is the index of what is
still outstanding, not a second place to argue design.

## Before any of it goes to anyone

These apply to a release of any kind, including one that is only "clone it and
`cargo run`". They are cheap, and every one of them is invisible from inside the
project — which is why they were not written down until somebody asked what a
beta needs.

- **Dependency licences are unreviewed.** The AGPL is strong copyleft, so a
  dependency under terms it cannot be combined with is a real problem rather
  than a paperwork one. The Rust and npm trees here are almost entirely
  MIT/Apache-2.0, which is fine in this direction, but nothing has actually
  checked — `cargo-license` or `cargo-deny` over the workspace, and
  `pnpm licenses list`, would say so in a minute. Worth doing once before
  anybody is handed a copy, and worth having in CI after that.
- **No per-file licence notices.** `LICENSE` and the `license` fields in the
  manifests are what a tool reads; the AGPL's own appendix also asks for a
  short notice at the top of each source file, which is what a human reads when
  a file has been copied somewhere on its own. Not done, because it touches
  every file in the repository and is worth doing in one deliberate pass.
- **There is no release process, and no single definition of "the version".**
  `backend/Cargo.toml`, `desktop/Cargo.toml` and `desktop/tauri.conf.json` all
  say `0.1.0` and are bumped by hand in step; `frontend/package.json` says
  `0.0.0` and nothing publishes it. The number travels in `server.json` and
  `/api/health`, so it is the thing a bug report will quote. Tauri takes the
  version from `Cargo.toml` when the field is omitted from `tauri.conf.json`,
  which removes one of the three. There is also no changelog and no tag.
- **The README is written for a contributor.** Its Quick Start opens with
  `pnpm install`, which is right for somebody building the thing and useless to
  somebody who was handed it. A reader who did not clone the repository needs
  four things: what it does, where its data lives, that it binds loopback with
  no authentication, and where to send a bug.

## Before the desktop app goes to anyone else

The app works. These are the things that make the difference between "runs on
the machine that built it" and "is a download".

- **Real icons.** `desktop/icons/` are placeholders — a small branching glyph
  generated so `tauri-build` would produce an executable at all. They are not
  artwork and should not ship as any.
- **Detect a missing WebView2 runtime.** Tauri's `webviewInstallMode` only
  governs the NSIS and MSI installers, so a portable `.exe` gets no help from
  it. The runtime ships with Windows 10 (April 2018 or later) and Windows 11, so
  in practice it is there — but when it is not, the current failure mode is a
  blank window rather than a sentence explaining what to install.
- **Set `WEBVIEW2_USER_DATA_FOLDER` deliberately.** Otherwise "portable" leaks
  webview state into a directory the user did not choose and will not think to
  clean up. It should sit beside the executable, like the settings file.
- **Sign the executable.** An unsigned download gets a SmartScreen warning. This
  is a cost to plan for rather than a detail to discover.

See [The desktop app](knowledge-base/desktop-app.md), "Portable, on Windows".

## Accounts and visibility

Both halves are built: accounts on disk, sessions, a gate in front of `/api`,
and a page's `visibility` applied to every query that can return one. See
[Accounts](knowledge-base/accounts.md) and
[Page visibility](knowledge-base/visibility.md) for the reasoning. These are the
pieces that are known to be missing.

- **No per-directory or per-tag visibility defaults.** Every page carries its own
  line, which is fine for a handful and tedious for a branch. The natural shape
  is a `.rhizolog/visibility.toml` of prefix rules; the reason to wait is that it
  introduces a *second* place a page's visibility is decided, and the first
  second place already costs a test to keep honest.
- **No groups, and no write permission distinct from read.** `readers:` is a list
  of accounts, which on a wiki with three people is a group and clearer than one.
  Anybody who can read a page can edit it; splitting the two is a real feature
  and a different one.
- **Pins and the time log have no visibility of their own.** They are wiki-wide
  state shared by every account. Their page *titles* are filtered, so a pin to a
  page you cannot read has none — but the slug stays, because it is the pin's own
  content.
- **No rate limiting on sign-in.** Argon2 is a real natural throttle — roughly
  twenty attempts a second per core, and each costs the attacker what it costs
  the server — but it is not a lockout, and a network instance wants one. Also
  the first thing on this list that needs shared mutable state keyed by
  something other than a session, so it is not a five-line change.
- **`/api/health` discloses `wiki_root` to anonymous callers.** The counts are
  already withheld; the path stays because `endpoint::live` compares it to
  confirm a published `server.json`, and that handshake happens before anybody
  could sign in. Restricting the field to loopback callers would need
  `ConnectInfo`, which means `into_make_service_with_connect_info` in
  `server.rs`. A filesystem path is the least interesting thing behind that
  door, which is why this is a note rather than work in progress.
- **No named API tokens.** A session token works for a script today. What it
  does not do is survive a password change or carry a label saying what it is
  for, and revoking one script means signing that account out everywhere.
- **No password reset.** No email and no second factor, so recovery is an owner
  setting a new password — or editing the file on the server, which a remote
  administrator cannot do. Worth a plan before anybody but its author runs one.
- **Sessions are not listable.** `count_sessions` exists and nothing exposes it.
  "Where am I signed in, and sign that one out" is the natural next endpoint.

## Rough edges

- **"Frontmatter is not valid YAML" is said about valid YAML.**
  `FrontmatterError::InvalidYaml` wraps every `serde_yaml_ng::Error`, and that
  type covers deserialisation as well as parsing — so `created: 2026-08-19`, a
  bare date in a field that wants a full timestamp, reports *"frontmatter is not
  valid YAML: created: premature end of input"*. The YAML is fine; the value is
  not a timestamp. Worse than the wording is the consequence: `created` is the
  one frontmatter field strict enough for a plausible hand-written value to make
  a page malformed, which takes it out of every listing. Every other field is a
  string, and YAML's reserved words all survive the round trip
  (`every_yaml_reserved_word_survives_a_round_trip` in `backend/src/page.rs`).
  Splitting the two cases means telling a syntax error from a type error through
  `serde_yaml_ng::Error`, which does not distinguish them in its API; accepting a
  bare date is the other fix and is a separate decision.
- **A splash window while a large wiki reconciles.** `server::start` returns
  once the index is in step with the files, and the window is only built after
  that — so on a big wiki the gap between double-click and anything appearing is
  a full scan, with nothing on screen to say so. The server is already started
  on a task rather than blocking the event loop, so this is a window to show,
  not a restructure.

  Measuring that scan first found a defect rather than physics, and fixing it
  took 5,000 pages from 27 s to 4.4 s and 20,000 from nearly nine minutes to
  18 s. What is left is real but much smaller, and it is now linear, so the
  window is worth showing on a large wiki and nothing is hiding behind it.
- **The single-instance check is a courtesy, not a mutex.** Two launches close
  enough together both confirm nothing is serving the wiki before either
  publishes, and both start. Closing it properly needs an OS-level lock taken
  before the bind. What is there covers the case that actually happens —
  launching while a window is already open.

## Verification gaps

**There is no CI.** Everything below is run by hand today, which is the gap
worth closing first, because two of these are configurations that break quietly.
The remote is a self-hosted git rather than GitHub, so which runner this uses is
itself an unmade decision.

- **Large wikis, measured once by hand.** Synthetic wikis of 1,000 to 20,000
  pages against the release server, timed from process start to
  `.rhizolog/server.json` appearing. It found the quadratic FTS delete fixed in
  `index/mod.rs`, and after that a first index runs at about 0.9 ms per page,
  flat from 1,000 pages to 20,000 (0.88 s to 18 s). Warm starts are 0.1–0.7 s
  throughout.

  Worth becoming something that runs rather than something that was done once:
  a reindex that grows with the wiki is invisible on `example-wiki/`'s nine
  pages, which is exactly why it survived this long. Two things are still
  unmeasured — the desktop app's own first launch, which puts a window and a
  webview around the same scan, and how long a single page save takes on a wiki
  that size.

  Whatever runs it needs to handle a noisy machine. The first re-measurement
  after the fix reported 5,000 pages as *slower* than before and 10,000 as
  faster than 5,000, which is impossible for work that grows with the wiki; it
  was background indexing of the 36,000 files the test had just created.
  Repeating each size three times and taking the minimum gave a clean linear
  result. A single timing on this machine is not evidence.
- **`cargo test --features embed-assets`.** Six tests only compile under that
  feature — the ones covering the dashboard served out of the binary. A plain
  `cargo test` skips them silently, and a feature nothing exercises is a feature
  that breaks without telling anyone. It needs `pnpm build` to have run.
- **The settings form, clicked.** `desktop/src/settings_window.rs` unit-tests
  everything on the Rust side of the webview: what the form parses to, what the
  page renders, that `RHIZOLOG_ADDR` disables it, that a request from any other
  window is refused. What none of them touch is whether WebView2 hands a form
  post on a custom scheme to the handler — the half where being wrong is a
  button that does nothing. `respond` logs at info on both the GET and the post,
  so the app log says which half happened.
- **`cargo build -p rhizolog` somewhere with no GUI toolkit.** The crate graph
  is what guarantees the headless server needs no display libraries; a path
  dependency added in the wrong direction would revoke that, and only a build on
  a bare container would notice.
- **`swagger_ui_serves_its_own_assets` against a release artifact.**
  `rust-embed` reads from disk in debug builds and the path it reads from is the
  absolute one baked in at compile time, so the test passing under `cargo test`
  says nothing about what a shipped binary serves. This has already gone wrong
  once, for the related reason recorded in `CLAUDE.md`.

## Decided against, for now

Not oversights. Each of these was considered and deferred, and the reason is
worth keeping so the question does not get reopened from scratch.

- **Changing the port without a restart.** Closer than the wiki case below and
  blocked by something else: the stores are all bound to a root that is not
  moving, and `server::start`/`shutdown` already round-trip. What is on the old
  origin is the *window* — the webview, the `target="_blank"` handler that
  closes over the URL, and any second window opened from it. Rebinding means
  rebuilding all of that, which is more than a restart costs.
- **Switching wikis without a restart.** `Store`, `TimeStore`, `Index` and the
  watcher are each bound to one root at startup. File → Open Wiki… saves the
  choice, stops the server properly and relaunches, which is correct and cheap.
  Live switching needs `Server::start`/`shutdown` to be genuinely re-entrant and
  the endpoint file to move with the root.
- **CORS.** Everything is same-origin today, including the webview, so nothing
  needs it. `tower-http`'s `cors` feature was enabled and unused; it is now off,
  because a feature switched on in advance of a decision is how the decision
  gets made by accident. An agent reaching a remote instance from a browser
  context is what would change this — and accounts make that more likely than it
  was, so this is closer than the rest of this list.
- **Auto-update.** A portable executable that rewrites itself is a different
  product decision. For now, replacing the file is the update.
- **macOS and Linux bundles.** The architecture is portable; the packaging work
  is not, and development is on Windows.
- **Revisions and history.** A wiki directory is very likely a git repository
  already, which covers history for whoever holds the disk. Accounts make "who
  changed this" a question with more than one answer, so this is closer than it
  was — but git still answers it, and answering it twice is worse. See
  [Architecture](knowledge-base/architecture.md).
