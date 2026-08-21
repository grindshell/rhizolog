# The dashboard

The admin UI in `frontend/`. It is a client of the [API](api-design.md) like any
other — it holds no wiki state of its own and every screen is one or two calls.
Setup and versions live in [Tech stack](tech-stack.md).

## Screens

| Route | What it is |
|---|---|
| `/` | Stats: counts, orphans, wanted pages, tag histogram, API usage, and where the time went |
| `/pages` | Listing, or search results when there is a `?q=`; narrowed by `?tag=`, `?prefix=`, `?segment=` |
| `/pages/*slug` | One page, rendered, with both directions of its links and the time spent on it |
| `/new`, `/edit/*slug` | The editor |
| `/tags` | Every tag, linking into the filtered listing |
| `/graph` | The link graph, drawn; narrowed by `?root=`, `?depth=`, `?prefix=`, `?tag=`, `?wanted=` |
| `/times` | The time log: running timers, entries, groups; narrowed by `?q=`, `?name=`, `?page=` |
| `/inbox` | Idea Inbox: the capture field, the chronological inbox, candidate suggestions and one rediscovery card; narrowed by `?q=` and `?show=` |
| `/ideas` | Idea threads grouped by lifecycle state; narrowed by `?state=` and `?integrity=` |
| `/ideas/:id` | One thread: its receipt, its captures, every decision that can be taken about it, and the way out into the wiki |
| `/accounts` | Accounts, and — on a wiki that has none — the form that creates the first |

`/new` accepts `?slug=`, which is how a wanted page offers to be written.

`/inbox` accepts `?capture=1`, which focuses the text field. It is what the
Create menu links to, and it is a parameter rather than the default because
arriving at the inbox to read it should not put a keyboard over half a phone
screen.

There is deliberately no `/login`. See below.

There is deliberately no `/times/:id`. An id is a machine's handle — unlike a
slug it is not something anyone would link to — so an entry is read and edited
in the log itself.

`/ideas/:id` is a plain path parameter rather than a splat, because an idea id
contains no slashes. There is no `/captures/:id` beside it for the `/times/:id`
reason and one more: a capture is working material rather than a document, and
the deliberate way to make one into something you would send somebody is to
promote the idea holding it into a page.

## Promoting is three requests, and the button says which one is left

The promotion panel on `/ideas/:id` is the API's three steps with a form around
them: read the draft, create an ordinary page, record what the idea became. It
does not hide that it is three, because the interesting case is the one where the
second succeeds and the third does not, and a UI that presented the whole thing
as one action would have nothing useful to say when that happened.

So it keeps one piece of state: the slug of a page it knows exists. Until then
the button reads **Create the page and record it**; afterwards it reads **Record
the page**, and pressing it does the association alone. That covers the failure
and one more case for free: a `409 page_already_exists` sets the same flag, so
somebody who wrote the page by hand first gets a form offering to record it
rather than a refusal to work around. Recording is idempotent at the server, so
pressing the button twice is safe; creating is not, so it is never repeated.

A `409` handled that way is a step and not a failure, so it is not shown as one.
An error panel above a line explaining what to do next is two contradictory
answers to the same press. What the line has to add in that case is whose
writing the page is: this form did not save the box above into it, and it says
so, because the alternative is a screen that looks like the draft went
somewhere.

The draft is fetched only when the panel is opened, with `open` as the resource's
source. It is assembled from every capture in the thread and most visits to this
screen are about reading the receipt.

Three fields, and no more: slug, title, and the markdown. The slug is suggested
from the idea's name and everything is editable, because the draft is a starting
point rather than an output. The title is the one field that starts empty, and
that is not an oversight: the markdown opens with the idea's name as a heading,
and a page whose frontmatter has no `title` takes its title from the heading.
Suggesting one would write the same words down twice and let the copies drift
apart the first time somebody edits the heading, which is the reason the editor
leaves its own title field empty too. Tags and visibility are deliberately
absent: the page is an ordinary page from the moment it exists, the editor
already has both controls, and a second set here would be a second place for
them to disagree. On a wiki with accounts the form says what it is about to do,
because a capture is private working material and a page is not.

The title field's hint is `aria-describedby` rather than part of its label. A
sentence folded into a label becomes the accessible name, and a name that reads
out a whole sentence is worse than one that reads out "Page title"; the visible
"Title" is inside that name, which is what somebody using both eyes and a screen
reader needs it to be.

## A screen that has an answer keeps showing it

`Async` is the loading / error / data switch every route reads a resource
through, and it deliberately does **not** blank to a spinner on a refetch. Every
screen that can change something re-reads after a decision, and a spinner in
place of the idea you were reading, four times in a row as you retire and reopen
it, makes the page look like it is falling over rather than working.

It renders the last settled value with a quiet "Refreshing..." line above it, and
that value is read through the same guard the timer store uses: `resource.latest`
*rethrows* when the fetch failed, so reading it unguarded would throw out of the
route rather than render the error. A failed refresh shows the failure rather
than the value it used to have, because stale data with nothing saying so reads
as though the thing you just did worked.

`Async.test.tsx` covers all four states, and the one that matters is
"keeps the answer it has while fetching the next one", which fails against the
older component.

## Tabs say which one they are

Both tab strips, the inbox's Inbox / Archived / Everything and the time section's
day / week / month / year, carry `aria-selected` and point at a panel with
`aria-controls`. `tab-active` is a class, which is to say it is for eyes: without
`aria-selected` a screen reader is told these are tabs and never told which one it
is looking at. ARIA requires the attribute on `role="tab"` and both went without
it until the Idea Inbox work put a second one on screen.

## The top bar renders its navigation twice

`Layout.tsx` holds one `DESTINATIONS` array and draws it in two places: a
horizontal menu that appears at `lg` and above, and a dropdown that appears below
it. Two renderings of one list rather than two lists, because two lists drift and
the one that drifts is always the one only phones see.

Creating is one control containing Capture and New page, in that order. Capture
is the primary action on a phone and costs one text field; a page costs a slug, a
title and a decision about where it goes. The standalone New button this replaced
was fine on a laptop and was competing for space with timers, pins and an account
everywhere else.

Timers, pins, create and the account never collapse into the menu at any width.
A pin left in place is harmless and a timer left running overnight is not, and an
account menu you cannot reach is a session you cannot end. What gives instead, in
this order, is the two idle labels (glyphs below `sm`), the account's display
name (truncated harder), and finally the wordmark, which is the only thing in the
bar that is decoration. At 375 pixels with an account signed in the row comes to
exactly the viewport width with the wordmark still whole.

### Two CSS rules cost this project three overflowing screens

Checking every route at 375 found three, all pre-dating the work that went
looking, and between them two causes worth knowing before writing another grid:

- **A grid item's default `min-width` is `auto`**, so it refuses to be narrower
  than its content. An `<input>` carries an intrinsic width, and an
  `overflow-x-auto` child never gets to scroll because the item grows instead. It
  is why the account form and the time heat map each set the width of the page
  they were on, and why `min-w-0` on the item is the fix in both.
- **daisyUI's `.label` and `.stat-desc` are `white-space: nowrap`**, which
  silently defeats `break-all` beside it. A long Windows path in the dashboard's
  Wiki root stat and a two-line hint under the username field were each one
  unbreakable line. `whitespace-normal` is the fix, and it has to be said
  explicitly because the component library said the opposite first.

## Signing in is not a route

`SessionGate` wraps the router rather than living inside it, and swaps the whole
app for a sign-in form when the session says to. There is no `/login` and
nothing redirects.

The reason is that in this app **an address is a page**. A link to
`/pages/notes/rust/async` sent to somebody who is not signed in should still
open that page once they are, and a redirect to `/login` throws that away — it
has to be stashed somewhere and put back, which is a small amount of state that
is wrong exactly when it matters. Rendering the form in place leaves the URL
untouched, so signing in re-renders the router at the address the browser is
already on and there is nothing to restore.

It costs one thing: the gate cannot use anything router-shaped, since it sits
outside the `Router`. That is fine — its whole body is one three-way switch, and
the three cases are worth naming:

- **Still asking.** A spinner, and nothing else. Guessing "open" flashes the
  dashboard at somebody who is not signed in; guessing "signed out" flashes a
  login page at the single user of an open wiki. Both last one round trip and
  both look like a bug.
- **Unreachable.** The error, not the form. A server that is not answering is
  not a sign-in problem, and a login nobody can complete is a worse answer than
  saying what is wrong.
- **Answered.** The form, or the app.

On a wiki with no accounts only the last case is ever reached and the login page
is never built. See [Accounts](accounts.md).

### A session can end without this tab doing anything

It expires, an owner deletes the account, or a password change elsewhere ends
every session it had. The first sign of any of those is a `401` on an ordinary
request, so `client.ts` calls a handler on exactly that — `unauthorized`, and
not `forbidden`, which says the session is fine and the account is not allowed,
and not `invalid_credentials`, which is a failed sign-in the form should keep
its message about.

The handler is registered once at module load rather than per component. One
that came and went with a mounted route would miss the requests made while
navigating, which is most of them.

The app shell carries three dropdowns that are not navigation. **Accounts**
renders nothing at all on a wiki with no accounts — that is the point rather
than an edge case, since the single-user local dashboard should not grow a menu
telling it that it is signed in as nobody. **Pins** is
described in [Pins](pins.md). **Timers** sits before it and shows the
longest-running timer's clock rather than a count, because a count tells you
something is running and a clock tells you whether it should be; a pin left in
place is harmless and a timer left running overnight is not. Both follow the
same pattern — one shared store rather than a resource per component, so
starting a timer from a page is visible in the top bar immediately. See
[Time tracking](time-tracking.md).

## Every part of a slug is a place you can go

A slug is never printed as flat text where it could be printed as links. Both
readings of it are offered, and they are deliberately two controls rather than
one that guesses:

- **The breadcrumb** on a page is one `<li>` per segment, each leading to
  `?prefix=` — everything at or under that path. Hierarchical: following `rust`
  in `notes/rust/async` stays inside `notes`.
- **A badge row** beside the page's tags offers `/notes` and `/rust`, each
  leading to `?segment=` — every directory of that name in the wiki. Flat, and
  sitting next to the tags because it is the same kind of filter.

The badges are mono and slash-prefixed so the two kinds do not read as one
list, and both carry a `title` saying which is which — they look alike and do
different things, so the distinction has to be legible without a click.

The last segment of a slug names the page rather than a directory holding it,
so it is text everywhere. Wherever a slug appears the page's own title is
already a link right beside it, and a second link that looked the same but led
to a filtered listing would only mislead.

Everything above lives in `slugSegments` in the API client and the `SlugPath`
component, so the listing, the search results, and the backlink panels all
behave the same way without repeating the rule.

## Editing is not `/pages/*slug/edit`

The same constraint that shaped the backend's routes applies here: a splat has
to be the final segment, so that path cannot be expressed. And a literal
`/pages/edit` would shadow a page actually slugged `edit`. Editing therefore
lives outside the `/pages` namespace, mirroring `/api/move`.

## The editor is a textarea

Deliberately. A markdown editor is a bottomless project, and the expensive part
of one — rendering — is already done by the server, correctly and including
wikilinks. So the MVP is a `textarea` beside a preview pane fed by
`POST /api/render` on a 300 ms debounce.

Two consequences worth knowing:

**The title field is empty when the title is derived**, with the derived title
as its placeholder. Filling it in would store the title and stop it tracking the
body's heading — see `title_derived` in [API design](api-design.md).

**Typing in the slug field does not mark the form dirty.** This looks like an
omission and is not: renaming is disabled while there are unsaved edits, so if
typing a new slug counted as one, the Rename button would disable itself the
moment it appeared and could never be clicked.

Renaming is a `POST /api/move`, which operates on the file rather than on what is
in the textarea — hence the interlock. Inbound links are not rewritten, so a
rename turns them into wanted pages, visible immediately on the dashboard.

## The visibility control is not there until it means something

The editor's "Who can read this" select and its readers field appear only when
the session says the wiki has accounts. On a wiki with no accounts the four rungs
in [Page visibility](visibility.md) are a distinction between nobody and nobody,
and a control that does nothing is worse than no control — it invites somebody to
mark a page `private` and believe it.

The badge on a read page is narrower still: it says nothing for an `internal`
page, which is most of them. A badge on every page is a badge nobody reads, and
the one that matters — `public`, the only rung where a mistake is a disclosure
rather than an inconvenience — is coloured as a warning so it is not the same
grey as the rest.

**Saving is a `PUT`, so the editor sends the owner back whether or not it shows
it.** A field left out of a `PUT` is a field cleared, and the backend fills a
missing owner in from whoever is saving — so an editor that dropped it would hand
every page it touched to the last person who pressed Save, including pages shared
*with* that person by somebody else.

## The editor divides its space three ways

Editor / Split / Preview, chosen from a segmented control in the editor's
header. Three states rather than one collapse toggle because "give the editor
the room" and "give the preview the room" are both things you want, and a single
button that cycled between them would make you guess which way it goes.

The choice is remembered in `localStorage`, since it is a working preference
rather than a property of the page: somebody who collapsed the preview to write
does not want it back on the next page they open. It is the one piece of state
in the dashboard that lives in the browser — contrast [Pins](pins.md), which are
about the wiki and therefore live on the server.

Two things about it are less obvious than they look, and both are covered by
tests:

**A collapsed preview issues no `POST /api/render` at all.** Not one per pause
in typing, and not the one at mount either. The debounce effect returns before
reading `content`, so it depends only on the layout; and the draft signal is
`null` while the pane is hidden, which is what makes Solid skip the fetcher. A
pane nobody is looking at is not free here — this wiki counts its own API usage
and puts the numbers on its own dashboard.

**The resource's source is the draft signal alone**, not
`showPreview() && previewOf()`. Solid settles pure computations before user
effects, so a source that read the layout directly would see it flip to visible
while the draft signal still held the content from *before* the pane was
collapsed, and render that — a visible flash of stale text on every re-open. The
effect owns the transition instead, so re-opening is one write and one render,
with the right content and no debounce (nobody is typing; they clicked).

## The log's search box is one more filter, not a mode

`/times` searches with `?q=`, alongside `?name=` and `?page=` rather than
instead of them — see [Time tracking](time-tracking.md) for why the backend
shapes it that way. On this side that makes the screen simpler than `/pages`,
which swaps between two endpoints depending on whether anything is typed: here
there is one call and one list, and a search just narrows it.

Two details are load-bearing and both are tested:

**An empty box sends no `q` at all.** Not `q=`, which the API reads as a search
for nothing and correctly answers with nothing. A cleared box means no filter.

**Clearing the filter chip empties the box too.** The box is a signal of its
own, debounced into the URL 250 ms later; a chip that only cleared the URL would
have the search written straight back underneath it and would look broken. This
is the kind of thing that reads as obviously fine in the source and is not.

## Only one place sets `innerHTML`

`components/Markdown.tsx`, and only with HTML the **server** rendered. That is
safe for a specific reason rather than by convention: comrak runs with raw HTML
disabled, so markup in a page body is dropped by the renderer instead of passed
through.

Everything else that carries page content is text and is rendered as text:

- **Page source**, shown by the editor and the Source toggle.
- **Search snippets.** This is the one that looks like it wants `innerHTML`,
  because SQLite's `snippet()` wraps matches in `<mark>`. It does **not** escape
  the text around them — that is the page body verbatim, and page bodies are
  what agents write through the API. `components/Snippet.tsx` parses the marks
  out and emits every piece as a text node.

The rule this leaves: HTML from `?render=true` or `/api/render` may be injected;
nothing else may be.

Links inside rendered bodies are rewritten by the server to `/pages/...`, so the
component recognises in-wiki links with a prefix check and navigates them
client-side. Everything else gets `target="_blank"` and `rel="noopener
noreferrer"` — nothing inside a page body should be able to navigate the
dashboard's own tab.

## Tests

Vitest over jsdom, with `@solidjs/testing-library`. `vitest.config.ts` is
deliberately separate from `vite.config.ts`: tests need `resolve.conditions` set
to Solid's **development** build, which is precisely what a production bundle
must not have. Two files means the test setup cannot leak into what `pnpm build`
ships — checked by grepping the bundle for test code.

Two settings there are not obvious. `solid({ hot: false })` disables
solid-refresh, whose transform emits an import of `/@solid-refresh` — a
dev-server virtual module that nothing resolves under the runner, so leaving it
on fails every component file outright. And `src/test-setup.ts` stubs
`window.scrollTo`, which jsdom does not implement and the router calls on every
navigation.

What is covered is the part where the bugs were, not the part that is easy:

- **The rule about injecting HTML**, from both sides. `Snippet` is asserted to
  produce no `script` or `img` element from a hostile page body, and to show the
  markup as text instead. `Markdown` is asserted to run server HTML, send
  in-wiki links through the router, and give everything else `target="_blank"`
  and `rel="noopener noreferrer"` — including after its content is replaced. A
  time entry's snippet goes through the same check, because a note is written
  through the API exactly as a page body is.
- **The log's search wiring**: that an empty box sends no `q`, that emptying a
  full one drops the filter rather than searching for `""`, that a search
  composes with `?page=`, and that clearing the chip leaves the box empty and
  keeps it that way. The last one is the guard described above; it fails
  without the one line that resets the draft.
- **The derived-title fix**, both halves: the field is left empty when the title
  is derived, and an empty field saves as `null` rather than `""`.
- **Slug encoding**, round-tripped over every shape the backend allows.
- **The editor's layout modes**, including the two behaviours above — that a
  collapsed preview stops rendering, and that re-opening renders what is in the
  textarea *now*. The second test is what found the stale-draft flash.
- **The pin store's writes**, which update the list locally rather than
  refetching: that a new pin appends, that re-pinning keeps its position, and
  that a refused pin reaches the caller instead of silently doing nothing.
- **The timer store**, for the same reasons and one more: that a backend which
  is merely down leaves the list empty rather than throwing out of the navbar.
  The guard on `resource.latest` is what makes that true and it is invisible in
  the source.
- **The graph layout's determinism**, from both ends: the same graph twice, and
  the same graph with its nodes in reverse order, must produce identical
  coordinates. Also the three cases that produce `NaN` if the forces are written
  naively — nodes seeded on the same point, an edge naming a node nobody drew,
  and a page that links to itself — and that an orphan stays in frame, which is
  the whole job of the gravity term.
- **What the graph draws**, since none of it is legible from the markup: a
  wanted page is a node rather than an absence, the root of a walk is ringed,
  the `viewBox` is four finite numbers, and labels are rationed once the graph
  outgrows reading them all.
- **Duration formatting**, which has three spellings on purpose — a list drops
  seconds, a running clock keeps them, an axis label uses hours — and none of
  them may render a negative.
- **The error envelope**, including the transport cases and the one case that
  must *not* become an `ApiError`: an abort, which is a caller who stopped
  caring rather than a failure.
- **That a capture survives a failed save.** The text stays in the field when the
  request fails, clears only once the server has it, and never becomes a request
  at all when it is blank. Beside it, that candidates are asked for only after
  the capture is saved: two requests in that order and never the other way round,
  because analysis is derived and retryable and the capture is not.
- **That a suggestion connects nothing by itself**, from both sides: the shared
  signals and the score render, no connect or create call is made, and accepting
  a loose capture asks for the idea's name first, because Rhizolog never invents
  one.
- **That a failed resource stays inside its own panel.** An analyzer that is
  behind leaves the capture field working; a receipt that cannot be worked out
  leaves the thread readable. Both are one guard on `resource.latest`, and both
  are invisible in the source: reading a Solid resource that failed rethrows, and
  an unguarded read takes the whole route with it.
- **The rediscovery choice**, which is a pure function and tested as one: that it
  takes only dormant threads with more than one capture, that a dismissal holds
  for exactly thirty days, that the same day gives the same card however the
  listing was ordered, and that the date it reads is the reader's local one.
- **That answering the card is the end of it**, with three dormant threads
  waiting and neither answer producing a second one, and that a refused answer
  leaves the card where it was. Both halves fail without the flag, which is the
  point of them: the eligible pool shrinks when you answer, so the next name
  comes up on its own.
- **That deleting a capture says what it cost**, naming the ideas that held it
  and badging the ones that now need repair, and says nothing at all when it held
  nothing up, which is the ordinary case.
- **That the two listings on the inbox re-read separately**, so archiving a
  capture does not go and fetch two hundred ideas to find out whether any of them
  became dormant.
- **That the shell offers every destination twice**, that Capture and New page
  are behind one control with no standalone New beside it, and that timers, pins
  and the account are all still reachable. It is the check that a navigation
  rewritten for a phone did not quietly drop a route on the way.
- **That promoting retries only what is missing.** The page is created, the
  association fails, and the second attempt makes exactly one request: the
  association. A page that already exists takes the same path from the other end,
  from a `409` rather than from a success. Both fail without the one condition
  that skips the create, which is the whole reason promotion is three steps.
- **That an edited draft survives a refused page.** It is the same rule the
  capture field lives by: what somebody typed is the only copy of it, and a form
  that emptied itself on a failure would have thrown the edit away.

Modified clicks are covered too, because intercepting one would break opening a
page in a new tab, and nothing about the code makes that obvious.

## Link panels collapse to one row per page

The graph holds two edges when a page is linked both as `[[a]]` and as
`[a](a.md)`, and `/api/links` is right to report both. Rendering the same page
twice under the same title just reads as a bug, so the panels group by target
and keep the kinds as badges.

The time panel above them is shaped differently on purpose, and it is not a
third link panel — see [Time tracking](time-tracking.md) for why a hundred time
entries have to be one line with a total on it. It is absent entirely when
nothing has been tracked, so a wiki nobody times looks exactly as it did.

## The graph screen draws its own layout, too

For the same reason the charts below do, plus one the charts do not have: the
layout has to be **deterministic**, and no published force layout is. Seeds come
from a hash of the slug rather than `Math.random()`, so the same wiki draws the
same picture every visit and a changed shape means the wiki changed. That is the
whole argument for the screen existing, and it is in
[Drawing the link graph](link-graph.md) along with what the three node
appearances mean, why edges bow, and why labels are rationed.

Selection is a signal rather than a URL parameter, unlike every filter on the
screen. The filters say what is drawn and are worth linking to; which node you
happen to be pointing at is not, and putting it in the URL would push a history
entry on every click.

## The time section draws its own charts

A bar chart of at most thirty-one bars and a 7×24 grid of squares, both plain
divs. A charting library would add a dependency and a second way for the
dashboard to fail, in exchange for nothing this needs.

Two details are load-bearing. An empty bucket still gets two pixels of height,
so it reads as a column rather than a gap in the axis. And the heat map shades
by share of its own busiest cell rather than by an absolute scale, because the
question it answers is *when*, not *how much* — on an absolute scale a light
week is indistinguishable from an empty one.

The clock on a running timer is recomputed locally from its `start` rather than
re-fetched, which is the same arithmetic the server does; only the *set* of
running timers is polled, because a second tab or a hand-edited file can change
it and nothing would otherwise say so.
