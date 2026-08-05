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

/** Minimal loading / error / data switch over a `createResource` result. */
export function Async<T>(props: {
  resource: Resource<T>
  children: (value: T) => JSX.Element
}) {
  return (
    <Switch>
      <Match when={props.resource.loading}>
        <div class="flex items-center gap-3 py-6 text-base-content/60">
          <span class="loading loading-spinner loading-sm" />
          Loading...
        </div>
      </Match>
      <Match when={props.resource.error}>
        <ErrorNotice error={props.resource.error} />
      </Match>
      <Match when={props.resource() !== undefined}>
        {props.children(props.resource() as T)}
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
