import { For, Show, createMemo } from 'solid-js'
import { A } from '@solidjs/router'
import { pageHref } from '../api/client'
import type { DayView, WordStatsResponse } from '../api/client'

/**
 * Where the words went.
 *
 * The hours have a heat map; this is the same thing beside it, on the same
 * terms. A chart, not a streak: nothing here congratulates you, nothing counts
 * consecutive days, and the colour does not change when the number goes up.
 *
 * ## Why the bars go both ways
 *
 * The whole feature exists because a net word count is broken. An assistant
 * rewriting two thousand words into nineteen hundred is not "minus one hundred",
 * and a chart drawing one bar per day would say exactly that. So the day's bar
 * is split: words added above the line, words removed below it, and the net is a
 * number in the tooltip rather than the shape you see.
 *
 * Removed words are drawn as work, in the same weight as added ones, because
 * they are. A day of revision is not a day of damage.
 */
export default function WordStats(props: { stats: WordStatsResponse }) {
  const totals = () => props.stats.totals

  return (
    <section class="flex flex-col gap-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <h2 class="text-lg font-semibold">Words</h2>
        <span class="text-xs opacity-50">
          {/*
            `resolution` is here to say what it means rather than to be branched
            on: this is not keystroke history. The watcher collapses a burst of
            saves into one batch, edits made while the server was down are one
            observation at the next startup, and only an API write is genuinely
            one per save. The counts are right in all three; what is lost is
            resolution in time.
          */}
          {props.stats.days.length} days · {props.stats.resolution}
        </span>
      </div>

      <div class="stats stats-vertical sm:stats-horizontal shadow">
        <div class="stat">
          <div class="stat-title">Added</div>
          <div class="stat-value text-success text-2xl">
            {totals().added.toLocaleString()}
          </div>
          <div class="stat-desc">
            across {totals().observations.toLocaleString()}{' '}
            {totals().observations === 1 ? 'observation' : 'observations'}
          </div>
        </div>
        <div class="stat">
          <div class="stat-title">Removed</div>
          <div class="stat-value text-2xl opacity-70">
            {totals().removed.toLocaleString()}
          </div>
          <div class="stat-desc">
            {totals().pages.toLocaleString()}{' '}
            {totals().pages === 1 ? 'page' : 'pages'} written to
          </div>
        </div>
        <div class="stat">
          <div class="stat-title">Net</div>
          <div class="stat-value text-2xl">{signed(totals().delta)}</div>
          {/*
            Last of the three, and deliberately. It is arithmetic over the other
            two rather than a stored figure, and it is the number this feature
            exists to stop being the only one on offer.
          */}
          <div class="stat-desc">added minus removed</div>
        </div>
      </div>

      <div class="grid gap-4 lg:grid-cols-3">
        <div class="card bg-base-100 min-w-0 shadow lg:col-span-2">
          <div class="card-body">
            <h3 class="card-title text-base">By day</h3>
            <DayChart days={props.stats.days} />
          </div>
        </div>

        <div class="card bg-base-100 shadow">
          <div class="card-body">
            <h3 class="card-title text-base">By tool</h3>
            <ul class="flex flex-col gap-2 text-sm">
              <For
                each={props.stats.actors}
                fallback={
                  <li class="opacity-60">
                    Nothing written in this window. Saves are labelled by
                    whatever tool made them.
                  </li>
                }
              >
                {(actor) => (
                  <li class="flex items-baseline justify-between gap-2">
                    {/*
                      A label is a claim rather than a proof: anything that can
                      write can send the header. That is fine, because the
                      question is bookkeeping about your own tools rather than
                      security. It is also why this is a plain name and not a
                      badge that looks authoritative.
                    */}
                    <span class="min-w-0 truncate font-mono text-xs">{actor.actor}</span>
                    <span class="whitespace-nowrap">
                      <span class="text-success font-mono text-xs">
                        +{actor.added.toLocaleString()}
                      </span>{' '}
                      <span class="font-mono text-xs opacity-60">
                        &minus;{actor.removed.toLocaleString()}
                      </span>
                    </span>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </div>
      </div>

      <div class="card bg-base-100 shadow">
        <div class="card-body">
          <h3 class="card-title text-base">Pages written to</h3>
          <ul class="grid gap-2 text-sm sm:grid-cols-2">
            <For
              each={props.stats.pages}
              fallback={<li class="opacity-60">No page was written to in this window.</li>}
            >
              {(page) => (
                <li class="flex items-baseline justify-between gap-2">
                  {/*
                    A page this reader cannot see is ranked under its slug and
                    the row stays. The words really were written, and hiding
                    somebody's own working history from them would be the wrong
                    reading of a rule that protects other people's pages.
                  */}
                  <A class="link min-w-0 truncate" href={pageHref(page.slug)}>
                    {page.title}
                  </A>
                  <span class="whitespace-nowrap">
                    <span class="text-success font-mono text-xs">
                      +{page.added.toLocaleString()}
                    </span>{' '}
                    <span class="font-mono text-xs opacity-60">
                      &minus;{page.removed.toLocaleString()}
                    </span>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </div>
    </section>
  )
}

/**
 * One column per local day, added above the line and removed below it.
 *
 * Plain divs rather than a charting library, for the reason the hours chart
 * already gives: this is two series of at most a few hundred bars, and the whole
 * of what a library would add is a dependency and a second way for the dashboard
 * to break.
 *
 * Both halves are scaled to the same peak, so the two sides of the line are
 * comparable. Scaling them separately would make a day that removed forty words
 * look like a day that removed four thousand.
 */
function DayChart(props: { days: DayView[] }) {
  const peak = createMemo(() =>
    Math.max(1, ...props.days.flatMap((day) => [day.added, day.removed])),
  )

  const busiest = createMemo(() =>
    props.days.reduce<DayView | undefined>(
      (best, day) =>
        !best || day.added + day.removed > best.added + best.removed ? day : best,
      undefined,
    ),
  )

  return (
    <div>
      <div class="flex h-40 items-center gap-px">
        <For each={props.days}>
          {(day) => (
            <div
              class="flex h-full flex-1 flex-col justify-center"
              title={`${formatDay(day.date)}: added ${day.added}, removed ${day.removed}, net ${signed(day.delta)}`}
            >
              {/*
                Two boxes of equal height meeting at the middle, each holding a
                bar that grows away from the line. `justify-end` on the top half
                is what makes it grow upward.
              */}
              <div class="flex h-1/2 flex-col justify-end">
                <div
                  class="bg-success w-full rounded-t"
                  style={{ height: `${share(day.added, peak())}%` }}
                />
              </div>
              <div class="bg-base-300 h-px w-full" />
              <div class="h-1/2">
                <div
                  class="bg-base-content/40 w-full rounded-b"
                  style={{ height: `${share(day.removed, peak())}%` }}
                />
              </div>
            </div>
          )}
        </For>
      </div>

      <div class="flex justify-between pt-1 text-xs opacity-60">
        <span>{formatDay(props.days[0]?.date)}</span>
        <Show when={busiest()} fallback={<span>nothing yet</span>}>
          {(day) => (
            <span>
              busiest {formatDay(day().date)} · +{day().added.toLocaleString()}{' '}
              &minus;{day().removed.toLocaleString()}
            </span>
          )}
        </Show>
        <span>{formatDay(props.days[props.days.length - 1]?.date)}</span>
      </div>
    </div>
  )
}

/**
 * A bar's height as a percentage of its half of the chart.
 *
 * Zero stays zero rather than getting the minimum a bucket chart gives an empty
 * hour. A day with nothing written is not a small amount of writing, and drawing
 * it as a sliver on both sides of the line would make an empty quarter look
 * busy.
 */
function share(value: number, peak: number): number {
  return value === 0 ? 0 : Math.max(3, Math.round((value / peak) * 100))
}

function signed(delta: number): string {
  return delta > 0 ? `+${delta.toLocaleString()}` : delta.toLocaleString()
}

/**
 * A local date, in the reader's locale.
 *
 * The server has already cut the day in the offset that was asked for and sends
 * back a bare `YYYY-MM-DD`, so this must not go through `new Date(value)`, which
 * would read it as midnight **UTC** and shift the label a day for half the
 * planet. The parts are read out of the string and handed over as parts.
 */
export function formatDay(date: string | undefined): string {
  if (!date) return ''

  const [year, month, day] = date.split('-').map(Number)
  if (!year || !month || !day) return date

  return new Date(year, month - 1, day).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
  })
}
