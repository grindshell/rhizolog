import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js'
import { A, useSearchParams } from '@solidjs/router'
import { listPages, pageHref, search, tagHref } from '../api/client'
import type { PageListResponse, SearchResponse } from '../api/client'
import { Async } from '../components/Async'
import Snippet from '../components/Snippet'
import SlugPath from '../components/SlugPath'
import { formatDate } from './PageDetail'

/** How long typing has to pause before the search runs. */
const SEARCH_DELAY_MS = 250

const SORTS = [
  { value: 'updated', label: 'Recently updated' },
  { value: 'slug', label: 'Slug' },
  { value: 'title', label: 'Title' },
] as const

/**
 * Browse and search.
 *
 * Two endpoints behind one screen: `/api/search` when there is a query,
 * `/api/pages` otherwise. They answer different questions — "where is this
 * word" and "what is in here" — and which one you want is decided entirely by
 * whether you typed something.
 */
export default function PagesBrowse() {
  const [searchParams, setSearchParams] = useSearchParams()
  const [draft, setDraft] = createSignal(first(searchParams.q) ?? '')

  // Typing updates the URL, so a search can be linked to and survives a
  // refresh. `replace` keeps the back button from having to walk out through
  // every intermediate keystroke.
  createEffect(() => {
    const typed = draft().trim()
    const timer = setTimeout(
      () => setSearchParams({ q: typed || undefined }, { replace: true }),
      SEARCH_DELAY_MS,
    )
    onCleanup(() => clearTimeout(timer))
  })

  const query = () => ({
    q: first(searchParams.q) ?? '',
    tag: first(searchParams.tag) ?? '',
    prefix: first(searchParams.prefix) ?? '',
    segment: first(searchParams.segment) ?? '',
    sort: first(searchParams.sort) ?? 'updated',
  })

  const [result] = createResource(
    query,
    async ({ q, tag, prefix, segment, sort }): Promise<PageListResponse | SearchResponse> => {
      if (q) return await search({ q, limit: 25 })
      return await listPages({
        tag: tag || undefined,
        prefix: prefix || undefined,
        segment: segment || undefined,
        limit: 100,
        sort,
        order: sort === 'updated' ? 'desc' : 'asc',
      })
    },
  )

  /**
   * The narrowing currently in force, in the order it reads.
   *
   * `/api/search` takes none of these, so typing a query drops all of them —
   * the banner goes with them rather than describing a listing that is not on
   * screen.
   */
  const filters = () =>
    [
      { label: 'tagged', value: first(searchParams.tag) },
      { label: 'under', value: first(searchParams.prefix) },
      { label: 'in a directory named', value: first(searchParams.segment) },
    ].filter((filter) => filter.value)

  return (
    <div class="flex flex-col gap-4">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-semibold">Pages</h1>
        <A class="btn btn-primary btn-sm" href="/new">
          New page
        </A>
      </header>

      <div class="flex flex-wrap gap-2">
        <input
          class="input input-bordered grow"
          type="search"
          placeholder="Search titles and bodies — trailing * matches by prefix"
          value={draft()}
          onInput={(event) => setDraft(event.currentTarget.value)}
        />
        <Show when={!first(searchParams.q)}>
          <select
            class="select select-bordered"
            value={query().sort}
            onChange={(event) =>
              setSearchParams({ sort: event.currentTarget.value }, { replace: true })
            }
          >
            <For each={SORTS}>
              {(sort) => <option value={sort.value}>{sort.label}</option>}
            </For>
          </select>
        </Show>
      </div>

      <Show when={!first(searchParams.q) && filters().length > 0}>
        <div class="flex flex-wrap items-center gap-2 text-sm">
          <span class="opacity-70">Showing pages</span>
          <For each={filters()}>
            {(filter) => (
              <span class="flex items-center gap-1">
                <span class="opacity-70">{filter.label}</span>
                <span class="badge badge-primary font-mono">{filter.value}</span>
              </span>
            )}
          </For>
          <A class="link" href="/pages">
            clear
          </A>
        </div>
      </Show>

      <Async resource={result}>
        {(data) => (
          <Show
            when={'hits' in data ? data : undefined}
            fallback={<PageTable data={data as PageListResponse} />}
          >
            {(hits) => <SearchResults data={hits()} />}
          </Show>
        )}
      </Async>
    </div>
  )
}

function PageTable(props: { data: PageListResponse }) {
  return (
    <div class="overflow-x-auto">
      <div class="mb-2 text-sm opacity-70">
        {props.data.pages.length} of {props.data.total} pages
      </div>
      <table class="table table-zebra">
        <thead>
          <tr>
            <th>Title</th>
            <th>Slug</th>
            <th>Tags</th>
            <th>Updated</th>
          </tr>
        </thead>
        <tbody>
          <For
            each={props.data.pages}
            fallback={
              <tr>
                <td colSpan={4} class="opacity-60">
                  No pages here yet. Write one.
                </td>
              </tr>
            }
          >
            {(page) => (
              <tr>
                <td>
                  <A class="link font-medium" href={pageHref(page.slug)}>
                    {page.title}
                  </A>
                </td>
                <td>
                  <SlugPath class="font-mono text-sm opacity-70" slug={page.slug} />
                </td>
                <td>
                  <For each={page.tags}>
                    {(tag) => (
                      <A class="badge badge-ghost badge-sm mr-1" href={tagHref(tag)}>
                        {tag}
                      </A>
                    )}
                  </For>
                </td>
                <td class="text-xs opacity-70">{formatDate(page.updated)}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  )
}

function SearchResults(props: { data: SearchResponse }) {
  return (
    <div class="flex flex-col gap-3">
      <div class="text-sm opacity-70">
        {props.data.hits.length} of {props.data.total} matches
      </div>
      <For
        each={props.data.hits}
        fallback={<div class="opacity-60">Nothing matched.</div>}
      >
        {(hit) => (
          <div class="card bg-base-100 shadow-sm">
            <div class="card-body gap-1 p-4">
              <A class="link font-medium" href={pageHref(hit.slug)}>
                {hit.title}
              </A>
              <SlugPath class="font-mono text-xs opacity-60" slug={hit.slug} />
              <div class="text-sm">
                <Snippet text={hit.snippet} />
              </div>
            </div>
          </div>
        )}
      </For>
    </div>
  )
}

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
