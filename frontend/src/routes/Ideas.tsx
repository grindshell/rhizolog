import { For, Show, createResource, createSignal } from 'solid-js'
import { A, useSearchParams } from '@solidjs/router'
import { ideaHref, lifecycleHref, listIdeas } from '../api/client'
import type { IdeaSummaryView, Lifecycle } from '../api/client'
import { Async } from '../components/Async'
import { formatDate } from './PageDetail'

/**
 * How many ideas one request asks for.
 *
 * The API's ceiling, and deliberately the whole listing rather than a page of
 * it: the states are grouped on this screen, and a page boundary in the middle
 * of Dormant would be a grouping that lied. A person who has named two hundred
 * threads has earned a paging control, and that is the point to build one.
 */
const PAGE_SIZE = 200

/**
 * The groups, in the order they are worth reading.
 *
 * Needs repair first because it is the only one asking for anything. Then the
 * live states in descending order of what is going on, and Retired last and
 * shut, because setting something aside should not mean seeing it every time.
 */
const GROUPS: { state: Lifecycle; title: string; blurb: string; closed?: boolean }[] = [
  {
    state: 'active',
    title: 'Active',
    blurb: 'Enough is happening to score four or more.',
  },
  {
    state: 'recurring',
    title: 'Recurring',
    blurb: 'More than one capture, still connected, quieter than active.',
  },
  { state: 'new', title: 'New', blurb: 'One capture so far.' },
  {
    state: 'dormant',
    title: 'Dormant',
    blurb: 'Nothing for sixty days. These are what rediscovery offers back.',
  },
  {
    state: 'retired',
    title: 'Retired',
    blurb: 'Set aside on purpose. Nothing is deleted and reopening is one click.',
    closed: true,
  },
]

/**
 * Every thread, grouped by where the rules put it.
 *
 * The state and momentum on this screen are worked out at the instant it was
 * asked for and stored nowhere, so the same files answer differently tomorrow.
 * That is the feature rather than staleness, and it is why the instant and the
 * ruleset are printed at the bottom.
 */
export default function Ideas() {
  const [searchParams, setSearchParams] = useSearchParams()

  const filter = () => {
    const state = first(searchParams.state)
    return GROUPS.some((group) => group.state === state)
      ? (state as Lifecycle)
      : undefined
  }
  const repairs = () => first(searchParams.integrity) === 'evidence_missing'

  const [ideas] = createResource(
    () => ({ state: filter(), repair: repairs() }),
    ({ state, repair }) =>
      listIdeas({
        state,
        integrity: repair ? 'evidence_missing' : undefined,
        limit: PAGE_SIZE,
      }),
  )

  const broken = (list: IdeaSummaryView[]) => list.filter((idea) => idea.needs_repair)
  const of = (list: IdeaSummaryView[], state: Lifecycle) =>
    list.filter((idea) => idea.state === state)

  return (
    <div class="flex flex-col gap-6">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-semibold">Ideas</h1>
        <A class="btn btn-primary btn-sm" href="/inbox?capture=1">
          Capture a thought
        </A>
      </header>

      <Show when={filter() ?? (repairs() ? 'evidence_missing' : undefined)}>
        {(active) => (
          <div class="flex flex-wrap items-center gap-2 text-sm">
            <span class="opacity-60">Showing</span>
            <span class="badge badge-outline gap-2">
              {active()}
              <button
                class="opacity-60"
                aria-label="Clear the filter"
                onClick={() => setSearchParams({ state: undefined, integrity: undefined })}
              >
                ✕
              </button>
            </span>
          </div>
        )}
      </Show>

      <Async resource={ideas}>
        {(data) => (
          <div class="flex flex-col gap-4">
            <Show when={data.ideas.length === 0}>
              <p class="text-sm opacity-60">
                <Show
                  when={filter() || repairs()}
                  fallback="No ideas yet. Connect two captures in the inbox and the first one appears here."
                >
                  Nothing is in that state right now.
                </Show>
              </p>
            </Show>

            {/*
              Ahead of every lifecycle group and outside them, because an idea
              whose captures are gone has no state to be grouped by. Inventing
              one out of missing evidence is the thing this feature must not do,
              so it says what it lost instead.
            */}
            <Group
              title="Needs repair"
              blurb="The files these rest on are gone. Reconnect a capture, restore it on disk, or retire the thread."
              ideas={broken(data.ideas)}
              href="/ideas?integrity=evidence_missing"
            />

            <For each={GROUPS}>
              {(group) => (
                <Show when={!repairs()}>
                  <Group
                    title={group.title}
                    blurb={group.blurb}
                    closed={group.closed}
                    ideas={of(data.ideas, group.state)}
                    href={lifecycleHref(group.state)}
                  />
                </Show>
              )}
            </For>

            <p class="text-xs opacity-50">
              {data.total} {data.total === 1 ? 'idea' : 'ideas'} · {data.ruleset} as of{' '}
              {formatDate(data.at)}
            </p>
          </div>
        )}
      </Async>
    </div>
  )
}

/** One lifecycle state, and everything currently in it. */
function Group(props: {
  title: string
  blurb: string
  ideas: IdeaSummaryView[]
  href: string
  closed?: boolean
}) {
  const [open, setOpen] = createSignal(!props.closed)

  return (
    <Show when={props.ideas.length > 0}>
      <section class="card bg-base-100 shadow">
        <div class="card-body gap-2">
          <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="card-title text-base">
              <A class="link-hover" href={props.href}>
                {props.title}
              </A>
              <span class="badge badge-ghost badge-sm">{props.ideas.length}</span>
            </h2>
            <button
              class="btn btn-ghost btn-xs"
              aria-expanded={open()}
              onClick={() => setOpen(!open())}
            >
              {open() ? 'Hide' : 'Show'}
            </button>
          </div>
          <p class="text-xs opacity-60">{props.blurb}</p>

          <Show when={open()}>
            <ul class="flex flex-col divide-y divide-base-200">
              <For each={props.ideas}>{(idea) => <Row idea={idea} />}</For>
            </ul>
          </Show>
        </div>
      </section>
    </Show>
  )
}

/** One idea in a group. */
function Row(props: { idea: IdeaSummaryView }) {
  return (
    <li class="flex flex-wrap items-baseline justify-between gap-2 py-2">
      <div class="min-w-0 flex-1">
        <A class="link font-medium" href={ideaHref(props.idea.id)}>
          {props.idea.name}
        </A>
        <div class="text-xs opacity-60">
          {props.idea.captures} {props.idea.captures === 1 ? 'capture' : 'captures'}
          <Show when={props.idea.missing > 0}>
            {' '}
            · {props.idea.missing} missing
          </Show>
          <Show when={props.idea.last_signal}>
            {(signal) => <> · last signal {formatDate(signal())}</>}
          </Show>
          <Show when={props.idea.promoted_to}>
            {(slug) => <> · became {slug()}</>}
          </Show>
        </div>
      </div>

      {/*
        A momentum score is never shown without the way to open what produced
        it. The badge is the link, so there is no arrangement of this row in
        which the number appears on its own.
      */}
      <Show
        when={props.idea.momentum !== null && props.idea.momentum !== undefined}
        fallback={<span class="badge badge-warning badge-sm">no evidence</span>}
      >
        <A
          class="badge badge-ghost badge-sm font-mono"
          href={`${ideaHref(props.idea.id)}#receipt`}
          title="See how this was worked out"
        >
          momentum {props.idea.momentum}
        </A>
      </Show>
    </li>
  )
}

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
