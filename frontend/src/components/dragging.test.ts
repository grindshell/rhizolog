import { describe, expect, it } from 'vitest'
import { rowAt } from './dragging'

/** Four rows twenty tall, four apart, which is what the panel draws. */
const spans = [
  { top: 0, bottom: 20 },
  { top: 24, bottom: 44 },
  { top: 48, bottom: 68 },
  { top: 72, bottom: 92 },
]

describe('which row a pointer is over', () => {
  it('is the one it is inside', () => {
    expect(rowAt(spans, 0)).toBe(0)
    expect(rowAt(spans, 10)).toBe(0)
    expect(rowAt(spans, 20)).toBe(0)
    expect(rowAt(spans, 58)).toBe(2)
    expect(rowAt(spans, 92)).toBe(3)
  })

  /**
   * The rows are a few pixels apart, and a gap belonging to neither of them
   * would put the mark out every time a pointer crossed one. It goes to the
   * nearer side, so a drag between two rows is over one of them.
   */
  it('is the nearer one when it is in the gap between two', () => {
    expect(rowAt(spans, 21)).toBe(0)
    expect(rowAt(spans, 23)).toBe(1)
    expect(rowAt(spans, 46)).toBe(1)
    expect(rowAt(spans, 47)).toBe(2)
  })

  /**
   * Off the end of the list there is no row to be over, which is what makes
   * letting go out there a way to call a drag off rather than a move nobody
   * asked for. The reach is a few pixels so that the edges are not sharp.
   */
  it('is nothing once it is properly off the end', () => {
    expect(rowAt(spans, -8)).toBe(0)
    expect(rowAt(spans, -9)).toBeUndefined()
    expect(rowAt(spans, 100)).toBe(3)
    expect(rowAt(spans, 101)).toBeUndefined()
    expect(rowAt(spans, -400)).toBeUndefined()
  })

  it('is nothing at all when there are no rows', () => {
    expect(rowAt([], 40)).toBeUndefined()
  })

  /**
   * Only the vertical position is consulted. A finger that wanders off the side
   * of the list is still plainly pointing at a row, and losing the target there
   * would feel like the gesture had dropped something.
   */
  it('does not care where across the list the pointer is', () => {
    expect(rowAt(spans, 58)).toBe(2)
  })
})
