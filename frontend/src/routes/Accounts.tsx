import { createResource, createSignal, For, Show } from 'solid-js'
import type { Role, UserView } from '../api/client'
import { ApiError, createUser, deleteUser, listUsers, patchUser } from '../api/client'
import { refreshSession, sessionState } from '../api/session'
import { Async, ErrorNotice } from '../components/Async'

/**
 * Accounts, and the switch that turns authentication on.
 *
 * The screen has two quite different jobs depending on which side of that switch
 * the wiki is on, and it is one screen rather than two because they are the same
 * question asked at different times.
 *
 * - **No accounts.** The wiki is open. The panel explains what creating the
 *   first account does — closes the wiki, immediately, including to the person
 *   filling the form in — and then does it.
 * - **Accounts.** The list, plus whatever this account is allowed to change.
 */
export default function Accounts() {
  const [users, { refetch }] = createResource(() => listUsers())

  const viewer = () => sessionState()?.user ?? undefined
  /** True on an open wiki too, where there is nobody to withhold it from. */
  const mayAdminister = (): boolean => {
    const state = sessionState()
    if (state === undefined) return false
    return !state.authentication_required || state.user?.role === 'owner'
  }

  return (
    <div class="flex flex-col gap-6">
      <div>
        <h1 class="text-2xl font-semibold">Accounts</h1>
        <p class="text-sm opacity-70">
          Accounts are files under <code>.rhizolog/users/</code>. A wiki with none
          is open to anybody who can reach it.
        </p>
      </div>

      <Async resource={users}>
        {(data) => (
          <Show
            when={data.total > 0}
            fallback={<FirstAccount onCreated={() => void refreshSession()} />}
          >
            <div class="flex flex-col gap-6">
              <AccountList
                users={data.users}
                viewer={viewer()}
                mayAdminister={mayAdminister()}
                onChanged={() => void refetch()}
              />
              <Show when={mayAdminister()}>
                <NewAccount onCreated={() => void refetch()} />
              </Show>
              <Show when={viewer()}>
                {(account) => <ChangePassword username={account().username} />}
              </Show>
            </div>
          </Show>
        )}
      </Async>
    </div>
  )
}

/** The list, with whatever controls this account is allowed. */
function AccountList(props: {
  users: UserView[]
  viewer: UserView | undefined
  mayAdminister: boolean
  onChanged: () => void
}) {
  const [error, setError] = createSignal<unknown>()

  const owners = () => props.users.filter((user) => user.role === 'owner').length

  async function act(work: () => Promise<unknown>) {
    setError(undefined)
    try {
      await work()
      props.onChanged()
    } catch (cause) {
      setError(cause)
    }
  }

  return (
    <div class="flex flex-col gap-3">
      <Show when={error()}>{(problem) => <ErrorNotice error={problem()} />}</Show>

      <div class="overflow-x-auto rounded-box border border-base-300">
        <table class="table">
          <thead>
            <tr>
              <th>Account</th>
              <th>Role</th>
              <th>Created</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <For each={props.users}>
              {(user) => {
                const isSelf = () => user.username === props.viewer?.username
                // The server refuses both of these with `last_owner`; the UI
                // says so first rather than offering a button that fails.
                const isLastOwner = () => user.role === 'owner' && owners() === 1

                return (
                  <tr>
                    <td>
                      <div class="font-medium">
                        {user.display_name}
                        <Show when={isSelf()}>
                          <span class="ml-2 badge badge-ghost badge-sm">you</span>
                        </Show>
                      </div>
                      <div class="font-mono text-xs opacity-60">{user.username}</div>
                      <Show when={!user.has_password}>
                        <div class="text-xs text-warning">
                          No password set — cannot sign in
                        </div>
                      </Show>
                    </td>
                    <td>
                      <Show
                        when={props.mayAdminister && !isLastOwner()}
                        fallback={<span class="badge badge-ghost">{user.role}</span>}
                      >
                        <select
                          class="select select-bordered select-sm"
                          value={user.role}
                          onChange={(event) =>
                            void act(() =>
                              patchUser(user.username, {
                                role: event.currentTarget.value as Role,
                              }),
                            )
                          }
                        >
                          <option value="owner">owner</option>
                          <option value="member">member</option>
                        </select>
                      </Show>
                    </td>
                    <td class="text-sm opacity-70">
                      {new Date(user.created).toLocaleDateString()}
                    </td>
                    <td class="text-right">
                      <Show when={props.mayAdminister && !isLastOwner()}>
                        <button
                          type="button"
                          class="btn btn-ghost btn-sm text-error"
                          onClick={() => {
                            if (
                              !confirm(
                                `Delete the account ${user.username}? Its pages are left alone; ` +
                                  `it is signed out everywhere immediately.`,
                              )
                            ) {
                              return
                            }
                            void act(() => deleteUser(user.username))
                          }}
                        >
                          Delete
                        </button>
                      </Show>
                    </td>
                  </tr>
                )
              }}
            </For>
          </tbody>
        </table>
      </div>

      <Show when={owners() === 1}>
        <p class="text-xs opacity-60">
          The only owner cannot be deleted or demoted — that would leave a wiki
          nobody can administer, recoverable only by editing files on the server.
        </p>
      </Show>
    </div>
  )
}

/** The bootstrap: the first account, on a wiki that is currently open. */
function FirstAccount(props: { onCreated: () => void }) {
  return (
    <div class="flex flex-col gap-4">
      <div role="alert" class="alert alert-warning">
        <div class="text-sm">
          <p class="font-semibold">This wiki is open.</p>
          <p>
            Anybody who can reach it can read and write every page. Creating the
            first account closes it — including for this browser, which will be
            asked to sign in straight away. The first account is always an owner.
          </p>
        </div>
      </div>
      <AccountForm
        submitLabel="Create the first account"
        onCreated={props.onCreated}
        showRole={false}
      />
    </div>
  )
}

/** Adding an account to a wiki that already has some. */
function NewAccount(props: { onCreated: () => void }) {
  return (
    <div class="flex flex-col gap-3">
      <h2 class="text-lg font-semibold">Add an account</h2>
      <AccountForm submitLabel="Create account" onCreated={props.onCreated} showRole />
    </div>
  )
}

function AccountForm(props: {
  submitLabel: string
  showRole: boolean
  onCreated: () => void
}) {
  const [username, setUsername] = createSignal('')
  const [displayName, setDisplayName] = createSignal('')
  const [password, setPassword] = createSignal('')
  const [role, setRole] = createSignal<Role>('member')
  const [busy, setBusy] = createSignal(false)
  const [error, setError] = createSignal<unknown>()

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (busy()) return

    setBusy(true)
    setError(undefined)
    try {
      await createUser({
        username: username(),
        password: password(),
        display_name: displayName() || undefined,
        role: props.showRole ? role() : undefined,
      })
      setUsername('')
      setDisplayName('')
      setPassword('')
      props.onCreated()
    } catch (cause) {
      setError(cause)
    } finally {
      setBusy(false)
    }
  }

  return (
    <form class="flex flex-col gap-3 rounded-box border border-base-300 p-4" onSubmit={submit}>
      <Show when={error()}>{(problem) => <ErrorNotice error={problem()} />}</Show>

      <div class="grid gap-3 sm:grid-cols-2">
        <label class="form-control">
          <div class="label">
            <span class="label-text">Username</span>
          </div>
          <input
            class="input input-bordered"
            required
            autocapitalize="none"
            autocorrect="off"
            spellcheck={false}
            placeholder="tim"
            value={username()}
            onInput={(event) => setUsername(event.currentTarget.value)}
          />
          <div class="label">
            <span class="label-text-alt opacity-60">
              Lowercase letters, digits, <code>-</code> and <code>_</code>. It is
              also the filename.
            </span>
          </div>
        </label>

        <label class="form-control">
          <div class="label">
            <span class="label-text">Display name</span>
          </div>
          <input
            class="input input-bordered"
            placeholder="Optional"
            value={displayName()}
            onInput={(event) => setDisplayName(event.currentTarget.value)}
          />
        </label>

        <label class="form-control">
          <div class="label">
            <span class="label-text">Password</span>
          </div>
          <input
            class="input input-bordered"
            type="password"
            autocomplete="new-password"
            required
            value={password()}
            onInput={(event) => setPassword(event.currentTarget.value)}
          />
          <div class="label">
            <span class="label-text-alt opacity-60">
              At least 8 characters. No other rules.
            </span>
          </div>
        </label>

        <Show when={props.showRole}>
          <label class="form-control">
            <div class="label">
              <span class="label-text">Role</span>
            </div>
            <select
              class="select select-bordered"
              value={role()}
              onChange={(event) => setRole(event.currentTarget.value as Role)}
            >
              <option value="member">member</option>
              <option value="owner">owner</option>
            </select>
          </label>
        </Show>
      </div>

      <div>
        <button class="btn btn-primary" type="submit" disabled={busy()}>
          <Show when={busy()}>
            <span class="loading loading-spinner loading-sm" />
          </Show>
          {props.submitLabel}
        </button>
      </div>
    </form>
  )
}

/**
 * Change your own password.
 *
 * Deliberately blunt about the consequence: the server ends every session this
 * account has, this one included, so the next thing that happens is the login
 * page. That is what makes it a revocation rather than a rename.
 */
function ChangePassword(props: { username: string }) {
  const [password, setPassword] = createSignal('')
  const [busy, setBusy] = createSignal(false)
  const [error, setError] = createSignal<unknown>()

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    if (busy()) return

    setBusy(true)
    setError(undefined)
    try {
      await patchUser(props.username, { password: password() })
      setPassword('')
      // No success message and nothing to navigate to: every session just
      // ended, so the next request 401s and the gate shows the login page.
      // Saying "saved!" a moment before that would be the confusing version.
    } catch (cause) {
      setError(cause)
      setBusy(false)
      return
    }
    setBusy(false)
  }

  return (
    <form class="flex flex-col gap-3 rounded-box border border-base-300 p-4" onSubmit={submit}>
      <h2 class="text-lg font-semibold">Change your password</h2>
      <p class="text-sm opacity-70">
        This signs <code>{props.username}</code> out everywhere, including here.
        You will be asked to sign in again.
      </p>

      <Show when={error()}>
        {(problem) => (
          <ErrorNotice
            error={
              problem() instanceof ApiError
                ? problem()
                : new ApiError('unknown_error', String(problem()), 0)
            }
          />
        )}
      </Show>

      <label class="form-control max-w-sm">
        <div class="label">
          <span class="label-text">New password</span>
        </div>
        <input
          class="input input-bordered"
          type="password"
          autocomplete="new-password"
          required
          value={password()}
          onInput={(event) => setPassword(event.currentTarget.value)}
        />
      </label>

      <div>
        <button class="btn" type="submit" disabled={busy()}>
          <Show when={busy()}>
            <span class="loading loading-spinner loading-sm" />
          </Show>
          Change password
        </button>
      </div>
    </form>
  )
}
