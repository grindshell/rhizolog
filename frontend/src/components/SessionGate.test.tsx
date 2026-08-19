import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { ApiError } from '../api/client'

const api = vi.hoisted(() => ({
  session: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

/**
 * The session store is a module-level `createRoot`, which means it resolves once
 * per module instance. Each test therefore re-imports it with a fresh registry
 * so the resource starts over — otherwise the first test's answer would be
 * every test's answer.
 */
async function mount(status: unknown) {
  if (status instanceof Error) {
    api.session.mockRejectedValue(status)
  } else {
    api.session.mockResolvedValue(status)
  }

  vi.resetModules()
  const { default: SessionGate } = await import('./SessionGate')

  return render(() => <SessionGate>{<div>the dashboard</div>}</SessionGate>)
}

/** The login form, and the two fields it is made of. */
function form(container: HTMLElement) {
  const element = container.querySelector('form')
  if (!element) throw new Error('no login form on screen')

  const field = (name: string): HTMLInputElement => {
    const input = element.querySelector<HTMLInputElement>(`input[name="${name}"]`)
    if (!input) throw new Error(`the login form has no ${name} field`)
    return input
  }

  return { element, username: field('username'), password: field('password') }
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the session gate', () => {
  /**
   * The state a wiki starts in and the desktop app spends its life in. If a
   * login page ever appears here, accounts have stopped being opt-in.
   */
  it('shows the app and no login page on a wiki with no accounts', async () => {
    const screen = await mount({
      authentication_required: false,
      authenticated: true,
      user: null,
    })

    await waitFor(() => expect(screen.getByText('the dashboard')).toBeTruthy())
    expect(screen.queryByText('Sign in')).toBeNull()
  })

  it('shows the app when this browser is signed in', async () => {
    const screen = await mount({
      authentication_required: true,
      authenticated: true,
      user: { username: 'tim', display_name: 'Tim', role: 'owner' },
    })

    await waitFor(() => expect(screen.getByText('the dashboard')).toBeTruthy())
  })

  it('shows the login page instead of the app when it is not', async () => {
    const screen = await mount({
      authentication_required: true,
      authenticated: false,
      user: null,
    })

    await waitFor(() => expect(screen.getByText('Sign in')).toBeTruthy())
    expect(screen.queryByText('the dashboard')).toBeNull()
  })

  /**
   * A server that is not answering is not a sign-in problem, and a login form
   * nobody can submit is a worse answer than saying what is wrong.
   */
  it('reports an unreachable server rather than asking for a password', async () => {
    const screen = await mount(
      new ApiError('network_error', 'Could not reach the server', 0),
    )

    await waitFor(() => expect(screen.getByText(/network_error/)).toBeTruthy())
    expect(screen.queryByText('Sign in')).toBeNull()
  })

  /**
   * Signing in swaps the gate's contents with no navigation at all, which is
   * what lets the address somebody arrived at survive the sign-in.
   */
  it('replaces the login page with the app once a sign-in succeeds', async () => {
    const screen = await mount({
      authentication_required: true,
      authenticated: false,
      user: null,
    })
    await waitFor(() => expect(screen.getByText('Sign in')).toBeTruthy())

    api.login.mockResolvedValue({
      user: { username: 'tim', display_name: 'Tim', role: 'owner' },
      token: '3f2a',
      expires: '2026-09-18T10:00:00Z',
    })

    const login = form(screen.container)
    fireEvent.input(login.username, { target: { value: 'tim' } })
    fireEvent.input(login.password, { target: { value: 'correct horse battery' } })
    fireEvent.submit(login.element)

    await waitFor(() => expect(screen.getByText('the dashboard')).toBeTruthy())
    expect(api.login).toHaveBeenCalledWith({
      username: 'tim',
      password: 'correct horse battery',
    })
  })

  it('keeps the form on screen and says why when the password is wrong', async () => {
    const screen = await mount({
      authentication_required: true,
      authenticated: false,
      user: null,
    })
    await waitFor(() => expect(screen.getByText('Sign in')).toBeTruthy())

    api.login.mockRejectedValue(
      new ApiError('invalid_credentials', 'incorrect username or password', 401),
    )

    const login = form(screen.container)
    fireEvent.input(login.username, { target: { value: 'tim' } })
    fireEvent.input(login.password, { target: { value: 'wrong' } })
    fireEvent.submit(login.element)

    await waitFor(() =>
      expect(screen.getByText('incorrect username or password')).toBeTruthy(),
    )
    expect(screen.queryByText('the dashboard')).toBeNull()
    // The password is cleared and the username is not: retyping the part that
    // was probably right is busywork.
    expect(form(screen.container).password.value).toBe('')
    expect(form(screen.container).username.value).toBe('tim')
  })

  /**
   * The server's prose for this one is aimed at an operator. It is also the one
   * failure a person filling in the form genuinely cannot fix themselves.
   */
  it('rewords an account with no password into something actionable', async () => {
    const screen = await mount({
      authentication_required: true,
      authenticated: false,
      user: null,
    })
    await waitFor(() => expect(screen.getByText('Sign in')).toBeTruthy())

    api.login.mockRejectedValue(
      new ApiError('no_password_set', 'the account alice has no password set', 409),
    )

    fireEvent.submit(form(screen.container).element)

    await waitFor(() => expect(screen.getByText(/An owner has to set one/)).toBeTruthy())
  })
})
