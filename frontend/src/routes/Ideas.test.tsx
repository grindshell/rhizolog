import { afterEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import type { IdeaSummaryView } from '../api/client'

const api = vi.hoisted(() => ({
  listIdeas: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  // `ideaHref` and `lifecycleHref` stay real: where a group and a row lead is
  // most of what this screen is.
  return { ...actual, ...api }
})

const { default: Ideas } = await import('./Ideas')

function idea(overrides: Partial<IdeaSummaryView> = {}): IdeaSummaryView {
  return {
    id: '20260801T090000-000000001',
    name: 'Dungeon seeds',
    created: '2026-08-01T09:00:00Z',
    captures: 3,
    missing: 0,
    retired: false,
    needs_repair: false,
    integrity: 'sound',
    state: 'active',
    momentum: 5,
    last_signal: '2026-08-18T09:00:00Z',
    updated: '2026-08-18T09:00:00Z',
    ...overrides,
  }
}

function response(ideas: IdeaSummaryView[]) {
  return {
    ideas,
    total: ideas.length,
    limit: 200,
    offset: 0,
    at: '2026-08-20T18:00:00Z',
    ruleset: 'idea-momentum/v1',
  }
}

async function open(path = '/ideas', ideas: IdeaSummaryView[] = []) {
  api.listIdeas.mockResolvedValue(response(ideas))
  const history = createMemoryHistory()
  history.set({ value: path })

  const screen = render(() => (
    <MemoryRouter history={history}>
      <Route path="/ideas" component={Ideas} />
    </MemoryRouter>
  ))
  await waitFor(() => expect(api.listIdeas).toHaveBeenCalled())
  return { ...screen, history }
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the ideas listing', () => {
  it('groups by the state the rules put each idea in', async () => {
    const { getByText } = await open('/ideas', [
      idea(),
      idea({ id: '2', name: 'Seeded loot', state: 'dormant', momentum: 2 }),
      idea({ id: '3', name: 'Old thread', state: 'retired', retired: true, momentum: 1 }),
    ])

    expect(getByText('Active')).toBeTruthy()
    expect(getByText('Dungeon seeds')).toBeTruthy()
    expect(getByText('Dormant')).toBeTruthy()
    expect(getByText('Seeded loot')).toBeTruthy()
    expect(getByText('Retired')).toBeTruthy()
  })

  /** Setting something aside should not mean seeing it every time. */
  it('keeps retired shut until it is asked for', async () => {
    const { getByText, queryByText, getAllByText } = await open('/ideas', [
      idea({ id: '3', name: 'Old thread', state: 'retired', retired: true, momentum: 1 }),
    ])

    expect(getByText('Retired')).toBeTruthy()
    expect(queryByText('Old thread')).toBeNull()

    fireEvent.click(getAllByText('Show')[0]!)
    expect(getByText('Old thread')).toBeTruthy()
  })

  /**
   * A number nobody can check is the thing this whole feature exists not to
   * produce, so the momentum badge *is* the link to what produced it. There is
   * no arrangement of a row in which the score appears without it.
   */
  it('never shows a score without the way to open its receipt', async () => {
    const { getByTitle } = await open('/ideas', [idea()])

    const badge = getByTitle('See how this was worked out')
    expect(badge.textContent).toContain('5')
    expect(badge.getAttribute('href')).toBe(
      '/ideas/20260801T090000-000000001#receipt',
    )
  })

  /**
   * An idea whose captures are gone has no state to be grouped by, and
   * inventing one out of missing evidence is the one thing this must not do.
   */
  it('puts ideas with no evidence left in their own group and gives them no score', async () => {
    const { getByText } = await open('/ideas', [
      idea({ needs_repair: true, integrity: 'evidence_missing', state: null, momentum: null, captures: 0, missing: 2 }),
    ])

    expect(getByText('Needs repair')).toBeTruthy()
    expect(getByText('no evidence')).toBeTruthy()
  })

  it('takes its filter from the URL and asks the server for it', async () => {
    await open('/ideas?state=dormant', [idea({ state: 'dormant', momentum: 2 })])

    expect(api.listIdeas).toHaveBeenCalledWith({
      state: 'dormant',
      integrity: undefined,
      limit: 200,
    })
  })

  it('asks for the broken ones when the URL says so', async () => {
    await open('/ideas?integrity=evidence_missing', [])

    expect(api.listIdeas).toHaveBeenCalledWith({
      state: undefined,
      integrity: 'evidence_missing',
      limit: 200,
    })
  })

  /** A state nobody has is not a filter; it is a typo in a link. */
  it('ignores a state it does not have', async () => {
    await open('/ideas?state=simmering', [])

    expect(api.listIdeas).toHaveBeenCalledWith({
      state: undefined,
      integrity: undefined,
      limit: 200,
    })
  })

  /**
   * The state on this screen was worked out when it was asked for and stored
   * nowhere, so the instant and the ruleset are printed rather than implied.
   */
  it('says what rules answered, and when', async () => {
    const { getByText } = await open('/ideas', [idea()])

    expect(getByText(/idea-momentum\/v1/)).toBeTruthy()
  })

  it('says so plainly when there are no ideas at all', async () => {
    const { getByText } = await open('/ideas', [])

    expect(getByText(/No ideas yet/)).toBeTruthy()
  })
})
