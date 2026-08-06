import { Show, createSignal, onCleanup } from 'solid-js'

/**
 * Durations arrive from the API in seconds and are read in three different
 * ways, so there are three spellings of them here rather than one that tries
 * to serve every caller.
 */

/** How often a running timer's display catches up with the clock. */
const TICK_MS = 1000

/**
 * `1h 14m` — the one for reading in a list.
 *
 * Seconds are dropped above a minute because nobody skimming a week's log
 * cares about them, and their changing is what makes a table of running timers
 * impossible to read. Below a minute they are all there is, so they stay.
 */
export function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds))
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)

  if (total < 60) return `${total}s`
  if (hours === 0) return `${minutes}m`
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`
}

/**
 * `1:14:30` — the one for a stopwatch.
 *
 * A running timer is the one place seconds are worth showing: they are the
 * evidence that it is running at all.
 */
export function formatClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds))
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const rest = total % 60
  const pad = (value: number) => String(value).padStart(2, '0')

  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(rest)}`
    : `${minutes}:${pad(rest)}`
}

/** `2.5 h` — the one for an axis label or a tooltip, where units must line up. */
export function formatHours(seconds: number): string {
  const hours = Math.max(0, seconds) / 3600
  if (hours === 0) return '0 h'
  return `${hours < 10 ? hours.toFixed(1) : Math.round(hours)} h`
}

/** Whole seconds between two instants, never negative. */
export function secondsBetween(from: string, to: Date = new Date()): number {
  const started = new Date(from).getTime()
  if (Number.isNaN(started)) return 0
  return Math.max(0, Math.floor((to.getTime() - started) / 1000))
}

/**
 * A duration that keeps up with the clock while its entry is running.
 *
 * The server sends `seconds` as of the moment it answered, which is right for
 * a finished entry and stale a second later for a running one. Rather than
 * re-fetching every second, a running entry is recomputed locally from its
 * `start` — the same arithmetic the server does, and the only value it can
 * disagree about is one caused by a clock that is genuinely wrong.
 */
export default function Duration(props: {
  /** Elapsed seconds as the server reported them. */
  seconds: number
  /** Set for a running entry: its start, so the count can continue locally. */
  runningSince?: string | null
  /** Show seconds, stopwatch style. Implied for a running entry. */
  clock?: boolean
  class?: string
}) {
  const [now, setNow] = createSignal(new Date())

  // One interval per rendered duration is more than a shared ticker would
  // need, but a page shows a handful of running timers at most and the shared
  // version has to solve subscription teardown for no measurable gain.
  const timer = setInterval(() => {
    if (props.runningSince) setNow(new Date())
  }, TICK_MS)
  onCleanup(() => clearInterval(timer))

  const seconds = () =>
    props.runningSince ? secondsBetween(props.runningSince, now()) : props.seconds

  return (
    <span class={props.class} classList={{ 'tabular-nums': true }}>
      <Show when={props.runningSince || props.clock} fallback={formatDuration(seconds())}>
        {formatClock(seconds())}
      </Show>
    </span>
  )
}
