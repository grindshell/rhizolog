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
export type GraphResponse = Schemas['GraphResponse']
export type GraphNodeView = Schemas['GraphNodeView']
export type GraphEdgeView = Schemas['GraphEdgeView']
export type PinView = Schemas['PinView']
export type PinsResponse = Schemas['PinsResponse']
export type PageTimesView = Schemas['PageTimesView']
export type TimeRefView = Schemas['TimeRefView']
export type TimeId = Schemas['TimeId']
export type TimeSummary = Schemas['TimeSummary']
export type TimeView = Schemas['TimeView']
export type TimePageView = Schemas['TimePageView']
export type TimeListResponse = Schemas['TimeListResponse']
export type CreateTime = Schemas['CreateTime']
export type PatchTime = Schemas['PatchTime']
export type TimeGroupView = Schemas['TimeGroupView']
export type TimeGroupsResponse = Schemas['TimeGroupsResponse']
export type TimeTotalsView = Schemas['TimeTotalsView']
export type TimeStatsResponse = Schemas['TimeStatsResponse']
export type PeriodStatsView = Schemas['PeriodStatsView']
export type BucketView = Schemas['BucketView']
export type NameTotalView = Schemas['NameTotalView']
export type PageTotalView = Schemas['PageTotalView']
export type HeatCellView = Schemas['HeatCellView']
export type HeatmapView = Schemas['HeatmapView']
export type StatsResponse = Schemas['StatsResponse']
export type RenderRequest = Schemas['RenderRequest']
export type RenderedHtml = Schemas['RenderedHtml']
export type ReindexResponse = Schemas['ReindexResponse']
export type Health = Schemas['Health']
export type ErrorResponse = Schemas['ErrorResponse']
export type ErrorDetail = Schemas['ErrorDetail']
export type Username = Schemas['Username']
export type Role = Schemas['Role']
export type UserView = Schemas['UserView']
export type UsersResponse = Schemas['UsersResponse']
export type CreateUser = Schemas['CreateUser']
export type PatchUser = Schemas['PatchUser']
export type PatchUserResponse = Schemas['PatchUserResponse']
export type LoginRequest = Schemas['LoginRequest']
export type LoginResponse = Schemas['LoginResponse']
export type SessionStatus = Schemas['SessionStatus']

/** Query parameters, taken straight from the generated operations. */
export type ListPagesQuery = NonNullable<operations['list']['parameters']['query']>
export type SearchQuery = operations['search']['parameters']['query']
export type ReadPageQuery = NonNullable<operations['read']['parameters']['query']>
export type GraphQuery = NonNullable<operations['link_graph']['parameters']['query']>
export type ListTimesQuery = NonNullable<operations['list_times']['parameters']['query']>
export type TimeStatsQuery = NonNullable<operations['time_statistics']['parameters']['query']>

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

/**
 * Called whenever the server says this request had no valid session.
 *
 * A session can end without this tab doing anything: it expires, an owner
 * deletes the account, or a password change elsewhere ends every session it had.
 * The first sign of any of those is a `401` on an ordinary request, and the app
 * has to turn back into a login page rather than showing a wall of error
 * notices.
 *
 * A callback rather than an import of the session store, because that store is
 * built on the functions in this file and the cycle would be real.
 */
let unauthorizedHandler: (() => void) | undefined

export function onUnauthorized(handler: () => void): void {
  unauthorizedHandler = handler
}

/**
 * Whether a failure means "you are not signed in".
 *
 * Two codes are deliberately not here. `forbidden` (403) says the session is
 * perfectly good and the account is not allowed, so signing out in response
 * would be both wrong and infuriating. `invalid_credentials` (401) is a failed
 * sign-in, where the form should keep its message rather than reset the state it
 * is already in.
 */
function isSignedOut(status: number, code: string): boolean {
  return status === 401 && code === 'unauthorized'
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
      // Told before the error is thrown, so the app is already showing a login
      // page by the time whoever called this decides what to do about it.
      if (isSignedOut(response.status, code)) {
        unauthorizedHandler?.()
      }
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

/**
 * `GET /api/graph` — the whole link graph, as nodes and edges.
 *
 * `pageLinks` answers "where does this page sit"; this answers "what shape is
 * the wiki". Nodes include pages nobody has written, which is most of the point
 * of drawing it at all.
 */
export function linkGraph(
  query?: GraphQuery,
  signal?: AbortSignal,
): Promise<GraphResponse> {
  return request<GraphResponse>('/graph', { query, signal })
}

/** Where a page's neighbourhood is drawn. */
export function graphHref(slug: string): string {
  return `/graph?root=${encodeURIComponent(slug)}`
}

/** `GET /api/stats` — meta-stats for the dashboard. */
export function stats(signal?: AbortSignal): Promise<StatsResponse> {
  return request<StatsResponse>('/stats', { signal })
}

/* ----------------------------------------------------------------- pins -- */

/** `GET /api/pins` — the pinned pages, oldest first. */
export function listPins(signal?: AbortSignal): Promise<PinsResponse> {
  return request<PinsResponse>('/pins', { signal })
}

/**
 * `PUT /api/pins/{slug}` — pin a page. Idempotent, so a caller does not have to
 * check whether it is pinned already. `404` if there is no page there, `409`
 * (`too_many_pins`) at the limit.
 */
export function pinPage(slug: string, signal?: AbortSignal): Promise<PinView> {
  return request<PinView>(`/pins/${encodeSlug(slug)}`, { method: 'PUT', signal })
}

/**
 * `DELETE /api/pins/{slug}` — unpin. The page itself is untouched. `404`
 * (`pin_not_found`) if it was not pinned — distinct from `page_not_found`,
 * which would mean something else entirely.
 */
export function unpinPage(slug: string, signal?: AbortSignal): Promise<void> {
  return request<void>(`/pins/${encodeSlug(slug)}`, { method: 'DELETE', signal })
}

/* ---------------------------------------------------------------- times -- */

/** Where a group's entries are browsed. */
export function groupHref(name: string): string {
  return `/times?name=${encodeURIComponent(name)}`
}

/** Where the time tracked against a page is browsed. */
export function pageTimesHref(slug: string): string {
  return `/times?page=${encodeURIComponent(slug)}`
}

/**
 * The browser's offset from UTC, in minutes **east** — the sign the API wants.
 *
 * `getTimezoneOffset` reports minutes to *add to local time to get UTC*, which
 * is the opposite sign to every other convention, so this is negated exactly
 * once and in one place.
 */
export function utcOffsetMinutes(at: Date = new Date()): number {
  return -at.getTimezoneOffset()
}

/** `GET /api/times` — entries without their notes, newest first. */
export function listTimes(
  query?: ListTimesQuery,
  signal?: AbortSignal,
): Promise<TimeListResponse> {
  return request<TimeListResponse>('/times', { query, signal })
}

/**
 * `POST /api/times` — start a timer, or log time that is already over.
 *
 * With only a `name` it starts now and keeps running. With a `start` and an
 * `end` it records a finished entry. There is no separate start endpoint
 * because there is no separate thing: a running entry is one whose end has not
 * been written yet.
 */
export function createTime(body: CreateTime, signal?: AbortSignal): Promise<TimeView> {
  return request<TimeView>('/times', { method: 'POST', body, signal })
}

/** `GET /api/times/{id}` — one entry, with its note. */
export function getTime(
  id: string,
  query?: { render?: boolean },
  signal?: AbortSignal,
): Promise<TimeView> {
  return request<TimeView>(`/times/${encodeURIComponent(id)}`, { query, signal })
}

/** `PATCH /api/times/{id}` — merge only the fields present. `end: null` restarts it. */
export function patchTime(
  id: string,
  body: PatchTime,
  signal?: AbortSignal,
): Promise<TimeView> {
  return request<TimeView>(`/times/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body,
    signal,
  })
}

/**
 * `POST /api/times/{id}/stop` — stop a running timer, now.
 *
 * `409` (`time_not_running`) if it had already stopped, which usually means
 * another tab got there first.
 */
export function stopTime(id: string, signal?: AbortSignal): Promise<TimeView> {
  return request<TimeView>(`/times/${encodeURIComponent(id)}/stop`, {
    method: 'POST',
    signal,
  })
}

/** `DELETE /api/times/{id}` — 204, or 404 (`time_not_found`). */
export function deleteTime(id: string, signal?: AbortSignal): Promise<void> {
  return request<void>(`/times/${encodeURIComponent(id)}`, { method: 'DELETE', signal })
}

/** `GET /api/time-groups` — every activity name with its totals. */
export function timeGroups(signal?: AbortSignal): Promise<TimeGroupsResponse> {
  return request<TimeGroupsResponse>('/time-groups', { signal })
}

/**
 * `GET /api/time-stats` — day, week, month and year at once, plus the heat map.
 *
 * The offset defaults to this browser's, because every window the server cuts
 * is a local one and the server has no way to guess.
 */
export function timeStats(
  query: TimeStatsQuery = {},
  signal?: AbortSignal,
): Promise<TimeStatsResponse> {
  return request<TimeStatsResponse>('/time-stats', {
    query: { offset: utcOffsetMinutes(), ...query },
    signal,
  })
}

/* -------------------------------------------------------------- accounts -- */

/**
 * `GET /api/auth/session` — does this wiki want a sign-in, and is this browser
 * signed in?
 *
 * The first call the app makes, and the one endpoint that never returns 401:
 * a refusal here would be indistinguishable from the session having just
 * expired, which is the thing being asked about.
 *
 * `authentication_required: false` is a wiki with no accounts. There is no login
 * page in that case and nothing is refused — see `knowledge-base/accounts.md`.
 */
export function session(signal?: AbortSignal): Promise<SessionStatus> {
  return request<SessionStatus>('/auth/session', { signal })
}

/**
 * `POST /api/auth/login` — sign in.
 *
 * The browser never touches the `token` in the response. The same session
 * arrives as an `HttpOnly` cookie the browser attaches by itself, and that
 * cookie is deliberately unreadable from JavaScript: this app renders markdown
 * somebody else may have written. The token field is there for scripts and
 * agents, which cannot use a cookie jar.
 *
 * `401` (`invalid_credentials`) covers both a wrong password and an account that
 * does not exist, on purpose.
 */
export function login(body: LoginRequest, signal?: AbortSignal): Promise<LoginResponse> {
  return request<LoginResponse>('/auth/login', { method: 'POST', body, signal })
}

/** `POST /api/auth/logout` — end this session. `204` even if there was none. */
export function logout(signal?: AbortSignal): Promise<void> {
  return request<void>('/auth/logout', { method: 'POST', signal })
}

/** `GET /api/users` — every account. Any signed-in account may ask. */
export function listUsers(signal?: AbortSignal): Promise<UsersResponse> {
  return request<UsersResponse>('/users', { signal })
}

/**
 * `POST /api/users` — create an account.
 *
 * Needs no credentials for the **first** account on a wiki, which is what turns
 * authentication on; every one after that needs an owner. The first account is
 * an owner whatever this asks for.
 */
export function createUser(body: CreateUser, signal?: AbortSignal): Promise<UserView> {
  return request<UserView>('/users', { method: 'POST', body, signal })
}

/**
 * `PATCH /api/users/{username}` — merge only the fields present.
 *
 * Sending a `password` ends every session that account has, **including this
 * one**. `sessions_ended` in the response says how many, and the caller has to
 * sign in again.
 */
export function patchUser(
  username: string,
  body: PatchUser,
  signal?: AbortSignal,
): Promise<PatchUserResponse> {
  return request<PatchUserResponse>(`/users/${encodeURIComponent(username)}`, {
    method: 'PATCH',
    body,
    signal,
  })
}

/** `DELETE /api/users/{username}` — 204, or 409 (`last_owner`). */
export function deleteUser(username: string, signal?: AbortSignal): Promise<void> {
  return request<void>(`/users/${encodeURIComponent(username)}`, {
    method: 'DELETE',
    signal,
  })
}

/* ----------------------------------------------------------------- meta -- */

/** `GET /api/health` — liveness plus index freshness. */
export function health(signal?: AbortSignal): Promise<Health> {
  return request<Health>('/health', { signal })
}
