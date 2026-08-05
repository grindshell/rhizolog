import { describe, expect, it } from 'vitest'
import { collapse, formatDate } from './PageDetail'

/** Just enough of an outbound link to group. */
const link = (target: string, kind: string) => ({ target, kind })

describe('collapsing link panels', () => {
  it('leaves distinct targets alone', () => {
    const collapsed = collapse(
      [link('notes/a', 'wiki'), link('notes/b', 'wiki')],
      (item) => item.target,
    )

    expect(collapsed.map((entry) => entry.first.target)).toEqual(['notes/a', 'notes/b'])
    expect(collapsed.map((entry) => entry.kinds)).toEqual([['wiki'], ['wiki']])
  })

  /**
   * The case this exists for. A page linked both as `[[a]]` and as `[a](a.md)`
   * is two edges in the graph and the API is right to report both — but two
   * identical rows under the same title read as a bug.
   */
  it('merges the same page reached by different kinds', () => {
    const collapsed = collapse(
      [link('notes/a', 'wiki'), link('notes/a', 'internal')],
      (item) => item.target,
    )

    expect(collapsed.length).toBe(1)
    expect(collapsed[0]?.kinds).toEqual(['wiki', 'internal'])
  })

  it('does not repeat a kind that reached the same page twice', () => {
    const collapsed = collapse(
      [link('notes/a', 'wiki'), link('notes/a', 'wiki')],
      (item) => item.target,
    )

    expect(collapsed[0]?.kinds).toEqual(['wiki'])
  })

  /**
   * Links arrive in document order, which is the order a reader expects to see
   * them in. Grouping must not reshuffle them.
   */
  it('keeps the order the links arrived in', () => {
    const collapsed = collapse(
      [
        link('notes/c', 'wiki'),
        link('notes/a', 'wiki'),
        link('notes/c', 'internal'),
        link('notes/b', 'wiki'),
      ],
      (item) => item.target,
    )

    expect(collapsed.map((entry) => entry.first.target)).toEqual([
      'notes/c',
      'notes/a',
      'notes/b',
    ])
  })

  it('keeps the first entry, not the last', () => {
    const first = { target: 'notes/a', kind: 'wiki', title: 'A' }
    const second = { target: 'notes/a', kind: 'internal', title: 'stale' }

    expect(collapse([first, second], (item) => item.target)[0]?.first).toBe(first)
  })

  it('handles an empty list', () => {
    expect(collapse([], (item: { kind: string }) => item.kind)).toEqual([])
  })
})

describe('formatting timestamps', () => {
  it('renders an RFC 3339 timestamp in the reader locale', () => {
    // Not asserting the exact string: it is locale- and zone-dependent, and
    // pinning it would be testing the platform rather than this function.
    expect(formatDate('2026-08-05T14:00:00Z')).not.toBe('2026-08-05T14:00:00Z')
    expect(formatDate('2026-08-05T14:00:00Z')).toMatch(/2026/)
  })

  /** A value the server never sends is still not worth rendering as `Invalid Date`. */
  it('passes through anything it cannot parse', () => {
    expect(formatDate('not a date')).toBe('not a date')
    expect(formatDate('')).toBe('')
  })
})
