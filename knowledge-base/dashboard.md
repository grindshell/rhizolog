# The dashboard

The admin UI in `frontend/`. It is a client of the [API](api-design.md) like any
other — it holds no wiki state of its own and every screen is one or two calls.
Setup and versions live in [Tech stack](tech-stack.md).

## Screens

| Route | What it is |
|---|---|
| `/` | Stats: counts, orphans, wanted pages, tag histogram, API usage |
| `/pages` | Listing, or search results when there is a `?q=`; narrowed by `?tag=`, `?prefix=`, `?segment=` |
| `/pages/*slug` | One page, rendered, with both directions of its links |
| `/new`, `/edit/*slug` | The editor |
| `/tags` | Every tag, linking into the filtered listing |

`/new` accepts `?slug=`, which is how a wanted page offers to be written.

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
  and `rel="noopener noreferrer"` — including after its content is replaced.
- **The derived-title fix**, both halves: the field is left empty when the title
  is derived, and an empty field saves as `null` rather than `""`.
- **Slug encoding**, round-tripped over every shape the backend allows.
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
