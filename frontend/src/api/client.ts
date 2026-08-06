/**
 * Thin typed wrapper over the Rhizolog HTTP API.
 *
 * Every shape here comes from `schema.d.ts`, which is generated from
 * `openapi.json` by `pnpm gen:api`. Nothing in this file re-declares a
 * response body: if the backend changes the spec, regenerating turns the
 * mismatch into a compile error rather than a runtime surprise.
 */
import type { components, operations } from './schema'

type Schemas = components['schemas']

export type Slug = Schemas['Slug']
export type PageView = Schemas['PageView']
export type PageSummary = Schemas['PageSummary']
export type PageListResponse = Schemas['PageListResponse']
export type CreatePage = Schemas['CreatePage']
export type ReplacePage = Schemas['ReplacePage']
export type PatchPage = Schemas['PatchPage']
export type MovePage = Schemas['MovePage']
export type SearchResponse = Schemas['SearchResponse']
export type SearchHitView = Schemas['SearchHitView']
export type PageLinksResponse = Schemas['PageLinksResponse']
export type OutboundLinkView = Schemas['OutboundLinkView']
export type InboundLinkView = Schemas['InboundLinkView']
export type TagsResponse = Schemas['TagsResponse']
export type TagCountView = Schemas['TagCountView']
export type StatsResponse = Schemas['StatsResponse']
export type RenderRequest = Schemas['RenderRequest']
export type RenderedHtml = Schemas['RenderedHtml']
export type ReindexResponse = Schemas['ReindexResponse']
export type Health = Schemas['Health']
export type ErrorResponse = Schemas['ErrorResponse']
export type ErrorDetail = Schemas['ErrorDetail']

/** Query parameters, taken straight from the generated operations. */
export type ListPagesQuery = NonNullable<operations['list']['parameters']['query']>
export type SearchQuery = operations['search']['parameters']['query']
export type ReadPageQuery = NonNullable<operations['read']['parameters']['query']>

/**
 * Every failure the API reports, whatever the status, arrives as
 * `{"error": {"code", "message", "details"}}`. `code` is the stable part —
 * branch on it, not on `status` and never on `message`.
 *
 * Failures that never reached a handler (the network was down, the body was
 * not JSON) are reported the same way with a synthetic code, so a caller only
 * ever has to catch one type.
 */
export class ApiError extends Error {
  /** Stable machine-readable code, e.g. `page_not_found`. */
  readonly code: string
  /** HTTP status, or 0 when the request never got a response. */
  readonly status: number
  /** Whatever context the server attached, when there was any. */
  readonly details: unknown

  constructor(code: string, message: string, status: number, details?: unknown) {
    super(message)
    this.name = 'ApiError'
    this.code = code
    this.status = status
    this.details = details
  }

  /** True for codes the server does not own: transport and parse problems. */
  get isTransport(): boolean {
    return this.code === 'network_error' || this.code === 'malformed_error_response'
  }
}

function isErrorResponse(value: unknown): value is ErrorResponse {
  if (typeof value !== 'object' || value === null) return false
  const error = (value as { error?: unknown }).error
  if (typeof error !== 'object' || error === null) return false
  return typeof (error as { code?: unknown }).code === 'string'
}

/**
 * Slugs contain `/` (`notes/rust/async`) and the backend route is a catch-all,
 * so the separators must survive into the path. Escape each segment and
 * rejoin, rather than percent-encoding the whole thing.
 */
export function encodeSlug(slug: string): string {
  return slug.split('/').map(encodeURIComponent).join('/')
}

/**
 * Inverse of {@link encodeSlug}. `@solidjs/router` reads `location.pathname`
 * verbatim and does not decode path params, so a wildcard param arrives
 * percent-encoded and has to be turned back into a slug before it is used.
 */
export function decodeSlug(param: string): string {
  return param.split('/').map(decodeURIComponent).join('/')
}

/**
 * Where a page is read in the browser.
 *
 * The server knows this prefix too — it rewrites links inside rendered markdown
 * to `/pages/...` so they are clickable. The two have to agree, which is why
 * this is a named constant on both sides rather than a string scattered through
 * the routes.
 */
export const PAGE_ROUTE_PREFIX = '/pages/'

/** The browser URL for a page. */
export function pageHref(slug: string): string {
  return PAGE_ROUTE_PREFIX + encodeSlug(slug)
}

/** The browser URL for editing a page. */
export function editHref(slug: string): string {
  return '/edit/' + encodeSlug(slug)
}

/** One clickable part of a slug. */
export interface SlugSegment {
  /** The segment on its own: `rust`. */
  name: string
  /** The path up to and including it: `notes/rust`. */
  path: string
  /** True for the last segment, which names the page rather than a directory. */
  last: boolean
}

/**
 * Break a slug into its parts, each of which is a place you can go.
 *
 * A segment can be read two ways, and both are useful. `notes/rust/async`
 * sits *under* `notes/rust` — that is `path`, and following it stays inside
 * this branch of the wiki. It is also simply *in a `rust` directory* — that is
 * `name`, and following it leaves the branch behind and finds
 * `code/rust/traits` too. The first is a breadcrumb, the second is a tag, and
 * this returns both because the UI offers both.
 *
 * The final segment names the page itself rather than anything containing it,
 * so it is marked and callers leave it as text.
 */
export function slugSegments(slug: string): SlugSegment[] {
  const names = slug.split('/')
  return names.map((name, position) => ({
    name,
    path: names.slice(0, position + 1).join('/'),
    last: position === names.length - 1,
  }))
}

/** Browse everything carrying a tag. */
export function tagHref(tag: string): string {
  return `/pages?tag=${encodeURIComponent(tag)}`
}

/** Browse everything at or under a slug path. */
export function prefixHref(path: string): string {
  return `/pages?prefix=${encodeURIComponent(path)}`
}

/** Browse every page in a directory of this name, wherever it sits. */
export function segmentHref(name: string): string {
  return `/pages?segment=${encodeURIComponent(name)}`
}

function queryString(params: Record<string, unknown> | undefined): string {
  if (!params) return ''
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null) continue
    search.append(key, String(value))
  }
  const rendered = search.toString()
  return rendered ? `?${rendered}` : ''
}

/**
 * Base path for the API. Same origin in both modes: in dev the Vite proxy
 * forwards `/api` to the backend, in production the backend serves this app.
 */
const BASE = '/api'

interface RequestOptions {
  method?: string
  query?: Record<string, unknown>
  body?: unknown
  signal?: AbortSignal
}

async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = 'GET', query, body, signal } = options

  const init: RequestInit = { method, signal, headers: { Accept: 'application/json' } }
  if (body !== undefined) {
    init.headers = { ...init.headers, 'Content-Type': 'application/json' }
    init.body = JSON.stringify(body)
  }

  let response: Response
  try {
    response = await fetch(`${BASE}${path}${queryString(query)}`, init)
  } catch (cause) {
    if (cause instanceof DOMException && cause.name === 'AbortError') throw cause
    const message = cause instanceof Error ? cause.message : String(cause)
    throw new ApiError('network_error', `Could not reach the server: ${message}`, 0)
  }

  if (!response.ok) {
    let payload: unknown
    try {
      payload = await response.json()
    } catch {
      throw new ApiError(
        'malformed_error_response',
        `HTTP ${response.status} with a body that was not JSON`,
        response.status,
      )
    }
    if (isErrorResponse(payload)) {
      const { code, message, details } = payload.error
      throw new ApiError(code, message, response.status, details)
    }
    throw new ApiError(
      'malformed_error_response',
      `HTTP ${response.status} without the standard error envelope`,
      response.status,
      payload,
    )
  }

  if (response.status === 204) return undefined as T
  return (await response.json()) as T
}

/* ---------------------------------------------------------------- pages -- */

/** `GET /api/pages` — list pages without their bodies. */
export function listPages(
  query?: ListPagesQuery,
  signal?: AbortSignal,
): Promise<PageListResponse> {
  return request<PageListResponse>('/pages', { query, signal })
}

/** `POST /api/pages` — create a page; 409 (`page_already_exists`) if taken. */
export function createPage(body: CreatePage, signal?: AbortSignal): Promise<PageView> {
  return request<PageView>('/pages', { method: 'POST', body, signal })
}

/** `GET /api/pages/{slug}` — read a page as markdown, optionally rendered. */
export function getPage(
  slug: string,
  query?: ReadPageQuery,
  signal?: AbortSignal,
): Promise<PageView> {
  return request<PageView>(`/pages/${encodeSlug(slug)}`, { query, signal })
}

/** `PUT /api/pages/{slug}` — create or wholly replace. Idempotent. */
export function replacePage(
  slug: string,
  body: ReplacePage,
  signal?: AbortSignal,
): Promise<PageView> {
  return request<PageView>(`/pages/${encodeSlug(slug)}`, { method: 'PUT', body, signal })
}

/** `PATCH /api/pages/{slug}` — merge only the fields present. */
export function patchPage(
  slug: string,
  body: PatchPage,
  signal?: AbortSignal,
): Promise<PageView> {
  return request<PageView>(`/pages/${encodeSlug(slug)}`, { method: 'PATCH', body, signal })
}

/** `DELETE /api/pages/{slug}` — 204 on success, 404 if there was nothing there. */
export function deletePage(slug: string, signal?: AbortSignal): Promise<void> {
  return request<void>(`/pages/${encodeSlug(slug)}`, { method: 'DELETE', signal })
}

/** `POST /api/move` — move a page to a new slug. Inbound links are left alone. */
export function movePage(body: MovePage, signal?: AbortSignal): Promise<PageView> {
  return request<PageView>('/move', { method: 'POST', body, signal })
}

/**
 * `POST /api/render` — render markdown that has not been saved.
 *
 * The editor's preview goes through here rather than through a markdown library
 * in the browser, so that what the preview shows and what the page becomes
 * cannot disagree. A client-side renderer would not know about wikilinks, which
 * is most of what this wiki's pages are made of.
 */
export function renderMarkdown(
  body: RenderRequest,
  signal?: AbortSignal,
): Promise<RenderedHtml> {
  return request<RenderedHtml>('/render', { method: 'POST', body, signal })
}

/* --------------------------------------------------------------- search -- */

/** `GET /api/search` — full-text search with `<mark>`-highlighted snippets. */
export function search(query: SearchQuery, signal?: AbortSignal): Promise<SearchResponse> {
  return request<SearchResponse>('/search', { query, signal })
}

/** `POST /api/reindex` — rebuild the index from disk. Always safe. */
export function reindex(signal?: AbortSignal): Promise<ReindexResponse> {
  return request<ReindexResponse>('/reindex', { method: 'POST', signal })
}

/* ---------------------------------------------------------------- graph -- */

/**
 * `GET /api/links/{slug}` — both directions at once. The slug need not name a
 * page that exists; `exists` says which case you are in.
 */
export function pageLinks(slug: string, signal?: AbortSignal): Promise<PageLinksResponse> {
  return request<PageLinksResponse>(`/links/${encodeSlug(slug)}`, { signal })
}

/** `GET /api/tags` — every tag with its page count, most-used first. */
export function tags(signal?: AbortSignal): Promise<TagsResponse> {
  return request<TagsResponse>('/tags', { signal })
}

/** `GET /api/stats` — meta-stats for the dashboard. */
export function stats(signal?: AbortSignal): Promise<StatsResponse> {
  return request<StatsResponse>('/stats', { signal })
}

/* ----------------------------------------------------------------- meta -- */

/** `GET /api/health` — liveness plus index freshness. */
export function health(signal?: AbortSignal): Promise<Health> {
  return request<Health>('/health', { signal })
}
