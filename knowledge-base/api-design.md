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
| `POST` | `/api/pages/{slug}/move` | Move to a new slug |
| `GET` | `/api/pages/{slug}/links` | Outbound links |
| `GET` | `/api/pages/{slug}/backlinks` | Inbound links |
| `GET` | `/api/search` | Full-text search with snippets |
| `GET` | `/api/tags` | All tags with page counts |
| `GET` | `/api/stats` | Meta-stats for the dashboard |
| `POST` | `/api/reindex` | Force a full rebuild of the index |
| `GET` | `/api/health` | Liveness + index freshness |
| `GET` | `/api-docs/openapi.json` | Generated OpenAPI document |
| `GET` | `/swagger-ui` | Swagger UI |

Slugs contain `/`, so the path parameter is a wildcard capture
(`/api/pages/{*slug}` in axum 0.8) rather than a single segment.

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
slug should be able to recover from the response alone.

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
