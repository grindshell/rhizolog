import { Show, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { ideaHref } from '../api/client'
import type { IdeaSummaryView } from '../api/client'
import { formatDate } from '../routes/PageDetail'

/** How long a dismissal keeps an idea off this card. */
export const DISMISSAL_DAYS = 30

/** Below this an idea is one thought, and resurfacing it is just the inbox. */
export const MIN_CAPTURES = 2

const DAY_MS = 24 * 60 * 60 * 1000

/**
 * Whether an idea is worth putting in front of somebody again.
 *
 * Dormant is the whole point: these are the thoughts that stopped, and the ones
 * still moving need no help being remembered. Nothing about affirmation is
 * checked here because dormancy has already checked it. An affirmation moves
 * `last_signal`, and an idea whose last signal is inside sixty days is not
 * dormant, so "not affirmed in the last thirty days" is implied rather than
 * omitted.
 */
export function eligible(idea: IdeaSummaryView, at: Date): boolean {
  if (idea.state !== 'dormant' || idea.retired) return false
  if (idea.captures < MIN_CAPTURES) return false
  if (!idea.dismissed) return true

  const since = at.getTime() - new Date(idea.dismissed).getTime()
  return since >= DISMISSAL_DAYS * DAY_MS
}

/**
 * The reader's calendar date, which is the unit a card lasts for.
 *
 * Local rather than UTC, because "today" is a thing that happens where the
 * person is. Built by hand rather than through `toISOString`, which would
 * convert to UTC first and roll the date over an evening.
 */
export function calendarDate(at: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`
}

/** FNV-1a, for turning a date into a number without pulling in a dependency. */
function hash(text: string): number {
  let value = 0x811c9dc5
  for (let index = 0; index < text.length; index += 1) {
    value ^= text.charCodeAt(index)
    value = Math.imul(value, 0x01000193) >>> 0
  }
  return value
}

/**
 * Today's one idea, or none.
 *
 * One card, chosen deterministically from the local date, so refreshing does
 * not deal a new thought and the screen is not a feed. Tomorrow it moves on its
 * own, which is the only scheduling this feature has and the only kind it
 * should have: nothing here notifies anybody, and merely being shown a card
 * writes nothing down.
 *
 * Sorted by id before choosing, so the answer does not depend on the order the
 * listing happened to arrive in.
 */
export function chooseRediscovery(
  ideas: IdeaSummaryView[],
  at: Date,
): IdeaSummaryView | undefined {
  const candidates = ideas
    .filter((idea) => eligible(idea, at))
    .sort((first, second) => first.id.localeCompare(second.id))

  if (candidates.length === 0) return undefined
  return candidates[hash(calendarDate(at)) % candidates.length]
}

/**
 * Whether the card has been answered since the page loaded.
 *
 * One card, and one card is what answering it should leave: still interested and
 * not now are both answers, and being handed another thought for having given
 * one is the feed this feature is written not to be. Both answers change the
 * eligible set as well, so without this the pool simply shrinks and the next
 * name comes up.
 *
 * It deliberately does not survive a reload, and nothing about it is written
 * down. Rediscovery is meant to be a thing that happens once when you open the
 * inbox, not a quota with a ledger, and an authored record of having looked at a
 * card would be exactly the engagement bookkeeping the plan rules out.
 */
export interface RediscoveryState {
  answered: () => boolean
  /** Record that the card was answered. Nothing undoes this; a reload does. */
  answer: () => void
}

/**
 * Build one. Exported for tests, which need a fresh one per case rather than the
 * module-wide singleton below.
 */
export function createRediscoveryState(): RediscoveryState {
  const [answered, setAnswered] = createSignal(false)
  return { answered, answer: () => setAnswered(true) }
}

/**
 * The app's, at module scope so that it lasts as long as the page rather than as
 * long as one visit to the inbox. Walking to an idea and back is not a new day.
 */
export const rediscovery: RediscoveryState = createRediscoveryState()

/**
 * One dormant idea, offered back.
 *
 * Two answers and both of them authored: still interested writes an
 * affirmation, and not now writes a dismissal that keeps it away for a month.
 * Closing the tab writes nothing, which is why looking at this card cannot
 * inflate anything it shows.
 */
export default function Rediscovery(props: {
  idea: IdeaSummaryView
  onAffirm: () => void
  onDismiss: () => void
}) {
  return (
    <section class="card border border-base-300 bg-base-100 shadow">
      <div class="card-body gap-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
          <h2 class="card-title text-base">
            You were thinking about this
            <span class="badge badge-ghost badge-sm">dormant</span>
          </h2>
          <Show when={props.idea.last_signal}>
            {(signal) => (
              <span class="text-xs opacity-60">nothing since {formatDate(signal())}</span>
            )}
          </Show>
        </div>

        <A class="link text-lg font-medium" href={ideaHref(props.idea.id)}>
          {props.idea.name}
        </A>
        <p class="text-sm opacity-70">
          {props.idea.captures} {props.idea.captures === 1 ? 'capture' : 'captures'}
          <Show when={props.idea.momentum !== null && props.idea.momentum !== undefined}>
            {' '}
            · momentum {props.idea.momentum}
          </Show>
        </p>

        <div class="flex flex-wrap gap-2">
          <button class="btn btn-primary btn-sm" onClick={props.onAffirm}>
            Still interested
          </button>
          <button class="btn btn-ghost btn-sm" onClick={props.onDismiss}>
            Not now
          </button>
          <A class="btn btn-ghost btn-sm" href={ideaHref(props.idea.id)}>
            Open it
          </A>
        </div>
      </div>
    </section>
  )
}
