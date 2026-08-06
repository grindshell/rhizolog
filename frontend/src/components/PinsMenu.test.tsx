import { afterEach, describe, expect, it, vi } from 'vitest'
import { createRoot } from 'solid-js'
import { Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { PinView } from '../api/client'

const api = vi.hoisted(() => ({
  listPins: vi.fn(),
  pinPage: vi.fn(),
  unpinPage: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  // `pageHref` and `encodeSlug` stay real: where a pin leads is most of what
  // this component is for.
  return { ...actual, ...api }
})

const { createPinStore } = await import('../api/pins')
const { default: PinsMenu } = await import('./PinsMenu')

function pin(overrides: Partial<PinView> = {}): PinView {
  return {
    slug: 'notes/quick',
    title: 'Quick Notes',
    exists: true,
    pinned_at: '2026-08-05T14:00:00Z',
    ...overrides,
  }
}

async function openMenu(pins: PinView[] = []) {
  api.listPins.mockResolvedValue({ pins, limit: 50 })
  const store = createRoot(() => createPinStore())
  await vi.waitFor(() => expect(store.loading()).toBe(false))

  const screen = render(() => (
    <Router root={() => <PinsMenu store={store} />}>{[]}</Router>
  ))
  return { ...screen, store }
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the pins menu', () => {
  it('lists each pinned page with a link to it', async () => {
    const { container, getByText } = await openMenu([
      pin(),
      pin({ slug: 'notes/rust/async', title: 'Async in Rust' }),
    ])

    const links = [...container.querySelectorAll('a')]
    expect(links.map((link) => link.getAttribute('href'))).toEqual([
      '/pages/notes/quick',
      '/pages/notes/rust/async',
    ])
    // The title leads and the slug is underneath, so two pages that share a
    // title are still distinguishable.
    expect(getByText('Quick Notes')).toBeTruthy()
    expect(getByText('notes/quick')).toBeTruthy()
  })

  it('says how to pin something when nothing is pinned', async () => {
    const { getByText, container } = await openMenu()

    expect(getByText(/Nothing pinned yet/)).toBeTruthy()
    expect(container.querySelectorAll('a')).toHaveLength(0)
  })

  /**
   * A pin can outlive its page — a file deleted or renamed outside Rhizolog
   * leaves one behind. The entry stays, badged, because it is the only thing
   * there is to click to get rid of it.
   */
  it('badges a pin whose page is gone', async () => {
    const { getByText } = await openMenu([pin({ exists: false })])

    expect(getByText('missing')).toBeTruthy()
  })

  it('unpins from the menu and drops the entry', async () => {
    const { getByLabelText, container } = await openMenu([pin()])
    api.unpinPage.mockResolvedValue(undefined)

    getByLabelText('Unpin notes/quick').click()

    await waitFor(() => expect(container.querySelectorAll('a')).toHaveLength(0))
    expect(api.unpinPage).toHaveBeenCalledWith('notes/quick')
  })

  /**
   * A failed unpin must not leave a phantom row that will not go away, so the
   * menu says so and re-reads the list rather than guessing.
   */
  it('reports a failed unpin and refetches', async () => {
    const { getByLabelText, getByText } = await openMenu([pin()])
    api.unpinPage.mockRejectedValue(new Error('gone'))
    api.listPins.mockClear()

    getByLabelText('Unpin notes/quick').click()

    await waitFor(() => expect(getByText(/Could not unpin/)).toBeTruthy())
    expect(api.listPins).toHaveBeenCalled()
  })
})
