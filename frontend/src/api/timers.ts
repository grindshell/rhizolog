/**
 * The timers that are running right now, shared by everything that shows them.
 *
 * Three places need this list and all three must agree instantly: the counter
 * in the top bar, the Time screen, and the start/stop button on a page. Giving
 * each its own resource would mean stopping a timer from a page left the top
 * bar still counting.
 *
 * It follows [`pins`](./pins.ts) deliberately — same shape, same
 * `resource.error` guard, same "update from what the server returned rather
 * than refetching". The one thing it adds is polling, because a running timer
 * is state that changes without this tab doing anything: a second tab, an
 * agent, or a hand-edited file can all start or stop one.
 */
import { createResource, createRoot, onCleanup } from 'solid-js'
import { createTime, deleteTime, listTimes, patchTime, stopTime } from './client'
import type { CreateTime, PatchTime, TimeSummary, TimeView } from './client'

/**
 * How often the running list is re-read.
 *
 * Long enough to be nearly free, short enough that a timer stopped in another
 * tab does not sit there counting for a minute. The displayed duration ticks
 * every second on its own — see `Duration.tsx` — so this is only about the
 * *set* of timers changing, not about the numbers moving.
 */
export const POLL_MS = 15_000

export interface TimerStore {
  /** Timers running right now. Empty while loading or after a failure. */
  running: () => TimeSummary[]
  loading: () => boolean
  /** Whatever went wrong fetching the list, if anything. */
  error: () => unknown
  /** The running timers tracked against a page, if any. */
  forPage: (slug: string) => TimeSummary[]
  /** Start a timer now. Throws `ApiError`. */
  start: (body: CreateTime) => Promise<TimeView>
  /** Stop one. Throws `ApiError`, notably `time_not_running`. */
  stop: (id: string) => Promise<TimeView>
  /** Edit one; here so an edit to a running timer updates the top bar. */
  patch: (id: string, body: PatchTime) => Promise<TimeView>
  remove: (id: string) => Promise<void>
  /** Re-read from the server. */
  refresh: () => void
}

/**
 * Build a store. Exported for tests, which need a fresh one per case rather
 * than the module-wide singleton below.
 */
export function createTimerStore(poll = POLL_MS): TimerStore {
  const [resource, { mutate, refetch }] = createResource(() =>
    listTimes({ running: true, order: 'asc' }),
  )

  /**
   * `latest` rather than the resource call, so a poll does not blank the bar
   * currently on screen — but guarded, because `latest` *rethrows* when the
   * most recent fetch failed. Unguarded, a backend that is merely down would
   * throw out of the app shell and take every page with it.
   */
  const snapshot = () => (resource.error ? undefined : resource.latest)
  const running = () => snapshot()?.times ?? []

  const replace = (next: TimeSummary[]) => {
    const current = snapshot()
    mutate(
      current
        ? { ...current, times: next, total: next.length }
        : { times: next, total: next.length, limit: next.length, offset: 0 },
    )
  }

  /**
   * Fold whatever the server just said about one entry back into the list.
   *
   * `TimeView` and `TimeSummary` differ only by the note, so the summary is
   * rebuilt from the view rather than refetched — the server is the authority
   * on the entry and it just handed the whole thing back.
   */
  const absorb = (entry: TimeView) => {
    const summary: TimeSummary = {
      id: entry.id,
      name: entry.name,
      start: entry.start,
      end: entry.end,
      running: entry.running,
      seconds: entry.seconds,
      pages: entry.pages,
      has_note: entry.note.trim().length > 0,
      updated: entry.updated,
      size: entry.size,
    }

    const without = running().filter((timer) => timer.id !== entry.id)
    // Oldest first, matching what the server sends, so the bar does not
    // reshuffle when one of its entries is edited.
    replace(
      entry.running
        ? [...without, summary].sort((a, b) => a.start.localeCompare(b.start))
        : without,
    )
    return entry
  }

  if (poll > 0) {
    const ticker = setInterval(() => void refetch(), poll)
    onCleanup(() => clearInterval(ticker))
  }

  return {
    running,
    loading: () => resource.loading,
    error: () => resource.error,
    forPage: (slug: string) =>
      running().filter((timer) => timer.pages.some((page) => page.slug === slug)),
    start: async (body: CreateTime) => absorb(await createTime(body)),
    stop: async (id: string) => absorb(await stopTime(id)),
    patch: async (id: string, body: PatchTime) => absorb(await patchTime(id, body)),
    remove: async (id: string) => {
      await deleteTime(id)
      replace(running().filter((timer) => timer.id !== id))
    },
    refresh: () => void refetch(),
  }
}

/**
 * The app's store.
 *
 * `createRoot` because this lives outside any component: the resource and the
 * poll are reactive computations, and ones created without an owner would warn
 * and never be disposed. It is deliberately never disposed — it lasts as long
 * as the tab.
 */
export const timers: TimerStore = createRoot(() => createTimerStore())
