import type { JSX } from 'solid-js'
import { Match, Switch } from 'solid-js'
import { sessionState } from '../api/session'
import { ErrorNotice } from './Async'
import Login from '../routes/Login'

/**
 * Decides whether to show the app or a sign-in page.
 *
 * It wraps the router rather than sitting inside it, which is what lets signing
 * in preserve the address the browser is already on — see `Login`. The cost is
 * that this component cannot use anything router-shaped, which is fine: its
 * whole job is one three-way switch.
 *
 * The three states, in the order they matter:
 *
 * - **Still asking.** Nothing is rendered but a spinner. Guessing "open" here
 *   would flash the entire dashboard at somebody who is not signed in, and
 *   guessing "signed out" would flash a login page at the single user of an
 *   open wiki. Both are one round trip long and both look like a bug.
 * - **Unreachable.** The server is not answering, which is not a sign-in
 *   problem and must not be presented as one — a login form nobody can submit
 *   is a worse answer than saying what is wrong.
 * - **Answered.** Sign-in page, or the app.
 *
 * On a wiki with no accounts this only ever reaches the last one, and the login
 * page is never built.
 */
export default function SessionGate(props: { children: JSX.Element }) {
  const needsSignIn = (): boolean => {
    const state = sessionState()
    return state !== undefined && state.authentication_required && !state.authenticated
  }

  return (
    <Switch>
      <Match when={sessionState.loading}>
        <div class="flex min-h-screen items-center justify-center bg-base-200">
          <span class="loading loading-spinner loading-lg text-base-content/40" />
        </div>
      </Match>

      <Match when={sessionState.error}>
        <div class="mx-auto max-w-lg p-6">
          <ErrorNotice error={sessionState.error} />
        </div>
      </Match>

      <Match when={needsSignIn()}>
        <Login />
      </Match>

      <Match when={sessionState() !== undefined}>{props.children}</Match>
    </Switch>
  )
}
