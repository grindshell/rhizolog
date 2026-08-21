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
export type Visibility = Schemas['Visibility']
export type UserView = Schemas['UserView']
export type UsersResponse = Schemas['UsersResponse']
export type CreateUser = Schemas['CreateUser']
export type PatchUser = Schemas['PatchUser']
export type PatchUserResponse = Schemas['PatchUserResponse']
export type LoginRequest = Schemas['LoginRequest']
export type LoginResponse = Schemas['LoginResponse']
export type SessionStatus = Schemas['SessionStatus']
export type CaptureId = Schemas['CaptureId']
export type IdeaId = Schemas['IdeaId']
export type EventId = Schemas['EventId']
export type EventKind = Schemas['EventKind']
export type Lifecycle = Schemas['Lifecycle']
export type Integrity = Schemas['Integrity']
export type CaptureView = Schemas['CaptureView']
export type CaptureListResponse = Schemas['CaptureListResponse']
export type CreateCapture = Schemas['CreateCapture']
export type PatchCapture = Schemas['PatchCapture']
export type DeletedCapture = Schemas['DeletedCapture']
export type AffectedIdea = Schemas['AffectedIdea']
export type CandidateResponse = Schemas['CandidateResponse']
export type CandidateView = Schemas['CandidateView']
export type SignalView = Schemas['SignalView']
export type TargetKind = Schemas['TargetKind']
export type IdeaTargetView = Schemas['IdeaTargetView']
export type IdeaView = Schemas['IdeaView']
export type IdeaSummaryView = Schemas['IdeaSummaryView']
export type IdeaListResponse = Schemas['IdeaListResponse']
export type CreateIdea = Schemas['CreateIdea']
export type PatchIdea = Schemas['PatchIdea']
export type ReceiptResponse = Schemas['ReceiptResponse']
export type DraftResponse = Schemas['DraftResponse']
export type DraftSource = Schemas['DraftSource']
export type RecordPromotion = Schemas['RecordPromotion']
export type Components = Schemas['Components']
export type Boundaries = Schemas['Boundaries']
export type CountedCapture = Schemas['CountedCapture']
export type CountedAffirmation = Schemas['CountedAffirmation']

/** Query parameters, taken straight from the generated operations. */
export type ListPagesQuery = NonNullable<operations['list']['parameters']['query']>
export type SearchQuery = operations['search']['parameters']['query']
export type ReadPageQuery = NonNullable<operations['read']['parameters']['query']>
export type GraphQuery = NonNullable<operations['link_graph']['parameters']['query']>
export type ListTimesQuery = NonNullable<operations['list_times']['parameters']['query']>
export type TimeStatsQuery = NonNullable<operations['time_statistics']['parameters']['query']>
export type ListCapturesQuery = NonNullable<operations['list_captures']['parameters']['query']>
export type ListIdeasQuery = NonNullable<operations['list_ideas']['parameters']['query']>
export type ReceiptQuery = NonNullable<operations['read_idea_receipt']['parameters']['query']>

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

/* ----------------------------------------------------------- idea inbox -- */

/**
 * Ids are minted by the server and contain nothing that needs escaping, but
 * they go through `encodeURIComponent` anyway. A path built by concatenation is
 * a path somebody will one day hand something else.
 */
function capturePath(id: string, suffix = ''): string {
  return `/captures/${encodeURIComponent(id)}${suffix}`
}

function ideaPath(id: string, suffix = ''): string {
  return `/ideas/${encodeURIComponent(id)}${suffix}`
}

/** Where one idea is read in the browser. */
export function ideaHref(id: string): string {
  return `/ideas/${encodeURIComponent(id)}`
}

/**
 * Where a lifecycle state is browsed.
 *
 * A state is a filter in the URL rather than a tab in component state, so
 * "everything dormant" is a link somebody can keep.
 */
export function lifecycleHref(state: string): string {
  return `/ideas?state=${encodeURIComponent(state)}`
}

/** `GET /api/captures`: the inbox, newest first. */
export function listCaptures(
  query?: ListCapturesQuery,
  signal?: AbortSignal,
): Promise<CaptureListResponse> {
  return request<CaptureListResponse>('/captures', { query, signal })
}

/**
 * `POST /api/captures`: save text with a server timestamp.
 *
 * The primary operation of the whole feature, and deliberately the one that
 * asks for nothing else: no title, no tag, no idea. Nothing is analyzed here,
 * so nothing about the analyzer can lose what somebody just typed.
 */
export function createCapture(
  body: CreateCapture,
  signal?: AbortSignal,
): Promise<CaptureView> {
  return request<CaptureView>('/captures', { method: 'POST', body, signal })
}

/** `GET /api/captures/{id}`: one capture. */
export function getCapture(id: string, signal?: AbortSignal): Promise<CaptureView> {
  return request<CaptureView>(capturePath(id), { signal })
}

/** `PATCH /api/captures/{id}`: correct the text. `created` does not move. */
export function patchCapture(
  id: string,
  body: PatchCapture,
  signal?: AbortSignal,
): Promise<CaptureView> {
  return request<CaptureView>(capturePath(id), { method: 'PATCH', body, signal })
}

/**
 * `DELETE /api/captures/{id}`: permanent, and refused with `409`
 * (`capture_required_by_idea`) when it is the last thing an idea stands on.
 *
 * The response names the ideas that held it so the caller can say what changed
 * without going and reading them.
 */
export function deleteCapture(
  id: string,
  signal?: AbortSignal,
): Promise<DeletedCapture> {
  return request<DeletedCapture>(capturePath(id), { method: 'DELETE', signal })
}

/** `POST /api/captures/{id}/archive`: out of the inbox, still evidence. */
export function archiveCapture(id: string, signal?: AbortSignal): Promise<CaptureView> {
  return request<CaptureView>(capturePath(id, '/archive'), { method: 'POST', signal })
}

/** `POST /api/captures/{id}/restore`: back into the inbox. */
export function restoreCapture(id: string, signal?: AbortSignal): Promise<CaptureView> {
  return request<CaptureView>(capturePath(id, '/restore'), { method: 'POST', signal })
}

/**
 * `GET /api/captures/{id}/candidates`: what this might belong with, and the
 * arithmetic that says so.
 *
 * Advisory. Nothing here has connected anything, and `503`
 * (`idea_analysis_unavailable`) means the index is behind rather than that
 * something is broken: the capture is on disk and a reindex fixes it.
 */
export function captureCandidates(
  id: string,
  signal?: AbortSignal,
): Promise<CandidateResponse> {
  return request<CandidateResponse>(capturePath(id, '/candidates'), { signal })
}

/**
 * `PUT /api/captures/{id}/rejections/{other}`: stop suggesting these two
 * together. Idempotent, and the pair is one decision whichever side asked.
 */
export function rejectCapturePair(
  id: string,
  other: string,
  signal?: AbortSignal,
): Promise<CaptureView> {
  return request<CaptureView>(
    capturePath(id, `/rejections/${encodeURIComponent(other)}`),
    { method: 'PUT', signal },
  )
}

/** `DELETE /api/captures/{id}/rejections/{other}`: suggest it again. */
export function reconsiderCapturePair(
  id: string,
  other: string,
  signal?: AbortSignal,
): Promise<CaptureView> {
  return request<CaptureView>(
    capturePath(id, `/rejections/${encodeURIComponent(other)}`),
    { method: 'DELETE', signal },
  )
}

/**
 * `GET /api/ideas`: the threads, most recently active first.
 *
 * `state` and `integrity` are worked out by the rules rather than by SQL, so
 * `total` counts what matched rather than what existed.
 */
export function listIdeas(
  query?: ListIdeasQuery,
  signal?: AbortSignal,
): Promise<IdeaListResponse> {
  return request<IdeaListResponse>('/ideas', { query, signal })
}

/** `POST /api/ideas`: name a thread and give it the captures it starts from. */
export function createIdea(body: CreateIdea, signal?: AbortSignal): Promise<IdeaView> {
  return request<IdeaView>('/ideas', { method: 'POST', body, signal })
}

/** `GET /api/ideas/{id}`: the thread, what it holds and where it stands. */
export function getIdea(id: string, signal?: AbortSignal): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id), { signal })
}

/** `PATCH /api/ideas/{id}`: rename it or rewrite its note. */
export function patchIdea(
  id: string,
  body: PatchIdea,
  signal?: AbortSignal,
): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id), { method: 'PATCH', body, signal })
}

/**
 * `PUT /api/ideas/{id}/captures/{capture}`: connect one. Idempotent, because
 * the state being asked for is in the URL: repeating it writes no second event.
 */
export function connectCapture(
  id: string,
  capture: string,
  signal?: AbortSignal,
): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, `/captures/${encodeURIComponent(capture)}`), {
    method: 'PUT',
    signal,
  })
}

/**
 * `DELETE /api/ideas/{id}/captures/{capture}`: disconnect, refused with `409`
 * (`idea_would_be_empty`) for the last one. Retiring is how an idea is set
 * aside, and it is reversible.
 */
export function disconnectCapture(
  id: string,
  capture: string,
  signal?: AbortSignal,
): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, `/captures/${encodeURIComponent(capture)}`), {
    method: 'DELETE',
    signal,
  })
}

/** `PUT /api/ideas/{id}/rejections/{capture}`: never suggest this one again. */
export function rejectCandidate(
  id: string,
  capture: string,
  signal?: AbortSignal,
): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, `/rejections/${encodeURIComponent(capture)}`), {
    method: 'PUT',
    signal,
  })
}

/** `DELETE /api/ideas/{id}/rejections/{capture}`: reconsider it. */
export function reconsiderCandidate(
  id: string,
  capture: string,
  signal?: AbortSignal,
): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, `/rejections/${encodeURIComponent(capture)}`), {
    method: 'DELETE',
    signal,
  })
}

/**
 * `POST /api/ideas/{id}/affirm`: say you are still interested.
 *
 * Not idempotent, and right not to be: affirming twice is two affirmations at
 * two moments and both of them happened.
 */
export function affirmIdea(id: string, signal?: AbortSignal): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, '/affirm'), { method: 'POST', signal })
}

/** `POST /api/ideas/{id}/retire`: set it aside. `409` if it already is. */
export function retireIdea(id: string, signal?: AbortSignal): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, '/retire'), { method: 'POST', signal })
}

/** `POST /api/ideas/{id}/reopen`: take it back up. `409` if it was not retired. */
export function reopenIdea(id: string, signal?: AbortSignal): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, '/reopen'), { method: 'POST', signal })
}

/** `POST /api/ideas/{id}/dismiss`: stop resurfacing it for thirty days. */
export function dismissIdea(id: string, signal?: AbortSignal): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, '/dismiss'), { method: 'POST', signal })
}

/**
 * `GET /api/ideas/{id}/receipt`: every number behind the state and momentum,
 * and every authored record they were counted from.
 *
 * `at` asks about another moment. The dashboard asks about now; the parameter
 * exists so that "why was this dormant last month" is answerable.
 */
export function ideaReceipt(
  id: string,
  query?: ReceiptQuery,
  signal?: AbortSignal,
): Promise<ReceiptResponse> {
  return request<ReceiptResponse>(ideaPath(id, '/receipt'), { query, signal })
}

/**
 * `GET /api/ideas/{id}/draft`: the page this idea would make, assembled and not
 * written.
 *
 * Reading it creates nothing and records nothing. The markdown is the thread's
 * note and every capture it holds, oldest first and word for word, which is
 * what makes it a starting point rather than a summary.
 */
export function ideaDraft(id: string, signal?: AbortSignal): Promise<DraftResponse> {
  return request<DraftResponse>(ideaPath(id, '/draft'), { signal })
}

/**
 * `PUT /api/ideas/{id}/promotion`: record the page an idea produced.
 *
 * The page has to exist first, which is why this is the third step and not the
 * only one. Idempotent, and that is what makes the sequence recoverable: if the
 * page was created and this failed, sending it again finishes the job rather
 * than writing a second page.
 */
export function recordPromotion(
  id: string,
  body: RecordPromotion,
  signal?: AbortSignal,
): Promise<IdeaView> {
  return request<IdeaView>(ideaPath(id, '/promotion'), { method: 'PUT', body, signal })
}

/* ----------------------------------------------------------------- meta -- */

/** `GET /api/health` — liveness plus index freshness. */
export function health(signal?: AbortSignal): Promise<Health> {
  return request<Health>('/health', { signal })
}
