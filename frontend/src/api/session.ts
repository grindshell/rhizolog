/**
 * Who this browser is, as one shared answer.
 *
 * Every part of the app that cares — the gate in front of the routes, the
 * account menu, the accounts screen — reads the same resource, so signing in or
 * out updates all of them at once and nothing has to be told twice.
 *
 * It lives in a `createRoot` at module scope rather than in a context provider,
 * because the thing that needs it first is the gate *around* the router. A
 * provider would have to sit outside the router to be visible there, at which
 * point it is a global with extra steps.
 *
 * ## `authentication_required: false` is the ordinary case
 *
 * A wiki with no accounts is open, and this whole module is inert: no login
 * page, no account menu, nothing refused. See `knowledge-base/accounts.md`.
 */
import { createResource, createRoot } from 'solid-js'
import type { SessionStatus, UserView } from './client'
import { login, logout, onUnauthorized, session } from './client'

const store = createRoot(() => {
  const [state, { refetch, mutate }] = createResource<SessionStatus>(() => session())
  return { state, refetch, mutate }
})

/**
 * The current session. `undefined` while the first call is in flight — which is
 * why the gate shows nothing rather than guessing, since guessing "open" would
 * flash the whole dashboard at somebody who is not signed in.
 */
export const sessionState = store.state

/** Ask the server again. */
export const refreshSession = store.refetch

/** What a signed-out state looks like on a wiki that has accounts. */
const SIGNED_OUT: SessionStatus = {
  authentication_required: true,
  authenticated: false,
  user: null,
}

/**
 * Sign in, and update every part of the app that shows who you are.
 *
 * Throws `ApiError` on a refusal, so the form can show it. The token in the
 * response is deliberately ignored: the browser's session is the `HttpOnly`
 * cookie that arrived with it, and putting a copy anywhere JavaScript can read
 * would undo the reason it is `HttpOnly`.
 */
export async function signIn(username: string, password: string): Promise<UserView> {
  const result = await login({ username, password })

  store.mutate({
    authentication_required: true,
    authenticated: true,
    user: result.user,
  })

  return result.user
}

/**
 * Sign out.
 *
 * The local state is cleared even if the request failed. A `logout` that did not
 * reach the server leaves a session alive there, but continuing to show somebody
 * as signed in when they have asked not to be is the worse of the two — and the
 * next request will find out either way.
 */
export async function signOut(): Promise<void> {
  try {
    await logout()
  } finally {
    store.mutate(SIGNED_OUT)
  }
}

/**
 * A session can end without this tab being told: it expires, an owner deletes
 * the account, or a password change somewhere else ends every session it had.
 * The first anybody here knows about it is a `401` on an ordinary request, so
 * that is what turns the app back into a login page.
 *
 * Registered once, at module load, rather than per component — a handler that
 * came and went with a mounted route would miss exactly the requests made while
 * navigating.
 */
onUnauthorized(() => {
  // Only a change, so a burst of parallel requests all failing at once does not
  // re-render the gate once per request.
  if (store.state()?.authenticated !== false) {
    store.mutate(SIGNED_OUT)
  }
})
