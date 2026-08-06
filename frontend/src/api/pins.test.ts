import { afterEach, describe, expect, it, vi } from 'vitest'
import { createRoot } from 'solid-js'
import type { PinView } from './client'

const api = vi.hoisted(() => ({
  listPins: vi.fn(),
  pinPage: vi.fn(),
  unpinPage: vi.fn(),
}))

vi.mock('./client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./client')>()
  return { ...actual, ...api }
})

const { createPinStore } = await import('./pins')

function pin(overrides: Partial<PinView> = {}): PinView {
  return {
    slug: 'notes/quick',
    title: 'Quick Notes',
    exists: true,
    pinned_at: '2026-08-05T14:00:00Z',
    ...overrides,
  }
}

/** A store with its first fetch already settled. */
async function store(pins: PinView[] = []) {
  api.listPins.mockResolvedValue({ pins, limit: 50 })
  const created = createRoot(() => createPinStore())
  await vi.waitFor(() => expect(created.loading()).toBe(false))
  return created
}

const slugs = (store: { pins: () => PinView[] }) => store.pins().map((entry) => entry.slug)

afterEach(() => {
  vi.clearAllMocks()
})

describe('reading', () => {
  it('exposes what the server returned', async () => {
    const pins = await store([pin(), pin({ slug: 'index', title: 'Index' })])

    expect(slugs(pins)).toEqual(['notes/quick', 'index'])
    expect(pins.limit()).toBe(50)
    expect(pins.isPinned('notes/quick')).toBe(true)
    expect(pins.isPinned('notes/rust/async')).toBe(false)
  })

  it('is empty rather than broken when the list cannot be fetched', async () => {
    api.listPins.mockRejectedValue(new Error('offline'))
    const pins = createRoot(() => createPinStore())

    await vi.waitFor(() => expect(pins.error()).toBeDefined())
    expect(pins.pins()).toEqual([])
  })

  it('knows when the list is full', async () => {
    api.listPins.mockResolvedValue({ pins: [pin()], limit: 1 })
    const pins = createRoot(() => createPinStore())

    await vi.waitFor(() => expect(pins.loading()).toBe(false))
    expect(pins.full()).toBe(true)
  })
})

describe('writing', () => {
  /**
   * The whole point of one shared store: a write updates the list from what the
   * server returned, so the menu in the top bar and the toggle on the page
   * never disagree while a refetch is in flight.
   */
  it('appends a new pin without refetching', async () => {
    const pins = await store([pin({ slug: 'index', title: 'Index' })])
    api.pinPage.mockResolvedValue(pin())
    api.listPins.mockClear()

    await pins.pin('notes/quick')

    expect(slugs(pins)).toEqual(['index', 'notes/quick'])
    expect(api.listPins).not.toHaveBeenCalled()
  })

  /**
   * Pinning is idempotent server-side, so this is a real case rather than a
   * defensive one — and an entry that jumped to the end of the menu because it
   * was clicked twice would be a menu you have to re-read before every click.
   */
  it('re-pinning keeps the entry where it was', async () => {
    const pins = await store([pin(), pin({ slug: 'index', title: 'Index' })])
    api.pinPage.mockResolvedValue(pin({ title: 'Quick Notes, renamed' }))

    await pins.pin('notes/quick')

    expect(slugs(pins)).toEqual(['notes/quick', 'index'])
    expect(pins.pins()[0]?.title).toBe('Quick Notes, renamed')
  })

  it('removes an unpinned entry', async () => {
    const pins = await store([pin(), pin({ slug: 'index', title: 'Index' })])
    api.unpinPage.mockResolvedValue(undefined)

    await pins.unpin('notes/quick')

    expect(slugs(pins)).toEqual(['index'])
    expect(pins.isPinned('notes/quick')).toBe(false)
  })

  it('toggles both ways', async () => {
    const pins = await store()
    api.pinPage.mockResolvedValue(pin())
    api.unpinPage.mockResolvedValue(undefined)

    await pins.toggle('notes/quick')
    expect(pins.isPinned('notes/quick')).toBe(true)

    await pins.toggle('notes/quick')
    expect(pins.isPinned('notes/quick')).toBe(false)
  })

  /**
   * Hitting the pin limit comes back as a `409`, and the caller is the one with
   * somewhere to show it — so the failure has to travel rather than being
   * swallowed into an unchanged list.
   */
  it('lets a failed pin reach the caller and changes nothing', async () => {
    const pins = await store([pin()])
    api.pinPage.mockRejectedValue(new Error('too many pins'))

    await expect(pins.pin('notes/other')).rejects.toThrow('too many pins')
    expect(slugs(pins)).toEqual(['notes/quick'])
  })
})
