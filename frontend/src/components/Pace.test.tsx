import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { Pace as PaceView, PageView } from '../api/client'

const api = vi.hoisted(() => ({ manuscriptPace: vi.fn() }))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

const { default: Pace, formatRate, signed } = await import('./Pace')

function pace(over: Partial<PaceView> = {}): PaceView {
  return {
    ruleset: 'pace/v1',
    root: 'book',
    at: '2026-08-06T18:00:00Z',
    offset_minutes: 0,
    words: 606,
    target: 2000,
    remaining: 1394,
    due: '2026-09-30T00:00:00Z',
    days_remaining: 56,
    required_per_day: 24.892857142857142,
    window: {
      days: 14,
      from: '2026-07-24T00:00:00Z',
      to: '2026-08-07T00:00:00Z',
      added: 726,
      removed: 120,
      net: 606,
      observations: 8,
      active_days: 4,
      per_day: 43.285714285714285,
      pages: [],
    },
    projected_days: 33,
    projected_finish: '2026-09-07T00:00:00Z',
    uncounted: { added: 0, removed: 0, net: 0, observations: 0, pages: [] },
    ...over,
  }
}

function page(over: Partial<PageView> = {}): PageView {
  return {
    slug: 'book',
    title: 'The Long Way Round',
    title_derived: false,
    tags: [],
    created: '2026-08-05T14:00:00Z',
    updated: '2026-08-05T14:00:00Z',
    size: 312,
    words: 43,
    content: '# The Long Way Round\n',
    visibility: 'internal',
    compile: true,
    target: 2000,
    due: '2026-09-30T00:00:00Z',
    ...over,
  }
}

/** `<A>` needs a router around it, and a root with no routes is the least of one. */
function strip(over: Partial<PageView> = {}) {
  return render(() => <Router root={() => <Pace page={page(over)} />}>{[]}</Router>)
}

/** Everything the strip rendered, with its whitespace flattened. */
function text(container: HTMLElement): string {
  return (container.textContent ?? '').replace(/\s+/g, ' ').trim()
}

beforeEach(() => {
  api.manuscriptPace.mockResolvedValue(pace())
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the pace strip', () => {
  it('says what is left, how long there is, and what that comes to a day', async () => {
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('to go'))
    expect(text(container)).toContain('1,394 to go')
    expect(text(container)).toContain('56 days left')
    expect(text(container)).toContain('24.9 a day to make it')
  })

  /**
   * The two rates are the point of the strip: the same unit, side by side, and
   * the reader compares them. Both come with what they were divided from.
   */
  it('shows the observed rate beside both halves it came from', async () => {
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('Last 14 days'))
    expect(text(container)).toContain('+606')
    expect(text(container)).toContain('(726 added, 120 removed) on 4 days')
    expect(text(container)).toContain('43.3 a day')
  })

  it('projects the finish as a day rather than a moment', async () => {
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('At that rate'))
    // Read back in UTC: rendering midnight UTC in the reader's zone would show
    // the day before to anybody west of Greenwich.
    expect(text(container)).toContain('Sep 7, 2026')
    expect(text(container)).toContain('(33 days)')
  })

  /**
   * The contrast the block exists for. A day spent on a scene that is out of the
   * book is a day the compiled total did not move, and the words are still in
   * the chart.
   */
  it('reports words that went to pages the document does not carry', async () => {
    api.manuscriptPace.mockResolvedValue(
      pace({
        uncounted: {
          added: 107,
          removed: 0,
          net: 107,
          observations: 1,
          pages: [{ slug: 'book/two/the-argument', added: 107, removed: 0, net: 107, observations: 1 }],
        },
      }),
    )
    const { container, findByText } = strip()

    await waitFor(() =>
      expect(text(container)).toContain('pages this document does not carry'),
    )
    expect(text(container)).toContain('+107')
    expect(await findByText('book/two/the-argument')).toBeTruthy()
  })

  it('says nothing about pages left behind when there are none', async () => {
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('Last 14 days'))
    expect(text(container)).not.toContain('does not carry')
  })

  /**
   * A fortnight of revision projects nothing, and no line at all is more use
   * than a date arrived at by dividing by zero.
   */
  it('draws no projection when the window took words away', async () => {
    api.manuscriptPace.mockResolvedValue(
      pace({
        projected_days: undefined,
        projected_finish: undefined,
        window: { ...pace().window, added: 100, removed: 400, net: -300, per_day: -21.4 },
      }),
    )
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('Last 14 days'))
    expect(text(container)).toContain('-300')
    expect(text(container)).not.toContain('At that rate')
  })

  /** A target is a length somebody is aiming at rather than a ceiling. */
  it('says a manuscript is over its target rather than clamping it to nothing', async () => {
    api.manuscriptPace.mockResolvedValue(
      pace({ words: 2400, remaining: -400, required_per_day: undefined }),
    )
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('over target'))
    expect(text(container)).toContain('400 over target')
    expect(text(container)).not.toContain('to make it')
  })

  /** Nothing left to write is nothing to go, not an overshoot of nothing. */
  it('reads a target met exactly as nothing to go rather than nothing over', async () => {
    api.manuscriptPace.mockResolvedValue(
      pace({ words: 2000, remaining: 0, required_per_day: undefined }),
    )
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('to go'))
    expect(text(container)).toContain('0 to go')
    expect(text(container)).not.toContain('over target')
  })

  it('says a deadline has gone rather than counting backwards', async () => {
    api.manuscriptPace.mockResolvedValue(
      pace({ days_remaining: 0, required_per_day: undefined }),
    )
    const { container } = strip()

    await waitFor(() => expect(text(container)).toContain('past its day'))
    expect(text(container)).not.toContain('to make it')
  })

  /**
   * A page with nothing to be paced against asks nothing, which is the whole
   * reason the source is nullable: this endpoint walks the book again.
   */
  it('asks for no pace on a page that names neither a target nor a day', async () => {
    strip({ target: undefined, due: undefined })

    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(api.manuscriptPace).not.toHaveBeenCalled()
  })

  /**
   * The expected failure rather than an exceptional one: half of this is read off
   * the word log, which is refused to a caller with no account. Reading an
   * errored resource rethrows, so an unguarded read would take the manuscript
   * panel down with it.
   */
  it('renders nothing at all when the request is refused', async () => {
    api.manuscriptPace.mockRejectedValue(new Error('unauthorized'))
    const { container } = strip()

    await waitFor(() => expect(api.manuscriptPace).toHaveBeenCalled())
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(text(container)).toBe('')
  })
})

describe('formatting a rate', () => {
  /** Rounding "0.4 a day" to zero would report a manuscript that is moving as one that is not. */
  it('keeps a decimal below a hundred and drops it above', () => {
    expect(formatRate(0.42)).toBe('0.4')
    expect(formatRate(43.285714285714285)).toBe('43.3')
    expect(formatRate(1204.6)).toBe('1,205')
  })

  it('is a zero rather than an infinity when there was no rate to divide', () => {
    expect(formatRate(Number.POSITIVE_INFINITY)).toBe('0')
    expect(formatRate(Number.NaN)).toBe('0')
  })
})

describe('signing a net', () => {
  /** A fortnight of revision is a real answer and has to read as one. */
  it('keeps the sign in both directions', () => {
    expect(signed(606)).toBe('+606')
    expect(signed(-1200)).toBe('-1,200')
    expect(signed(0)).toBe('+0')
  })
})
