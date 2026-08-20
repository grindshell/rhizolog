import { describe, expect, it } from 'vitest'
import type { IdeaSummaryView } from '../api/client'
import { DISMISSAL_DAYS, calendarDate, chooseRediscovery, eligible } from './Rediscovery'

const DAY_MS = 24 * 60 * 60 * 1000

/** A dormant idea with two captures: the shape rediscovery is looking for. */
function idea(overrides: Partial<IdeaSummaryView> = {}): IdeaSummaryView {
  return {
    id: '20260101T090000-000000001',
    name: 'Dungeon seeds',
    created: '2026-01-01T09:00:00Z',
    captures: 2,
    missing: 0,
    retired: false,
    needs_repair: false,
    integrity: 'sound',
    state: 'dormant',
    momentum: 2,
    last_signal: '2026-02-01T09:00:00Z',
    updated: '2026-02-01T09:00:00Z',
    ...overrides,
  }
}

/** A fixed local instant, so nothing here depends on when the suite runs. */
const NOW = new Date(2026, 7, 20, 18, 0, 0)

describe('what is worth resurfacing', () => {
  it('takes a dormant thread with more than one capture', () => {
    expect(eligible(idea(), NOW)).toBe(true)
  })

  /**
   * Every other state is either moving on its own or was set aside deliberately.
   * Resurfacing something you retired last week would be the product arguing
   * with you.
   */
  it('leaves every other state alone', () => {
    for (const state of ['active', 'recurring', 'new', 'retired'] as const) {
      expect(eligible(idea({ state }), NOW)).toBe(false)
    }
    expect(eligible(idea({ retired: true }), NOW)).toBe(false)
    expect(eligible(idea({ state: null, needs_repair: true }), NOW)).toBe(false)
  })

  /** One capture is a thought, not a thread. The inbox already has it. */
  it('leaves a single capture to the inbox', () => {
    expect(eligible(idea({ captures: 1 }), NOW)).toBe(false)
  })

  it('honours a dismissal for thirty days and then stops', () => {
    const yesterday = new Date(NOW.getTime() - DAY_MS).toISOString()
    expect(eligible(idea({ dismissed: yesterday }), NOW)).toBe(false)

    const exactly = new Date(NOW.getTime() - DISMISSAL_DAYS * DAY_MS).toISOString()
    expect(eligible(idea({ dismissed: exactly }), NOW)).toBe(true)

    const older = new Date(NOW.getTime() - 60 * DAY_MS).toISOString()
    expect(eligible(idea({ dismissed: older }), NOW)).toBe(true)
  })
})

describe('choosing one card for the day', () => {
  const many = [
    idea({ id: '20260101T090000-000000001', name: 'One' }),
    idea({ id: '20260102T090000-000000002', name: 'Two' }),
    idea({ id: '20260103T090000-000000003', name: 'Three' }),
  ]

  it('offers nothing when nothing qualifies', () => {
    expect(chooseRediscovery([], NOW)).toBeUndefined()
    expect(chooseRediscovery([idea({ state: 'active' })], NOW)).toBeUndefined()
  })

  /**
   * The whole point of a deterministic choice: refreshing the inbox must not
   * deal a new thought. A card you can reload past is a feed.
   */
  it('gives the same answer all day', () => {
    const morning = new Date(2026, 7, 20, 8, 15, 0)
    const evening = new Date(2026, 7, 20, 23, 45, 0)

    expect(chooseRediscovery(many, morning)?.id).toBe(
      chooseRediscovery(many, evening)?.id,
    )
  })

  /** And does not depend on the order the listing happened to arrive in. */
  it('gives the same answer whatever order the ideas arrive in', () => {
    const reversed = [...many].reverse()
    expect(chooseRediscovery(reversed, NOW)?.id).toBe(chooseRediscovery(many, NOW)?.id)
  })

  /**
   * It should move on its own, which is the only scheduling this feature has.
   * Over a month of dates a three-idea pool should not sit on one of them.
   */
  it('moves between days', () => {
    const seen = new Set<string>()
    for (let day = 1; day <= 28; day += 1) {
      const at = new Date(2026, 7, day, 12, 0, 0)
      seen.add(chooseRediscovery(many, at)!.id)
    }
    expect(seen.size).toBeGreaterThan(1)
  })

  /**
   * A local date, not a UTC one. Somewhere east of Greenwich an evening would
   * otherwise already be tomorrow, and the card would change over dinner.
   */
  it('reads the date where the reader is', () => {
    expect(calendarDate(new Date(2026, 7, 20, 23, 30, 0))).toBe('2026-08-20')
    expect(calendarDate(new Date(2026, 0, 5, 0, 30, 0))).toBe('2026-01-05')
  })
})
