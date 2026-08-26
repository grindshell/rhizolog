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
- **The landing page does not know Idea Inbox exists.** It sells files on disk,
  the API and the link graph, all of which predate it. Capture, explainable
  recurrence and promotion are the part of this product that nothing else does,
  and the page that argues for it says nothing about them. Needs its own section
  and probably its own figure, which is a writing job rather than a code one.
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
- **Creating a page says whether a slug is taken, whoever it is taken by.**
  `POST /api/pages` never consults visibility: `409 page_already_exists` for a
  slug holding somebody else's private page, `201` for a free one, which is the
  existence bit that `404, never 403` exists to withhold everywhere else. The
  destination of `POST /api/move` says the same. It is not a missing check: a
  create has to answer, and any refusal is the oracle, so hiding it would mean
  overwriting a page its author cannot see going or claiming to have written one
  that was never written. One bit per guess, no title or content or owner with
  it, an account needed to ask, and a wrong guess leaves a page to clean up.
  The real fix is private pages not sharing one global slug space, which is a
  design change and not one to make before somebody serves a wiki where it
  matters. Reasoned through in
  [Page visibility](knowledge-base/visibility.md).
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

**Built**, phases I0 through I5, and recorded in
[Idea Inbox](knowledge-base/idea-inbox.md): the authored model and store, the
derived index that folds decisions into current state, the owner-scoped HTTP API
over both, the explainable half (`tfidf/v1` candidates and `idea-momentum/v1`
lifecycle receipts), the dashboard over the lot, and promotion into an ordinary
page. **The whole loop can be walked in a browser, on a phone**: capture,
connect, see why, reject a wrong suggestion, retire, reopen, rediscover and
promote.

What is not done, in the order it is likely to matter:

- **The draft takes no capture selection.** Every capture the idea holds is in
  it, and choosing a subset is an edit in the form. The plan's endpoint table
  implies a selector; the reasoning for not building one is on the Idea Inbox
  page, and it is the kind of thing use will settle.
- **The ideas screen reads the whole listing and groups it in the browser.** It
  asks for 200, which is the API's ceiling, and there is no paging control. The
  states are grouped on that screen, and a page boundary in the middle of Dormant
  would be a grouping that lied, so the choice was the whole list or a redesign.
  Somebody who has named two hundred threads has earned the redesign.
- **An idea's decision events are not listed anywhere.** The receipt shows the
  affirmations and reopenings, because those are what the rules counted, and
  nothing shows connects, rejections or archivings as a history. There is no
  endpoint for it either. The events are all on disk and folded; what is missing
  is a reason to read them back that is better than `git log`.
- **A capture has no address of its own.** No `/captures/:id`, so an idea's
  receipt links to the capture's row inside the same page rather than to the
  capture. Working material is not a document, and a link somebody could send
  somebody else is the thing promotion is for.
- **Candidates are scored against the whole corpus every request.** Every vector
  is rebuilt per call and every thread's centroid with it, which is fine for an
  inbox of hundreds and unmeasured beyond that. It is the one item here with a
  number attached and the number is unmet: the plan's gate is 1,000, 5,000 and
  10,000 scratch captures, three runs each, and those figures should exist
  before anybody adds a centroid cache, let alone approximate search.
- **`tfidf/v1` has no stemming and no stop-word list, on purpose.** So `dungeon`
  does not match `dungeons`. That is the explainability trade: every signal
  shown appears literally in text the user wrote. Revisit only after real false
  negatives, and only with a version bump.

Do not add LLM summaries, embeddings, automatic membership, notifications, tasks
or a native mobile app. Those are gated on observed use of what is now built,
not missing pieces of it.

## Long-form writing

**Built**, L0 through L4, and recorded in
[Long-form writing](knowledge-base/long-form.md): compile a tree of pages into
one addressable document, give a page and a manuscript a length and a target, and
check prose against rules the author wrote down. It is aimed at drafting
long-form work alone with an assistant, which is what makes compile a context
loader before it is an export and what makes a net word count worth splitting by
who wrote it.

Every phase and what it turned out to be:

- **L0: words.** **Built.** `markdown::count_words`, `pages.words` at schema
  version 10, `target`, `due` and `contents` in frontmatter, `?sort=words`, and
  a `words` total on the listing summed over the whole filtered set. What the
  plan did not say and the code had to decide is on the plan page under "What L0
  turned out to be".
- **L1: compile and the manifest.** **Built.** `page_parts` at schema version 11,
  `GET /api/compile` in three formats, the heading shift, the five section
  statuses, and the three limits. See "What L1 turned out to be" on the plan
  page. The graph draws part edges as of L4, marked `part` rather than given a
  sixth link `kind`, and a walk crosses them.
- **L2: `prose/v1`.** **Built.** Rules in `.rhizolog/prose.toml`, five rule
  kinds, every finding quoting the text it fired on and carrying the arithmetic
  behind it, and `GET /api/prose/rules` so a remote caller can reproduce one. No
  dismissal store, on purpose. Spans are byte offsets into the page source, which
  works because nothing is extracted: `markdown::extract` keeps the body byte for
  byte and blanks what is not prose. `?compiled=true` runs the rules over the
  whole assembled manuscript, which is the only way the two cross-page rules see
  anything. See "What L2 turned out to be" on the plan page, and in particular
  the two filters `consistent` needed before it stopped reporting `If` as a
  misspelling of `It`.
- **L3: actor and the word log.** **Built.** An `X-Rhizolog-Actor` header, and
  `.rhizolog/words/`: a fourth authored tree, one file a month and one line an
  observation, with `page_words` derived from it at schema version 12. Every
  observation carries words **added and removed** rather than their difference,
  diffed against the previous body out of `pages_fts`. `GET /api/word-stats`
  answers the series by day, by tool and by page, and refuses a caller who has
  not signed in even under `RHIZOLOG_ANONYMOUS_READ`. Deleting `index.db` and
  restarting reproduces the whole series and adds nothing to the log, which is
  the property that made this a file rather than a table. The four places that
  enumerate the authored trees were updated here rather than in L4, because the
  tree exists now. See "What L3 turned out to be" on the plan page, and in
  particular the `*.log` line in `.gitignore` that had been quietly ignoring it.
- **L4: dashboard and documentation closure.** **Built.** The Manuscript panel on
  a page that carries `contents`, `target` or `due`; `?assembled=1` for the
  compiled document; a findings strip under the editor's textarea that issues no
  request while it is closed and selects a finding in the text when clicked; and
  a words chart beside the hours, drawing added above the line and removed below
  it. Part edges are drawn. The word log is in the sync report, in a shape of its
  own rather than pretending to five fields that would be zero. `example-wiki/`
  gained the starter `prose.toml` and a committed word log. See "What L4 turned
  out to be" on the plan page.

**Two things L4 found rather than built**, both worth knowing about:

- **The editor had been clearing `contents`, `target` and `due` on every save**
  since L0. Saving is a `PUT` and a field left out is a field cleared; the plan
  says so and the editor did not do it. Any page opened in the dashboard and
  saved lost its manuscript fields, and nothing could catch it before the panel
  that would have shown the damage existed. It is the same failure that once
  handed pages to the wrong owner, with a different field.
- **L3 had made the example wiki dirty itself.** Merely starting a server against
  the fixture appended nine baseline lines to `example-wiki/.rhizolog/words/`,
  while `AGENTS.md` still said that pointing `RHIZOLOG_ROOT` there to look was
  fine. Eighteen committed lines fix it as a property rather than a warning: every
  page already agrees with the log, so the scan finds nothing to record.

**`example-wiki/` now has a manuscript in it**, which is what the Manuscript panel
and the assembled view had nothing to be shown against. `book` is seven pages in
two parts, and its manifest is ten sections: seven assembled, one gap, one page
listed under both parts and one entry that is not a slug. `index.md` states the
whole manifest section by section, along with the compiled total, the target and
what `prose/v1` says about the assembled book, which is where the `names` rule
finds the one thing it is for and no single page reports it. It cost what was
predicted: sixteen pages rather than nine, eight more lines in the word log, and
a rebased `index` baseline. The orphan and wanted counts are unmoved, because the
book is linked from the index and a contents gap is not a wikilink.

What is not done:

- **Compile has no performance evidence.** It is the one thing here whose cost
  grows with the work, and the plan's gate is scratch manuscripts of 50, 200 and
  500 sections, measured three times each with the minimum kept. Nothing has been
  measured. The note under "Verification gaps" about a noisy machine applies: a
  single timing on this machine is not evidence.
- **The Manuscript panel compiles the whole book on every page view** of a page
  that has one. That is one walk, the same one `?assembled=1` does, and it is
  fine at the sizes anybody has written here. It is the first thing to look at if
  a large manuscript makes its own contents page slow to open.

**The recursion rule is settled.** A page contributes its body, then each page in
its `contents:` list, in order, recursively; a link in prose is never structure,
anywhere. Six alternatives and why each lost are on the plan page. What it left as
work rather than as an objection, all of it now done: `contents:` entries are
indexed so a chapter is not an orphan, the Manuscript panel is the only rendering
of the spine, and the list is read as strings and validated at compile time so a
mistyped chapter does not make the whole page malformed and drop it out of every
listing.

**A review of the plan found five things worth fixing and they are fixed on the
page**, three of them contradictions with decisions this repository had already
made. The word history recorded a signed net change, which is precisely the
metric the feature's own motivating example rejects; it now records words added
and removed, diffed against the previous body, which `pages_fts` already holds.
It lived in the durable half of `index.db`, which survives a schema bump but not
`rm index.db`, and every document here promises that deleting the database costs
one scan: it is an authored log under `.rhizolog/words/` now, with the derived
table rebuilt from it. It claimed one row per save, which the watcher's 500 ms
debounce makes untrue for anyone editing in their own editor. `prose/v1` had no
way to read its own rules over HTTP, so a remote assistant could be handed
findings it could not reproduce. And a `contents:` entry was going to be a row in
`links`, whose key cannot hold the same child twice under one parent, so it is
its own `page_parts` table.

**A contents gap is not counted as a wanted page, and the graph draws it as one.**
The orphan query unions `page_parts`, so a chapter has a parent; the wanted count
does not, so a `contents:` entry pointing at nothing leaves `/api/stats` reporting
only the wikilink gaps. The graph disagrees with its own card: an unwritten
chapter is a node with `exists: false`, drawn exactly as a wanted page is. Left
alone deliberately, since a hole in a manuscript is already reported in position
by the manifest and merging the two would make one number answer two questions,
but the dashboard says both without saying they differ. `example-wiki/index.md`
explains it; the dashboard does not.

Two decisions are still open and both are cheap: whether `target` on a leaf page
earns the recursive definition, and whether the word log ever wants pruning.
Compile's limits are no longer among them: depth 16, 2,000 sections, 8 MiB, each
a refusal rather than a truncation, because a manuscript that quietly stops being
the book is the worst thing this endpoint could return.

## Rough edges

- **The no-em-dash rule is enforced going forward and was never applied
  backwards.** `be05e64` made it project-wide and cleared the root documents;
  everything written before that still has them. As of the Idea Inbox dashboard
  there are about 949 across 109 files, the worst offenders being
  `knowledge-base/desktop-app.md`, `frontend/src/api/client.ts` and
  `knowledge-base/architecture.md`. It is a prose sweep rather than a change of
  behaviour, which is exactly why it should be one deliberate pass and not a few
  lines smuggled into a feature commit. Find them with
  `Select-String -Pattern ([char]0x2014)`, and do not do it by round-tripping
  files through PowerShell: `AGENTS.md` records what that costs.

  `prose/v1` now counts them properly, which is a better tool than the search
  for this job: pointed at a copy of `knowledge-base/` it reports **353 across
  thirteen pages**, every one of them in prose rather than in a code span, and
  `AGENTS.md`'s single specimen correctly not among them. The wider figure above
  includes source files, which the linter does not read.
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
