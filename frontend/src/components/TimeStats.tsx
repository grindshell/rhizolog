import { For, Show, createMemo, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { groupHref, pageHref } from '../api/client'
import type { PeriodStatsView, TimeStatsResponse } from '../api/client'
import { formatDuration, formatHours } from './Duration'

/** Monday first, matching the week the server's periods are cut on. */
const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/** Which hours get a label. Every third keeps the axis readable at any width. */
const LABELLED_HOURS = [0, 3, 6, 9, 12, 15, 18, 21]

/**
 * What a period looks like before one has been picked.
 *
 * The server always sends four, so this is unreachable — but the generated
 * types describe an array, and an array index is `T | undefined`. A blank
 * period renders as an empty chart, which is a better answer to an impossible
 * case than a crash in the dashboard.
 */
const NO_PERIOD: PeriodStatsView = {
  period: 'day',
  from: new Date().toISOString(),
  to: new Date().toISOString(),
  seconds: 0,
  entries: 0,
  names: [],
  pages: [],
  buckets: [],
}

/**
 * Where the time went.
 *
 * One request answers day, week, month and year, because the interesting thing
 * about a time log is the comparison between them and four round trips to make
 * one sentence would be silly. The server does the bucketing — including
 * splitting a session that ran past midnight across both days — so nothing
 * here has to know what a month is.
 */
export default function TimeStats(props: { stats: TimeStatsResponse }) {
  const [selected, setSelected] = createSignal('week')

  const period = createMemo(
    () =>
      props.stats.periods.find((entry) => entry.period === selected()) ??
      props.stats.periods[0] ??
      NO_PERIOD,
  )

  return (
    <section class="flex flex-col gap-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <h2 class="text-lg font-semibold">
          Time
          <Show when={props.stats.all_time.running > 0}>
            <span class="badge badge-secondary badge-sm ml-2">
              {props.stats.all_time.running} running
            </span>
          </Show>
        </h2>
        <div class="flex items-center gap-2">
          <div role="tablist" class="tabs tabs-box tabs-sm">
            <For each={props.stats.periods}>
              {(entry) => (
                <button
                  role="tab"
                  class="tab"
                  // Required on `role="tab"`, and the only thing that tells a
                  // screen reader which window is being shown. `tab-active` is
                  // a class, which is to say it is visible and nothing else.
                  aria-selected={selected() === entry.period}
                  aria-controls="time-stats-panel"
                  classList={{ 'tab-active': selected() === entry.period }}
                  onClick={() => setSelected(entry.period)}
                >
                  {label(entry.period)}
                </button>
              )}
            </For>
          </div>
          <A class="btn btn-ghost btn-sm" href="/times">
            Open log
          </A>
        </div>
      </div>

      <div
        id="time-stats-panel"
        role="tabpanel"
        class="stats stats-vertical sm:stats-horizontal shadow"
      >
        <div class="stat">
          <div class="stat-title">{label(period().period)}</div>
          <div class="stat-value text-2xl">{formatDuration(period().seconds)}</div>
          <div class="stat-desc">{period().entries} entries</div>
        </div>
        <div class="stat">
          <div class="stat-title">Most used</div>
          <div class="stat-value truncate text-2xl">
            {period().names[0]?.name ?? '—'}
          </div>
          <div class="stat-desc">
            <Show when={period().names[0]} fallback="nothing tracked">
              {(top) => <>{formatDuration(top().seconds)} across {top().entries}</>}
            </Show>
          </div>
        </div>
        <div class="stat">
          <div class="stat-title">All time</div>
          <div class="stat-value text-2xl">
            {formatDuration(props.stats.all_time.seconds)}
          </div>
          <div class="stat-desc">
            {props.stats.all_time.entries} entries · {props.stats.all_time.groups} groups
          </div>
        </div>
      </div>

      <div class="grid gap-4 lg:grid-cols-3">
        <div class="card bg-base-100 shadow lg:col-span-2">
          <div class="card-body">
            <h3 class="card-title text-base">{label(period().period)} by {bucketUnit(period().period)}</h3>
            <BucketChart period={period()} />
          </div>
        </div>

        <div class="card bg-base-100 shadow">
          <div class="card-body">
            <h3 class="card-title text-base">Most used</h3>
            <ul class="flex flex-col gap-2 text-sm">
              <For
                each={period().names}
                fallback={<li class="opacity-60">Nothing tracked in this window.</li>}
              >
                {(name) => (
                  <li class="flex items-baseline justify-between gap-2">
                    <A class="link min-w-0 truncate" href={groupHref(name.name)}>
                      {name.name}
                    </A>
                    <span class="font-mono text-xs whitespace-nowrap opacity-70">
                      {formatDuration(name.seconds)}
                    </span>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </div>
      </div>

      <div class="grid gap-4 lg:grid-cols-3">
        {/*
          `min-w-0`, because a grid item refuses by default to be narrower than
          its content, and the heat map inside is deliberately wider than a
          phone. Without it the item wins, the heat map's own `overflow-x-auto`
          never engages, and the whole row hangs off the side of the screen.
        */}
        <div class="card bg-base-100 min-w-0 shadow lg:col-span-2">
          <div class="card-body">
            <h3 class="card-title text-base">Active hours</h3>
            <p class="text-xs opacity-60">
              Every hour of the week, over {yearOf(props.stats)}. A session that ran
              past midnight lights every hour it touched, not just the one it
              started in.
            </p>
            <Heatmap stats={props.stats} />
          </div>
        </div>

        <div class="card bg-base-100 shadow">
          <div class="card-body">
            <h3 class="card-title text-base">Pages worked on</h3>
            <ul class="flex flex-col gap-2 text-sm">
              <For
                each={period().pages}
                fallback={
                  <li class="opacity-60">
                    No time attached to a page in this window. Add pages to an entry
                    to see where the hours land in the wiki.
                  </li>
                }
              >
                {(page) => (
                  <li class="flex items-baseline justify-between gap-2">
                    <A class="link min-w-0 truncate" href={pageHref(page.slug)}>
                      {page.title}
                    </A>
                    <span class="font-mono text-xs whitespace-nowrap opacity-70">
                      {formatDuration(page.seconds)}
                    </span>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </div>
      </div>
    </section>
  )
}

/**
 * A bar per bucket, scaled to the busiest one.
 *
 * Plain divs rather than a charting library: this is one series of at most
 * thirty-one bars, and the whole of what a library would add is a dependency
 * and a second way for the dashboard to break.
 */
function BucketChart(props: { period: PeriodStatsView }) {
  const peak = createMemo(() =>
    Math.max(1, ...props.period.buckets.map((bucket) => bucket.seconds)),
  )

  return (
    <div>
      <div class="flex h-32 items-end gap-px">
        <For each={props.period.buckets}>
          {(bucket) => (
            <div
              class="group relative flex-1"
              title={`${bucketLabel(props.period.period, bucket.start)}: ${formatDuration(bucket.seconds)}`}
            >
              <div
                class="w-full rounded-t"
                classList={{
                  'bg-primary': bucket.seconds > 0,
                  'bg-base-200': bucket.seconds === 0,
                }}
                style={{
                  // A minimum of a pixel or two so an empty bucket still reads
                  // as a column rather than as a gap in the axis.
                  height: `${Math.max(2, Math.round((bucket.seconds / peak()) * 100))}%`,
                }}
              />
            </div>
          )}
        </For>
      </div>
      <div class="flex justify-between pt-1 text-xs opacity-60">
        <span>{bucketLabel(props.period.period, props.period.buckets[0]?.start)}</span>
        <span>peak {formatHours(peak())}</span>
        <span>
          {bucketLabel(
            props.period.period,
            props.period.buckets[props.period.buckets.length - 1]?.start,
          )}
        </span>
      </div>
    </div>
  )
}

/**
 * Seven rows of twenty-four squares.
 *
 * Shaded by share of the busiest cell rather than by an absolute scale: the
 * question this answers is "when", not "how much", and an absolute scale makes
 * a light week look like an empty one.
 */
function Heatmap(props: { stats: TimeStatsResponse }) {
  const peak = createMemo(() =>
    Math.max(1, ...props.stats.heatmap.cells.map((cell) => cell.seconds)),
  )

  const rows = createMemo(() =>
    WEEKDAYS.map((day, weekday) => ({
      day,
      cells: props.stats.heatmap.cells.filter((cell) => cell.weekday === weekday),
    })),
  )

  return (
    <div class="overflow-x-auto">
      <div class="min-w-96">
        <For each={rows()}>
          {(row) => (
            <div class="flex items-center gap-1 py-px">
              <span class="w-8 shrink-0 text-right text-xs opacity-60">{row.day}</span>
              <div class="flex flex-1 gap-px">
                <For each={row.cells}>
                  {(cell) => (
                    <div
                      class="bg-primary aspect-square min-w-2 flex-1 rounded-xs"
                      style={{
                        // Never fully transparent for a cell with time in it:
                        // an hour in a busy year would otherwise be invisible.
                        opacity:
                          cell.seconds === 0
                            ? 0.07
                            : 0.25 + 0.75 * (cell.seconds / peak()),
                      }}
                      title={`${row.day} ${String(cell.hour).padStart(2, '0')}:00 — ${formatDuration(cell.seconds)}`}
                    />
                  )}
                </For>
              </div>
            </div>
          )}
        </For>

        <div class="flex items-center gap-1 pt-1">
          <span class="w-8 shrink-0" />
          <div class="flex flex-1 gap-px">
            <For each={Array.from({ length: 24 }, (_, hour) => hour)}>
              {(hour) => (
                <span class="min-w-2 flex-1 text-center text-[10px] opacity-50">
                  {LABELLED_HOURS.includes(hour) ? hour : ''}
                </span>
              )}
            </For>
          </div>
        </div>
      </div>
    </div>
  )
}

function label(period: string): string {
  switch (period) {
    case 'day':
      return 'Today'
    case 'week':
      return 'This week'
    case 'month':
      return 'This month'
    default:
      return 'This year'
  }
}

function bucketUnit(period: string): string {
  return period === 'day' ? 'hour' : period === 'year' ? 'month' : 'day'
}

/** A bucket's own label, in the reader's locale rather than the server's. */
function bucketLabel(period: string, start: string | undefined): string {
  if (!start) return ''
  const at = new Date(start)
  if (Number.isNaN(at.getTime())) return start

  switch (period) {
    case 'day':
      return at.toLocaleTimeString(undefined, { hour: 'numeric' })
    case 'year':
      return at.toLocaleDateString(undefined, { month: 'short' })
    default:
      return at.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }
}

function yearOf(stats: TimeStatsResponse): string {
  const at = new Date(stats.heatmap.from)
  return Number.isNaN(at.getTime()) ? 'the year' : String(at.getFullYear())
}
