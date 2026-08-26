import { describe, expect, it } from 'vitest'
import type { SectionView } from '../api/client'
import { bounds, moved, spines } from './spine'

function section(over: Partial<SectionView> = {}): SectionView {
  return {
    slug: 'book/one',
    depth: 1,
    words: 0,
    subtree: 0,
    offset: 0,
    length: 0,
    status: 'included',
    ...over,
  }
}

/** The manifest of a book whose contents list has a repeat and a typo in it. */
function manifest(): SectionView[] {
  return [
    section({ slug: 'book', depth: 0, parent: undefined, ordinal: undefined }),
    section({ slug: 'book/gone', parent: 'book', ordinal: 0, status: 'wanted' }),
    section({ slug: 'book/one', parent: 'book', ordinal: 1 }),
    section({ slug: 'book/one', parent: 'book', ordinal: 2, status: 'duplicate' }),
    section({ slug: '../nope', parent: 'book', ordinal: 3, status: 'invalid' }),
  ]
}

describe('rebuilding a contents list', () => {
  /**
   * Byte for byte what the frontmatter says. A gap, a repeat and a typo are all
   * entries somebody wrote, and a reorder that dropped one would be losing work
   * to a button press.
   */
  it('is the authored list, repeats and typos included', () => {
    expect(spines(manifest()).get('book')).toEqual([
      'book/gone',
      'book/one',
      'book/one',
      '../nope',
    ])
  })

  it('has no list for a page nothing named', () => {
    expect(spines(manifest()).has('book/one')).toBe(false)
  })

  /**
   * The case that makes counting rows wrong. A page under one excluded part and
   * one included part is walked down both, so its children arrive twice with the
   * same ordinals.
   */
  it('counts a child met down two paths once', () => {
    const doubled = [
      section({ slug: 'book/a', parent: 'book/shared', ordinal: 0, status: 'excluded' }),
      section({ slug: 'book/b', parent: 'book/shared', ordinal: 1, status: 'excluded' }),
      section({ slug: 'book/a', parent: 'book/shared', ordinal: 0 }),
      section({ slug: 'book/b', parent: 'book/shared', ordinal: 1 }),
    ]

    expect(spines(doubled).get('book/shared')).toEqual(['book/a', 'book/b'])
  })

  it('reads the entries in ordinal order however they arrived', () => {
    const jumbled = [
      section({ slug: 'c', parent: 'book', ordinal: 2 }),
      section({ slug: 'a', parent: 'book', ordinal: 0 }),
      section({ slug: 'b', parent: 'book', ordinal: 1 }),
    ]

    expect(spines(jumbled).get('book')).toEqual(['a', 'b', 'c'])
  })

  /**
   * Nothing in a successful compile produces a hole, since the limits refuse
   * rather than truncate. If one ever arrives, guessing at the list would guess
   * at somebody's book.
   */
  it('refuses a list with a hole in it rather than closing the gap', () => {
    const holed = [
      section({ slug: 'a', parent: 'book', ordinal: 0 }),
      section({ slug: 'c', parent: 'book', ordinal: 2 }),
    ]

    expect(spines(holed).has('book')).toBe(false)
  })
})

describe('moving an entry', () => {
  const list = ['a', 'b', 'c', 'd']

  it('takes it out and puts it back', () => {
    expect(moved(list, 0, 2)).toEqual(['b', 'c', 'a', 'd'])
    expect(moved(list, 3, 1)).toEqual(['a', 'd', 'b', 'c'])
  })

  it('is a swap when the places are next to each other', () => {
    expect(moved(list, 1, 2)).toEqual(['a', 'c', 'b', 'd'])
  })

  it('leaves the list alone rather than throwing on a place that is not there', () => {
    expect(moved(list, 0, 0)).toEqual(list)
    expect(moved(list, -1, 2)).toEqual(list)
    expect(moved(list, 1, 9)).toEqual(list)
  })

  it('copies rather than moving in place', () => {
    const original = [...list]
    moved(list, 0, 3)
    expect(list).toEqual(original)
  })

  /** Two entries with the same slug are two entries, and only one of them moves. */
  it('moves a position rather than a name', () => {
    expect(moved(['x', 'x', 'y'], 2, 0)).toEqual(['y', 'x', 'x'])
  })
})

describe('where an entry sits in its list', () => {
  const lists = spines(manifest())

  it('knows both ends', () => {
    expect(bounds(lists, section({ parent: 'book', ordinal: 0 }))).toEqual({
      first: true,
      last: false,
    })
    expect(bounds(lists, section({ parent: 'book', ordinal: 3 }))).toEqual({
      first: false,
      last: true,
    })
    expect(bounds(lists, section({ parent: 'book', ordinal: 1 }))).toEqual({
      first: false,
      last: false,
    })
  })

  /** Nothing named the root, so there is no list to move it within. */
  it('says nothing about a section nothing named', () => {
    expect(bounds(lists, section({ parent: undefined, ordinal: undefined }))).toBeUndefined()
  })

  it('says nothing about a list it could not rebuild', () => {
    expect(bounds(lists, section({ parent: 'book/missing', ordinal: 0 }))).toBeUndefined()
    expect(bounds(lists, section({ parent: 'book', ordinal: 99 }))).toBeUndefined()
  })
})
