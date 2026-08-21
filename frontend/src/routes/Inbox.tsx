import {
  For,
  Show,
  createEffect,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js'
import { useSearchParams } from '@solidjs/router'
import {
  affirmIdea,
  archiveCapture,
  createCapture,
  deleteCapture,
  dismissIdea,
  listCaptures,
  listIdeas,
  restoreCapture,
} from '../api/client'
import type { CaptureView } from '../api/client'
import { Async, ErrorNotice } from '../components/Async'
import Candidates from '../components/Candidates'
import Rediscovery, {
  chooseRediscovery,
  rediscovery as sharedRediscovery,
} from '../components/Rediscovery'
import type { RediscoveryState } from '../components/Rediscovery'
import { formatDate } from './PageDetail'

/** How many captures one page of the inbox holds. */
const PAGE_SIZE = 50

/** How long typing settles before the search reaches the URL and the API. */
const SEARCH_DELAY_MS = 250

/** What the inbox is showing, and what that means to the API. */
const VIEWS = {
  inbox: { label: 'Inbox', archived: false as boolean | undefined },
  archived: { label: 'Archived', archived: true as boolean | undefined },
  all: { label: 'Everything', archived: undefined as boolean | undefined },
}

type ViewName = keyof typeof VIEWS

/**
 * Somewhere to put a thought down.
 *
 * The text field is the first thing on the screen and the only thing a capture
 * needs. No title, no slug, no tag, no decision about where it belongs: those
 * are the questions that stop somebody writing the thought down at all, and
 * every one of them can be answered later or never.
 *
 * Saving and analyzing are two requests on purpose. The capture is on disk
 * before anything looks at it, so a slow or broken analyzer can annoy you but
 * cannot lose what you typed.
 */
export default function Inbox(props: { rediscovery?: RediscoveryState }) {
  // Its store as an optional prop purely so tests get a fresh one; the app
  // always uses the shared singleton, exactly as the timer and pin menus do.
  const rediscovery = () => props.rediscovery ?? sharedRediscovery

  const [searchParams, setSearchParams] = useSearchParams()
  const [draft, setDraft] = createSignal('')
  const [saving, setSaving] = createSignal(false)
  const [failure, setFailure] = createSignal<unknown>()
  const [reloads, setReloads] = createSignal(0)
  const [captured, setCaptured] = createSignal<CaptureView>()
  const [opened, setOpened] = createSignal<string>()
  const [search, setSearch] = createSignal(first(searchParams.q) ?? '')

  // Fixed once, rather than read on every render: the rediscovery card is meant
  // to be the same one all day, and a clock consulted continuously would be a
  // clock that could change it mid-session.
  const today = new Date()

  let field: HTMLTextAreaElement | undefined

  // Only when somebody asked to capture. Arriving here to read the inbox should
  // not throw a keyboard over half a phone screen.
  onMount(() => {
    if (first(searchParams.capture)) field?.focus()
  })

  createEffect(() => {
    const typed = search().trim()
    const timer = setTimeout(
      () => setSearchParams({ q: typed || undefined }, { replace: true }),
      SEARCH_DELAY_MS,
    )
    onCleanup(() => clearTimeout(timer))
  })

  const view = (): ViewName => {
    const name = first(searchParams.show)
    return name && name in VIEWS ? (name as ViewName) : 'inbox'
  }

  const query = () => ({
    q: first(searchParams.q) ?? '',
    show: view(),
    reloads: reloads(),
  })

  const [captures] = createResource(query, ({ q, show }) =>
    listCaptures({
      // Dropped when empty rather than sent as `q=`, which the API reads as a
      // search for nothing and answers with nothing.
      q: q || undefined,
      archived: VIEWS[show].archived,
      limit: PAGE_SIZE,
    }),
  )

  /*
    Dormant threads, for the one card at the top. Its own request rather than a
    field on the inbox, because it asks a different question of a different
    tree, and because a rediscovery that failed to load must not be able to take
    the capture field down with it.
  */
  const [dormant] = createResource(reloads, () =>
    listIdeas({ state: 'dormant', limit: 200 }),
  )

  const card = () => {
    if (rediscovery().answered()) return undefined
    const list = dormant.error ? undefined : dormant.latest
    return list ? chooseRediscovery(list.ideas, today) : undefined
  }

  const reload = () => setReloads((count) => count + 1)

  const guard = async (work: () => Promise<unknown>) => {
    setFailure(undefined)
    try {
      await work()
      reload()
    } catch (error) {
      setFailure(error)
    }
  }

  /**
   * Answer the rediscovery card, and put it away.
   *
   * Only once the decision is written. A dismissal that never reached the server
   * has not suppressed anything, and taking the card away would leave nothing to
   * press again.
   */
  const answerCard = (work: () => Promise<unknown>) =>
    void guard(async () => {
      await work()
      rediscovery().answer()
    })

  /**
   * Save what was typed, and only then clear the field.
   *
   * The order is the whole point. A form that empties optimistically and then
   * fails has thrown away the one copy of a thought that existed, and the
   * thought is the entire product.
   */
  const capture = async () => {
    const text = draft().trim()
    if (!text || saving()) return

    setSaving(true)
    setFailure(undefined)
    try {
      const saved = await createCapture({ text })
      setDraft('')
      setCaptured(saved)
      // Deliberately *not* opening the same capture's row below. The panel
      // above is already asking about it, and two of them would be two
      // identical requests and two places to click Connect.
      setOpened(undefined)
      reload()
      // Focus goes back where it was, because the next thought usually follows
      // the last one within a few seconds.
      field?.focus()
    } catch (error) {
      setFailure(error)
    } finally {
      setSaving(false)
    }
  }

  const keys = (event: KeyboardEvent) => {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault()
      void capture()
    }
  }

  return (
    <div class="flex flex-col gap-6">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-semibold">Inbox</h1>
        <input
          class="input input-bordered input-sm min-w-56 grow sm:max-w-md"
          type="search"
          placeholder="Search your captures"
          aria-label="Search captures"
          value={search()}
          onInput={(event) => setSearch(event.currentTarget.value)}
        />
      </header>

      <form
        class="flex flex-col gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          void capture()
        }}
      >
        <textarea
          ref={field}
          class="textarea textarea-bordered min-h-28 w-full text-base"
          placeholder="What are you thinking?"
          aria-label="Capture a thought"
          value={draft()}
          onInput={(event) => setDraft(event.currentTarget.value)}
          onKeyDown={keys}
        />
        <div class="flex flex-wrap items-center gap-3">
          <button
            class="btn btn-primary"
            type="submit"
            disabled={!draft().trim() || saving()}
          >
            <Show when={saving()}>
              <span class="loading loading-spinner loading-xs" />
            </Show>
            Capture
          </button>
          <span class="text-xs opacity-50">Ctrl+Enter saves it too.</span>
        </div>
      </form>

      <Show when={failure()}>
        <ErrorNotice error={failure()} />
      </Show>

      {/*
        Suggestions for what was just written, fetched after it was saved and
        never before. Nothing here has connected anything.
      */}
      <Show when={captured()}>
        {(saved) => (
          <section class="card bg-base-100 shadow">
            <div class="card-body gap-3">
              <div class="flex flex-wrap items-baseline justify-between gap-2">
                <h2 class="card-title text-base">Saved. Does it belong with anything?</h2>
                <button
                  class="btn btn-ghost btn-xs"
                  onClick={() => {
                    setCaptured(undefined)
                    setOpened(undefined)
                  }}
                >
                  Dismiss
                </button>
              </div>
              <Candidates
                capture={saved()}
                onChanged={() => {
                  setCaptured(undefined)
                  setOpened(undefined)
                  reload()
                }}
              />
            </div>
          </section>
        )}
      </Show>

      <Show when={card()}>
        {(idea) => (
          <Rediscovery
            idea={idea()}
            onAffirm={() => answerCard(() => affirmIdea(idea().id))}
            onDismiss={() => answerCard(() => dismissIdea(idea().id))}
          />
        )}
      </Show>

      <div class="flex flex-wrap items-center gap-2">
        <div role="tablist" class="tabs tabs-box tabs-sm">
          <For each={Object.entries(VIEWS)}>
            {([name, config]) => (
              <button
                role="tab"
                class="tab"
                classList={{ 'tab-active': view() === name }}
                onClick={() =>
                  setSearchParams({ show: name === 'inbox' ? undefined : name })
                }
              >
                {config.label}
              </button>
            )}
          </For>
        </div>
      </div>

      <Async resource={captures}>
        {(data) => (
          <section class="card bg-base-100 shadow">
            <div class="card-body gap-3">
              <h2 class="card-title text-base">
                {VIEWS[view()].label}
                <span class="badge badge-ghost badge-sm">{data.total}</span>
              </h2>

              <ul class="flex flex-col divide-y divide-base-200">
                <For
                  each={data.captures}
                  fallback={
                    <li class="py-4 text-sm opacity-60">
                      <Show
                        when={!first(searchParams.q)}
                        fallback="Nothing here matches. Clear the search to widen it."
                      >
                        Nothing captured yet. The field above is the whole of it.
                      </Show>
                    </li>
                  }
                >
                  {(entry) => (
                    <li class="py-3">
                      <Capture
                        capture={entry}
                        open={opened() === entry.id}
                        onToggle={() =>
                          setOpened(opened() === entry.id ? undefined : entry.id)
                        }
                        onArchive={() => void guard(() => archiveCapture(entry.id))}
                        onRestore={() => void guard(() => restoreCapture(entry.id))}
                        onDelete={() => void guard(() => deleteCapture(entry.id))}
                        onChanged={() => {
                          setOpened(undefined)
                          reload()
                        }}
                      />
                    </li>
                  )}
                </For>
              </ul>

              <Show when={data.total > data.captures.length}>
                <p class="text-xs opacity-60">
                  Showing the {data.captures.length} most recent of {data.total}. Search
                  to narrow it.
                </p>
              </Show>
            </div>
          </section>
        )}
      </Async>
    </div>
  )
}

/** One capture, and what can be done with it without leaving the list. */
function Capture(props: {
  capture: CaptureView
  open: boolean
  onToggle: () => void
  onArchive: () => void
  onRestore: () => void
  onDelete: () => void
  onChanged: () => void
}) {
  const [confirming, setConfirming] = createSignal(false)

  return (
    <div class="flex flex-col gap-2">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <p class="text-sm whitespace-pre-wrap">{props.capture.text}</p>
          <div class="pt-1 text-xs opacity-60">
            {formatDate(props.capture.created)}
            <Show when={props.capture.archived}>
              <span class="badge badge-ghost badge-xs ml-2">archived</span>
            </Show>
          </div>
        </div>

        <div class="flex flex-wrap items-center gap-1">
          <button class="btn btn-ghost btn-xs" onClick={props.onToggle}>
            {props.open ? 'Hide' : 'Add to idea'}
          </button>
          <Show
            when={props.capture.archived}
            fallback={
              <button class="btn btn-ghost btn-xs" onClick={props.onArchive}>
                Archive
              </button>
            }
          >
            <button class="btn btn-ghost btn-xs" onClick={props.onRestore}>
              Restore
            </button>
          </Show>
          {/*
            Deletion is the one thing here that is not reversible, so it asks.
            The refusal that protects an idea standing on this capture comes
            from the server, and arrives as an error saying which thread.
          */}
          <Show
            when={confirming()}
            fallback={
              <button
                class="btn btn-ghost btn-xs text-error"
                aria-label="Delete this capture"
                onClick={() => setConfirming(true)}
              >
                Delete
              </button>
            }
          >
            <button
              class="btn btn-error btn-xs"
              onClick={() => {
                setConfirming(false)
                props.onDelete()
              }}
            >
              Delete for good
            </button>
            <button class="btn btn-ghost btn-xs" onClick={() => setConfirming(false)}>
              Keep
            </button>
          </Show>
        </div>
      </div>

      <Show when={props.open}>
        <div class="rounded-box bg-base-200 p-3">
          <Candidates capture={props.capture} onChanged={props.onChanged} />
        </div>
      </Show>
    </div>
  )
}

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
