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

- **The GitHub mirror holds the previous Rhizolog, not this one.**
  `github.com/grindshell/rhizolog` exists and is public, but its last push was
  4 June 2026, its head commit (`dc671e6`) is not in this repository's history,
  and its README describes the earlier app: git in the browser, "no server", "No
  SQLite". Every link that leaves the site, apart from the licence, resolves
  there through `site/src/links.ts`: both calls to action in the hero, Docs and
  Source in the nav, four of the five footer entries, and the second button on
  the 404. Docs is the worst of them, because it lands on a README that
  contradicts the page the reader just left. Putting this repository there
  means replacing unrelated history, by force-pushing over it or by starting a
  fresh repository under the name, and that is a decision rather than a step.
  The site should not go up until it is made. The mirror is also where CI will
  build releases, so a download waits on the same decision.
- **`site/dist` has not been deployed, and the domain is serving something in
  between.** DNS is no longer the unknown: `rhizolog.com` and `www.rhizolog.com`
  both resolve through Cloudflare and answer (checked 10 September 2026). What
  answers is half of each version. The rewritten Caddy block is live, but
  `/var/www/rhizolog` still holds the old single-page app, so `/` serves that
  app, its client-side routes 404 on a reload now that the SPA fallback is gone,
  and the 404 is an empty body because the old release has no `404.html`.
  Deploying ends that. The command is
  `./deploy.sh ../../../rhizolog/site/dist rhizolog` from
  `server-configs/static`; it used to say `rhizolog/dist`, which is where the
  old app built to. See the Deployment section of the knowledge base page for
  the three ways the old Caddy block was wrong. `www` serves the same pages
  rather than redirecting to the apex, which the canonical link covers for
  search engines and nothing covers for anyone else.
- **A missing file under `/_astro/` is cached for a year, and Cloudflare keeps
  it.** The Caddy block's `header @immutable` line applies to error responses
  as well as files, so a request for an asset that is not there answers `404`
  with `Cache-Control: public, max-age=31536000, immutable`, and Cloudflare
  stores it like any other response. This is not hypothetical. On 10 September
  2026 a check of `/_astro/Base.DwAu8oM9.css`, this build's stylesheet, came
  back `404` and then `cf-cache-status: HIT`. Content hashing gives unchanged CSS
  the same name, so unless the stylesheet changes before the first deploy,
  visitors served by that edge get the page without it until the cache is
  purged. Purge Cloudflare's cache after deploying, which is worth doing every
  time anyway, and override the header inside `handle_errors`:
  `header Cache-Control "no-store"` should do it, and is untested, so check it
  with `curl -I` against a missing `/_astro/` path after pushing the Caddyfile.
- **No `og:image`, so every shared link renders as a text-only card.**
  `site/src/layouts/Base.astro` emits the title, description and canonical URL
  and declares `twitter:card: summary`, and there is no image for either to
  point at. One static PNG of the mark on the dark ground, and
  `summary_large_image` with it.
- **No `robots.txt` of the site's own.** `site/public/` holds the favicon and
  nothing else. The domain answers `/robots.txt` anyway, because Cloudflare
  serves a managed one at the edge, and it disallows ClaudeBot, GPTBot,
  Google-Extended, CCBot and several more. For a product whose pitch is an API
  meant for agents, whether their crawlers may read the page selling it is worth
  deciding on purpose rather than inheriting; it is a Cloudflare setting, not
  anything in this repository. Check what Cloudflare does with an origin
  `robots.txt` before adding one. The `Sitemap:` line in it is the reason to add
  a sitemap at the same time. A sitemap alone is marginal at two pages and stops
  being marginal with the docs.
- **Both figures on the landing page are hand-authored placeholders**, their
  components say so at the top, and since 10 September 2026 their captions say
  so too. Before that, both captions described `example-wiki` as though the
  drawings were of it, and the graph's had fallen behind the fixture: it still
  said nine pages after the book took the wiki to seventeen. Three smaller
  things on the page are copies rather than captures: the two API responses in
  the agents section and the log under the hero were taken by hand from a server
  run against `example-wiki` on that date. They were true the day they were
  taken and nothing keeps them true, so the capture should take all five.

  The figures are meant to be fixtures captured at build time from a real
  server run against `example-wiki/`, for the same reason the OpenAPI document
  is generated from the routes rather than written: a figure that has quietly
  stopped being true is worse than no figure. The graph's node positions come
  from a hash of the slug, so the picture has to be the one `/api/graph`
  produces. The heat map's capture needs `?at=2026-08-06T18:00:00Z&offset=0`,
  because the committed entries are pinned to 30 July to 6 August 2026 and a
  request without it returns an empty week and renders blank.
- **The landing page does not know Idea Inbox exists.** It sells files on disk,
  the API and the link graph, all of which predate it. Capture, explainable
  recurrence and promotion are the part of this product that nothing else does,
  and the page that argues for it says nothing about them. Needs its own section
  and probably its own figure, which is a writing job rather than a code one.
- **`/docs` does not exist.** The nav's Docs entry points at the mirror's
  README, which would be the documentation until there is something here if the
  mirror held this repository. Today it holds the previous app's; see the first
  item in this section.
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
and the assembled view had nothing to be shown against. `book` is eight pages in
two parts, and its manifest is eleven sections: seven assembled, one gap, one page
listed under both parts, one cut scene and one entry that is not a slug.
`index.md` states the whole manifest section by section, along with the compiled
total, the target and what `prose/v1` says about the assembled book, which is
where the `names` rule finds the one thing it is for and no single page reports
it. It cost what was predicted: sixteen pages rather than nine, eight more lines
in the word log, and a rebased `index` baseline. Drafting added the eighth book
page and a ninth log line. The orphan and wanted counts are unmoved throughout,
because the book is linked from the index and a contents gap is not a wikilink.

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

  **Pacing doubles it** on a page that also names a `target` or a `due`, since
  `GET /api/pace` walks the same tree to know what the document carries. Paid
  deliberately rather than avoided: the alternatives were trusting a client's
  arithmetic or hiding the one glanceable figure behind a click. A page with only
  a `contents:` list is unaffected, because the strip asks for nothing there.

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

**A contents gap is a wanted page now, which it was not until the fixture said
so.** Adding a book turned up an asymmetry: the orphan query unioned
`page_parts` the moment manuscripts existed, and the wanted count never did, so
an outlined and unwritten chapter left `/api/stats` reporting a wiki that wanted
nothing while the graph beside it drew the gap as a wanted node. Orphans and
wanted pages are meant to be one phenomenon read from either end, so the union
belongs on both sides: `named_but_unwritten()` sits next to `referenced()` and is
the same rule turned around. `links.wanted` stays link-only, inside the link
totals where it belongs. The rule about which contents entries name a page at all
became `page_parts.is_slug` at schema version 13, decided once by `Slug::parse`
at index time, because a wanted page is named on the dashboard as somewhere to
write and a query that forgot to filter would have put `../etc/passwd` there.

One decision is still open and it is cheap: whether the word log ever wants
pruning. The other, whether `target` on a leaf page earns the recursive
definition, is answered in [Drafting](knowledge-base\drafting.md): it does, and
what it was missing was a reader rather than a different rule.
Compile's limits are no longer among them: depth 16, 2,000 sections, 8 MiB, each
a refusal rather than a truncation, because a manuscript that quietly stops being
the book is the worst thing this endpoint could return.

## Drafting

**Built**, D0 to D3, and [Drafting](knowledge-base\drafting.md) is now the record
rather than the plan. Long-form got a manuscript as far as existing; nothing in it
said what a chapter was *for* or whether it was done, so a book of forty pages
answered those two questions only by being read.

Three optional frontmatter fields, and the fourth was already there: `synopsis`
(authored, never inferred from the prose), `stage` (a lenient string, four known
names that get a colour and anything else shown as itself), `compile: false` for a
page that stays in the spine and out of the book, and `target`, whose recursive
definition turned out to need a reader rather than a change. `pages.synopsis` and
`pages.stage` are columns at schema version 14; `compile` gets none, because
nothing queries it. The manifest gains all four plus `subtree`, which is what a
target compares against on a page with children.

Two things it settles by renaming or refusing. `status` loses to `stage`, because
`SectionView.status` already means what compile did with an entry and the manifest
is exactly where both would meet. And nothing computes a stage or rolls one up: a
chapter is drafted when its author says so, and the panel's summary is a count
rather than a verdict.

What it turned up on the way: `page_parts` and `page_words` were in
`CREATE_DERIVED` and not in `DROP_DERIVED`, so the next schema bump would have
failed on `create table` and the index would not have opened at all. Two tests
guard it now, one comparing the two lists and one opening a database stamped with
an older version.

Three gaps were found in the survey this plan came out of. **All three are
built** and each has a page and a section below:
[Pacing](knowledge-base/pacing.md),
[Reordering the spine](knowledge-base/reordering.md) and
[Splitting and merging](knowledge-base/split-and-merge.md).

**So "what is left for drafting" is spread over four sections**, this one and
those three, and it is worth saying once where the rest of the answer is rather
than leaving somebody to find out by reading all of them. Two things cut across
the family and are not repeated in each: the read-modify-write window, which is
one entry under Rough edges and belongs to the editor as much as to these; and
undo, which is answered by the wiki directory being a git repository, under
Decided against. What drafting **refuses** rather than defers is not here at all,
because it is argued rather than outstanding: no computed or rolled-up stage, no
synopsis derived from the prose, no custom metadata fields, no colour as data,
and no reordering from the card view. Those live under Not goals in
[Drafting](knowledge-base/drafting.md) and
[Reordering the spine](knowledge-base/reordering.md).

Three questions the plan left open and the build did not close:

- **A synopsis is not searchable.** It is the natural way to find "the chapter
  where they cross", and `pages_fts` indexes slug, title and body. A fourth
  column moves `FTS_BODY_COLUMN`, which `snippet()` indexes by position, so it is
  a real change rather than a line. Deferred, not declined.
- **There is no wiki-wide stage summary.** `/api/stats` counts orphans, wanted
  pages and tags. Stages across a whole wiki is a different question from stages
  across one manuscript, and it is not obvious anybody is asking it.
- **Promotion from Idea Inbox sets no stage.** `todo` would be defensible and so
  would nothing, and nothing is the smaller claim, so nothing is what it does.

## Pacing

**Built**, `pace/v1`, and recorded in [Pacing](knowledge-base/pacing.md). Words
remaining over days remaining, against what the last fortnight actually came to,
from `target`, `due`, the compiled total and the word log. No new fields: every
input was already on disk and drafting had already given the manifest the two
counts the arithmetic needed.

`GET /api/pace?root=` is its own endpoint rather than a flag on `/api/compile`,
because half of it is read off the word log and is therefore refused to a caller
with no account even under `RHIZOLOG_ANONYMOUS_READ`, which a compile is not. It
is arithmetic and not encouragement: two rates in the same unit, every figure
beside the values it was divided from, and no verdict anywhere.

The one figure worth knowing about before reading the page is `uncounted`: words
written in the same fortnight on pages the document does not carry. The rate has
to be in the same currency as the remainder, so a cut scene is in neither, and
saying so is what stops "excluded words do not count" being read as a claim about
the chart, where they very much do.

What is not done:

- **Nothing paces a whole wiki.** One manuscript at a time, so somebody writing
  two books asks twice. `/api/word-stats` answers the wiki-wide half already.
- **The dashboard always asks about now.** `?at=` is on the endpoint, so the
  fixture's pinned figures are reproducible over HTTP and not in a browser. The
  hours heat map and the words chart have the same shape, so this is the
  dashboard's rather than pacing's.
- **A rate over a fortnight says nothing about which fortnight.** One enormous
  day and fourteen steady ones give the same number, and `active_days` is all
  that separates them.

## Reordering the spine

**Built**, and recorded in [Reordering the spine](knowledge-base/reordering.md).
The Manuscript panel gains a Reorder view where each row moves within the
contents list that names it, which is a `PATCH` of that list and no new endpoint.
`GET /api/compile` gained the two fields that made it possible: `parent` and
`ordinal` per section, saying which list named the entry and where in it.

A row moves by being dragged by its grip onto another or by pressing one of its
two buttons, and both are the same write, because the move takes a **position**
rather than a direction. The buttons came first and the drag was laid over them,
which is the order rather than the delay: a drag has no keyboard, so it can only
ever be the second way in.

The drag is **pointer events rather than HTML5 drag and drop**, which is what
makes it a gesture on a phone: a native drag is a mouse gesture with no touch
equivalent. `frontend/src/components/dragging.ts` is what that costs, since
everything the browser did for free is done there by hand: a threshold, a hit test
by vertical position, scrolling near the edges of the window, Escape and
`pointercancel`. It buys back two things. The grip is the only thing that takes
the pointer, so the row is still scrollable on a phone and the move buttons are no
longer inside a drag source. And the gesture can be driven by a test, which a
native drag never could be, since no event a script dispatches can start one.

The one rule worth carrying: **`ordinal` is an identity, not a row number.** A
page reached down both an excluded path and an included one is walked twice, so
its children appear in the manifest twice with the same ordinals. Rebuilding a
contents list by counting rows would double it and write a book with every
chapter in it twice.

The one a drag added: the manifest the panel draws is **flat and recursive**, so
a chapter's own scenes sit between it and the next chapter and are most of what a
dragged row passes over. A drop on one of those reads as both "before the part"
and "into the part", so it is refused, and every row a drop cannot land on dims
while one is in hand.

What is not done:

- **A row that follows the finger.** Nothing moves during a drag: the held row
  dims where it is and a ring says where it would land. That is a real difference
  on a phone, where the finger covers the row it is holding, and it is one
  transform on one element away. Deliberately not there yet, because the ring is
  what actually answers "where will this end up".
- **A long press to drag from anywhere on the row.** The grip is smaller than the
  forty-four pixels a finger wants, and a long press is the usual answer. It needs
  a timer, a way to tell it from a scroll that began slowly, and a decision about
  text selection. The move buttons beside it are the same size.
- **Landing between two rows rather than on one.** An insertion line needs a
  geometry a flat, recursive manifest does not have: as often as not the gap
  between two rows is a gap between two different lists.
- **Moving a chapter between parts.** Two lists change, which is two writes and a
  question about the second failing. The panel says so rather than leaving
  somebody to find out that neither gesture will do it.
- **The list written back is as old as the compile on screen.** One face of the
  read-modify-write window under Rough edges, where the whole of it is. Re-reading
  the book after every move is what makes a collision visible rather than silent.

## Splitting and merging

**Built**, and recorded in
[Splitting and merging](knowledge-base/split-and-merge.md). `POST /api/split`
cuts a page in two at a byte offset and puts the second half into every
`contents:` list that named the first, immediately after it. `POST /api/merge`
folds one page into another and takes it out of every list that named it. Both
answer with which lists were rewritten and what each one says now.

The word log gained two kinds, `split` and `merged`, and that is the part that
would have been a defect rather than a gap: recorded the ordinary way, a split
would report a chapter losing two thousand words and another gaining them on a
day nobody wrote a sentence, which is the signed net this whole feature exists to
refuse. Both markers are zero and zero and carry the total, which is what stops
the next scan reporting the same wrong number as a `net`.

Neither endpoint will touch a page that assembles others. A merge moves text to
where the caller said; a split has to **derive** where the second half goes, and
on a page with chapters under it that position is after every one of them.

What is not done:

- **Splitting from the Manuscript panel.** There is no cursor there, and a
  control that cut at the first heading would be a guess at where the seam is.
- **Merging a chapter into its neighbour in one gesture.** The panel knows which
  entry precedes which and the editor does not, so it is the one thing a panel
  control would add. It needs a spine and a page on screen at once, which is a
  layout question rather than an API one.
- **The lists repaired are as old as the read that found them**, which is the
  same face of the same window, under Rough edges.
- **A repair that cannot write a parent answers `500` after the pages are
  written.** The pages and the word log are right and one contents list is not,
  which the manifest shows, because the spine is read from there anyway. A review
  moved the log markers ahead of the repair so that this costs a list rather than
  a page's whole history; what is left would need a partial-success shape, which
  is a protocol for a failure that needs an unwritable file to reach.

## Rough edges

- **Every write in the dashboard is read-modify-write, and the window is
  open.** Saving in the editor replaces a page with what was on screen when it
  opened. Reordering rebuilds a `contents:` list from the compile the panel is
  showing, so a chapter added in another tab since then is written out of the
  spine. Splitting and merging repair the lists they found a moment earlier. It
  is one problem with three faces, which is why it is here once rather than in
  each of those sections: what closes it is conditional writes, and this API has
  none anywhere. Growing them in one corner would be worse than the window,
  because it would make the other two look deliberate.

  What each of them does instead is make a collision visible rather than silent.
  The panel re-reads the book after every move; a split answers with every list
  it rewrote and what each says now. The editor, which has the largest surface of
  the three, does the least about it.
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
- **A pull while the server is running can still count an edit twice.** The
  watcher ignores `.rhizolog/words/`, and has to, since the server appends to it
  on every save; so the log is only read back in by a scan. A `git pull` that
  brings in a page and the log line describing its edit, while a server is
  watching, weighs the page against the log as that server last read it, and
  records the edit again. A pull large enough to arrive as a directory event goes
  through the scan and is fine, and so is pulling with the server stopped, which
  is what the fix for the startup case covers. Closing it means the watcher
  telling the server's own appends from somebody else's, or re-reading a slug's
  tail of the log before weighing an external edit. See
  [Long-form writing](knowledge-base/long-form.md), "A stale index is not a
  previous body".

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
