import { afterEach, describe, expect, it } from 'vitest'
import { Router } from '@solidjs/router'
import { cleanup, render } from '@solidjs/testing-library'
import WordStats, { formatDay } from './WordStats'
import type { WordStatsResponse } from '../api/client'

afterEach(cleanup)

function stats(over: Partial<WordStatsResponse> = {}): WordStatsResponse {
  return {
    at: '2026-08-25T14:00:00Z',
    offset_minutes: 0,
    from: '2026-08-23T00:00:00Z',
    to: '2026-08-26T00:00:00Z',
    resolution: 'observed',
    days: [
      { date: '2026-08-23', added: 0, removed: 0, delta: 0, observations: 0 },
      { date: '2026-08-24', added: 1900, removed: 2000, delta: -100, observations: 4 },
      { date: '2026-08-25', added: 300, removed: 20, delta: 280, observations: 2 },
    ],
    actors: [
      { actor: 'claude-code', added: 1900, removed: 2000, delta: -100, observations: 4 },
      { actor: 'web', added: 300, removed: 20, delta: 280, observations: 2 },
    ],
    pages: [
      {
        slug: 'book/one/the-ferry',
        title: 'The Ferry',
        added: 2200,
        removed: 2020,
        delta: 180,
        observations: 6,
      },
    ],
    totals: { added: 2200, removed: 2020, delta: 180, observations: 6, pages: 1 },
    ...over,
  }
}

function chart(over: Partial<WordStatsResponse> = {}) {
  return render(() => (
    <Router root={() => <WordStats stats={stats(over)} />}>{[]}</Router>
  ))
}

describe('the word chart', () => {
  /**
   * The claim the whole feature rests on: a day of revision is not "minus one
   * hundred". Added and removed are both on screen, and the net is third.
   */
  it('shows added and removed rather than only their difference', () => {
    const { getByText } = chart()

    expect(getByText('2,200')).toBeTruthy()
    expect(getByText('2,020')).toBeTruthy()
    expect(getByText('+180')).toBeTruthy()
  })

  it('draws a column per local day, including the empty ones', () => {
    const { container } = chart()

    // Three days, two bars each: added above the line and removed below it.
    const columns = container.querySelectorAll('[title^="Aug"]')
    expect(columns.length).toBe(3)
    expect(columns[1]?.getAttribute('title')).toContain('added 1900')
    expect(columns[1]?.getAttribute('title')).toContain('removed 2000')
    expect(columns[1]?.getAttribute('title')).toContain('net -100')
  })

  it('lists every tool that wrote', () => {
    const { getByText } = chart()

    expect(getByText('claude-code')).toBeTruthy()
    expect(getByText('web')).toBeTruthy()
  })

  it('renders with one tool', () => {
    const { getByText, queryByText } = chart({
      actors: [{ actor: 'file', added: 40, removed: 0, delta: 40, observations: 1 }],
    })

    expect(getByText('file')).toBeTruthy()
    expect(queryByText('claude-code')).toBeNull()
  })

  /**
   * A wiki nobody has written in yet, which is every wiki on its first day. It
   * has to look like a quiet chart rather than like a broken one.
   */
  it('renders with nothing at all', () => {
    const { getByText, container } = chart({
      days: [],
      actors: [],
      pages: [],
      totals: { added: 0, removed: 0, delta: 0, observations: 0, pages: 0 },
    })

    expect(getByText(/Nothing written in this window/)).toBeTruthy()
    expect(getByText(/No page was written to/)).toBeTruthy()
    expect(getByText('nothing yet')).toBeTruthy()
    expect(container.querySelectorAll('[title^="Aug"]').length).toBe(0)
  })

  /** Not keystroke history, and the field is on screen to say so. */
  it('says what resolution the series is at', () => {
    const { getByText } = chart()

    expect(getByText(/observed/)).toBeTruthy()
  })
})

/**
 * The server has already cut the day in the offset that was asked for. Putting
 * `2026-08-23` through `new Date()` reads it as midnight **UTC**, which labels
 * it as the 22nd for everybody west of Greenwich.
 */
describe('a local date label', () => {
  it('reads the parts rather than parsing an instant', () => {
    expect(formatDay('2026-08-23')).toBe(
      new Date(2026, 7, 23).toLocaleDateString(undefined, {
        month: 'short',
        day: 'numeric',
      }),
    )
  })

  it('is empty for a day that is not there', () => {
    expect(formatDay(undefined)).toBe('')
  })

  it('hands back anything it cannot read', () => {
    expect(formatDay('not a date')).toBe('not a date')
  })
})
