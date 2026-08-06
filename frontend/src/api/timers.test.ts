import { afterEach, describe, expect, it, vi } from 'vitest'
import { createRoot } from 'solid-js'
import type { TimeSummary, TimeView } from './client'

const api = vi.hoisted(() => ({
  listTimes: vi.fn(),
  createTime: vi.fn(),
  stopTime: vi.fn(),
  patchTime: vi.fn(),
  deleteTime: vi.fn(),
}))

vi.mock('./client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./client')>()
  return { ...actual, ...api }
})

const { createTimerStore } = await import('./timers')

function summary(overrides: Partial<TimeSummary> = {}): TimeSummary {
  return {
    id: '20260806T140000-000000000',
    name: 'Deep work',
    start: '2026-08-06T14:00:00Z',
    end: null,
    running: true,
    seconds: 600,
    pages: [],
    has_note: false,
    updated: '2026-08-06T14:00:00Z',
    size: 96,
    ...overrides,
  }
}

function view(overrides: Partial<TimeView> = {}): TimeView {
  return { ...summary(), note: '', ...overrides }
}

function page(slug: string) {
  return { slug, title: slug.toUpperCase(), exists: true }
}

/** A store with its first fetch settled and no polling to interfere. */
async function store(running: TimeSummary[] = []) {
  api.listTimes.mockResolvedValue({
    times: running,
    total: running.length,
    limit: 50,
    offset: 0,
  })
  const created = createRoot(() => createTimerStore(0))
  await vi.waitFor(() => expect(created.loading()).toBe(false))
  return created
}

const ids = (store: { running: () => TimeSummary[] }) =>
  store.running().map((timer) => timer.id)

afterEach(() => {
  vi.clearAllMocks()
})

describe('reading', () => {
  it('asks only for running timers', async () => {
    await store()
    expect(api.listTimes).toHaveBeenCalledWith({ running: true, order: 'asc' })
  })

  it('exposes what the server returned', async () => {
    const timers = await store([summary(), summary({ id: '20260806T150000-000000000' })])

    expect(ids(timers)).toEqual([
      '20260806T140000-000000000',
      '20260806T150000-000000000',
    ])
  })

  /**
   * The reason `resource.latest` is guarded. Read unguarded it rethrows, and
   * this store is read from the app shell — a backend that is merely down
   * would throw out of the navbar and take every page with it.
   */
  it('is empty rather than broken when the list cannot be fetched', async () => {
    api.listTimes.mockRejectedValue(new Error('offline'))
    const timers = createRoot(() => createTimerStore(0))

    await vi.waitFor(() => expect(timers.error()).toBeDefined())
    expect(timers.running()).toEqual([])
  })

  it('finds the timers tracking a page', async () => {
    const timers = await store([
      summary({ pages: [page('notes/a')] }),
      summary({ id: '20260806T150000-000000000', pages: [page('notes/b')] }),
    ])

    expect(timers.forPage('notes/a').map((timer) => timer.id)).toEqual([
      '20260806T140000-000000000',
    ])
    expect(timers.forPage('notes/c')).toEqual([])
  })
})

describe('writing', () => {
  it('adds a started timer without refetching', async () => {
    const timers = await store()
    api.createTime.mockResolvedValue(view())

    await timers.start({ name: 'Deep work' })

    expect(ids(timers)).toEqual(['20260806T140000-000000000'])
    expect(api.listTimes).toHaveBeenCalledTimes(1)
  })

  it('drops a stopped timer from the list', async () => {
    const timers = await store([summary(), summary({ id: '20260806T150000-000000000' })])
    api.stopTime.mockResolvedValue(
      view({ running: false, end: '2026-08-06T14:30:00Z' }),
    )

    await timers.stop('20260806T140000-000000000')

    expect(ids(timers)).toEqual(['20260806T150000-000000000'])
  })

  /** Editing a running timer must not reshuffle the bar it is displayed in. */
  it('keeps the list oldest-first when an entry is edited', async () => {
    const timers = await store([
      summary({ id: '20260806T100000-000000000', start: '2026-08-06T10:00:00Z' }),
      summary({ id: '20260806T140000-000000000', start: '2026-08-06T14:00:00Z' }),
    ])
    api.patchTime.mockResolvedValue(
      view({
        id: '20260806T100000-000000000',
        start: '2026-08-06T10:00:00Z',
        name: 'Renamed',
      }),
    )

    await timers.patch('20260806T100000-000000000', { name: 'Renamed' })

    expect(ids(timers)).toEqual([
      '20260806T100000-000000000',
      '20260806T140000-000000000',
    ])
    expect(timers.running()[0]?.name).toBe('Renamed')
  })

  /** A patch that clears the end sets a timer running, so it comes back. */
  it('re-adds a timer that a patch set running again', async () => {
    const timers = await store()
    api.patchTime.mockResolvedValue(view({ running: true, end: null }))

    await timers.patch('20260806T140000-000000000', { end: null })

    expect(ids(timers)).toEqual(['20260806T140000-000000000'])
  })

  it('drops a deleted timer', async () => {
    const timers = await store([summary()])
    api.deleteTime.mockResolvedValue(undefined)

    await timers.remove('20260806T140000-000000000')

    expect(timers.running()).toEqual([])
  })

  it('derives has_note from the note the server returned', async () => {
    const timers = await store()
    api.createTime.mockResolvedValue(view({ note: 'Something.\n' }))

    await timers.start({ name: 'Deep work' })

    expect(timers.running()[0]?.has_note).toBe(true)
  })
})
