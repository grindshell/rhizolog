import { For, Show, createResource } from 'solid-js'
import { A } from '@solidjs/router'
import { manuscriptPace, pageHref } from '../api/client'
import type { Pace as PaceView, PageView } from '../api/client'

/**
 * Words remaining over days remaining, under the progress bar.
 *
 * `pace/v1`, and it is arithmetic rather than encouragement. Two rates are shown
 * in the same unit, what the deadline asks for and what the last fortnight came
 * to, and the reader compares them. Nothing here changes colour, nothing counts
 * a streak, and there is no sentence anywhere that says how you are doing: those
 * are opinions about somebody's week, and this wiki does not have them. The
 * hours heat map and the words chart are already on those terms.
 *
 * The figures come with what they were divided from, which is the same rule the
 * momentum receipt in Idea Inbox is held to. A rate with no window and no total
 * beside it is a number somebody has to trust.
 */
export default function Pace(props: { page: PageView }) {
  /**
   * Asked for only where there is something to be paced against.
   *
   * A null source is what makes that true: Solid skips the fetcher for one, so a
   * manuscript that names neither a target nor a day costs no second compile.
   * That matters because this endpoint walks the whole book again, exactly as
   * the panel above it does, and a page carrying only a `contents:` list has
   * nothing here to say.
   */
  const [pace] = createResource(
    () =>
      props.page.target !== undefined || props.page.due !== undefined
        ? props.page.slug
        : null,
    (root) => manuscriptPace({ root }),
  )

  /**
   * The figures, or nothing at all when the request failed.
   *
   * Guarded because reading an errored resource **rethrows**, which unguarded
   * would take the panel above down with it. That is the same guard `Async`
   * carries, and here the answer to a failure is different: no error notice,
   * deliberately. The one failure that is expected rather than exceptional is a
   * 401, since half of this is read off the word log and that is never served to
   * a caller with no account, and on a wiki published with
   * `RHIZOLOG_ANONYMOUS_READ` the spine above is still theirs to read. Anything
   * else that could fail here fails the compile drawing that spine too, which is
   * where it is reported once.
   */
  const figures = () => (pace.error ? undefined : pace())

  return <Show when={figures()}>{(found) => <Figures pace={found()} />}</Show>
}

function Figures(props: { pace: PaceView }) {
  // `recent` rather than `window`, which would shadow the DOM global inside this
  // component and make the next person to reach for `window.matchMedia` here
  // debug something that has nothing to do with what they wrote.
  const recent = () => props.pace.window
  const uncounted = () => props.pace.uncounted

  return (
    <div class="border-base-300 flex flex-col gap-1 border-t pt-2 text-xs">
      <Deadline pace={props.pace} />

      <div class="flex flex-wrap items-baseline gap-x-2 gap-y-1 opacity-70">
        <span>Last {recent().days} days</span>
        {/*
          Both halves, and never only their difference. The word log refuses to
          store a net for exactly this reason: a rewrite of two thousand words
          into nineteen hundred is not "minus one hundred". The net beside them
          is the one place the difference is the question, because a target is a
          length rather than an amount of effort, and it is shown with the
          figures it came from rather than instead of them.
        */}
        <span>
          <span class="font-mono">{signed(recent().net)}</span>{' '}
          <span class="opacity-70">
            ({recent().added.toLocaleString()} added,{' '}
            {recent().removed.toLocaleString()} removed)
          </span>{' '}
          {/*
            Beside the totals it describes rather than beside the rate, which
            divides by the window and not by this. Both divisions are worth
            having and only one of them projects against a calendar.
          */}
          on <span class="font-mono">{recent().active_days}</span>{' '}
          {recent().active_days === 1 ? 'day' : 'days'}
        </span>
        <span aria-hidden="true" class="opacity-40">
          ·
        </span>
        {/*
          Last on the line, so it sits under the rate the deadline asks for and
          the two can be read against each other. Neither is coloured and neither
          says anything about the other.
        */}
        <span>
          <span class="font-mono">{formatRate(recent().per_day)}</span> a day
        </span>
      </div>

      <Projection pace={props.pace} />

      {/*
        The contrast this whole block exists for. A day spent on a scene that is
        out of the book is a day the compiled total did not move, so those words
        are in no rate above; saying so is what stops "excluded words do not
        count" from being read as a claim about the chart, where they very much
        do.
      */}
      <Show when={uncounted().observations > 0}>
        <div class="flex flex-wrap items-baseline gap-x-2 opacity-60">
          <span>
            <span class="font-mono">{signed(uncounted().net)}</span> went to pages
            this document does not carry
          </span>
          <span class="flex flex-wrap gap-x-2">
            <For each={uncounted().pages}>
              {(page) => (
                <A class="link font-mono" href={pageHref(page.slug)}>
                  {page.slug}
                </A>
              )}
            </For>
          </span>
        </div>
      </Show>
    </div>
  )
}

/**
 * What the deadline asks for, when there is one to ask.
 *
 * Absent rather than zeroed on a manuscript aiming at nothing: a page with no
 * target has no words remaining, and a row of dashes would be a claim that it
 * did.
 */
function Deadline(props: { pace: PaceView }) {
  const remaining = () => props.pace.remaining
  const left = () => props.pace.days_remaining

  return (
    <Show when={remaining() != null}>
      <div class="flex flex-wrap items-baseline gap-x-2 gap-y-1">
        {/*
          A target is a length somebody is aiming at rather than a ceiling, so
          passing it is worth saying rather than clamping to zero and reading as
          finished.

          Zero belongs on the "to go" side, not the other one. A book of exactly
          two thousand words against a target of two thousand has nothing left to
          write, and "0 over target" would be a sentence about an overshoot that
          did not happen.
        */}
        <Show
          when={(remaining() ?? 0) >= 0}
          fallback={
            <span>
              <span class="font-mono">
                {Math.abs(remaining() ?? 0).toLocaleString()}
              </span>{' '}
              <span class="opacity-70">over target</span>
            </span>
          }
        >
          <span>
            <span class="font-mono">{(remaining() ?? 0).toLocaleString()}</span>{' '}
            <span class="opacity-70">to go</span>
          </span>
        </Show>

        <Show when={left() != null}>
          <span aria-hidden="true" class="opacity-40">
            ·
          </span>
          <Show
            when={(left() ?? 0) > 0}
            fallback={<span class="opacity-70">past its day</span>}
          >
            <span>
              <span class="font-mono">{(left() ?? 0).toLocaleString()}</span>{' '}
              <span class="opacity-70">
                {left() === 1 ? 'day left' : 'days left'}
              </span>
            </span>
          </Show>
        </Show>

        <Show when={props.pace.required_per_day != null}>
          <span aria-hidden="true" class="opacity-40">
            ·
          </span>
          <span>
            <span class="font-mono">
              {formatRate(props.pace.required_per_day ?? 0)}
            </span>{' '}
            <span class="opacity-70">a day to make it</span>
          </span>
        </Show>
      </div>
    </Show>
  )
}

/**
 * Where the observed rate lands, if it lands anywhere.
 *
 * A fortnight of cutting projects nothing at all, and the line is absent rather
 * than showing a date arrived at by dividing by zero. A projection past a
 * century keeps its number and loses its date, which is the server's rule and is
 * rendered as the server sends it.
 */
function Projection(props: { pace: PaceView }) {
  const days = () => props.pace.projected_days
  const finish = () => props.pace.projected_finish

  return (
    <Show when={days() != null}>
      <div class="opacity-70">
        <Show
          when={finish()}
          fallback={
            <span>
              At that rate, <span class="font-mono">{(days() ?? 0).toLocaleString()}</span>{' '}
              more days
            </span>
          }
        >
          {(day) => (
            <span>
              At that rate, {formatDay(day())}{' '}
              <span class="opacity-70">
                ({(days() ?? 0).toLocaleString()} days)
              </span>
            </span>
          )}
        </Show>
      </div>
    </Show>
  )
}

/**
 * A rate, to a precision the number can carry.
 *
 * Rounded to a whole word from a hundred up, where a decimal is noise, and to
 * one below it, where rounding "0.4 a day" to zero would report a manuscript
 * that is moving as one that is not.
 */
export function formatRate(words: number): string {
  if (!Number.isFinite(words)) return '0'
  const rounded =
    Math.abs(words) >= 100 ? Math.round(words) : Math.round(words * 10) / 10
  return rounded.toLocaleString()
}

/** A net, with the sign kept, because a fortnight of revision is a real answer. */
export function signed(words: number): string {
  return `${words < 0 ? '-' : '+'}${Math.abs(words).toLocaleString()}`
}

/**
 * A due date is a **day**, not an instant, so it is shown as one.
 *
 * It arrives as a full timestamp because this is JSON and a client has a clock,
 * and a bare `2027-03-01` in a file reads as midnight UTC. Rendering that in the
 * reader's own zone would show 28 February to anybody west of Greenwich, so the
 * day is read back in UTC and only its name is shown.
 *
 * Here rather than in `Manuscript.tsx`, which imports it back, so that the one
 * import between the two files points in one direction.
 */
export function formatDay(value: string): string {
  const at = new Date(value)
  if (Number.isNaN(at.getTime())) return value
  return at.toLocaleDateString(undefined, {
    timeZone: 'UTC',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}
