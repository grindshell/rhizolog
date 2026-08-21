import { afterEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import type { CandidateResponse, CaptureView, IdeaSummaryView } from '../api/client'

const api = vi.hoisted(() => ({
  listCaptures: vi.fn(),
  createCapture: vi.fn(),
  archiveCapture: vi.fn(),
  restoreCapture: vi.fn(),
  deleteCapture: vi.fn(),
  captureCandidates: vi.fn(),
  connectCapture: vi.fn(),
  createIdea: vi.fn(),
  rejectCandidate: vi.fn(),
  rejectCapturePair: vi.fn(),
  listIdeas: vi.fn(),
  affirmIdea: vi.fn(),
  dismissIdea: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  // `ideaHref` and `ApiError` stay real: where a suggestion leads, and how a
  // failure is described, are both part of what this screen is for.
  return { ...actual, ...api }
})

const { default: Inbox } = await import('./Inbox')
const { createRediscoveryState } = await import('../components/Rediscovery')

function capture(overrides: Partial<CaptureView> = {}): CaptureView {
  return {
    id: '20260820T141530-123456789',
    text: 'Maybe dungeon quests should require finding particular seeds.\n',
    created: '2026-08-20T14:15:30Z',
    archived: false,
    updated: '2026-08-20T14:15:30Z',
    size: 92,
    ...overrides,
  }
}

function candidates(overrides: Partial<CandidateResponse> = {}): CandidateResponse {
  return {
    capture: '20260820T141530-123456789',
    analyzer: 'tfidf/v1',
    corpus: 4,
    terms: 11,
    threshold: 0.35,
    candidates: [
      {
        kind: 'idea',
        idea: { id: '20260801T090000-000000001', name: 'Dungeon seeds', captures: 2 },
        similarity: 0.482913,
        explained: 0.44021,
        signals: [
          {
            term: 'dungeon seeds',
            documents: 4,
            capture_weight: 0.51203,
            target_weight: 0.44107,
            contribution: 0.225837,
          },
          {
            term: 'seeds',
            documents: 5,
            capture_weight: 0.4102,
            target_weight: 0.523,
            contribution: 0.214373,
          },
        ],
      },
    ],
    ...overrides,
  }
}

function dormant(overrides: Partial<IdeaSummaryView> = {}): IdeaSummaryView {
  return {
    id: '20260101T090000-000000001',
    name: 'Seeded loot tables',
    created: '2026-01-01T09:00:00Z',
    captures: 3,
    missing: 0,
    retired: false,
    needs_repair: false,
    integrity: 'sound',
    state: 'dormant',
    momentum: 3,
    last_signal: '2026-02-01T09:00:00Z',
    updated: '2026-02-01T09:00:00Z',
    ...overrides,
  }
}

function list(captures: CaptureView[] = []) {
  return { captures, total: captures.length, limit: 50, offset: 0 }
}

function ideas(list: IdeaSummaryView[] = []) {
  return {
    ideas: list,
    total: list.length,
    limit: 200,
    offset: 0,
    at: '2026-08-20T18:00:00Z',
    ruleset: 'idea-momentum/v1',
  }
}

function open(path = '/inbox') {
  const history = createMemoryHistory()
  history.set({ value: path })
  // A fresh rediscovery state per case. The app's is module-wide on purpose, so
  // that answering the card outlives one visit to the inbox, which would
  // otherwise make the first case here decide every later one.
  const state = createRediscoveryState()
  return render(() => (
    <MemoryRouter history={history}>
      <Route path="/inbox" component={() => <Inbox rediscovery={state} />} />
    </MemoryRouter>
  ))
}

/** The inbox with its first listing already settled. */
async function inbox(path = '/inbox', entries: CaptureView[] = []) {
  api.listCaptures.mockResolvedValue(list(entries))
  api.listIdeas.mockResolvedValue(ideas())
  const screen = open(path)
  await waitFor(() => expect(api.listCaptures).toHaveBeenCalled())
  return screen
}

function field(screen: { getByLabelText: (text: string) => HTMLElement }) {
  return screen.getByLabelText('Capture a thought') as HTMLTextAreaElement
}

function type(screen: { getByLabelText: (text: string) => HTMLElement }, text: string) {
  fireEvent.input(field(screen), { target: { value: text } })
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('capturing', () => {
  /**
   * The thought is the entire product. A form that empties optimistically and
   * then fails has thrown away the only copy of it that existed.
   */
  it('keeps the text when the request fails', async () => {
    const screen = await inbox()
    api.createCapture.mockRejectedValue(new Error('the disk is full'))

    type(screen, 'Dungeon seeds decide the loot.')
    fireEvent.submit(screen.container.querySelector('form')!)

    await waitFor(() => expect(screen.getByText(/the disk is full/)).toBeTruthy())
    expect(field(screen).value).toBe('Dungeon seeds decide the loot.')
  })

  it('clears the text only once the server has it', async () => {
    const screen = await inbox()
    api.createCapture.mockResolvedValue(capture())
    api.captureCandidates.mockResolvedValue(candidates({ candidates: [] }))

    type(screen, 'Dungeon seeds decide the loot.')
    fireEvent.submit(screen.container.querySelector('form')!)

    await waitFor(() => expect(field(screen).value).toBe(''))
    expect(api.createCapture).toHaveBeenCalledWith({
      text: 'Dungeon seeds decide the loot.',
    })
  })

  /** No request at all, rather than one the server has to refuse. */
  it('refuses an empty capture without asking the server', async () => {
    const screen = await inbox()

    type(screen, '   \n  ')
    fireEvent.keyDown(field(screen), { key: 'Enter', ctrlKey: true })

    expect(api.createCapture).not.toHaveBeenCalled()
    expect(screen.getByText('Capture').closest('button')!.disabled).toBe(true)
  })

  it('saves on Ctrl+Enter as well as the button', async () => {
    const screen = await inbox()
    api.createCapture.mockResolvedValue(capture())
    api.captureCandidates.mockResolvedValue(candidates({ candidates: [] }))

    type(screen, 'Dungeon seeds decide the loot.')
    fireEvent.keyDown(field(screen), { key: 'Enter', ctrlKey: true })

    await waitFor(() => expect(api.createCapture).toHaveBeenCalled())
  })

  /**
   * Two requests, in that order, and never the other way round. Analysis is
   * derived and retryable; the capture is not.
   */
  it('asks for candidates only after the capture is saved', async () => {
    const screen = await inbox()
    api.createCapture.mockResolvedValue(capture())
    api.captureCandidates.mockResolvedValue(candidates())

    type(screen, 'Dungeon seeds decide the loot.')
    expect(api.captureCandidates).not.toHaveBeenCalled()

    fireEvent.submit(screen.container.querySelector('form')!)

    await waitFor(() =>
      expect(api.captureCandidates).toHaveBeenCalledWith('20260820T141530-123456789'),
    )
  })
})

describe('candidates', () => {
  async function suggested(response = candidates()) {
    const screen = await inbox()
    api.createCapture.mockResolvedValue(capture())
    api.captureCandidates.mockResolvedValue(response)

    type(screen, 'Dungeon seeds decide the loot.')
    fireEvent.submit(screen.container.querySelector('form')!)
    // The offer to start a thread from scratch is on the panel whatever the
    // analyzer came back with, so it is what says the panel has arrived.
    await waitFor(() =>
      expect(screen.getByText('Start a new idea from this')).toBeTruthy(),
    )
    return screen
  }

  /**
   * The suggestion is advisory and only advisory. Nothing on this screen may
   * create a connection the person did not ask for.
   */
  it('shows the shared signals and connects nothing by itself', async () => {
    const screen = await suggested()

    expect(screen.getByText('dungeon seeds')).toBeTruthy()
    expect(screen.getByText('seeds')).toBeTruthy()
    expect(screen.getByText('0.482913')).toBeTruthy()
    expect(screen.getByText('lexical similarity')).toBeTruthy()
    expect(api.connectCapture).not.toHaveBeenCalled()
    expect(api.createIdea).not.toHaveBeenCalled()
  })

  /** The contributions are the score, so they have to be readable as a sum. */
  it('shows the arithmetic on request', async () => {
    const screen = await suggested()

    expect(screen.queryByText('These add up to')).toBeNull()
    fireEvent.click(screen.getByText('Why?'))

    expect(screen.getByText('These add up to')).toBeTruthy()
    expect(screen.getByText('0.225837')).toBeTruthy()
    expect(screen.getByText('0.44021')).toBeTruthy()
  })

  it('connects to an idea when asked, naming both sides', async () => {
    const screen = await suggested()
    api.connectCapture.mockResolvedValue({})

    fireEvent.click(screen.getByText('Connect'))

    await waitFor(() =>
      expect(api.connectCapture).toHaveBeenCalledWith(
        '20260801T090000-000000001',
        '20260820T141530-123456789',
      ),
    )
  })

  it('rejects one so it is not suggested again', async () => {
    const screen = await suggested()
    api.rejectCandidate.mockResolvedValue({})

    fireEvent.click(screen.getByText('Not this'))

    await waitFor(() =>
      expect(api.rejectCandidate).toHaveBeenCalledWith(
        '20260801T090000-000000001',
        '20260820T141530-123456789',
      ),
    )
  })

  /**
   * A capture candidate has no thread to join yet, so accepting one means
   * naming the idea the two of them share. Rhizolog never invents that name.
   */
  it('asks for a name before joining two loose captures', async () => {
    const screen = await suggested(
      candidates({
        candidates: [
          {
            kind: 'capture',
            capture: capture({ id: '20260810T101010-101010101', text: 'Seeded loot.' }),
            similarity: 0.61,
            explained: 0.61,
            signals: [
              {
                term: 'seeded',
                documents: 2,
                capture_weight: 0.5,
                target_weight: 0.5,
                contribution: 0.25,
              },
            ],
          },
        ],
      }),
    )

    fireEvent.click(screen.getByText('Connect'))
    expect(api.createIdea).not.toHaveBeenCalled()

    fireEvent.input(screen.getByLabelText('Idea name'), {
      target: { value: 'Dungeon seeds' },
    })
    fireEvent.click(screen.getByText('Create'))

    await waitFor(() =>
      expect(api.createIdea).toHaveBeenCalledWith({
        name: 'Dungeon seeds',
        captures: ['20260810T101010-101010101', '20260820T141530-123456789'],
      }),
    )
  })

  /**
   * A capture is on disk whether or not the index has caught up with it, so a
   * failed analysis is an offer to try again rather than an apology.
   */
  it('keeps a failed analysis inside its own panel', async () => {
    const screen = await inbox()
    api.createCapture.mockResolvedValue(capture())
    api.captureCandidates.mockRejectedValue(new Error('the index is behind'))

    type(screen, 'Dungeon seeds decide the loot.')
    fireEvent.submit(screen.container.querySelector('form')!)

    await waitFor(() => expect(screen.getByText(/the index is behind/)).toBeTruthy())
    // The capture form is still there, and the capture itself was still saved.
    expect(field(screen)).toBeTruthy()
    expect(screen.getByText('Try again')).toBeTruthy()
  })
})

describe('the listing', () => {
  /** A cleared box means no filter, which is not the same as a search for "". */
  it('sends no search when the box is empty', async () => {
    await inbox()

    expect(api.listCaptures).toHaveBeenCalledWith({
      q: undefined,
      archived: false,
      limit: 50,
    })
  })

  it('takes its filter from the URL', async () => {
    await inbox('/inbox?show=archived')

    expect(api.listCaptures).toHaveBeenCalledWith({
      q: undefined,
      archived: true,
      limit: 50,
    })
  })

  it('shows everything when asked to', async () => {
    await inbox('/inbox?show=all')

    expect(api.listCaptures).toHaveBeenCalledWith({
      q: undefined,
      archived: undefined,
      limit: 50,
    })
  })

  it('archives a capture and re-reads the list', async () => {
    const screen = await inbox('/inbox', [capture()])
    api.archiveCapture.mockResolvedValue(capture({ archived: true }))
    api.listCaptures.mockClear()

    fireEvent.click(screen.getByText('Archive'))

    await waitFor(() =>
      expect(api.archiveCapture).toHaveBeenCalledWith('20260820T141530-123456789'),
    )
    await waitFor(() => expect(api.listCaptures).toHaveBeenCalled())
  })

  /** Deletion is the one thing here nothing undoes, so it asks first. */
  it('asks before deleting a capture for good', async () => {
    const screen = await inbox('/inbox', [capture()])
    api.deleteCapture.mockResolvedValue({ id: capture().id, ideas: [] })

    fireEvent.click(screen.getByLabelText('Delete this capture'))
    expect(api.deleteCapture).not.toHaveBeenCalled()

    fireEvent.click(screen.getByText('Delete for good'))
    await waitFor(() => expect(api.deleteCapture).toHaveBeenCalled())
  })

  /**
   * A listing that will not load is a listing that will not load. The capture
   * field above it has nothing to do with the failure and must keep working.
   */
  it('keeps a failed listing from taking the capture field with it', async () => {
    api.listCaptures.mockRejectedValue(new Error('the index is unreadable'))
    api.listIdeas.mockResolvedValue(ideas())
    const screen = open()

    await waitFor(() => expect(screen.getByText(/the index is unreadable/)).toBeTruthy())
    expect(field(screen)).toBeTruthy()
  })
})

describe('rediscovery', () => {
  it('offers one dormant idea back, and writes nothing to show it', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockResolvedValue(ideas([dormant()]))
    const screen = open()

    await waitFor(() => expect(screen.getByText('Seeded loot tables')).toBeTruthy())
    expect(screen.getByText('You were thinking about this')).toBeTruthy()
    expect(api.affirmIdea).not.toHaveBeenCalled()
    expect(api.dismissIdea).not.toHaveBeenCalled()
  })

  it('affirms and dismisses through the API rather than locally', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockResolvedValue(ideas([dormant()]))
    api.dismissIdea.mockResolvedValue({})
    const screen = open()

    await waitFor(() => expect(screen.getByText('Not now')).toBeTruthy())
    fireEvent.click(screen.getByText('Not now'))

    await waitFor(() =>
      expect(api.dismissIdea).toHaveBeenCalledWith('20260101T090000-000000001'),
    )
  })

  /**
   * The reason this is here at all: dismissing removes an idea from the eligible
   * pool, so without an answered flag the next name simply comes up, and saying
   * "not now" hands you another thought for having said it.
   */
  it('offers no second card once one has been answered', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockResolvedValue(
      ideas([
        dormant(),
        dormant({ id: '20260102T090000-000000002', name: 'Seeded corridors' }),
        dormant({ id: '20260103T090000-000000003', name: 'Seeded encounters' }),
      ]),
    )
    api.dismissIdea.mockResolvedValue({})
    const screen = open()

    await waitFor(() => expect(screen.getByText('Not now')).toBeTruthy())
    fireEvent.click(screen.getByText('Not now'))

    await waitFor(() => expect(api.dismissIdea).toHaveBeenCalled())
    await waitFor(() =>
      expect(screen.queryByText('You were thinking about this')).toBeNull(),
    )
  })

  it('puts the card away when it is affirmed too', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockResolvedValue(ideas([dormant(), dormant({ id: '2' })]))
    api.affirmIdea.mockResolvedValue({})
    const screen = open()

    await waitFor(() => expect(screen.getByText('Still interested')).toBeTruthy())
    fireEvent.click(screen.getByText('Still interested'))

    await waitFor(() => expect(api.affirmIdea).toHaveBeenCalled())
    await waitFor(() =>
      expect(screen.queryByText('You were thinking about this')).toBeNull(),
    )
  })

  /**
   * A dismissal that never reached the server has suppressed nothing, so taking
   * the card away would leave nothing to press again.
   */
  it('keeps the card when the answer could not be written', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockResolvedValue(ideas([dormant()]))
    api.dismissIdea.mockRejectedValue(new Error('the disk is full'))
    const screen = open()

    await waitFor(() => expect(screen.getByText('Not now')).toBeTruthy())
    fireEvent.click(screen.getByText('Not now'))

    await waitFor(() => expect(screen.getByText(/the disk is full/)).toBeTruthy())
    expect(screen.getByText('You were thinking about this')).toBeTruthy()
  })

  /** Nothing eligible is the ordinary case, and it shows no card at all. */
  it('shows nothing when nothing is dormant enough', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockResolvedValue(ideas([dormant({ captures: 1 })]))
    const screen = open()

    await waitFor(() => expect(api.listIdeas).toHaveBeenCalled())
    expect(screen.queryByText('You were thinking about this')).toBeNull()
  })

  /**
   * Two questions of two different trees. A rediscovery that cannot be worked
   * out is not a reason to stop somebody writing a thought down.
   */
  it('survives an ideas listing that will not load', async () => {
    api.listCaptures.mockResolvedValue(list())
    api.listIdeas.mockRejectedValue(new Error('offline'))
    const screen = open()

    await waitFor(() => expect(api.listCaptures).toHaveBeenCalled())
    expect(field(screen)).toBeTruthy()
    expect(screen.queryByText('You were thinking about this')).toBeNull()
  })
})
