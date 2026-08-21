import type { JSX, Resource } from 'solid-js'
import { Match, Show, Switch } from 'solid-js'
import { ApiError } from '../api/client'

/** Renders whatever the API said went wrong, keyed on the stable `code`. */
export function ErrorNotice(props: { error: unknown }) {
  const apiError = (): ApiError | undefined =>
    props.error instanceof ApiError ? props.error : undefined
  const message = (): string =>
    props.error instanceof Error ? props.error.message : String(props.error)

  return (
    <div role="alert" class="alert alert-error">
      <div>
        <div class="font-mono text-sm font-semibold">
          {apiError()?.code ?? 'unknown_error'}
          <Show when={apiError()?.status}>{(status) => <> · HTTP {status()}</>}</Show>
        </div>
        <div class="text-sm">{message()}</div>
        <Show when={apiError()?.details !== undefined}>
          <pre class="mt-2 overflow-x-auto text-xs">
            {JSON.stringify(apiError()?.details, null, 2)}
          </pre>
        </Show>
        <Show when={apiError()?.isTransport}>
          <div class="text-xs opacity-80">
            Is the backend running on http://127.0.0.1:3000?
          </div>
        </Show>
      </div>
    </div>
  )
}

/**
 * Minimal loading / error / data switch over a `createResource` result.
 *
 * A screen that already has an answer keeps showing it while the next one is
 * fetched. Re-reading after a decision is the ordinary case on every screen that
 * can change something, and blanking the whole route to a spinner each time
 * makes an idea flash out of existence for as long as the round trip takes.
 * There is a quiet line saying a refresh is happening instead.
 */
export function Async<T>(props: {
  resource: Resource<T>
  children: (value: T) => JSX.Element
}) {
  /**
   * The last answer, or nothing if the most recent attempt failed.
   *
   * `latest` rather than the resource call, so a refetch does not count as
   * having no value, and guarded because `latest` *rethrows* when the resource
   * errored. Unguarded it would throw out of here and take the app shell with
   * it, which is the same guard the timer store carries and for the same reason.
   */
  const settled = () => (props.resource.error ? undefined : props.resource.latest)

  return (
    <Switch>
      <Match when={props.resource.loading && settled() === undefined}>
        <div class="flex items-center gap-3 py-6 text-base-content/60">
          <span class="loading loading-spinner loading-sm" />
          Loading...
        </div>
      </Match>
      {/*
        A failed refresh shows the failure rather than the answer it used to
        have. Stale data with nothing saying so is the worse of the two: it
        reads as though the thing you just did worked.
      */}
      <Match when={props.resource.error}>
        <ErrorNotice error={props.resource.error} />
      </Match>
      <Match when={settled() !== undefined}>
        <Show when={props.resource.loading}>
          <div class="flex items-center gap-2 pb-2 text-xs text-base-content/50">
            <span class="loading loading-spinner loading-xs" />
            Refreshing...
          </div>
        </Show>
        {props.children(settled() as T)}
      </Match>
    </Switch>
  )
}

/** Raw JSON dump. The scaffold shows shapes; M7 replaces these with real UI. */
export function Json(props: { value: unknown }) {
  return (
    <pre class="mockup-code max-h-96 overflow-auto p-4 text-xs">
      <code>{JSON.stringify(props.value, null, 2)}</code>
    </pre>
  )
}
