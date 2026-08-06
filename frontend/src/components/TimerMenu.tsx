import { For, Show, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { timers as store } from '../api/timers'
import type { TimerStore } from '../api/timers'
import Duration from './Duration'

/**
 * Running timers, in the top bar.
 *
 * A timer you cannot see is a timer you leave running overnight, so this lives
 * in the app shell beside the pins menu and never unmounts. The button itself
 * shows the longest-running timer's clock rather than just a count: the count
 * tells you something is running, the clock tells you whether it should be.
 *
 * It takes its store as an optional prop purely so tests can hand it a fresh
 * one; the app always uses the shared singleton.
 */
export default function TimerMenu(props: { store?: TimerStore }) {
  const timers = () => props.store ?? store
  const [failure, setFailure] = createSignal<string>()
  const [busy, setBusy] = createSignal<string>()

  /**
   * daisyUI dropdowns stay open for as long as something inside them has
   * focus, and a client-side navigation moves no focus — so without this,
   * following a link leaves the menu hanging over the page it took you to.
   */
  const close = () => {
    const focused = document.activeElement
    if (focused instanceof HTMLElement) focused.blur()
  }

  const stop = async (id: string) => {
    setBusy(id)
    setFailure(undefined)
    try {
      await timers().stop(id)
    } catch {
      // The list is what is wrong, so say so and re-read rather than leaving
      // an entry that will not go away.
      setFailure('Could not stop that timer.')
      timers().refresh()
    } finally {
      setBusy(undefined)
    }
  }

  const running = () => timers().running()
  const oldest = () => running()[0]

  return (
    <div class="dropdown dropdown-end">
      <div
        tabindex="0"
        role="button"
        class="btn btn-sm"
        classList={{ 'btn-ghost': running().length === 0, 'btn-secondary': running().length > 0 }}
        aria-label="Running timers"
      >
        <Show when={oldest()} fallback={<>Timers</>}>
          {(timer) => (
            <>
              <span class="inline-block size-2 animate-pulse rounded-full bg-current" />
              <Duration seconds={timer().seconds} runningSince={timer().start} />
              <Show when={running().length > 1}>
                <span class="badge badge-ghost badge-sm">+{running().length - 1}</span>
              </Show>
            </>
          )}
        </Show>
      </div>

      <ul
        tabindex="0"
        class="dropdown-content menu menu-sm bg-base-100 rounded-box z-10 mt-2 w-96 gap-1 p-2 shadow"
      >
        <Show when={failure()}>
          {(message) => <li class="menu-title text-error px-3 py-1 text-xs">{message()}</li>}
        </Show>

        <For
          each={running()}
          fallback={
            <li class="px-3 py-2 text-sm opacity-60">
              <Show
                when={!timers().error()}
                fallback={<span>Could not load running timers.</span>}
              >
                Nothing running. Start one from the Time screen, or from any page.
              </Show>
            </li>
          }
        >
          {(timer) => (
            <li>
              {/*
                Two controls in one row, so the row itself is not the link —
                daisyUI's `menu` would otherwise style the wrapper as the
                clickable element and swallow the stop button's hit area.
              */}
              <div class="flex items-center gap-2 p-0">
                <A
                  class="flex min-w-0 flex-1 flex-col items-start gap-0 px-3 py-2"
                  href="/times"
                  onClick={close}
                >
                  <span class="flex w-full items-baseline justify-between gap-2">
                    <span class="truncate">{timer.name}</span>
                    <Duration
                      class="font-mono text-xs"
                      seconds={timer.seconds}
                      runningSince={timer.start}
                    />
                  </span>
                  <Show when={timer.pages.length > 0}>
                    <span class="w-full truncate font-mono text-xs opacity-60">
                      {timer.pages.map((page) => page.slug).join(', ')}
                    </span>
                  </Show>
                </A>
                <button
                  class="btn btn-ghost btn-xs"
                  aria-label={`Stop ${timer.name}`}
                  title="Stop"
                  disabled={busy() === timer.id}
                  onClick={() => void stop(timer.id)}
                >
                  Stop
                </button>
              </div>
            </li>
          )}
        </For>

        <Show when={running().length > 0}>
          <li>
            <A class="justify-center text-xs" href="/times" onClick={close}>
              Open the time log
            </A>
          </li>
        </Show>
      </ul>
    </div>
  )
}

/**
 * Start or stop a timer for one page, from anywhere that shows a page.
 *
 * Deliberately not a duplicate of the Time screen's form: the activity name is
 * the page's title, because the useful default when you are looking at a page
 * is "time on this". Anything more considered belongs on the Time screen.
 */
export function PageTimerButton(props: {
  slug: string
  title: string
  store?: TimerStore
  onChange?: () => void
}) {
  const timers = () => props.store ?? store
  const [busy, setBusy] = createSignal(false)
  const [failure, setFailure] = createSignal<string>()

  const current = () => timers().forPage(props.slug)[0]

  const toggle = async () => {
    if (busy()) return
    setBusy(true)
    setFailure(undefined)
    try {
      const timer = current()
      if (timer) {
        await timers().stop(timer.id)
      } else {
        await timers().start({ name: props.title, pages: [props.slug] })
      }
      props.onChange?.()
    } catch {
      setFailure('Could not change the timer.')
      timers().refresh()
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <button
        class="btn btn-sm"
        classList={{ 'btn-ghost': !current(), 'btn-secondary': Boolean(current()) }}
        disabled={busy()}
        title={
          current()
            ? 'Stop the timer tracking this page'
            : 'Start a timer tracking this page'
        }
        onClick={() => void toggle()}
      >
        <Show
          when={current()}
          fallback={<>Track time</>}
        >
          {(timer) => (
            <>
              <span class="inline-block size-2 animate-pulse rounded-full bg-current" />
              <Duration seconds={timer().seconds} runningSince={timer().start} />
            </>
          )}
        </Show>
      </button>
      <Show when={failure()}>
        {(message) => <span class="text-error text-xs">{message()}</span>}
      </Show>
    </>
  )
}
