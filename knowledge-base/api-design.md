# API design

The HTTP API is the primary interface, not a bolt-on to the UI. The admin
dashboard is just its first client. See [Architecture](architecture.md) for the
storage model it sits on.

## Endpoints (MVP)

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/pages` | List pages; `?tag=`, `?q=`, `?limit=`, `?offset=`, `?sort=` |
| `POST` | `/api/pages` | Create; `409` if the slug exists |
| `GET` | `/api/pages/{slug}` | Read; `?render=true` adds rendered HTML |
| `PUT` | `/api/pages/{slug}` | Create or replace |
| `PATCH` | `/api/pages/{slug}` | Partial update of title / tags / content |
| `DELETE` | `/api/pages/{slug}` | Delete |
| `POST` | `/api/move` | Move a page to a new slug |
| `GET` | `/api/pages/{slug}/links` | Outbound links — *not yet built* |
| `GET` | `/api/pages/{slug}/backlinks` | Inbound links — *not yet built* |
| `GET` | `/api/search` | Full-text search with snippets |
| `GET` | `/api/tags` | All tags with page counts — *not yet built* |
| `GET` | `/api/stats` | Meta-stats for the dashboard — *not yet built* |
| `POST` | `/api/reindex` | Force a full rebuild of the index |
| `GET` | `/api/health` | Liveness + index freshness |
| `GET` | `/api-docs/openapi.json` | Generated OpenAPI document |
| `GET` | `/swagger-ui` | Swagger UI |

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

## Deliberately out of scope for the MVP

No auth (single-user, loopback-bound), no rate limiting, no webhooks, no
batch/transaction endpoints, no revision or diff endpoints. All are plausible
later; none are needed to make the wiki usable.
