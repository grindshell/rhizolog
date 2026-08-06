import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import { A, useNavigate, useSearchParams } from '@solidjs/router'
import {
  editHref,
  linkGraph,
  pageHref,
  prefixHref,
  tagHref,
  tags as listTags,
} from '../api/client'
import type { GraphResponse } from '../api/client'
import { Async } from '../components/Async'
import LinkGraph from '../components/graph/LinkGraph'
import SlugPath from '../components/SlugPath'

/** The walk depths worth offering. Past a handful it is the whole wiki again. */
const DEPTHS = [1, 2, 3, 4]

/**
 * The wiki as a picture.
 *
 * The dashboard already counts the graph — links, orphans, wanted pages — and
 * counting is the wrong tool for the question this screen answers. "Six orphans"
 * does not tell you they are all in `scratch/`; a hairball with a detached
 * cluster off to one side does, at a glance, and that is the only thing a
 * drawing is better at than a list.
 *
 * Everything is in the URL, so a view is a link: `?root=` for a page's
 * neighbourhood, `?prefix=` and `?tag=` for a branch of the wiki, `?wanted=no`
 * for the written pages alone. The page view links here with its own slug as the
 * root, which is the way most people will arrive.
 *
 * Selection is a signal rather than a URL parameter, deliberately. The filters
 * say what is drawn and are worth sharing; which node you happen to be pointing
 * at is not, and putting it in the URL would push a history entry on every
 * click.
 */
export default function GraphView() {
  const [searchParams, setSearchParams] = useSearchParams()
  const navigate = useNavigate()
  const [selected, setSelected] = createSignal<string>()

  const root = () => first(searchParams.root) ?? ''
  const prefix = () => first(searchParams.prefix) ?? ''
  const tag = () => first(searchParams.tag) ?? ''
  const depth = () => Number(first(searchParams.depth) ?? 2) || 2
  // Spelled `no` rather than `false` in the URL because it is read by people.
  const wanted = () => first(searchParams.wanted) !== 'no'

  const query = () => ({
    root: root() || undefined,
    prefix: prefix() || undefined,
    tag: tag() || undefined,
    depth: root() ? depth() : undefined,
    wanted: wanted() ? undefined : false,
  })

  const [graph] = createResource(query, (params) => linkGraph(params))
  const [allTags] = createResource(() => listTags())

  /** Changing a filter drops a selection that may no longer be drawn. */
  const narrow = (patch: Record<string, string | undefined>) => {
    setSelected(undefined)
    setSearchParams(patch)
  }

  return (
    <div class="flex flex-col gap-4">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 class="text-2xl font-semibold">Graph</h1>
          <p class="text-sm opacity-60">
            Pages and the links between them. Time is not in here — see the
            dashboard for where the hours went.
          </p>
        </div>
        <Show when={root()}>
          <button class="btn btn-ghost btn-sm" onClick={() => narrow({ root: undefined, depth: undefined })}>
            Show the whole wiki
          </button>
        </Show>
      </header>

      <section class="card bg-base-100 shadow">
        <div class="card-body flex-row flex-wrap items-end gap-4 py-4">
          <label class="form-control">
            <span class="label-text text-xs opacity-70">Around</span>
            <input
              class="input input-bordered input-sm w-64 font-mono"
              placeholder="every page"
              value={root()}
              onChange={(event) =>
                narrow({ root: event.currentTarget.value.trim() || undefined })
              }
            />
          </label>

          <label class="form-control">
            <span class="label-text text-xs opacity-70">Hops</span>
            <select
              class="select select-bordered select-sm"
              disabled={!root()}
              value={String(depth())}
              onChange={(event) => narrow({ depth: event.currentTarget.value })}
            >
              <For each={DEPTHS}>{(value) => <option value={value}>{value}</option>}</For>
            </select>
          </label>

          <label class="form-control">
            <span class="label-text text-xs opacity-70">Under</span>
            <input
              class="input input-bordered input-sm w-56 font-mono"
              placeholder="any path"
              value={prefix()}
              onChange={(event) =>
                narrow({ prefix: event.currentTarget.value.trim() || undefined })
              }
            />
          </label>

          <label class="form-control">
            <span class="label-text text-xs opacity-70">Tagged</span>
            <select
              class="select select-bordered select-sm"
              value={tag()}
              onChange={(event) => narrow({ tag: event.currentTarget.value || undefined })}
            >
              <option value="">any tag</option>
              <For each={allTags()?.tags ?? []}>
                {(count) => <option value={count.tag}>{count.tag}</option>}
              </For>
            </select>
          </label>

          <label class="label cursor-pointer gap-2">
            <input
              type="checkbox"
              class="checkbox checkbox-sm"
              checked={wanted()}
              onChange={(event) =>
                narrow({ wanted: event.currentTarget.checked ? undefined : 'no' })
              }
            />
            <span class="label-text text-sm">Wanted pages</span>
          </label>
        </div>
      </section>

      <Async resource={graph}>
        {(data) => (
          <div class="flex flex-col gap-4 lg:flex-row">
            <div class="min-w-0 flex-1">
              <Show
                when={data.nodes.length > 0}
                fallback={
                  <div class="card bg-base-100 shadow">
                    <div class="card-body items-start">
                      <h2 class="card-title text-base">Nothing to draw</h2>
                      <p class="text-sm opacity-70">
                        No page matched. A wiki with pages but no links between
                        them still draws — this means the filters excluded
                        everything.
                      </p>
                    </div>
                  </div>
                }
              >
                <LinkGraph
                  graph={data}
                  root={root() || undefined}
                  selected={selected()}
                  onSelect={setSelected}
                  onOpen={(slug) => navigate(pageHref(slug))}
                />
              </Show>
            </div>

            <aside class="flex w-full flex-col gap-4 lg:w-80">
              <Summary graph={data} />
              <Show
                when={selected()}
                fallback={
                  <div class="card bg-base-100 shadow">
                    <div class="card-body text-sm opacity-60">
                      Click a node to read it here. Click it again — or
                      double-click — to open the page.
                    </div>
                  </div>
                }
              >
                {(slug) => (
                  <Selected
                    graph={data}
                    slug={slug()}
                    onFocus={() => narrow({ root: slug() })}
                  />
                )}
              </Show>
            </aside>
          </div>
        )}
      </Async>
    </div>
  )
}

function Summary(props: { graph: GraphResponse }) {
  const written = () => props.graph.nodes.filter((node) => node.exists).length
  const wanted = () => props.graph.nodes.length - written()
  /** Pages in the view that nothing in the view points at. */
  const orphans = () => {
    const arrived = new Set(props.graph.edges.map((edge) => edge.target))
    return props.graph.nodes.filter((node) => node.exists && !arrived.has(node.slug)).length
  }

  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body gap-2 py-4">
        <h2 class="card-title text-base">In this view</h2>
        <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
          <dt class="opacity-60">Pages</dt>
          <dd class="text-right font-mono">{written()}</dd>
          <dt class="opacity-60">Wanted</dt>
          <dd class="text-right font-mono">{wanted()}</dd>
          <dt class="opacity-60">Links</dt>
          <dd class="text-right font-mono">{props.graph.edges.length}</dd>
          <dt class="opacity-60">Unreached</dt>
          <dd class="text-right font-mono">{orphans()}</dd>
        </dl>
        <Show when={props.graph.truncated}>
          <p class="text-xs text-warning">
            {props.graph.matched} pages matched and the {props.graph.limit}
            {' '}best-connected are drawn. Narrow it with a path or a tag to see
            the rest.
          </p>
        </Show>
      </div>
    </section>
  )
}

/**
 * The node under the cursor, in words.
 *
 * The counts here are the wiki's, not the view's, which is why they can exceed
 * the lines drawn at the node — and that difference is worth reading: it says
 * this branch is attached to something outside what you asked for.
 */
function Selected(props: { graph: GraphResponse; slug: string; onFocus: () => void }) {
  const node = createMemo(() =>
    props.graph.nodes.find((candidate) => candidate.slug === props.slug),
  )
  const drawn = createMemo(() => {
    const edges = props.graph.edges
    return {
      out: edges.filter((edge) => edge.source === props.slug).length,
      in: edges.filter((edge) => edge.target === props.slug).length,
    }
  })

  return (
    <Show when={node()}>
      {(found) => (
        <section class="card bg-base-100 shadow">
          <div class="card-body gap-3">
            <div>
              <h2 class="card-title text-base">
                {found().title}
                <Show when={!found().exists}>
                  <span class="badge badge-warning badge-sm">wanted</span>
                </Show>
              </h2>
              <SlugPath class="font-mono text-xs opacity-60" slug={found().slug} />
            </div>

            <Show when={found().tags.length > 0}>
              <div class="flex flex-wrap gap-1">
                <For each={found().tags}>
                  {(tag) => (
                    <A class="badge badge-outline badge-sm" href={tagHref(tag)}>
                      {tag}
                    </A>
                  )}
                </For>
              </div>
            </Show>

            <dl class="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
              <dt class="opacity-60">Links in</dt>
              <dd class="text-right font-mono">
                {found().inbound}
                <Show when={found().inbound !== drawn().in}>
                  <span class="opacity-50"> ({drawn().in} here)</span>
                </Show>
              </dd>
              <dt class="opacity-60">Links out</dt>
              <dd class="text-right font-mono">
                {found().outbound}
                <Show when={found().outbound !== drawn().out}>
                  <span class="opacity-50"> ({drawn().out} here)</span>
                </Show>
              </dd>
              <Show when={found().distance !== null && found().distance !== undefined}>
                <dt class="opacity-60">Hops away</dt>
                <dd class="text-right font-mono">{found().distance}</dd>
              </Show>
            </dl>

            <div class="flex flex-wrap gap-2">
              <A class="btn btn-primary btn-sm" href={pageHref(found().slug)}>
                {found().exists ? 'Open' : 'What wants it'}
              </A>
              <button class="btn btn-sm" onClick={props.onFocus}>
                Centre here
              </button>
              <Show
                when={found().exists}
                fallback={
                  <A
                    class="btn btn-sm"
                    href={`/new?slug=${encodeURIComponent(found().slug)}`}
                  >
                    Write it
                  </A>
                }
              >
                <A class="btn btn-ghost btn-sm" href={editHref(found().slug)}>
                  Edit
                </A>
              </Show>
              <Show when={found().slug.includes('/')}>
                <A
                  class="btn btn-ghost btn-sm"
                  href={prefixHref(found().slug.slice(0, found().slug.lastIndexOf('/')))}
                >
                  Siblings
                </A>
              </Show>
            </div>
          </div>
        </section>
      )}
    </Show>
  )
}

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
