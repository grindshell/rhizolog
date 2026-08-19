import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ApiError, login, logout, onUnauthorized, session } from './client'

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

/** The envelope every failure arrives in. */
function failure(code: string, message = 'nope', details?: unknown) {
  return { error: { code, message, details } }
}

/** Await a call that is expected to be refused, and hand back why. */
async function refusal(work: Promise<unknown>): Promise<ApiError> {
  try {
    await work
  } catch (cause) {
    if (cause instanceof ApiError) return cause
    throw cause
  }
  throw new Error('expected the request to be refused, and it succeeded')
}

afterEach(() => {
  vi.unstubAllGlobals()
  // The handler is module-level, so a test that installs one would otherwise
  // leak into the next.
  onUnauthorized(() => {})
})

describe('signing in', () => {
  it('posts credentials and returns the session', async () => {
    const fetchMock = replyWith(200, {
      user: { username: 'tim', display_name: 'Tim', role: 'owner' },
      token: '3f2a',
      expires: '2026-09-18T10:00:00Z',
    })

    const result = await login({ username: 'tim', password: 'correct horse' })

    expect(result.user.username).toBe('tim')
    const [path, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(path).toBe('/api/auth/login')
    expect(init.method).toBe('POST')
    expect(JSON.parse(init.body as string)).toEqual({
      username: 'tim',
      password: 'correct horse',
    })
  })

  /**
   * A wrong password and an account that does not exist are one answer on
   * purpose. The client must not decorate that into two.
   */
  it('reports a refusal as invalid_credentials with nothing else', async () => {
    replyWith(401, failure('invalid_credentials', 'incorrect username or password'))

    const error = await refusal(login({ username: 'tim', password: 'wrong' }))

    expect(error.code).toBe('invalid_credentials')
    expect(error.status).toBe(401)
    expect(error.details).toBeUndefined()
  })

  it('surfaces an account that has no password as its own case', async () => {
    replyWith(409, failure('no_password_set', 'the account alice has no password set'))

    const error = await refusal(login({ username: 'alice', password: 'anything' }))

    expect(error.code).toBe('no_password_set')
    expect(error.status).toBe(409)
  })
})

describe('the session endpoint', () => {
  it('reports an open wiki as needing nothing', async () => {
    replyWith(200, {
      authentication_required: false,
      authenticated: true,
      user: null,
    })

    const status = await session()

    expect(status.authentication_required).toBe(false)
    expect(status.authenticated).toBe(true)
    // Deliberately nameless: an open wiki has no account to name.
    expect(status.user).toBeNull()
  })

  it('reports a closed wiki that this browser is not signed in to', async () => {
    replyWith(200, {
      authentication_required: true,
      authenticated: false,
      user: null,
    })

    const status = await session()

    expect(status.authentication_required).toBe(true)
    expect(status.authenticated).toBe(false)
  })
})

describe('the unauthorized handler', () => {
  let fired: number

  beforeEach(() => {
    fired = 0
    onUnauthorized(() => {
      fired += 1
    })
  })

  /**
   * A session can end without this tab doing anything — it expires, the account
   * is deleted, a password changes elsewhere. The only sign is a 401 on an
   * ordinary request, and that is what has to turn the app back into a login
   * page.
   */
  it('fires when an ordinary request finds no session', async () => {
    replyWith(401, failure('unauthorized', 'this wiki requires authentication'))

    await session().catch(() => {})

    expect(fired).toBe(1)
  })

  /**
   * A 403 says the session is perfectly good and this account may not do that.
   * Signing somebody out in response would be both wrong and infuriating.
   */
  it('does not fire on a forbidden request', async () => {
    replyWith(403, failure('forbidden', 'not permitted to administer accounts'))

    await session().catch(() => {})

    expect(fired).toBe(0)
  })

  /**
   * A wrong password should leave the form on screen with its message, not
   * reset the signed-out state it is already in.
   */
  it('does not fire on a failed sign-in', async () => {
    replyWith(401, failure('invalid_credentials'))

    await login({ username: 'tim', password: 'wrong' }).catch(() => {})

    expect(fired).toBe(0)
  })

  it('does not fire on an ordinary success', async () => {
    replyWith(200, { authentication_required: true, authenticated: true, user: null })

    await session()

    expect(fired).toBe(0)
  })
})

describe('signing out', () => {
  /** `204` has no body, and asking for one would throw. */
  it('handles the empty response', async () => {
    const fetchMock = replyWith(204, undefined)

    await expect(logout()).resolves.toBeUndefined()

    const [path, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit]
    expect(path).toBe('/api/auth/logout')
    expect(init.method).toBe('POST')
  })
})
