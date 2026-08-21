import { afterEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { ApiError } from '../api/client'
import type { CaptureView, DraftResponse, IdeaView, ReceiptResponse } from '../api/client'

const api = vi.hoisted(() => ({
  getIdea: vi.fn(),
  ideaReceipt: vi.fn(),
  patchIdea: vi.fn(),
  affirmIdea: vi.fn(),
  retireIdea: vi.fn(),
  reopenIdea: vi.fn(),
  disconnectCapture: vi.fn(),
  reconsiderCandidate: vi.fn(),
  ideaDraft: vi.fn(),
  createPage: vi.fn(),
  recordPromotion: vi.fn(),
  // The promotion form reads the session to decide whether to say anything
  // about visibility. Left real it would fire a `fetch` at module load, which
  // in jsdom has no origin to resolve `/api/auth/session` against.
  session: vi.fn(async () => ({
    authentication_required: false,
    authenticated: true,
    user: null,
  })),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

const { default: IdeaDetail, suggestSlug } = await import('./IdeaDetail')

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

const MARKDOWN =
  '# Dungeon seeds\n\nDungeon seeds decide the loot.\n\nDungeon seeds decide the rooms.\n'

function draft(overrides: Partial<DraftResponse> = {}): DraftResponse {
  return {
    idea: IDEA,
    title: 'Dungeon seeds',
    markdown: MARKDOWN,
    sources: [
      { id: FIRST, created: '2026-08-01T09:00:00Z', archived: false },
      { id: SECOND, created: '2026-08-18T09:00:00Z', archived: false },
    ],
    missing: [],
    promoted_to: null,
    ...overrides,
  }
}

/** Open the thread, then the promotion form, and wait for the draft. */
async function promoting(thread = idea(), assembled = draft()) {
  api.ideaDraft.mockResolvedValue(assembled)
  const screen = await open(thread)

  fireEvent.click(screen.getByText('Promote it to a page'))
  await waitFor(() => expect(screen.getByLabelText('Page content')).toBeTruthy())

  return screen
}

describe('promotion', () => {
  /**
   * A draft is assembled from every capture in the thread, and most visits to
   * this screen are not about promoting it.
   */
  it('asks for a draft only when somebody opens the form', async () => {
    const screen = await open()
    expect(api.ideaDraft).not.toHaveBeenCalled()

    api.ideaDraft.mockResolvedValue(draft())
    fireEvent.click(screen.getByText('Promote it to a page'))

    await waitFor(() => expect(api.ideaDraft).toHaveBeenCalledWith(IDEA))
  })

  /**
   * The whole sequence: an ordinary page through the ordinary page API, then
   * the association. Two requests, in that order, because that is what makes
   * the second one safe to repeat.
   */
  it('creates the page and then records what the idea became', async () => {
    const screen = await promoting()
    api.createPage.mockResolvedValue({ slug: 'dungeon-seeds' })
    api.recordPromotion.mockResolvedValue(idea({ promoted_to: 'dungeon-seeds' }))

    expect((screen.getByLabelText('Page content') as HTMLTextAreaElement).value).toBe(
      MARKDOWN,
    )
    expect((screen.getByLabelText('Page slug') as HTMLInputElement).value).toBe(
      'dungeon-seeds',
    )
    // Empty on purpose: the markdown opens with the name as a heading, and a
    // page with no `title` takes its title from there. Filling this in would
    // freeze a title the wiki is perfectly able to derive.
    expect((screen.getByLabelText('Page title') as HTMLInputElement).value).toBe('')

    fireEvent.click(screen.getByText('Create the page and record it'))

    await waitFor(() =>
      expect(api.createPage).toHaveBeenCalledWith({
        slug: 'dungeon-seeds',
        content: MARKDOWN,
      }),
    )
    await waitFor(() =>
      expect(api.recordPromotion).toHaveBeenCalledWith(IDEA, { page: 'dungeon-seeds' }),
    )
  })

  /**
   * The failure the three-step shape exists to survive. The page is written and
   * the association is not, so the only thing left to do is the association: a
   * second attempt that created a second page would be the bug this shape is
   * meant to make impossible.
   */
  it('retries only the association when the page has already been written', async () => {
    const screen = await promoting()
    api.createPage.mockResolvedValue({ slug: 'dungeon-seeds' })
    api.recordPromotion.mockRejectedValueOnce(new Error('the index would not take it'))

    fireEvent.click(screen.getByText('Create the page and record it'))
    await waitFor(() =>
      expect(screen.getByText(/the index would not take it/)).toBeTruthy(),
    )
    expect(screen.getByText(/The page at dungeon-seeds exists/)).toBeTruthy()

    api.recordPromotion.mockResolvedValue(idea({ promoted_to: 'dungeon-seeds' }))
    fireEvent.click(screen.getByText('Record the page'))

    await waitFor(() => expect(api.recordPromotion).toHaveBeenCalledTimes(2))
    expect(api.createPage).toHaveBeenCalledTimes(1)
  })

  /**
   * A page somebody made earlier is the same situation as one this form made a
   * moment ago: it exists, and only the association is left. So it is not a
   * failure, and showing it as one alongside the line saying what to do next
   * was two contradictory answers to the same press. It does have to say that
   * the page is somebody else's writing, since the draft was not saved.
   */
  it('offers to record a page that was already there', async () => {
    const screen = await promoting()
    api.createPage.mockRejectedValue(
      new ApiError('page_already_exists', 'a page is already at dungeon-seeds', 409),
    )

    fireEvent.click(screen.getByText('Create the page and record it'))

    await waitFor(() => expect(screen.getByText('Record the page')).toBeTruthy())
    expect(screen.queryByText(/a page is already at dungeon-seeds/)).toBeNull()
    expect(screen.getByText(/was not saved/)).toBeTruthy()
    expect(api.recordPromotion).not.toHaveBeenCalled()

    api.recordPromotion.mockResolvedValue(idea({ promoted_to: 'dungeon-seeds' }))
    fireEvent.click(screen.getByText('Record the page'))

    await waitFor(() =>
      expect(api.recordPromotion).toHaveBeenCalledWith(IDEA, { page: 'dungeon-seeds' }),
    )
    expect(api.createPage).toHaveBeenCalledTimes(1)
  })

  /** An edited draft is the only copy of that edit, exactly as a capture is. */
  it('keeps what was typed when the page could not be created', async () => {
    const screen = await promoting()
    api.createPage.mockRejectedValue(new Error('that slug is not allowed'))

    fireEvent.input(screen.getByLabelText('Page content'), {
      target: { value: '# Dungeon seeds\n\nRewritten by hand.\n' },
    })
    fireEvent.input(screen.getByLabelText('Page slug'), {
      target: { value: 'notes/dungeon-seeds' },
    })
    fireEvent.click(screen.getByText('Create the page and record it'))

    await waitFor(() => expect(screen.getByText(/that slug is not allowed/)).toBeTruthy())
    expect((screen.getByLabelText('Page content') as HTMLTextAreaElement).value).toBe(
      '# Dungeon seeds\n\nRewritten by hand.\n',
    )
    expect(api.recordPromotion).not.toHaveBeenCalled()
  })

  /** Nothing is consumed by promoting, and the screen says where it went. */
  it('shows the page an idea already became, and keeps its captures', async () => {
    const screen = await open(idea({ promoted_to: 'notes/dungeon-seeds' }))

    const link = screen.getAllByText('notes/dungeon-seeds')[0]!
    expect(link.getAttribute('href')).toBe('/pages/notes/dungeon-seeds')
    expect(screen.getByText(/kept every capture/)).toBeTruthy()
    // Twice over: once in the receipt's evidence, once in the thread itself.
    expect(screen.getAllByText('Dungeon seeds decide the loot.')).toHaveLength(2)
    expect(screen.getByText('Record a different page')).toBeTruthy()
  })

  /** A draft short of its evidence says so rather than coming back quieter. */
  it('says when a capture could not be read into the draft', async () => {
    const screen = await promoting(idea(), draft({ missing: [FIRST] }))

    expect(screen.getByText(/could not be read/)).toBeTruthy()
  })

  /**
   * The suggestion, not a rule: slugs may hold far more than this, and the
   * field is editable. Letters outside ASCII are letters, though, which a
   * `[^a-z0-9]` spelling would quietly turn into hyphens.
   */
  it('suggests a conventional slug without insisting on one', () => {
    expect(suggestSlug('Dungeon seeds')).toBe('dungeon-seeds')
    expect(suggestSlug('  Seeds, corridors & doors!  ')).toBe(
      'seeds-corridors-doors',
    )
    expect(suggestSlug('Café notes')).toBe('café-notes')
    expect(suggestSlug('!!!')).toBe('')
  })
})
