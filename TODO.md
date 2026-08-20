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
project, which is why they were not written down until somebody asked what a
beta needs.

- **Dependency licences are unreviewed.** The AGPL is strong copyleft, so a
  dependency under terms it cannot be combined with is a real problem rather
  than a paperwork one. The Rust and npm trees here are almost entirely
  MIT/Apache-2.0, which is fine in this direction, but nothing has actually
  checked. `cargo-license` or `cargo-deny` over the workspace, and
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

- **Real icons.** `desktop/icons/` are placeholders: a small branching glyph
  generated so `tauri-build` would produce an executable at all. They are not
  artwork and should not ship as any.
- **Detect a missing WebView2 runtime.** Tauri's `webviewInstallMode` only
  governs the NSIS and MSI installers, so a portable `.exe` gets no help from
  it. The runtime ships with Windows 10 (April 2018 or later) and Windows 11, so
  in practice it is there. When it is not, the current failure mode is a
  blank window rather than a sentence explaining what to install.
- **Set `WEBVIEW2_USER_DATA_FOLDER` deliberately.** Otherwise "portable" leaks
  webview state into a directory the user did not choose and will not think to
  clean up. It should sit beside the executable, like the settings file.
- **Sign the executable.** An unsigned download gets a SmartScreen warning. This
  is a cost to plan for rather than a detail to discover.

See [The desktop app](knowledge-base/desktop-app.md), "Portable, on Windows".

## The product site

`site/` is the static site at rhizolog.com. The landing page and a 404 page are
built and `site/dist` is a deployable tree, so most of what follows is the gap
between a directory of files and a site somebody can reach. The reasoning behind
the design is in [The product site](knowledge-base/product-site.md); this is
what is outstanding.

- **The GitHub mirror does not exist yet.** Every link that leaves the site,
  apart from the licence, resolves to `github.com/grindshell/rhizolog` through
  `site/src/links.ts`: both calls to action in the hero, Docs and Source in the
  nav, four of the five footer entries, and the second button on the 404. The
  site is honest about having nothing to download; it stops being honest if its
  primary call to action 404s. The mirror is decided on and named, and it is
  where CI will build releases. It has simply not been pushed.
- **`site/dist` has not been deployed, and the DNS record is unconfirmed.**
  Deployment is `./deploy.sh ../../../rhizolog/dist rhizolog` from
  `server-configs/static`, which unpacks a build into a timestamped release and
  swaps a symlink. The Caddy block for rhizolog.com has been rewritten for a
  statically generated tree; see the Deployment section of the knowledge base
  page for the three ways the old one was wrong. DNS is the piece nothing in
  this repository can check, and the failure is quiet: the certificate is issued
  over DNS-01 through Cloudflare, so it is obtained whether or not an A record
  points anywhere, and a working config and an unreachable site look identical
  from the server.
- **No `og:image`, so every shared link renders as a text-only card.**
  `site/src/layouts/Base.astro` emits the title, description and canonical URL
  and declares `twitter:card: summary`, and there is no image for either to
  point at. One static PNG of the mark on the dark ground, and
  `summary_large_image` with it.
- **No `robots.txt`.** `site/public/` holds the favicon and nothing else. It is
  three lines, it is requested on every crawl whether or not it exists, and the
  `Sitemap:` line in it is the reason to add a sitemap at the same time. A
  sitemap alone is marginal at two pages and stops being marginal with the docs.
- **Both figures on the landing page are hand-authored placeholders**, and their
  components say so at the top. They are meant to be fixtures captured at build
  time from a real server run against `example-wiki/`, for the same reason the
  OpenAPI document is generated from the routes rather than written: a figure
  that has quietly stopped being true is worse than no figure. The graph's node
  positions come from a hash of the slug, so the picture has to be the one
  `/api/graph` produces. The heat map's capture needs
  `?at=2026-08-06T18:00:00Z&offset=0`, because the committed entries are pinned
  to 30 July to 6 August 2026 and a request without it returns an empty week and
  renders blank.
- **`/docs` does not exist.** The nav's Docs entry points at the mirror's
  README, which is genuinely the documentation until there is something here.
  This is half of why Astro was chosen: the docs are markdown, and content
  collections are a documented path rather than something to invent.
- **`/demo` does not exist.** The page says "Browsable demo coming soon" under
  both figures. The shape is settled: one browsable static wiki over
  `example-wiki/`, captured at build time, rendered with the dashboard's own
  SolidJS components prerendered through `@astrojs/solid-js` rather than
  reimplemented, so the demo cannot show a UI the download does not have. That
  costs a real build step, which is the other half of why Astro is here.
  `Nav.astro` deliberately carries no Demo entry rather than a dead one, so
  landing it is a one-line change there.
- **Where `/api` points is deliberately unresolved.** Swagger UI is served by
  the backend, at an address that only exists once somebody is running one, so
  the site linking to it needs an answer to "whose instance". Publishing a
  rendered copy of the OpenAPI document is the obvious alternative and has not
  been decided on.
- **The download section is waiting on a first release.**
  `site/src/pages/index.astro` says "Windows build coming soon" in place of it.
  The section is written to take a real download without the page changing
  shape: two entries rather than one button that guesses at the platform, and
  the SmartScreen warning on an unsigned executable said out loud rather than
  discovered. Its actual blockers are the two sections above this one.

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
  page you cannot read has none, but the slug stays, because it is the pin's own
  content.
- **No rate limiting on sign-in.** Argon2 is a real natural throttle, roughly
  twenty attempts a second per core, and each costs the attacker what it costs
  the server. It is not a lockout, though, and a network instance wants one. Also
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
  setting a new password, or editing the file on the server, which a remote
  administrator cannot do. Worth a plan before anybody but its author runs one.
- **Sessions are not listable.** `count_sessions` exists and nothing exposes it.
  "Where am I signed in, and sign that one out" is the natural next endpoint.

## Idea Inbox

The implementation direction is settled and recorded in
[Idea Inbox implementation plan](knowledge-base/idea-inbox.md). **Phase I0, the
authored model and store, is built**; the five phases after it are not, so the
feature is not usable and capture alone is not the MVP. The work is phased so
that the authored store can land before analysis and UI without calling that
partial slice the feature.

- **Nothing outside `backend/src/ideas/` knows the store exists.** `AppState`,
  `server::start`, manual reindex and the watcher are all untouched, because
  wiring a store into startup before anything reconciles it would create a
  directory on every wiki for no reason. `AppState` should hold an
  `IdeaService` and reach the files through `IdeaService::store()`, which is
  where the owner rules stop being optional.
- **No derived index.** Folding decision events into current membership,
  rejections, lifecycle inputs and promotion state, plus startup
  reconciliation, a rebuild that equals incremental state, capture FTS, term
  rows and watcher targets. SQLite may accelerate the feature but may not
  become its only copy.
- **No API.** Owner-scoped CRUD, the decision operations, uniform errors,
  OpenAPI schemas and unique operation ids, then regenerating
  `frontend/openapi.json` and `frontend/src/api/schema.d.ts`.
- **No explainable recurrence.** TF-IDF version 1, candidate rejection and
  reconsideration, the deterministic lifecycle rules and evidence receipts are
  the part that distinguishes this from a second notes inbox.
- **No dashboard surface or promotion path.** `/inbox`, `/ideas` and idea detail
  need a responsive capture-first UI. Promotion should assemble a page draft,
  use the existing page API, then record the association idempotently.
- **Open captures are not yet adopted by the first account.** Owner comparison
  is equality, so a capture written while the wiki had no accounts belongs to
  no account once one exists, which would cost somebody their whole inbox for
  turning authentication on. The answer is decided: creating the first account
  stamps `owner:` onto every unowned capture, idea and event. What is not
  decided is where that runs, since an account can also be created by dropping
  a file into `.rhizolog/users/` and no API handler sees that. It rewrites
  authored files, so it is a migration and wants a log line and a refusal
  rather than a guess if more than one account already exists. Needed by I2.

Do not add LLM summaries, embeddings, automatic membership, notifications,
tasks or a native mobile app while this slice is in progress. Those are later
questions gated on observed use, not missing pieces of the MVP.

## Rough edges

- **A time entry's `start` and `end` do not take a bare date.** `created` does,
  on both a page and an account, and these were deliberately left out rather than
  forgotten: a bare date on `created` fills in a time that was never there, while
  a timer that "started on the 19th" is a claim about when, and midnight is a
  guess at it rather than a convention for it. Cheap to change, one attribute
  each, if hand-written entries turn out to want it.
- **A splash window while a large wiki reconciles.** `server::start` returns
  once the index is in step with the files, and the window is only built after
  that, so on a big wiki the gap between double-click and anything appearing is
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
  before the bind. What is there covers the case that actually happens:
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
  unmeasured: the desktop app's own first launch, which puts a window and a
  webview around the same scan, and how long a single page save takes on a wiki
  that size.

  Whatever runs it needs to handle a noisy machine. The first re-measurement
  after the fix reported 5,000 pages as *slower* than before and 10,000 as
  faster than 5,000, which is impossible for work that grows with the wiki; it
  was background indexing of the 36,000 files the test had just created.
  Repeating each size three times and taking the minimum gave a clean linear
  result. A single timing on this machine is not evidence.
- **`cargo test --features embed-assets`.** Six tests only compile under that
  feature: the ones covering the dashboard served out of the binary. A plain
  `cargo test` skips them silently, and a feature nothing exercises is a feature
  that breaks without telling anyone. It needs `pnpm build` to have run.
- **The settings form, clicked.** `desktop/src/settings_window.rs` unit-tests
  everything on the Rust side of the webview: what the form parses to, what the
  page renders, that `RHIZOLOG_ADDR` disables it, that a request from any other
  window is refused. What none of them touch is whether WebView2 hands a form
  post on a custom scheme to the handler, the half where being wrong is a
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
  origin is the *window*: the webview, the `target="_blank"` handler that
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
  context is what would change this, and accounts make that more likely than it
  was, so this is closer than the rest of this list.
- **Auto-update.** A portable executable that rewrites itself is a different
  product decision. For now, replacing the file is the update.
- **macOS and Linux bundles.** The architecture is portable; the packaging work
  is not, and development is on Windows.
- **Revisions and history.** A wiki directory is very likely a git repository
  already, which covers history for whoever holds the disk. Accounts make "who
  changed this" a question with more than one answer, so this is closer than it
  was, but git still answers it, and answering it twice is worse. See
  [Architecture](knowledge-base/architecture.md).
