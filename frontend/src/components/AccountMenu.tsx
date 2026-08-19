import { createSignal, Show } from 'solid-js'
import { A } from '@solidjs/router'
import { sessionState, signOut } from '../api/session'

/**
 * Who you are signed in as, and the way out.
 *
 * Renders **nothing at all** on a wiki with no accounts. That is the point
 * rather than an edge case: the single-user local wiki should not grow a menu
 * telling it that it is not signed in as anybody, and an empty account menu
 * would be the one visible trace of a feature it does not use.
 */
export default function AccountMenu() {
  const user = () => sessionState()?.user ?? undefined
  const [busy, setBusy] = createSignal(false)

  async function leave() {
    setBusy(true)
    try {
      // The gate is watching the session; there is nowhere to navigate to.
      await signOut()
    } finally {
      setBusy(false)
    }
  }

  return (
    <Show when={user()}>
      {(account) => (
        <div class="dropdown dropdown-end">
          <div tabindex="0" role="button" class="btn btn-ghost btn-sm">
            <span class="max-w-32 truncate">{account().display_name}</span>
            <Show when={account().role === 'owner'}>
              <span class="badge badge-ghost badge-sm">owner</span>
            </Show>
          </div>
          <ul
            tabindex="0"
            class="dropdown-content menu z-10 mt-2 w-56 rounded-box bg-base-100 p-2 shadow"
          >
            <li class="menu-title">
              <span class="font-mono">{account().username}</span>
            </li>
            <li>
              <A href="/accounts">Accounts</A>
            </li>
            <li>
              <button type="button" onClick={leave} disabled={busy()}>
                <Show when={busy()}>
                  <span class="loading loading-spinner loading-xs" />
                </Show>
                Sign out
              </button>
            </li>
          </ul>
        </div>
      )}
    </Show>
  )
}
