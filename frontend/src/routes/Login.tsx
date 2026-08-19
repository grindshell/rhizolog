import { createSignal, Show } from 'solid-js'
import { ApiError } from '../api/client'
import { signIn } from '../api/session'

/**
 * The sign-in page.
 *
 * Rendered **in place of** the whole app by `SessionGate`, rather than living at
 * a `/login` route. That is the interesting decision here: a redirect to
 * `/login` throws away the URL somebody arrived at, and this wiki's URLs are
 * pages — a link to `/pages/notes/rust/async` sent to a colleague should still
 * open that page after they sign in, not the dashboard. Signing in re-renders
 * the router at the address the browser is already on, so there is nothing to
 * remember and nothing to restore.
 *
 * It follows that this component is never reachable on a wiki with no accounts.
 * There is nothing to sign in to there, and the gate never asks for it.
 */
export default function Login() {
  const [username, setUsername] = createSignal('')
  const [password, setPassword] = createSignal('')
  const [busy, setBusy] = createSignal(false)
  const [error, setError] = createSignal<ApiError | undefined>()

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (busy()) return

    setBusy(true)
    setError(undefined)

    try {
      // No navigation on success. The gate is watching the session and swaps
      // this page for the app the moment it changes.
      await signIn(username(), password())
    } catch (cause) {
      setError(
        cause instanceof ApiError
          ? cause
          : new ApiError('unknown_error', String(cause), 0),
      )
      setPassword('')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class="flex min-h-screen items-center justify-center bg-base-200 p-6">
      <div class="card w-full max-w-sm bg-base-100 shadow-sm">
        <form class="card-body gap-4" onSubmit={submit}>
          <div>
            <h1 class="text-2xl font-semibold">Rhizolog</h1>
            <p class="text-sm opacity-70">This wiki requires you to sign in.</p>
          </div>

          <Show when={error()}>
            {(problem) => (
              <div role="alert" class="alert alert-error text-sm">
                <span>{message(problem())}</span>
              </div>
            )}
          </Show>

          <label class="form-control w-full">
            <div class="label">
              <span class="label-text">Username</span>
            </div>
            <input
              class="input input-bordered w-full"
              name="username"
              autocomplete="username"
              autocapitalize="none"
              autocorrect="off"
              spellcheck={false}
              required
              // Autofocus is right for a page whose entire content is this form
              // and which nobody arrives at wanting to read.
              autofocus
              value={username()}
              onInput={(event) => setUsername(event.currentTarget.value)}
            />
          </label>

          <label class="form-control w-full">
            <div class="label">
              <span class="label-text">Password</span>
            </div>
            <input
              class="input input-bordered w-full"
              name="password"
              type="password"
              autocomplete="current-password"
              required
              value={password()}
              onInput={(event) => setPassword(event.currentTarget.value)}
            />
          </label>

          <button class="btn btn-primary" type="submit" disabled={busy()}>
            <Show when={busy()}>
              <span class="loading loading-spinner loading-sm" />
            </Show>
            Sign in
          </button>
        </form>
      </div>
    </div>
  )
}

/**
 * What to put in front of somebody who could not sign in.
 *
 * The server says the same thing for a wrong password and an account that does
 * not exist, deliberately — telling them apart is a list of the accounts on the
 * instance, one guess at a time. This does not undo that by guessing.
 *
 * The two cases worth rewording are the ones where the server's prose is aimed
 * at an operator: an account with no password, and the transport being down.
 */
function message(error: ApiError): string {
  if (error.code === 'no_password_set') {
    return 'That account has no password set. An owner has to set one before it can be used.'
  }
  if (error.isTransport) {
    return 'Could not reach the server. Is it still running?'
  }
  return error.message
}
