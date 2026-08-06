import { describe, expect, it } from 'vitest'
import { EXTENT, boundsOf, hashSlug, runLayout, seedLayout, ticksFor } from './layout'
import type { LayoutInput, LayoutLink } from './layout'

const node = (slug: string, degree = 0): LayoutInput => ({ slug, degree })

function positionOf(placed: { slug: string; x: number; y: number }[], slug: string) {
  const found = placed.find((entry) => entry.slug === slug)
  if (!found) throw new Error(`${slug} was not placed`)
  return found
}

function distance(
  placed: { slug: string; x: number; y: number }[],
  a: string,
  b: string,
): number {
  const first = positionOf(placed, a)
  const second = positionOf(placed, b)
  return Math.hypot(second.x - first.x, second.y - first.y)
}

describe('the layout is deterministic', () => {
  const nodes = [node('index', 2), node('notes/rust', 3), node('notes/rhizome', 1)]
  const links: LayoutLink[] = [
    { source: 'index', target: 'notes/rust' },
    { source: 'index', target: 'notes/rhizome' },
  ]

  /**
   * The property the whole file exists for. A random seed would make the same
   * wiki draw a different picture on every visit, which is precisely the thing
   * that stops a graph being worth looking at twice.
   */
  it('draws the same wiki the same way twice', () => {
    expect(runLayout(nodes, links)).toEqual(runLayout(nodes, links))
  })

  /** The response is sorted by slug, but nothing here may depend on that. */
  it('does not depend on the order the nodes arrive in', () => {
    const forwards = runLayout(nodes, links)
    const backwards = runLayout([...nodes].reverse(), [...links].reverse())

    for (const { slug, x, y } of forwards) {
      const other = positionOf(backwards, slug)
      expect(other.x).toBeCloseTo(x, 6)
      expect(other.y).toBeCloseTo(y, 6)
    }
  })

  it('seeds a node from its slug alone', () => {
    const alone = seedLayout([node('notes/rust')], [])
    const crowd = seedLayout([node('notes/rust'), node('other')], [])

    // Different graphs, so the spiral has a different radius — but the same
    // slug lands in the same place *within* it, which is what makes a rename
    // the only thing that moves a node.
    expect(hashSlug('notes/rust')).toBe(hashSlug('notes/rust'))
    expect(hashSlug('notes/rust')).not.toBe(hashSlug('notes/rustlings'))
    expect(alone.nodes[0]?.slug).toBe('notes/rust')
    expect(crowd.nodes[0]?.slug).toBe('notes/rust')
  })
})

describe('the forces do what they claim', () => {
  it('pulls linked pages closer than unlinked ones', () => {
    const placed = runLayout(
      [node('a', 1), node('b', 1), node('far')],
      [{ source: 'a', target: 'b' }],
    )

    expect(distance(placed, 'a', 'b')).toBeLessThan(distance(placed, 'a', 'far'))
  })

  /**
   * An orphan has no spring holding it, only repulsion pushing it away, so
   * without gravity it accelerates out of the frame forever and the rest of the
   * graph collapses to a dot in the middle of a huge empty box.
   */
  it('keeps an orphan in the frame', () => {
    const placed = runLayout(
      [node('hub', 2), node('a', 1), node('b', 1), node('orphan')],
      [
        { source: 'hub', target: 'a' },
        { source: 'hub', target: 'b' },
      ],
    )

    const orphan = positionOf(placed, 'orphan')
    const fromCentre = Math.hypot(orphan.x - EXTENT / 2, orphan.y - EXTENT / 2)
    expect(fromCentre).toBeLessThan(EXTENT)
  })

  it('never produces a coordinate that is not a number', () => {
    // Every node on the same seed point is the case that divides by zero: the
    // repulsion term has no direction to push along.
    const identical = Array.from({ length: 6 }, (_, index) => node(`${index}`, 1))
    const placed = runLayout(identical, [
      { source: '0', target: '1' },
      { source: '1', target: '0' },
      // An edge to a node nobody drew, which the seeder has to drop rather
      // than index into `undefined`.
      { source: '0', target: 'missing' },
      // A self-link, which is a real thing to write and has no direction.
      { source: '2', target: '2' },
    ])

    for (const { x, y } of placed) {
      expect(Number.isFinite(x)).toBe(true)
      expect(Number.isFinite(y)).toBe(true)
    }
  })

  it('places every node exactly once', () => {
    const placed = runLayout([node('a'), node('b'), node('c')], [])
    expect(placed.map((entry) => entry.slug).sort()).toEqual(['a', 'b', 'c'])
  })

  /** O(n²) per tick, so the budget has to come down as the graph grows. */
  it('spends fewer ticks on a bigger graph', () => {
    expect(ticksFor(10)).toBeGreaterThan(ticksFor(300))
    expect(ticksFor(300)).toBeGreaterThan(ticksFor(1500))
  })
})

describe('the viewport', () => {
  it('fits the nodes rather than the whole coordinate space', () => {
    const bounds = boundsOf(
      [
        { slug: 'a', x: 400, y: 400 },
        { slug: 'b', x: 500, y: 450 },
      ],
      10,
    )

    expect(bounds).toEqual({ minX: 390, minY: 390, width: 120, height: 70 })
  })

  /** A `viewBox` of width zero renders an empty SVG, not a big circle. */
  it('gives a single node something to be drawn in', () => {
    const bounds = boundsOf([{ slug: 'only', x: 0, y: 0 }], 0)
    expect(bounds.width).toBeGreaterThan(0)
    expect(bounds.height).toBeGreaterThan(0)
  })

  it('falls back to the whole extent when there is nothing to fit', () => {
    expect(boundsOf([])).toEqual({ minX: 0, minY: 0, width: EXTENT, height: EXTENT })
  })
})
