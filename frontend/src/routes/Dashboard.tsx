import type { JSX } from 'solid-js'
import { For, Show, createResource, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { health, pageHref, reindex, stats, tagHref, timeStats, wordStats } from '../api/client'
import type { StatsResponse } from '../api/client'
import { Async, ErrorNotice } from '../components/Async'
import TimeStats from '../components/TimeStats'
import WordStats from '../components/WordStats'
import { formatDate } from './PageDetail'

/**
 * What shape the wiki is in.
 *
 * Orphans and wanted pages are the two numbers worth watching, and they are the
 * same phenomenon from either end: pages nothing names, and names with nothing
 * behind them. A wiki that branches the way this one is meant to accumulates
 * both, and the useful thing a dashboard can do is name them.
 *
 * That symmetry is load-bearing rather than a nice sentence, and it was false
 * for a while. Naming is a wikilink **or** a `contents:` entry: the orphan count
 * unioned the spine in the moment manuscripts existed, because a chapter
 * reported as unreferenced is a number being wrong, and the wanted count did not
 * follow until a book in the example wiki made the graph draw a gap this card
 * refused to count.
 */
export default function Dashboard() {
  const [live] = createResource(() => health())
  const [graph, { refetch }] = createResource(() => stats())
  // Its own request rather than a block on `/api/stats`: the wiki's shape and
  // where the hours went are two questions, the second one needs the reader's
  // timezone to mean anything, and only one of them changes minute to minute.
  const [time, { refetch: refetchTime }] = createResource(() => timeStats())
  // A third, for the same reason the second is separate: where the words went
  // is a question about a wall clock and about the word log, and neither the
  // wiki's shape nor the hours can answer it.
  const [words, { refetch: refetchWords }] = createResource(() => wordStats())
  const [rebuilding, setRebuilding] = createSignal(false)
  const [failure, setFailure] = createSignal<unknown>()

  const rebuild = async () => {
    setRebuilding(true)
    setFailure(undefined)
    try {
      const report = await reindex()
      // The word log is the one tree here with no other copy, so a rebuild that
      // could not read it is worth interrupting for rather than leaving as an
      // empty chart somebody might read as a quiet quarter.
      if (!report.words.read) {
        setFailure(
          new Error(
            'The index was rebuilt, but .rhizolog/words/ could not be read, so the word series is empty. Nothing was lost: the log is on disk and the server log says why.',
          ),
        )
      }
      await refetch()
      await refetchTime()
      await refetchWords()
    } catch (error) {
      setFailure(error)
    } finally {
      setRebuilding(false)
    }
  }

  return (
    <div class="flex flex-col gap-6">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-semibold">Dashboard</h1>
        <div class="flex gap-2">
          <button
            class="btn btn-ghost btn-sm"
            onClick={() => void rebuild()}
            disabled={rebuilding()}
            // Safe by construction: the index holds nothing that is not
            // already in the markdown files it was built from.
            title="Rebuild the search index from the wiki directory"
          >
            <Show when={rebuilding()}>
              <span class="loading loading-spinner loading-xs" />
            </Show>
            Reindex
          </button>
          <A class="btn btn-primary btn-sm" href="/new">
            New page
          </A>
        </div>
      </header>

      <Show when={failure()}>
        <ErrorNotice error={failure()} />
      </Show>

      <Async resource={live}>
        {(server) => (
          <div class="stats stats-vertical sm:stats-horizontal shadow">
            <div class="stat">
              <div class="stat-title">Server</div>
              <div class="stat-value text-success text-2xl">{server.status}</div>
              <div class="stat-desc">v{server.version}</div>
            </div>
            <div class="stat">
              <div class="stat-title">Indexed</div>
              <div class="stat-value text-2xl">{server.pages}</div>
              <div class="stat-desc">
                {server.last_indexed ? formatDate(server.last_indexed) : 'never scanned'}
              </div>
            </div>
            <div class="stat">
              <div class="stat-title">Tracked</div>
              <div class="stat-value text-2xl">{server.times}</div>
              <div class="stat-desc">
                {/*
                  `/api/health` withholds its counts from a caller that has not
                  signed in, so these are optional on the wire. This screen is
                  behind the session gate and never sees that case — but the
                  type is right to insist, and the fallback is the honest one
                  rather than a `!`.
                */}
                {(server.running_timers ?? 0) > 0
                  ? `${server.running_timers} running`
                  : 'time entries'}
              </div>
            </div>
            <div class="stat">
              <div class="stat-title">Wiki root</div>
              {/*
                `whitespace-normal` because daisyUI's `stat-desc` is `nowrap`,
                which defeats `break-all` on its own: a long Windows path then
                sizes the whole stats block and takes the dashboard off the
                right of a phone screen.
              */}
              <div class="stat-desc font-mono break-all whitespace-normal">
                {server.wiki_root}
              </div>
            </div>
          </div>
        )}
      </Async>

      <Async resource={graph}>{(data) => <Graph data={data} />}</Async>

      <Async resource={time}>{(data) => <TimeStats stats={data} />}</Async>

      <Async resource={words}>{(data) => <WordStats stats={data} />}</Async>
    </div>
  )
}

function Graph(props: { data: StatsResponse }) {
  return (
    <div class="flex flex-col gap-4">
      <div class="stats stats-vertical sm:stats-horizontal shadow">
        <div class="stat">
          <div class="stat-title">Pages</div>
          <div class="stat-value text-2xl">{props.data.pages}</div>
        </div>
        <div class="stat">
          <div class="stat-title">Tags</div>
          <div class="stat-value text-2xl">{props.data.tags}</div>
        </div>
        <div class="stat">
          <div class="stat-title">Links</div>
          <div class="stat-value text-2xl">{props.data.links.internal}</div>
          <div class="stat-desc">
            {props.data.links.resolved} resolved · {props.data.links.external} external
          </div>
        </div>
        <div class="stat">
          <div class="stat-title">Orphans</div>
          <div class="stat-value text-2xl">{props.data.orphan_count}</div>
          <div class="stat-desc">nothing names them</div>
        </div>
        <div class="stat">
          <div class="stat-title">Wanted</div>
          <div class="stat-value text-2xl">{props.data.wanted_count}</div>
          <div class="stat-desc">named but unwritten</div>
        </div>
      </div>

      <div class="grid gap-4 lg:grid-cols-3">
        <Panel
          title="Most linked"
          empty="Nothing links anywhere yet."
          items={props.data.most_linked}
        >
          {(page) => (
            <li class="flex items-baseline justify-between gap-2">
              <A class="link" href={pageHref(page.slug)}>
                {page.title}
              </A>
              <span class="badge badge-ghost badge-sm">{page.referrers}</span>
            </li>
          )}
        </Panel>

        <Panel
          title="Wanted pages"
          empty="Every link and every chapter lands somewhere."
          items={props.data.wanted}
        >
          {(page) => (
            <li class="flex items-baseline justify-between gap-2">
              {/*
                A wanted page is browsable: its page view offers to write it,
                and shows what is already pointing at it.
              */}
              <A class="link font-mono text-sm" href={pageHref(page.slug)}>
                {page.slug}
              </A>
              <span class="badge badge-warning badge-sm">{page.referrers}</span>
            </li>
          )}
        </Panel>

        <Panel
          title="Orphans"
          empty="Every page is reachable."
          items={props.data.orphans}
        >
          {(page) => (
            <li>
              <A class="link" href={pageHref(page.slug)}>
                {page.title}
              </A>
              <div class="font-mono text-xs opacity-60">{page.slug}</div>
            </li>
          )}
        </Panel>
      </div>

      <div class="grid gap-4 lg:grid-cols-2">
        <section class="card bg-base-100 shadow">
          <div class="card-body">
            <h2 class="card-title text-base">Tags</h2>
            <div class="flex flex-wrap gap-2">
              <For
                each={props.data.tag_counts}
                fallback={<span class="text-sm opacity-60">No tags in the wiki yet.</span>}
              >
                {(tag) => (
                  <A class="badge badge-outline gap-2" href={tagHref(tag.tag)}>
                    {tag.tag}
                    <span class="opacity-60">{tag.pages}</span>
                  </A>
                )}
              </For>
            </div>
          </div>
        </section>

        <Panel
          title="API usage"
          empty="No API calls recorded yet."
          items={props.data.api_usage}
        >
          {(route) => (
            <li class="flex items-baseline justify-between gap-2 font-mono text-xs">
              <span class="break-all">
                <span class="badge badge-ghost badge-xs mr-2">{route.method}</span>
                {route.route}
              </span>
              <span class="opacity-60">{route.count}</span>
            </li>
          )}
        </Panel>
      </div>
    </div>
  )
}

/** A card holding a list, with something to read when the list is empty. */
function Panel<T>(props: {
  title: string
  empty: string
  items: T[]
  children: (item: T) => JSX.Element
}) {
  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body">
        <h2 class="card-title text-base">{props.title}</h2>
        <ul class="flex flex-col gap-2 text-sm">
          <For each={props.items} fallback={<li class="opacity-60">{props.empty}</li>}>
            {props.children}
          </For>
        </ul>
      </div>
    </section>
  )
}
