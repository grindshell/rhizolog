# The dashboard

The admin UI in `frontend/`. It is a client of the [API](api-design.md) like any
other — it holds no wiki state of its own and every screen is one or two calls.
Setup and versions live in [Tech stack](tech-stack.md).

## Screens

| Route | What it is |
|---|---|
| `/` | Stats: counts, orphans, wanted pages, tag histogram, API usage |
| `/pages` | Listing, or search results when there is a `?q=` |
| `/pages/*slug` | One page, rendered, with both directions of its links |
| `/new`, `/edit/*slug` | The editor |
| `/tags` | Every tag, linking into the filtered listing |

`/new` accepts `?slug=`, which is how a wanted page offers to be written.

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

## Link panels collapse to one row per page

The graph holds two edges when a page is linked both as `[[a]]` and as
`[a](a.md)`, and `/api/links` is right to report both. Rendering the same page
twice under the same title just reads as a bug, so the panels group by target
and keep the kinds as badges.
