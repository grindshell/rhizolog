# Pins

A wiki that branches chaotically still has two or three pages you touch every
day — a scratch pad, a running index, whatever the current project is. Pins are
the shortcut to those: a dropdown in the dashboard's top bar, and
`/api/pins` behind it.

## Pins are server state, not a browser preference

`localStorage` would have been less code, and it is the wrong place. Rhizolog is
[API-first](api-design.md): "which pages does this wiki revolve around" is a
question an agent should be able to ask and answer, and a pin kept in a browser
is invisible to the API, lost when storage is cleared, and absent from a second
tab on a second machine.

Contrast the editor's split/collapse setting, which *is* in `localStorage` — see
[The dashboard](dashboard.md). That one is a property of the person looking at
the screen. A pin is a property of the wiki.

## They live in the durable half of the index

`.rhizolog/index.db` has two halves, described in `index/schema.rs`: derived
tables that are dropped and rebuilt whenever `SCHEMA_VERSION` changes, and
durable ones that a version bump leaves alone. Pins go in the durable half,
beside the API usage counters, for the same reason those do — **nothing derives
them from the markdown, so a rebuild has nowhere to get them back from**.

That has one consequence worth spelling out. The `pins` table deliberately has
**no `references pages(slug)`**. `pages` is derived; a foreign key from a
durable table into it would either block the rebuild or cascade the pins away
with it. Pins are resolved by joining `pages` at read time instead — the same
trick the [link graph](architecture.md) uses, and it means renaming a page's
title in its frontmatter relabels the menu entry with nothing to reindex.

## A pin can outlive its page, and that is not a bug

Three cases, deliberately handled three ways:

| What happened | What happens to the pin |
|---|---|
| `DELETE /api/pages/{slug}` | Removed with the page |
| `POST /api/move` | Follows the page to its new slug, keeping `pinned_at` |
| The file vanished from disk | Kept, and reported with `exists: false` |

The first two are deliberate acts on that page through the API, so the shortcut
to it should track them. The third is not: a file that disappeared is
indistinguishable from a move that has not finished yet, and silently forgetting
a pin because a file was briefly absent is the worse failure. Such a pin comes
back badged `missing` in the menu, which is also the only thing there is to
click to get rid of it.

**A pin follows a move; an inbound link does not.** That looks inconsistent and
is not. A link is something another page *said*, and rewriting it would be
editing that page's content on its behalf — so a move turns inbound links into
wanted pages, which is [the point](architecture.md). A pin is a bookmark, and a
bookmark that broke because you renamed its target is just broken.

## The limit exists so the menu stays a menu

`MAX_PINS` is 50. Past some length a dropdown is slower to use than the search
box it was meant to save you, and the cap is what stops a script turning the
shortcut menu into a second listing. It is generous enough that nobody reaches
it by hand.

Two details follow from it. Pinning something already pinned is **not** counted
against the limit, because it adds nothing — otherwise a re-pin at the cap
would fail for no reason a caller could act on. And `GET /api/pins` returns
`limit` alongside the list, so a client can say *why* a pin will be refused
before it tries.

## Endpoint shapes

| Method | Path | Notes |
|---|---|---|
| `GET` | `/api/pins` | `{pins, limit}`, oldest pin first |
| `PUT` | `/api/pins/{slug}` | Idempotent; `404` if no page, `409` at the limit |
| `DELETE` | `/api/pins/{slug}` | `404` (`pin_not_found`) if it was not pinned |

`PUT` rather than `POST`: pinning is idempotent, so a client never has to check
whether something is pinned before pinning it. Pinning twice leaves both the row
and its **position** alone — an entry that jumped to the end of the menu because
it was double-clicked would be a menu you have to re-read before every click.
That is also why the list is oldest-first rather than newest-first.

`pin_not_found` is a distinct code from `page_not_found` on purpose. Unpinning
something that was never pinned usually happens while the page is sitting right
there, and a caller that could not tell the two apart would retry the wrong
thing.

Existence is checked against the **store**, not the index: the store is the
source of truth, and a page written a moment ago is on disk before it is
indexed.

## The trap: operation ids are global

utoipa takes each operation id from its handler's function name, which is only
unique within a Rust module. The pins handlers were first written as
`list`/`create`/`delete` — the natural names inside `api/pins.rs` — and they
collided with `api/pages.rs`'s. The spec still validated and both routes still
worked; only the *generated client* was wrong, because `openapi-typescript`
keys on the operation id and one of each colliding pair silently won.

They are now `list_pins`/`pin_page`/`unpin_page`, and
`operation_ids_are_unique_across_the_document` in `tests/api.rs` fails if it
ever happens again. Nothing else would have caught it.

## In the dashboard

One shared store (`api/pins.ts`) rather than a resource per component, because
two places show the same list and both must agree instantly: the dropdown in the
app shell and the Pin toggle on a page. Writes update the list from what the
server returned rather than refetching — the server is the authority on order
and on `pinned_at`, and it hands both back.

One guard there is load-bearing: Solid's `resource.latest` **rethrows** when the
last fetch failed. Read unguarded from a menu that lives in the app shell, a
backend that is merely down would throw out of the navbar and take every page
with it. The store checks `resource.error` first and reports the failure in the
dropdown's own words.
