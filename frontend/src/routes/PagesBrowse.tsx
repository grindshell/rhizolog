import { createResource, createSignal, For, Show } from 'solid-js'
import { A, useSearchParams } from '@solidjs/router'
import { encodeSlug, listPages, search } from '../api/client'
import type { PageListResponse, SearchResponse } from '../api/client'
import { Async } from '../components/Async'

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}

/**
 * Browse and search. Two endpoints behind one screen: `/api/search` when
 * there is a query, `/api/pages` otherwise. Rendering is intentionally plain.
 */
export default function PagesBrowse() {
  const [searchParams, setSearchParams] = useSearchParams()
  const [draft, setDraft] = createSignal(first(searchParams.q) ?? '')

  const query = () => ({
    q: first(searchParams.q) ?? '',
    tag: first(searchParams.tag) ?? '',
  })

  const [result] = createResource(
    query,
    async ({ q, tag }): Promise<PageListResponse | SearchResponse> => {
      if (q.trim()) return await search({ q: q.trim(), limit: 25 })
      return await listPages({ tag: tag || undefined, limit: 50, sort: 'slug' })
    },
  )

  const submit = (event: SubmitEvent) => {
    event.preventDefault()
    const q = draft().trim()
    setSearchParams({ q: q || undefined })
  }

  return (
    <div class="flex flex-col gap-4">
      <h1 class="text-2xl font-semibold">Pages</h1>

      <form class="join w-full" onSubmit={submit}>
        <input
          class="input input-bordered join-item w-full"
          placeholder="Search page titles and bodies"
          value={draft()}
          onInput={(event) => setDraft(event.currentTarget.value)}
        />
        <button class="btn btn-primary join-item" type="submit">
          Search
        </button>
      </form>

      <Show when={first(searchParams.tag)}>
        {(tag) => (
          <div class="text-sm">
            Filtered by tag <span class="badge badge-primary">{tag()}</span>{' '}
            <A class="link ml-2" href="/pages">
              clear
            </A>
          </div>
        )}
      </Show>

      <Async resource={result}>
        {(data) => (
          <Show
            when={'hits' in data ? data : undefined}
            fallback={<PageTable data={data as PageListResponse} />}
          >
            {(hits) => <SearchTable data={hits()} />}
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
            <th>Slug</th>
            <th>Title</th>
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
                  No pages yet.
                </td>
              </tr>
            }
          >
            {(page) => (
              <tr>
                <td>
                  <A class="link font-mono text-sm" href={`/pages/${encodeSlug(page.slug)}`}>
                    {page.slug}
                  </A>
                </td>
                <td>{page.title}</td>
                <td>
                  <For each={page.tags}>
                    {(tag) => <span class="badge badge-ghost badge-sm mr-1">{tag}</span>}
                  </For>
                </td>
                <td class="text-xs opacity-70">{page.updated}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  )
}

function SearchTable(props: { data: SearchResponse }) {
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
              <A class="link font-medium" href={`/pages/${encodeSlug(hit.slug)}`}>
                {hit.title}
              </A>
              <div class="font-mono text-xs opacity-60">{hit.slug}</div>
              {/*
                The snippet arrives with matched terms wrapped in `<mark>`.
                It is shown as text here rather than injected as HTML — M7 can
                decide how to highlight safely.
              */}
              <div class="text-sm">{hit.snippet}</div>
            </div>
          </div>
        )}
      </For>
    </div>
  )
}
