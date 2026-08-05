import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  ApiError,
  createPage,
  decodeSlug,
  deletePage,
  editHref,
  encodeSlug,
  getPage,
  listPages,
  pageHref,
  prefixHref,
  segmentHref,
  slugSegments,
  tagHref,
} from './client'

/** Reply as the API would, with a given status and JSON body. */
function replyWith(status: number, body: unknown) {
  const response = {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  }
  const fetchMock = vi.fn(async () => response as unknown as Response)
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('slug encoding', () => {
  /**
   * The separators have to survive into the path: the backend route is a
   * catch-all, and a fully percent-encoded slug would arrive as one segment
   * with `%2F` in it rather than as the nested page it names.
   */
  it('escapes each segment but keeps the separators', () => {
    expect(encodeSlug('notes/rust/async')).toBe('notes/rust/async')
    expect(encodeSlug('notes/my page')).toBe('notes/my%20page')
    expect(encodeSlug('notes/a#b')).toBe('notes/a%23b')
    expect(encodeSlug('notes/100%')).toBe('notes/100%25')
  })

  it('round-trips every slug shape the backend allows', () => {
    for (const slug of [
      'index',
      'notes/rust/async',
      'notes/my page',
      'notes/a#b',
      'notes/100%',
      'notes/café',
      'notes/a+b&c=d',
    ]) {
      expect(decodeSlug(encodeSlug(slug))).toBe(slug)
    }
  })

  /**
   * `@solidjs/router` hands back `location.pathname` verbatim, so a slug read
   * from a route param is still encoded. Decoding an already-decoded slug is
   * the mistake this guards: `decodeSlug` is only ever applied to raw params.
   */
  it('decodes a param the router has not touched', () => {
    expect(decodeSlug('notes/my%20page')).toBe('notes/my page')
    expect(decodeSlug('notes/rust/async')).toBe('notes/rust/async')
  })

  it('builds browser URLs that agree with the server', () => {
    // The server rewrites links inside rendered markdown to exactly this shape.
    expect(pageHref('notes/rust/async')).toBe('/pages/notes/rust/async')
    expect(pageHref('notes/my page')).toBe('/pages/notes/my%20page')
    expect(editHref('notes/rust/async')).toBe('/edit/notes/rust/async')
  })
})

describe('slug segments', () => {
  it('gives each segment both of its readings', () => {
    expect(slugSegments('notes/rust/async')).toEqual([
      { name: 'notes', path: 'notes', last: false },
      { name: 'rust', path: 'notes/rust', last: false },
      { name: 'async', path: 'notes/rust/async', last: true },
    ])
  })

  /**
   * `path` accumulates and `name` does not, which is the whole distinction the
   * two filters rest on: the same directory name in two places is one `name`
   * and two `path`s.
   */
  it('keeps a repeated directory name apart by its path', () => {
    const segments = slugSegments('notes/rust/notes/pinning')
    expect(segments.map((segment) => segment.name)).toEqual([
      'notes',
      'rust',
      'notes',
      'pinning',
    ])
    expect(segments.map((segment) => segment.path)).toEqual([
      'notes',
      'notes/rust',
      'notes/rust/notes',
      'notes/rust/notes/pinning',
    ])
  })

  /** A top-level page is one segment, and that segment is the page itself. */
  it('marks the final segment, which names the page rather than a directory', () => {
    expect(slugSegments('index')).toEqual([{ name: 'index', path: 'index', last: true }])
  })

  it('builds filter URLs that escape the separator', () => {
    expect(tagHref('rust')).toBe('/pages?tag=rust')
    expect(segmentHref('rust')).toBe('/pages?segment=rust')
    // A prefix is a path, and it travels in a query parameter rather than the
    // URL path, so its separators are escaped rather than kept.
    expect(prefixHref('notes/rust')).toBe('/pages?prefix=notes%2Frust')
    expect(prefixHref('notes/my page')).toBe('/pages?prefix=notes%2Fmy%20page')
  })
})

describe('requests', () => {
  it('reads a page from the encoded path', async () => {
    const fetchMock = replyWith(200, { slug: 'notes/my page', title: 'Mine' })

    const page = await getPage('notes/my page', { render: true })

    expect(page.title).toBe('Mine')
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(url).toBe('/api/pages/notes/my%20page?render=true')
    expect(init.method).toBe('GET')
  })

  it('omits query parameters that were not given', async () => {
    const fetchMock = replyWith(200, { pages: [], total: 0 })

    await listPages({ tag: undefined, limit: 50 })

    const [url] = fetchMock.mock.calls[0] as unknown as [string]
    expect(url).toBe('/api/pages?limit=50')
  })

  it('sends a JSON body with the right content type', async () => {
    const fetchMock = replyWith(201, { slug: 'notes/a' })

    await createPage({ slug: 'notes/a', content: '# A\n' })

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(init.method).toBe('POST')
    expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json')
    expect(JSON.parse(init.body as string)).toEqual({ slug: 'notes/a', content: '# A\n' })
  })

  /** `DELETE` answers 204, which has no body to parse. */
  it('handles a response with no content', async () => {
    replyWith(204, undefined)

    await expect(deletePage('notes/a')).resolves.toBeUndefined()
  })
})

describe('errors', () => {
  /**
   * The whole point of the envelope: one shape for every failure, with a code
   * stable enough to branch on. Callers must never have to read `message`.
   */
  it('turns the error envelope into a typed error', async () => {
    replyWith(404, {
      error: {
        code: 'page_not_found',
        message: "no page at 'notes/asnyc'",
        details: { slug: 'notes/asnyc' },
      },
    })

    const error = await getPage('notes/asnyc').catch((caught: unknown) => caught)

    expect(error).toBeInstanceOf(ApiError)
    expect((error as ApiError).code).toBe('page_not_found')
    expect((error as ApiError).status).toBe(404)
    expect((error as ApiError).details).toEqual({ slug: 'notes/asnyc' })
    expect((error as ApiError).isTransport).toBe(false)
  })

  it('reports a JSON error that is not the envelope', async () => {
    replyWith(500, { something: 'else' })

    const error = (await getPage('notes/a').catch((caught) => caught)) as ApiError

    expect(error.code).toBe('malformed_error_response')
    expect(error.status).toBe(500)
    expect(error.details).toEqual({ something: 'else' })
  })

  it('reports an error body that is not JSON at all', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => ({
        ok: false,
        status: 502,
        json: async () => {
          throw new SyntaxError('Unexpected token <')
        },
      }) as unknown as Response),
    )

    const error = (await getPage('notes/a').catch((caught) => caught)) as ApiError

    expect(error.code).toBe('malformed_error_response')
    expect(error.status).toBe(502)
  })

  /**
   * A dead backend is the most common failure while developing, so it arrives
   * as an `ApiError` like everything else — a caller catches one type.
   */
  it('reports a request that never got a response', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        throw new TypeError('Failed to fetch')
      }),
    )

    const error = (await getPage('notes/a').catch((caught) => caught)) as ApiError

    expect(error).toBeInstanceOf(ApiError)
    expect(error.code).toBe('network_error')
    expect(error.status).toBe(0)
    expect(error.isTransport).toBe(true)
  })

  /**
   * An abort is not a failure — it is a caller who stopped caring, usually
   * because a newer request replaced this one. Wrapping it would turn every
   * superseded keystroke in the editor into an error box.
   */
  it('lets an abort through untouched', async () => {
    const abort = new DOMException('The operation was aborted.', 'AbortError')
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        throw abort
      }),
    )

    const error = await getPage('notes/a').catch((caught: unknown) => caught)

    expect(error).toBe(abort)
    expect(error).not.toBeInstanceOf(ApiError)
  })
})
