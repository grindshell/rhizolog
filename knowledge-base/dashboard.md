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

`/new` accepts `?slug=`, which is how a wanted page offers to be written.

There is deliberately no `/times/:id`. An id is a machine's handle — unlike a
slug it is not something anyone would link to — so an entry is read and edited
in the log itself.

The app shell carries two dropdowns that are not navigation. **Pins** is
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
