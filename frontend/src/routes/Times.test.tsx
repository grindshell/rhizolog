import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Route, Router } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import type { ListTimesQuery, TimeSummary } from '../api/client'

const api = vi.hoisted(() => ({
  listTimes: vi.fn(),
  timeGroups: vi.fn(),
  createTime: vi.fn(),
  patchTime: vi.fn(),
  deleteTime: vi.fn(),
  stopTime: vi.fn(),
  getTime: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  // `groupHref` and `pageHref` stay real: where a filter leads is part of what
  // this screen is.
  return { ...actual, ...api }
})

const { default: Times } = await import('./Times')

/** How long the search box waits before it reaches the URL. */
const SETTLE_MS = 250

function summary(overrides: Partial<TimeSummary> = {}): TimeSummary {
  return {
    id: '20260806T090000-000000000',
    name: 'Deep work',
    start: '2026-08-06T09:00:00Z',
    end: '2026-08-06T11:00:00Z',
    running: false,
    seconds: 7200,
    pages: [],
    has_note: true,
    updated: '2026-08-06T11:00:00Z',
    size: 148,
    ...overrides,
  }
}

/**
 * The queries the log itself made.
 *
 * The shared timer store polls the same endpoint for running entries, so its
 * calls are filtered out — they are not what any of this is about.
 */
function logQueries(): ListTimesQuery[] {
  return api.listTimes.mock.calls
    .map(([query]) => query as ListTimesQuery | undefined)
    .filter((query): query is ListTimesQuery => query !== undefined && !query.running)
}

function openLog(path: string, entries: TimeSummary[] = [summary()]) {
  api.listTimes.mockImplementation(async (query?: ListTimesQuery) => {
    const times = query?.running ? [] : entries
    return { times, total: times.length, limit: 50, offset: 0 }
  })

  window.history.pushState({}, '', path)
  return render(() => (
    <Router>
      <Route path="/times" component={Times} />
      <Route path="*" component={() => <div data-testid="elsewhere" />} />
    </Router>
  ))
}

const searchBox = (container: HTMLElement) =>
  container.querySelector('input[type="search"]') as HTMLInputElement

function type(field: HTMLInputElement, value: string) {
  field.value = value
  fireEvent.input(field)
}

const settle = () => new Promise((resolve) => setTimeout(resolve, SETTLE_MS * 2))

beforeEach(() => {
  api.timeGroups.mockResolvedValue({
    groups: [],
    totals: {
      entries: 1,
      groups: 1,
      seconds: 7200,
      running: 0,
      first_start: '2026-08-06T09:00:00Z',
      last_start: '2026-08-06T09:00:00Z',
    },
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('searching the log', () => {
  it('sends what was typed, and no `q` at all until something is', async () => {
    const { container } = openLog('/times')
    await waitFor(() => expect(logQueries().length).toBeGreaterThan(0))

    // An untouched box is not a search for the empty string — the API reads
    // that as a search for nothing and answers with nothing.
    expect(logQueries()[0]?.q).toBeUndefined()

    type(searchBox(container), 'poll loop')

    await waitFor(() => expect(logQueries().at(-1)?.q).toBe('poll loop'))
  })

  it('drops the filter again when the box is emptied', async () => {
    const { container } = openLog('/times?q=poll')
    await waitFor(() => expect(logQueries().at(-1)?.q).toBe('poll'))

    type(searchBox(container), '')

    await waitFor(() => expect(logQueries().at(-1)?.q).toBeUndefined())
  })

  /**
   * The half of clearing that is easy to leave out. The chip empties the URL,
   * but the box is a signal of its own and the debounced effect would write
   * the search straight back a quarter of a second later.
   */
  it('clearing the search chip empties the box, and it stays empty', async () => {
    const { container, findByLabelText, queryByLabelText } = openLog('/times?q=poll')
    const chip = await findByLabelText('Clear the matching filter')

    fireEvent.click(chip)

    await waitFor(() => expect(queryByLabelText('Clear the matching filter')).toBeNull())
    await settle()
    expect(queryByLabelText('Clear the matching filter')).toBeNull()
    expect(searchBox(container).value).toBe('')
    expect(logQueries().at(-1)?.q).toBeUndefined()
  })

  it('keeps a group or page filter alongside the search', async () => {
    openLog('/times?q=poll&page=notes/rust/async')

    await waitFor(() => {
      const last = logQueries().at(-1)
      expect(last?.q).toBe('poll')
      expect(last?.page).toBe('notes/rust/async')
    })
  })
})

describe('a matched entry', () => {
  /**
   * A snippet is the note verbatim with `<mark>` around the matches, and a note
   * is written through the API exactly as a page body is. Same rule as search
   * results over pages: the marks are markup and nothing else is.
   */
  it('shows the excerpt as text, never as markup', async () => {
    const { container, findByText } = openLog('/times?q=poll', [
      summary({
        snippet: 'Chased the <mark>poll</mark> loop <img src=x onerror=alert(1)>',
      }),
    ])

    expect(await findByText('poll')).toBeTruthy()
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('mark')?.textContent).toBe('poll')
    expect(container.textContent).toContain('<img src=x onerror=alert(1)>')
  })

  it('says nothing matched rather than nothing is tracked', async () => {
    const { findByText } = openLog('/times?q=kubernetes', [])

    expect(await findByText(/Nothing in the log matches/)).toBeTruthy()
  })

  it('says nothing is tracked when there is no filter to blame', async () => {
    const { findByText } = openLog('/times', [])

    expect(await findByText(/Nothing tracked yet/)).toBeTruthy()
  })
})
