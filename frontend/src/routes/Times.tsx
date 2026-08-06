import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js'
import { A, useSearchParams } from '@solidjs/router'
import {
  ApiError,
  createTime,
  deleteTime,
  getTime,
  groupHref,
  listTimes,
  pageHref,
  patchTime,
  timeGroups,
} from '../api/client'
import type { TimeGroupView, TimeSummary } from '../api/client'
import { timers } from '../api/timers'
import { Async, ErrorNotice } from '../components/Async'
import Duration, { formatDuration } from '../components/Duration'
import Snippet from '../components/Snippet'
import { formatDate } from './PageDetail'

/** How many entries one page of the log holds. */
const PAGE_SIZE = 50

/** How long typing settles before the search reaches the URL and the API. */
const SEARCH_DELAY_MS = 250

/**
 * The time log.
 *
 * Three things live here and they are deliberately not three screens. Starting
 * a timer, logging an hour you forgot, and reading back what you did are the
 * same activity ten seconds apart, and a wiki whose whole premise is that
 * knowledge branches chaotically should not make you navigate to admit it.
 *
 * Filters go in the URL, so "everything I did on the async notes" is a link.
 */
export default function Times() {
  const [searchParams, setSearchParams] = useSearchParams()
  const [failure, setFailure] = createSignal<unknown>()
  const [editing, setEditing] = createSignal<string>()
  const [reloads, setReloads] = createSignal(0)
  const [draft, setDraft] = createSignal(first(searchParams.q) ?? '')

  // Typing lands in the URL, so a search is a link and survives a refresh, and
  // it composes with the group and page filters already there. `replace` keeps
  // the back button from having to walk out through every keystroke.
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
    name: first(searchParams.name) ?? '',
    page: first(searchParams.page) ?? '',
    // Read so that a change to it re-runs the listing. Any write through the
    // forms below bumps it, which is what keeps the log and the totals in step
    // without either of them knowing about the other.
    reloads: reloads(),
  })

  const [entries] = createResource(query, ({ q, name, page }) =>
    listTimes({
      // Dropped when empty rather than sent as `q=`, which the API reads as a
      // search for nothing and answers with nothing. A cleared box means no
      // filter, not no results.
      q: q || undefined,
      name: name || undefined,
      page: page || undefined,
      limit: PAGE_SIZE,
    }),
  )
  const [groups] = createResource(reloads, () => timeGroups())

  /**
   * Everything on this screen reads from the log, so everything re-reads.
   *
   * Both resources take `reloads` as a source, so bumping it is the whole of
   * the refresh — calling `refetch` as well would double every request, and
   * the totals in the sidebar have to move in step with the entries beside
   * them or the screen contradicts itself for a moment.
   */
  const reload = () => {
    setReloads((count) => count + 1)
    timers.refresh()
  }

  const guard = async (work: () => Promise<unknown>) => {
    setFailure(undefined)
    try {
      await work()
      reload()
    } catch (error) {
      setFailure(error)
    }
  }

  const filters = () =>
    [
      { key: 'q' as const, label: 'matching', value: first(searchParams.q) },
      { key: 'name' as const, label: 'group', value: first(searchParams.name) },
      { key: 'page' as const, label: 'page', value: first(searchParams.page) },
    ].filter((filter) => Boolean(filter.value))

  /**
   * Clearing the search chip has to empty the box as well as the URL — the
   * effect above would otherwise put the search straight back a quarter of a
   * second later, and the chip would look broken.
   */
  const clearFilter = (key: 'q' | 'name' | 'page') => {
    if (key === 'q') setDraft('')
    setSearchParams({ [key]: undefined })
  }

  return (
    <div class="flex flex-col gap-6">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <h1 class="text-2xl font-semibold">Time</h1>
        <input
          class="input input-bordered input-sm min-w-64 grow sm:max-w-lg"
          type="search"
          placeholder="Search notes and activity names — trailing * matches by prefix"
          aria-label="Search the time log"
          value={draft()}
          onInput={(event) => setDraft(event.currentTarget.value)}
        />
      </header>

      <StartForm
        onStart={(name, pages) =>
          void guard(() => timers.start({ name, pages }))
        }
      />

      <Show when={failure()}>
        <ErrorNotice error={failure()} />
      </Show>

      <Running onChange={reload} onFailure={setFailure} />

      <LogForm onLog={(body) => void guard(() => createTime(body))} />

      <Show when={filters().length > 0}>
        <div class="flex flex-wrap items-center gap-2 text-sm">
          <span class="opacity-60">Showing</span>
          <For each={filters()}>
            {(filter) => (
              <span class="badge badge-outline gap-2">
                {filter.label}: {filter.value}
                <button
                  class="opacity-60"
                  aria-label={`Clear the ${filter.label} filter`}
                  onClick={() => clearFilter(filter.key)}
                >
                  ✕
                </button>
              </span>
            )}
          </For>
        </div>
      </Show>

      <div class="grid gap-4 lg:grid-cols-[18rem_1fr]">
        <Async resource={groups}>
          {(data) => (
            <Groups
              groups={data.groups}
              total={data.totals.seconds}
              selected={first(searchParams.name)}
            />
          )}
        </Async>

        <Async resource={entries}>
          {(data) => (
            <section class="card bg-base-100 shadow">
              <div class="card-body gap-3">
                <h2 class="card-title text-base">
                  Entries
                  <span class="badge badge-ghost badge-sm">{data.total}</span>
                </h2>

                <ul class="flex flex-col divide-y divide-base-200">
                  <For
                    each={data.times}
                    fallback={
                      <li class="py-4 text-sm opacity-60">
                        <Show
                          when={filters().length === 0}
                          fallback="Nothing in the log matches. Clear a filter above to widen it."
                        >
                          Nothing tracked yet. Start a timer above, or log time you
                          already spent.
                        </Show>
                      </li>
                    }
                  >
                    {(entry) => (
                      <li class="py-3">
                        <Show
                          when={editing() === entry.id}
                          fallback={
                            <Entry
                              entry={entry}
                              onEdit={() => setEditing(entry.id)}
                              onStop={() => void guard(() => timers.stop(entry.id))}
                              onDelete={() => void guard(() => deleteTime(entry.id))}
                            />
                          }
                        >
                          <EditForm
                            entry={entry}
                            onCancel={() => setEditing(undefined)}
                            onSave={(body) => {
                              setEditing(undefined)
                              void guard(() => patchTime(entry.id, body))
                            }}
                          />
                        </Show>
                      </li>
                    )}
                  </For>
                </ul>

                <Show when={data.total > data.times.length}>
                  <p class="text-xs opacity-60">
                    Showing the {data.times.length} most recent of {data.total}. Narrow
                    it with a group or a page.
                  </p>
                </Show>
              </div>
            </section>
          )}
        </Async>
      </div>
    </div>
  )
}

/** The timers running right now, at the top because they are what is happening. */
function Running(props: { onChange: () => void; onFailure: (error: unknown) => void }) {
  const stop = async (id: string) => {
    try {
      await timers.stop(id)
      props.onChange()
    } catch (error) {
      props.onFailure(error)
      timers.refresh()
    }
  }

  return (
    <Show when={timers.running().length > 0}>
      <section class="card bg-base-100 shadow">
        <div class="card-body gap-2">
          <h2 class="card-title text-base">
            Running
            <span class="badge badge-secondary badge-sm">
              {timers.running().length}
            </span>
          </h2>
          <ul class="flex flex-col gap-2">
            <For each={timers.running()}>
              {(timer) => (
                <li class="flex flex-wrap items-center justify-between gap-3">
                  <div class="min-w-0">
                    <div class="truncate font-medium">{timer.name}</div>
                    <div class="text-xs opacity-60">
                      since {formatDate(timer.start)}
                      <For each={timer.pages}>
                        {(page) => (
                          <>
                            {' · '}
                            <A class="link font-mono" href={pageHref(page.slug)}>
                              {page.slug}
                            </A>
                          </>
                        )}
                      </For>
                    </div>
                  </div>
                  <div class="flex items-center gap-3">
                    <Duration
                      class="font-mono text-lg"
                      seconds={timer.seconds}
                      runningSince={timer.start}
                    />
                    <button class="btn btn-sm" onClick={() => void stop(timer.id)}>
                      Stop
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </div>
      </section>
    </Show>
  )
}

/** Start a timer: a name, optionally some pages, and go. */
function StartForm(props: { onStart: (name: string, pages: string[]) => void }) {
  const [name, setName] = createSignal('')
  const [pages, setPages] = createSignal('')

  const submit = (event: Event) => {
    event.preventDefault()
    const activity = name().trim()
    if (!activity) return
    props.onStart(activity, parsePages(pages()))
    setName('')
    setPages('')
  }

  return (
    <form class="flex flex-wrap items-end gap-2" onSubmit={submit}>
      <label class="form-control min-w-64 flex-1">
        <span class="label-text text-xs opacity-60">What are you doing?</span>
        <input
          class="input input-bordered w-full"
          placeholder="Deep work"
          aria-label="Activity name"
          value={name()}
          onInput={(event) => setName(event.currentTarget.value)}
        />
      </label>
      <label class="form-control min-w-64 flex-1">
        <span class="label-text text-xs opacity-60">Pages (comma separated)</span>
        <input
          class="input input-bordered w-full font-mono text-sm"
          placeholder="notes/rust/async"
          aria-label="Pages"
          value={pages()}
          onInput={(event) => setPages(event.currentTarget.value)}
        />
      </label>
      <button class="btn btn-primary" type="submit" disabled={!name().trim()}>
        Start
      </button>
    </form>
  )
}

/** Log time that is already over. Collapsed, because it is the rarer case. */
function LogForm(props: {
  onLog: (body: {
    name: string
    start: string
    end: string
    pages: string[]
    note: string
  }) => void
}) {
  const [open, setOpen] = createSignal(false)
  const [name, setName] = createSignal('')
  const [start, setStart] = createSignal('')
  const [end, setEnd] = createSignal('')
  const [pages, setPages] = createSignal('')
  const [note, setNote] = createSignal('')

  const submit = (event: Event) => {
    event.preventDefault()
    if (!name().trim() || !start() || !end()) return
    props.onLog({
      name: name().trim(),
      start: toIso(start()),
      end: toIso(end()),
      pages: parsePages(pages()),
      note: note(),
    })
    setName('')
    setStart('')
    setEnd('')
    setPages('')
    setNote('')
    setOpen(false)
  }

  return (
    <Show
      when={open()}
      fallback={
        <button class="btn btn-ghost btn-sm self-start" onClick={() => setOpen(true)}>
          Log time you already spent
        </button>
      }
    >
      <form class="card bg-base-100 shadow" onSubmit={submit}>
        <div class="card-body gap-3">
          <h2 class="card-title text-base">Log an entry</h2>
          <div class="grid gap-3 sm:grid-cols-3">
            <label class="form-control">
              <span class="label-text text-xs opacity-60">Activity</span>
              <input
                class="input input-bordered"
                aria-label="Activity name"
                value={name()}
                onInput={(event) => setName(event.currentTarget.value)}
              />
            </label>
            <label class="form-control">
              <span class="label-text text-xs opacity-60">Start</span>
              <input
                class="input input-bordered"
                type="datetime-local"
                aria-label="Start"
                value={start()}
                onInput={(event) => setStart(event.currentTarget.value)}
              />
            </label>
            <label class="form-control">
              <span class="label-text text-xs opacity-60">End</span>
              <input
                class="input input-bordered"
                type="datetime-local"
                aria-label="End"
                value={end()}
                onInput={(event) => setEnd(event.currentTarget.value)}
              />
            </label>
          </div>
          <label class="form-control">
            <span class="label-text text-xs opacity-60">Pages (comma separated)</span>
            <input
              class="input input-bordered font-mono text-sm"
              aria-label="Pages"
              value={pages()}
              onInput={(event) => setPages(event.currentTarget.value)}
            />
          </label>
          <label class="form-control">
            <span class="label-text text-xs opacity-60">Note (markdown, optional)</span>
            <textarea
              class="textarea textarea-bordered min-h-24 font-mono text-sm"
              aria-label="Note"
              value={note()}
              onInput={(event) => setNote(event.currentTarget.value)}
            />
          </label>
          <div class="flex gap-2">
            <button class="btn btn-primary btn-sm" type="submit">
              Log it
            </button>
            <button
              class="btn btn-ghost btn-sm"
              type="button"
              onClick={() => setOpen(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      </form>
    </Show>
  )
}

/** One row of the log. */
function Entry(props: {
  entry: TimeSummary
  onEdit: () => void
  onStop: () => void
  onDelete: () => void
}) {
  return (
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-2">
          <A class="link font-medium" href={groupHref(props.entry.name)}>
            {props.entry.name}
          </A>
          <Show when={props.entry.running}>
            <span class="badge badge-secondary badge-xs">running</span>
          </Show>
          <Show when={props.entry.has_note}>
            <span class="badge badge-ghost badge-xs">note</span>
          </Show>
        </div>
        <div class="text-xs opacity-60">
          {formatDate(props.entry.start)}
          <Show when={props.entry.end}>{(end) => <> → {formatDate(end())}</>}</Show>
        </div>
        {/*
          Only present when a search matched the note, and rendered as text
          rather than markup — a note is written through the API like a page
          body, so the same rule applies. See `Snippet`.
        */}
        <Show when={props.entry.snippet}>
          {(snippet) => (
            <p class="pt-1 text-sm opacity-80">
              <Snippet text={snippet()} />
            </p>
          )}
        </Show>
        <div class="flex flex-wrap gap-1 pt-1">
          <For each={props.entry.pages}>
            {(page) => (
              <A
                class="badge badge-outline badge-sm font-mono"
                href={pageHref(page.slug)}
                title={page.exists ? page.title ?? page.slug : 'This page is not written yet'}
              >
                {page.slug}
                <Show when={!page.exists}>
                  <span class="text-warning">?</span>
                </Show>
              </A>
            )}
          </For>
        </div>
      </div>

      <div class="flex items-center gap-2">
        <Duration
          class="font-mono"
          seconds={props.entry.seconds}
          runningSince={props.entry.running ? props.entry.start : null}
        />
        <Show when={props.entry.running}>
          <button class="btn btn-xs" onClick={props.onStop}>
            Stop
          </button>
        </Show>
        <button class="btn btn-ghost btn-xs" onClick={props.onEdit}>
          Edit
        </button>
        <button
          class="btn btn-ghost btn-xs text-error"
          aria-label={`Delete ${props.entry.name}`}
          onClick={props.onDelete}
        >
          Delete
        </button>
      </div>
    </div>
  )
}

/**
 * Edit one entry in place.
 *
 * The note is fetched only when a row is opened: a listing carries `has_note`
 * rather than the notes themselves, precisely so a year of them is not on the
 * wire every time the log is read.
 */
function EditForm(props: {
  entry: TimeSummary
  onCancel: () => void
  onSave: (body: {
    name: string
    start: string
    end: string | null
    pages: string[]
    note: string
  }) => void
}) {
  const [full] = createResource(
    () => props.entry.id,
    (id) => getTime(id),
  )

  const [name, setName] = createSignal(props.entry.name)
  const [start, setStart] = createSignal(toLocalInput(props.entry.start))
  const [end, setEnd] = createSignal(
    props.entry.end ? toLocalInput(props.entry.end) : '',
  )
  const [pages, setPages] = createSignal(
    props.entry.pages.map((page) => page.slug).join(', '),
  )
  const [note, setNote] = createSignal<string>()

  // Filled in once the note arrives, unless the field has been typed in
  // meanwhile — a fetch that lands late must not overwrite what was typed.
  const noteValue = () => note() ?? full()?.note ?? ''

  const submit = (event: Event) => {
    event.preventDefault()
    props.onSave({
      name: name().trim(),
      start: toIso(start()),
      end: end() ? toIso(end()) : null,
      pages: parsePages(pages()),
      note: noteValue(),
    })
  }

  return (
    <form class="flex flex-col gap-3" onSubmit={submit}>
      <div class="grid gap-3 sm:grid-cols-3">
        <label class="form-control">
          <span class="label-text text-xs opacity-60">Activity</span>
          <input
            class="input input-bordered input-sm"
            aria-label="Activity name"
            value={name()}
            onInput={(event) => setName(event.currentTarget.value)}
          />
        </label>
        <label class="form-control">
          <span class="label-text text-xs opacity-60">Start</span>
          <input
            class="input input-bordered input-sm"
            type="datetime-local"
            aria-label="Start"
            value={start()}
            onInput={(event) => setStart(event.currentTarget.value)}
          />
        </label>
        <label class="form-control">
          <span class="label-text text-xs opacity-60">
            End — empty leaves it running
          </span>
          <input
            class="input input-bordered input-sm"
            type="datetime-local"
            aria-label="End"
            value={end()}
            onInput={(event) => setEnd(event.currentTarget.value)}
          />
        </label>
      </div>
      <label class="form-control">
        <span class="label-text text-xs opacity-60">Pages (comma separated)</span>
        <input
          class="input input-bordered input-sm font-mono text-sm"
          aria-label="Pages"
          value={pages()}
          onInput={(event) => setPages(event.currentTarget.value)}
        />
      </label>
      <label class="form-control">
        <span class="label-text text-xs opacity-60">Note (markdown)</span>
        <textarea
          class="textarea textarea-bordered min-h-24 font-mono text-sm"
          aria-label="Note"
          value={noteValue()}
          onInput={(event) => setNote(event.currentTarget.value)}
        />
      </label>
      <div class="flex items-center gap-2">
        <button class="btn btn-primary btn-sm" type="submit">
          Save
        </button>
        <button class="btn btn-ghost btn-sm" type="button" onClick={props.onCancel}>
          Cancel
        </button>
        <Show when={full.error}>
          <span class="text-error text-xs">
            {full.error instanceof ApiError ? full.error.message : 'Could not load the note.'}
          </span>
        </Show>
      </div>
    </form>
  )
}

/** Every activity, as a filter and as a total. */
function Groups(props: {
  groups: TimeGroupView[]
  total: number
  selected: string | undefined
}) {
  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body gap-2">
        <h2 class="card-title text-base">
          Groups
          <span class="badge badge-ghost badge-sm">{props.groups.length}</span>
        </h2>
        <ul class="flex flex-col gap-1 text-sm">
          <For
            each={props.groups}
            fallback={<li class="opacity-60">No time tracked yet.</li>}
          >
            {(group) => (
              <li>
                <A
                  class="flex items-baseline justify-between gap-2 rounded px-2 py-1 hover:bg-base-200"
                  classList={{ 'bg-base-200': props.selected === group.name }}
                  href={groupHref(group.name)}
                >
                  <span class="min-w-0 truncate">
                    {group.name}
                    <Show when={group.running > 0}>
                      <span class="badge badge-secondary badge-xs ml-2">running</span>
                    </Show>
                  </span>
                  <span class="font-mono text-xs whitespace-nowrap opacity-70">
                    {formatDuration(group.seconds)}
                  </span>
                </A>
              </li>
            )}
          </For>
        </ul>
        <Show when={props.groups.length > 0}>
          <div class="border-t border-base-200 pt-2 text-sm">
            <span class="opacity-60">All time</span>{' '}
            <span class="float-right font-mono">{formatDuration(props.total)}</span>
          </div>
        </Show>
      </div>
    </section>
  )
}

/**
 * `notes/a, notes/b` becomes two slugs.
 *
 * Comma separated rather than a picker: slugs are typed here the way they are
 * typed in a wikilink, and a picker would need the whole page list loaded to
 * offer something the keyboard already does.
 */
function parsePages(raw: string): string[] {
  return raw
    .split(',')
    .map((slug) => slug.trim())
    .filter((slug) => slug.length > 0)
}

/**
 * A `datetime-local` value is local wall-clock time with no zone. `new Date`
 * reads it in the browser's zone, which is exactly what the person typing it
 * meant, and `toISOString` hands the API the UTC instant it wants.
 */
function toIso(local: string): string {
  return new Date(local).toISOString()
}

/** The inverse, for filling a `datetime-local` field from an API timestamp. */
function toLocalInput(iso: string): string {
  const at = new Date(iso)
  if (Number.isNaN(at.getTime())) return ''
  const pad = (value: number) => String(value).padStart(2, '0')
  return (
    `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}` +
    `T${pad(at.getHours())}:${pad(at.getMinutes())}`
  )
}

export { parsePages, toLocalInput }

function first(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
