# API design

The HTTP API is the primary interface, not a bolt-on to the UI. The admin
dashboard is just its first client. See [Architecture](architecture.md) for the
storage model it sits on.

## Endpoints (MVP)

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/pages` | List pages; `?tag=`, `?prefix=`, `?segment=`, `?q=`, `?limit=`, `?offset=`, `?sort=` |
| `POST` | `/api/pages` | Create; `409` if the slug exists |
| `GET` | `/api/pages/{slug}` | Read; `?render=true` adds rendered HTML |
| `PUT` | `/api/pages/{slug}` | Create or replace |
| `PATCH` | `/api/pages/{slug}` | Partial update of title / tags / content |
| `DELETE` | `/api/pages/{slug}` | Delete |
| `POST` | `/api/move` | Move a page to a new slug |
| `POST` | `/api/render` | Render markdown that has not been saved |
| `GET` | `/api/links/{slug}` | Links in **both** directions |
| `GET` | `/api/graph` | The link graph as nodes and edges; `?root=`, `?depth=`, `?prefix=`, `?tag=`, `?wanted=`, `?limit=` |
| `GET` | `/api/search` | Full-text search with snippets |
| `GET` | `/api/tags` | All tags with page counts |
| `GET` | `/api/stats` | Meta-stats for the dashboard |
| `GET` | `/api/pins` | Pinned pages, oldest first, with the limit |
| `PUT` | `/api/pins/{slug}` | Pin a page; idempotent |
| `DELETE` | `/api/pins/{slug}` | Unpin a page; the page is untouched |
| `GET` | `/api/times` | Time entries, newest first; `?q=`, `?name=`, `?page=`, `?running=`, `?from=`, `?to=` |
| `POST` | `/api/times` | Start a timer, or log time already spent |
| `GET` | `/api/times/{id}` | One entry with its note; `?render=true` adds HTML |
| `PATCH` | `/api/times/{id}` | Partial update; `end: null` sets it running again |
| `DELETE` | `/api/times/{id}` | Delete an entry |
| `POST` | `/api/times/{id}/stop` | Stop a running timer, now |
| `GET` | `/api/time-groups` | Activity names with their totals |
| `GET` | `/api/time-stats` | Day, week, month, year, and an hours heat map |
| `POST` | `/api/reindex` | Force a full rebuild of the index |
| `POST` | `/api/auth/login` | Sign in; returns a session as a cookie **and** a bearer token |
| `POST` | `/api/auth/logout` | End this session; `204` even if there was none |
| `GET` | `/api/auth/session` | Whether this wiki wants a sign-in, and who this request is |
| `GET` | `/api/users` | Every account. Any signed-in account may ask |
| `POST` | `/api/users` | Create an account; unauthenticated **only** for the first one |
| `GET` | `/api/users/{username}` | One account |
| `PATCH` | `/api/users/{username}` | Partial update; a password change ends every session |
| `DELETE` | `/api/users/{username}` | Delete an account and its sessions; refused for the last owner |
| `GET` | `/api/health` | Liveness + index freshness |
| `GET` | `/api-docs/openapi.json` | Generated OpenAPI document |
| `GET` | `/swagger-ui` | Swagger UI |

### `?prefix=` and `?segment=` are two different questions

Both filter on a slug's path, and the difference is the point. For
`notes/rust/async`:

| Filter | Asks | Also returns |
|---|---|---|
| `?prefix=notes/rust` | what is at or under this path | `notes/rust` itself |
| `?segment=rust` | what is in a `rust` directory, anywhere | `code/rust/traits` |

`?prefix=` is hierarchical and stops at the separator, so `notes/rustlings` is
not under `notes/rust`. `?segment=` is flat and behaves like `?tag=` — see
"A slug has two readings" in [Architecture](architecture.md) for why the wiki
wants both. All three filters intersect: naming a tag *and* a path asks for
pages satisfying both.

Neither is validated as a slug. They are filters rather than lookups, so a path
nobody uses is an empty listing and a `200`, not a `404` — there is no such
thing as a missing directory in a wiki whose directories are implied by its
files.

`/api/search` takes none of them. Search answers "where is this word" and the
listing answers "what is in here"; the dashboard picks one endpoint or the
other rather than pretending the filters compose across both.

**The time log does the opposite, deliberately.** `GET /api/times?q=` searches
names and notes as one more filter beside `name`, `page` and the window, and it
does not reorder the results. That is not an inconsistency with the paragraph
above; it is the same reasoning reaching a different answer, because the two
searches are not the same shape. A page search returns hits ranked by relevance
and nothing else composes with that. A log search is asked things like "what did
I write about the poll loop, last week, under `Deep work`" — every part of which
is a filter the listing already has, and none of which a separate endpoint could
answer without growing all of them. Time entries are therefore *not* in
`/api/search`; see [Time tracking](time-tracking.md).

### The time endpoints are the exception to the wildcard rule

`/api/times/{id}/stop` has a static segment after its parameter, which the page
routes cannot have. That is not an inconsistency: `matchit` refuses a catch-all
anywhere but the final segment, and a slug needs a catch-all because it
contains `/`. A time id cannot — it is twenty-five characters of digits and two
separators — so it is an ordinary parameter and the restriction never applies.

The two things a caller most wants from the time API are also why it looks the
way it does. `POST /api/times` starts a timer *and* logs a finished entry,
because a running entry is just one whose `end` has not been written yet, and
an absent field says so more honestly than a mode flag would. `stop` is a
separate endpoint because its entire content is the word "now", and a `PATCH`
would make every client read its own clock. See
[Time tracking](time-tracking.md).

### Why moving a page is not `/api/pages/{slug}/move`

Slugs contain `/`, so the page routes capture the rest of the URL
(`/api/pages/{*slug}` in axum 0.8). `matchit` requires a catch-all to be the
**final** segment, so `/api/pages/{*slug}/move` will not compile as a route at
all.

The obvious repair — a literal `/api/pages/move` — is worse than it looks. Path
matching prefers the static segment, so `GET /api/pages/move` would resolve to
the move route (which has no `GET`) and return 405, making a page actually
slugged `move` permanently unreachable. `move` is a perfectly ordinary page
name.

So the operation lives at `/api/move`, outside the namespace slugs occupy, and
takes `{from, to}` rather than reading one slug from the path. `/api/reindex`
already has the same shape.

### Why `?render=true` was not enough

M7's plan assumed the editor's preview could use `GET /api/pages/{slug}?render=true`.
It cannot: that renders what is **saved**, and a preview exists precisely to show
what is not. There is no version of it that works — previewing by saving first is
not previewing.

So `POST /api/render` takes `{content, slug?}` and returns `{html}`. It reads and
writes nothing, so it is safe to call on every keystroke.

The alternative was rendering markdown in the browser, which is worse for a
reason specific to this project: a client-side renderer would not know about
`[[wikilinks]]`, so the preview would be wrong in exactly the construct the wiki
is mostly made of. One renderer means the preview cannot disagree with the page.

`slug` is optional and only affects relative markdown links — `[traits](traits.md)`
names a different page depending on where it is written. Wikilinks are absolute
and unaffected, which is why a draft with no slug still previews correctly.

### Rendered links point at `/pages/...`

comrak renders `[[notes/a]]` as `href="notes/a"`, which is *relative*. Read on
`/pages/notes/b` the browser resolves it to `/pages/notes/notes/a`, so every
wikilink in a rendered body lands somewhere that does not exist — and fails
differently depending on how deeply nested the page reading it was.

Rendering therefore rewrites page links to root-absolute `/pages/<slug>`, which
is why `render` needs to know the source page at all. This is the browsable URL
rather than the API one on purpose: rendered HTML is for a human, and
`/api/pages/notes/a` would hand them JSON. It is not an assumption about some
other frontend — the backend serves `/pages/...` itself.

### `title_derived`, or why read-modify-write was quietly lossy

A page with no frontmatter `title` takes its title from the body's first heading,
falling back to the slug. `GET` returns that resolved title, which is what a
caller wants to display — but a client that read a page and wrote it straight
back would send the derived value as a *stored* one. The title would freeze, and
the heading it came from would never update it again.

That is a round-trip asymmetry in an API whose whole point is being written to by
agents, so `PageView` carries `title_derived`. When it is true, a caller writing
the page back should send `title: null`. The dashboard's editor leaves its title
field empty in that case and shows the derived title as the placeholder.

### A page's links endpoint reports time separately

`GET /api/links/{slug}` returns `outbound`, `inbound`, and `times`. The third
is a summary — a count, a total, and a capped sample — not a list, and it is
not folded into `inbound`.

The reason is that the two sides of the graph are counted in different orders
of magnitude. A page might be linked from five others; a page you actually work
on collects a time entry every time you start a timer. Reported as backlinks
they would bury the backlinks, and `most_linked` in `/api/stats` would start
ranking pages by how long you sat with them. One line with a total on it says
the useful thing, and `GET /api/times?page={slug}` has the rest.

### `/api/graph` answers a different question from `/api/links/{slug}`

One says where a page sits; the other says what shape the wiki is. They are not
the same call with a different limit, and the difference shows up in what a
filter is allowed to remove: a wanted page has no row anywhere, so no filter can
apply to it, and it comes back wherever a link in the view reaches it. See
[Drawing the link graph](link-graph.md) for that rule and the two beside it,
and for why a node's degree counts the whole wiki rather than the view.

### One links endpoint, not two

The plan called for separate `/links` and `/backlinks`. They are one endpoint
returning `{outbound, inbound}`, for the same routing reason as above —
`{*slug}/links` cannot be a route — and because it is how they are used: a page
view shows its links and its backlinks together, so one round trip beats two.

The slug does not have to name a page that exists. Asking about a wanted page
returns what already points at it, which is exactly what you want to see before
deciding whether to write it. The response carries `exists` to say which case
you are in.

### Operation ids are global, and utoipa takes them from function names

A handler called `list` in `api/pages.rs` and one called `list` in `api/pins.rs`
publish two operations with the id `list`. The document still validates and both
routes still work — but `openapi-typescript` keys on the operation id, so one of
each colliding pair silently disappears from the generated client.

Handlers are therefore named for the document, not just for their module
(`list_pins`, not `list`), and
`operation_ids_are_unique_across_the_document` in `tests/api.rs` is what keeps
it that way. See [Pins](pins.md), where this was first hit.

### The wildcard does not appear in the spec

`utoipa-axum` hands the path string from `#[utoipa::path]` straight to
`axum::Router::route`, so the wildcard has to be written in the macro for
nested slugs to route. But `{*slug}` is an axum spelling: left in the document
it produces a parameter literally named `*slug`, which reads wrong in Swagger UI
and would generate a mangled name in the typed client planned for M6.

The router rewrites `{*slug}` back to `{slug}` in the published document. Routes
and spec still come from one declaration; only the published spelling is
normalised.

## Designing for agents

The vision says the API should be friendly to AI agents as well as humans.
Concretely, that means the following, and these are acceptance criteria rather
than aspirations:

**Raw markdown is the default representation.** `GET /api/pages/{slug}` returns
the source, because that is what an agent can reason about and edit. Rendered
HTML is opt-in via `?render=true` and arrives as an extra field, never as a
replacement.

**Search returns enough to decide without fetching.** Each hit carries the
slug, title, tags, and an FTS5 `snippet()` excerpt with the match highlighted.
An agent can triage twenty results and then fetch two, instead of pulling
twenty full bodies to find out which two mattered.

**Listing is projectable.** `GET /api/pages?fields=slug,title,tags` keeps a
whole-wiki listing cheap enough to be a reasonable first call.

**Errors are machine-readable and uniform.** Every failure, including 404 and
422, returns the same shape with a stable `code`:

```json
{ "error": { "code": "page_not_found", "message": "No page at 'notes/rust/asnyc'", "details": { "slug": "notes/rust/asnyc" } } }
```

For a bad slug, `details` names which rule was violated. An agent that typos a
slug should be able to recover from the response alone. The same principle
applies wherever a request is refused for being outside a fixed set: a bad
`fields` or `sort` value comes back with the values that *would* have worked,
not just a complaint.

Codes in use so far:

| Code | Status | Meaning |
|---|---|---|
| `page_not_found` | 404 | No page at that slug |
| `page_already_exists` | 409 | Create or move onto an occupied slug |
| `page_malformed` / `page_not_utf8` | 422 | The file exists but cannot be read |
| `slug_*` | 400 | Which slug rule was broken (one code per rule) |
| `invalid_request_body` | 400 | Body would not parse or validate |
| `unknown_fields` | 400 | `fields` named something that is not a field |
| `invalid_parameter` | 400 | `sort`/`order` outside its allowed set |
| `pin_not_found` | 404 | Unpinning a page that was not pinned |
| `too_many_pins` | 409 | The pin limit is already reached |
| `unauthorized` | 401 | This wiki has accounts and the request named none |
| `invalid_credentials` | 401 | A sign-in that did not work |
| `forbidden` | 403 | The request said who it was, and that is not enough |
| `no_password_set` | 409 | The account exists and nobody gave it a password |
| `last_owner` | 409 | Deleting or demoting the only account that can administer |
| `ownerless_page` | 400 | `owner: null` sent with a visibility that nobody could then read |
| `user_not_found` / `user_already_exists` | 404 / 409 | As for a page, for an account |
| `username_*` | 400 | Which username rule was broken (one code per rule) |
| `password_too_short` / `password_too_long` | 400 | The only two password rules |
| `index_error` / `io_error` / `internal_error` | 500 | The server's problem |

**Extraction failures use the envelope too.** `axum::Json` rejects a bad body in
its own format, which would leave the error a caller is most likely to hit while
finding its footing looking nothing like every other error. A wrapping extractor
converts those into the envelope, keeping serde's own message — which names the
offending field and the rule it broke.

That path returns **400, not axum's default 422**, so that a slug refused in a
body and the same slug refused in a URL come back identically. A caller should
not have to learn that one mistake has two statuses depending on where it
appeared.

**The OpenAPI document is documentation, not a schema dump.** Every operation
and field carries a real `description` and a realistic `example`. This is the
single highest-leverage thing for agent usability, because for a tool-using
agent the spec *is* the manual — and it is also the easiest thing to let rot.
Route registration goes through `utoipa-axum`'s `OpenApiRouter` so a handler
cannot be added without appearing in the spec.

Three things keep it honest:

- **A doc comment is not always a description.** utoipa publishes doc comments,
  and some of them are written for maintainers. `Slug`'s explained
  `Slug::parse` and linked to it in rustdoc — meaningless on the wire, where
  there is no crate to resolve the link against. Those types set `description`
  explicitly, and a test fails if a rustdoc intra-doc link reaches the document.
- **Every example describes the same page**, and that page exists in
  `example-wiki/`, so the document can be read against a running server.
- **The documented HTML is asserted against the renderer.** An example that has
  quietly stopped being true is worse than none, because a reader cannot tell
  which ones still hold.

## Verified dependency set

Resolved together against a single `axum 0.8.9` with no duplicate `axum-core`:

```toml
axum = "0.8"
utoipa = { version = "5", features = ["axum_extras", "chrono"] }
utoipa-axum = "0.2"
utoipa-swagger-ui = { version = "9", features = ["axum"] }
```

## Conventions

- Timestamps are RFC 3339 UTC.
- `POST`/`PUT`/`PATCH` return the full page object, so a write needs no
  follow-up read.
- `PUT` is a full replace and is idempotent; `PATCH` merges only the fields
  present.
- Pagination is `limit`/`offset` with a `total` in the response body. Cursors
  are overkill for a single-user wiki.
- `DELETE` on a missing page returns `404`, not a silent `204` — a deletion
  that did nothing is worth knowing about.
- Listings sort ascending by default. `GET /api/times` is the one exception,
  because a log is read from the end.
- Durations are whole seconds, never a formatted string. Formatting is a
  reader's business and `1h 14m` is not something a client can add up.

### Regenerating the spec

`cargo run --example dump-openapi` writes `frontend/openapi.json` straight from
the compiled routes, with no server involved. It exists because the obvious
way — `curl`ing a running backend — is a trap on Windows with two jaws, both
documented in `CLAUDE.md`: PowerShell 5.1's `curl` decodes an uncharsetted
`application/json` body as Latin-1, and `>` re-encodes it again with a BOM. The
result is still valid JSON on one line, so the damage reads as a normal
regeneration.

Note that `ApiDoc::openapi()` is *not* the document — it is only the `info` and
`tags` skeleton. Every path comes from the routes, so both the example and the
server go through `api::openapi()`, and neither can describe something the
other does not serve.

### Authentication does not change the API's shape

An agent reaches an instance that requires a sign-in the same way a browser
does: `POST /api/auth/login`, then `Authorization: Bearer <token>`. It is the
same session the browser's cookie names, so there is one lifetime and one way to
revoke — which is the point. A cookie-only design would make authentication the
exact place where "the API is the only interface" stopped being true.

`GET /api/auth/session` never refuses. A client has to be able to ask *whether*
it needs to sign in, and a `401` is an answer it cannot tell apart from a session
that has just expired.

Everything else under `/api` needs an account once one exists, with four
exceptions listed in [Accounts](accounts.md). `GET /api/health` is the one worth
knowing about: it stays reachable because the desktop app's discovery handshake
predates any sign-in, and it stops reporting the wiki's page and time counts to
callers who have not made one.

Setting `RHIZOLOG_ANONYMOUS_READ` widens that: the read-only page, search, tag,
graph and stats endpoints start answering callers who have not signed in, and
answer them with the `public` pages only. Nothing anonymous can write, and the
pins and times endpoints stay closed — they are the operator's working state,
and no page being public says anything about wanting that published.

### Every page-shaped response is filtered, not just the page read

A page carries `visibility`, `owner` and `readers`, and a caller who may not read
it sees a `404` — the same status, code, message and shape as a page that is not
there, because a `403` would confirm that something exists at a slug somebody
guessed. That applies to the writes too: `PUT`, `PATCH`, `DELETE` and
`POST /api/move` all check before doing anything.

It also applies to everything a page can leak *through*: the listing, a search
snippet, a backlink's title, the tag histogram, `most_linked`, the graph, every
`total`, and the titles a pin or a time entry resolves. See
[Page visibility](visibility.md) — the interesting part is that it is one SQL
predicate pasted into every query rather than a check in one handler.

## Deliberately out of scope for the MVP

No rate limiting, no webhooks, no batch/transaction endpoints, no revision or
diff endpoints. All are plausible later; none are needed to make the wiki usable.

Authentication *was* on this list — "single-user, loopback-bound" — and is now
built, because serving a wiki over a network is what it was waiting for, along
with the page visibility that makes it worth having. See
[Accounts](accounts.md) and [Page visibility](visibility.md).
