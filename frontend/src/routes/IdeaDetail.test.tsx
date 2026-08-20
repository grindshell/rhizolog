import { afterEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import type { CaptureView, IdeaView, ReceiptResponse } from '../api/client'

const api = vi.hoisted(() => ({
  getIdea: vi.fn(),
  ideaReceipt: vi.fn(),
  patchIdea: vi.fn(),
  affirmIdea: vi.fn(),
  retireIdea: vi.fn(),
  reopenIdea: vi.fn(),
  disconnectCapture: vi.fn(),
  reconsiderCandidate: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

const { default: IdeaDetail } = await import('./IdeaDetail')

const IDEA = '20260801T090000-000000001'
const FIRST = '20260801T090000-000000010'
const SECOND = '20260818T090000-000000020'

function capture(id: string, text: string, created: string): CaptureView {
  return { id, text, created, archived: false, updated: created, size: text.length }
}

function idea(overrides: Partial<IdeaView> = {}): IdeaView {
  return {
    id: IDEA,
    name: 'Dungeon seeds',
    note: '',
    created: '2026-08-01T09:00:00Z',
    captures: [
      capture(FIRST, 'Dungeon seeds decide the loot.', '2026-08-01T09:00:00Z'),
      capture(SECOND, 'Dungeon seeds decide the rooms.', '2026-08-18T09:00:00Z'),
    ],
    missing: [],
    rejected: [],
    retired: false,
    promoted_to: null,
    last_signal: '2026-08-18T09:00:00Z',
    dismissed: null,
    needs_repair: false,
    integrity: 'sound',
    state: 'recurring',
    momentum: 3,
    computed_at: '2026-08-20T18:00:00Z',
    updated: '2026-08-18T09:00:00Z',
    ...overrides,
  }
}

function receipt(overrides: Partial<ReceiptResponse> = {}): ReceiptResponse {
  return {
    idea: IDEA,
    name: 'Dungeon seeds',
    ruleset: 'idea-momentum/v1',
    computed_at: '2026-08-20T18:00:00Z',
    boundaries: {
      recent_14: '2026-08-06T18:00:00Z',
      recent_30: '2026-07-21T18:00:00Z',
      dormant: '2026-06-21T18:00:00Z',
    },
    integrity: 'sound',
    state: 'recurring',
    momentum: 3,
    last_signal: '2026-08-18T09:00:00Z',
    components: {
      total: 2,
      recent_14: 1,
      recent_30: 1,
      base: 2,
      recency: 1,
      affirmation: 0,
      momentum: 3,
    },
    captures: [
      {
        id: FIRST,
        created: '2026-08-01T09:00:00Z',
        within_14_days: false,
        within_30_days: false,
      },
      {
        id: SECOND,
        created: '2026-08-18T09:00:00Z',
        within_14_days: true,
        within_30_days: true,
      },
    ],
    affirmations: [],
    missing: [],
    explanation: [
      'Two captures are connected, one of them in the last 30 days.',
      'That is a momentum of 3, which the rules call recurring.',
    ],
    ...overrides,
  }
}

async function open(thread = idea(), paper: ReceiptResponse | Error = receipt()) {
  api.getIdea.mockResolvedValue(thread)
  if (paper instanceof Error) {
    api.ideaReceipt.mockRejectedValue(paper)
  } else {
    api.ideaReceipt.mockResolvedValue(paper)
  }

  const history = createMemoryHistory()
  history.set({ value: `/ideas/${IDEA}` })

  const screen = render(() => (
    <MemoryRouter history={history}>
      <Route path="/ideas/:id" component={IdeaDetail} />
    </MemoryRouter>
  ))
  await waitFor(() => expect(screen.getByText('Dungeon seeds')).toBeTruthy())
  return screen
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the receipt', () => {
  /**
   * The centrepiece, not an advanced drawer. Everything Rhizolog says about an
   * idea has to be on the same screen as the arithmetic that produced it and the
   * authored records the arithmetic counted.
   */
  it('renders the explanation, the arithmetic and the evidence from what the server said', async () => {
    const screen = await open()

    expect(
      screen.getByText('Two captures are connected, one of them in the last 30 days.'),
    ).toBeTruthy()

    // Every component of the sum, named and valued.
    expect(screen.getByText('Base')).toBeTruthy()
    expect(screen.getByText('Recency')).toBeTruthy()
    expect(screen.getByText('Affirmation')).toBeTruthy()
    expect(screen.getByText('Momentum')).toBeTruthy()

    // And the captures it was counted from, as text rather than as ids.
    expect(screen.getAllByText('Dungeon seeds decide the rooms.').length).toBe(2)
  })

  /** Each evidence row leads to the capture it came from. */
  it('links every counted capture to the capture itself', async () => {
    const screen = await open()

    const links = [...screen.container.querySelectorAll('a[href^="#capture-"]')]
    expect(links.map((link) => link.getAttribute('href'))).toEqual([
      `#capture-${FIRST}`,
      `#capture-${SECOND}`,
    ])
    expect(screen.container.querySelector(`#capture-${SECOND}`)).toBeTruthy()
  })

  it('asks for the receipt separately from the idea', async () => {
    await open()

    expect(api.getIdea).toHaveBeenCalledWith(IDEA)
    expect(api.ideaReceipt).toHaveBeenCalledWith(IDEA)
  })

  /**
   * Two questions, two requests, and the thread has to stay readable when only
   * the explanation could not be worked out.
   */
  it('keeps a failed receipt from taking the thread with it', async () => {
    const screen = await open(idea(), new Error('the rules could not be run'))

    await waitFor(() =>
      expect(screen.getByText(/the rules could not be run/)).toBeTruthy(),
    )
    expect(screen.getByText('Dungeon seeds decide the loot.')).toBeTruthy()
    expect(screen.getAllByLabelText('Disconnect this capture')).toHaveLength(2)
  })

  /**
   * An idea whose captures were deleted gets no state and no momentum at all.
   * Deriving a lifecycle from evidence that is not there is the one thing this
   * feature must never do, so there is no path that does it.
   */
  it('gives no arithmetic at all when the evidence is gone', async () => {
    const screen = await open(
      idea({
        captures: [],
        missing: [FIRST],
        needs_repair: true,
        integrity: 'evidence_missing',
        state: null,
        momentum: null,
      }),
      receipt({
        components: null,
        state: null,
        momentum: null,
        integrity: 'evidence_missing',
        captures: [],
        missing: [FIRST],
        explanation: ['One of its captures cannot be read.'],
      }),
    )

    expect(screen.getByText(/No state and no score/)).toBeTruthy()
    expect(screen.queryByText('Base')).toBeNull()
    expect(screen.getByText('no evidence left')).toBeTruthy()
  })
})

describe('deciding', () => {
  /** Retiring is how an idea is set aside, and it undoes. */
  it('retires and reopens through the API', async () => {
    const screen = await open()
    api.retireIdea.mockResolvedValue(idea({ retired: true, state: 'retired' }))

    fireEvent.click(screen.getByText('Retire'))
    await waitFor(() => expect(api.retireIdea).toHaveBeenCalledWith(IDEA))

    cleanup()
    const retired = await open(idea({ retired: true, state: 'retired', momentum: 3 }))
    api.reopenIdea.mockResolvedValue(idea())

    expect(retired.queryByText('Retire')).toBeNull()
    fireEvent.click(retired.getByText('Reopen'))
    await waitFor(() => expect(api.reopenIdea).toHaveBeenCalledWith(IDEA))
  })

  it('affirms current interest, and only when asked', async () => {
    const screen = await open()
    api.affirmIdea.mockResolvedValue(idea())

    expect(api.affirmIdea).not.toHaveBeenCalled()
    fireEvent.click(screen.getByText('Still interested'))

    await waitFor(() => expect(api.affirmIdea).toHaveBeenCalledWith(IDEA))
  })

  it('disconnects a capture and re-reads the thread', async () => {
    const screen = await open()
    api.disconnectCapture.mockResolvedValue(idea())
    api.getIdea.mockClear()

    fireEvent.click(screen.getAllByLabelText('Disconnect this capture')[0]!)

    await waitFor(() =>
      expect(api.disconnectCapture).toHaveBeenCalledWith(IDEA, FIRST),
    )
    await waitFor(() => expect(api.getIdea).toHaveBeenCalled())
  })

  /** A refusal is the server's to make, and the screen has to show which one. */
  it('shows the refusal when the last capture cannot be disconnected', async () => {
    const screen = await open()
    api.disconnectCapture.mockRejectedValue(new Error('an idea cannot be empty'))

    fireEvent.click(screen.getAllByLabelText('Disconnect this capture')[0]!)

    await waitFor(() =>
      expect(screen.getByText(/an idea cannot be empty/)).toBeTruthy(),
    )
  })

  /**
   * Reconsidering makes a rejected capture eligible again. It does not connect
   * it: the analyzer may propose and only the person disposes.
   */
  it('reconsiders a rejected candidate without connecting it', async () => {
    const screen = await open(idea({ rejected: ['20260805T090000-000000030'] }))
    api.reconsiderCandidate.mockResolvedValue(idea())

    fireEvent.click(screen.getByText('Reconsider'))

    await waitFor(() =>
      expect(api.reconsiderCandidate).toHaveBeenCalledWith(
        IDEA,
        '20260805T090000-000000030',
      ),
    )
  })

  it('renames the thread, because Rhizolog never names one', async () => {
    const screen = await open()
    api.patchIdea.mockResolvedValue(idea({ name: 'Seeded dungeons' }))

    fireEvent.click(screen.getByText('Rename'))
    fireEvent.input(screen.getByLabelText('Idea name'), {
      target: { value: 'Seeded dungeons' },
    })
    fireEvent.click(screen.getByText('Save'))

    await waitFor(() =>
      expect(api.patchIdea).toHaveBeenCalledWith(IDEA, { name: 'Seeded dungeons' }),
    )
  })
})
