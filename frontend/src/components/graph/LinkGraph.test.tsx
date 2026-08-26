import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import LinkGraph, { truncate } from './LinkGraph'
import type { GraphResponse } from '../../api/client'

afterEach(cleanup)

const graph = (over: Partial<GraphResponse> = {}): GraphResponse => ({
  nodes: [
    {
      slug: 'index',
      title: 'Index',
      exists: true,
      inbound: 0,
      outbound: 2,
      tags: [],
      distance: null,
    },
    {
      slug: 'notes/rust',
      title: 'Rust',
      exists: true,
      inbound: 1,
      outbound: 1,
      tags: ['rust'],
      distance: null,
    },
    {
      slug: 'notes/rust/streams',
      title: 'notes/rust/streams',
      exists: false,
      inbound: 1,
      outbound: 0,
      tags: [],
      distance: null,
    },
  ],
  edges: [
    { source: 'index', target: 'notes/rust', kinds: ['wiki'], part: false },
    { source: 'notes/rust', target: 'notes/rust/streams', kinds: ['wiki'], part: false },
  ],
  matched: 2,
  truncated: false,
  root: null,
  depth: null,
  limit: 400,
  ...over,
})

function draw(over: Partial<GraphResponse> = {}, props: Partial<Parameters<typeof LinkGraph>[0]> = {}) {
  const onSelect = vi.fn()
  const onOpen = vi.fn()
  const view = render(() => (
    <LinkGraph graph={graph(over)} onSelect={onSelect} onOpen={onOpen} {...props} />
  ))
  return { ...view, onSelect, onOpen }
}

/** Every node's `<title>`, which is also its tooltip. */
function titles(container: HTMLElement): string[] {
  return [...container.querySelectorAll('circle > title')].map((title) =>
    (title.textContent ?? '').trim(),
  )
}

describe('drawing the graph', () => {
  it('draws a node per page and a path per link', () => {
    const { container } = draw()

    expect(titles(container)).toEqual(['Index', 'Rust', 'notes/rust/streams (wanted)'])
    expect(container.querySelectorAll('path[marker-end]')).toHaveLength(2)
  })

  /**
   * A wanted page is a node, not an absence. Drawing only what exists would
   * hide the branches the wiki has reached for, which is most of what there is
   * to see in a rhizome.
   */
  it('marks a wanted page instead of leaving it out', () => {
    const { container } = draw()
    const circles = [...container.querySelectorAll('circle')]
    const wanted = circles.find((circle) =>
      (circle.querySelector('title')?.textContent ?? '').includes('wanted'),
    )

    expect(wanted).toBeTruthy()
    expect(wanted?.getAttribute('stroke-dasharray')).toBeTruthy()
    expect(wanted?.getAttribute('class')).toContain('stroke-warning')
  })

  it('rings the root of a walk', () => {
    const { container } = draw({}, { root: 'notes/rust' })
    const rings = [...container.querySelectorAll('circle')].filter((circle) =>
      (circle.getAttribute('class') ?? '').includes('stroke-secondary'),
    )
    expect(rings).toHaveLength(1)
  })

  /**
   * A `viewBox` has to be four finite numbers or the browser drops it and
   * renders the raw coordinate space, which puts the graph off-screen.
   */
  it('produces a usable viewBox', () => {
    const { container } = draw()
    const parts = (container.querySelector('svg')?.getAttribute('viewBox') ?? '')
      .split(' ')
      .map(Number)

    expect(parts).toHaveLength(4)
    expect(parts.every(Number.isFinite)).toBe(true)
    expect(parts[2]).toBeGreaterThan(0)
    expect(parts[3]).toBeGreaterThan(0)
  })

  it('draws an empty graph without falling over', () => {
    const { container } = draw({ nodes: [], edges: [], matched: 0 })

    expect(container.querySelectorAll('circle')).toHaveLength(0)
    expect(container.querySelector('svg')).toBeTruthy()
  })

  /**
   * A `contents:` entry is a different claim from a link: one page mentioning
   * another, against one page being inside another. Drawing them alike would
   * hide the one relationship in a wiki that has an order to it.
   */
  it('draws a part edge unlike a link, and keys it', () => {
    const { container, queryByText } = draw({
      edges: [
        { source: 'index', target: 'notes/rust', kinds: [], part: true },
        { source: 'notes/rust', target: 'notes/rust/streams', kinds: ['wiki'], part: false },
      ],
    })

    const paths = [...container.querySelectorAll('path[marker-end]')]
    const part = paths.find((path) =>
      (path.getAttribute('class') ?? '').includes('stroke-secondary'),
    )

    expect(part).toBeTruthy()
    expect(part?.getAttribute('marker-end')).toBe('url(#graph-arrow-part)')
    // Heavier at rest than a link, because there are far fewer of them and they
    // are what gives a branch of the wiki an order.
    expect(Number(part?.getAttribute('stroke-width'))).toBeGreaterThan(1.2)
    expect(queryByText('part of')).toBeTruthy()
  })

  /** A wiki with no manuscripts should not carry a key to a thing it has none of. */
  it('leaves the part key out when nothing is assembled', () => {
    const { queryByText } = draw()

    expect(queryByText('part of')).toBeNull()
  })
})

describe('picking a node', () => {
  it('selects on the first click and opens on the second', () => {
    const { container, onSelect, onOpen } = draw()
    const first = container.querySelector('g > g')!

    fireEvent.click(first)
    expect(onSelect).toHaveBeenCalledWith('index')
    expect(onOpen).not.toHaveBeenCalled()
  })

  /** A click on an already-selected node is the second click of the pair. */
  it('opens a node that is already selected', () => {
    const { container, onOpen } = draw({}, { selected: 'index' })

    fireEvent.click(container.querySelector('g > g')!)

    expect(onOpen).toHaveBeenCalledWith('index')
  })

  it('clears the selection when the canvas itself is clicked', () => {
    const { container, onSelect } = draw({}, { selected: 'index' })

    fireEvent.click(container.querySelector('svg')!)

    expect(onSelect).toHaveBeenCalledWith(undefined)
  })
})

/**
 * These are here because of a bug that every test above was blind to.
 *
 * Capturing the pointer on `pointerdown` retargets the `click` that follows it
 * to the capture element, so a click on a node arrives at the canvas instead:
 * nothing selects, and the canvas clears the selection it never got. jsdom does
 * not implement that retargeting, so the component tested green and did nothing
 * at all in a browser.
 *
 * jsdom still cannot show the retargeting, so what is asserted instead is the
 * rule that prevents it — capture happens only once a gesture is definitely a
 * pan, and by then there is no click left to break.
 */
describe('panning does not eat clicks', () => {
  /** jsdom implements neither method; a stub is enough to watch for the call. */
  function watchCapture(svg: SVGSVGElement) {
    const captured = vi.fn()
    Object.assign(svg, {
      setPointerCapture: captured,
      releasePointerCapture: vi.fn(),
    })
    return captured
  }

  const at = (clientX: number, clientY: number) => ({
    pointerId: 1,
    button: 0,
    clientX,
    clientY,
  })

  it('does not capture the pointer for a press that goes nowhere', () => {
    const { container } = draw()
    const svg = container.querySelector('svg')!
    const captured = watchCapture(svg)

    fireEvent.pointerDown(svg, at(100, 100))
    // Under the threshold: a hand resting on a mouse, not a drag.
    fireEvent.pointerMove(svg, at(101, 102))
    fireEvent.pointerUp(svg, at(101, 102))

    expect(captured).not.toHaveBeenCalled()
  })

  it('captures once the pointer has actually travelled', () => {
    const { container } = draw()
    const svg = container.querySelector('svg')!
    const captured = watchCapture(svg)

    fireEvent.pointerDown(svg, at(100, 100))
    fireEvent.pointerMove(svg, at(160, 140))

    expect(captured).toHaveBeenCalledWith(1)
  })

  /** A press that never moved is a click on the node under it. */
  it('still selects a node when the press wobbles', () => {
    const { container, onSelect } = draw()
    const svg = container.querySelector('svg')!
    watchCapture(svg)
    const node = container.querySelector('g > g')!

    fireEvent.pointerDown(svg, at(100, 100))
    fireEvent.pointerMove(svg, at(101, 100))
    fireEvent.pointerUp(svg, at(101, 100))
    fireEvent.click(node)

    expect(onSelect).toHaveBeenCalledWith('index')
  })

  /**
   * A pan ends in a click, over whatever the pointer happened to stop on. It
   * must not clear the selection the reader is holding.
   */
  it('swallows the click that ends a pan', () => {
    const { container, onSelect } = draw({}, { selected: 'index' })
    const svg = container.querySelector('svg')!
    watchCapture(svg)

    fireEvent.pointerDown(svg, at(100, 100))
    fireEvent.pointerMove(svg, at(200, 180))
    fireEvent.pointerUp(svg, at(200, 180))
    fireEvent.click(svg)

    expect(onSelect).not.toHaveBeenCalled()

    // ...and only that one click. The next is a real one again.
    fireEvent.click(svg)
    expect(onSelect).toHaveBeenCalledWith(undefined)
  })
})

describe('labels', () => {
  it('labels everything while the graph is small', () => {
    const { container } = draw()
    const labels = [...container.querySelectorAll('text')].map((text) => text.textContent)

    expect(labels).toContain('Index')
    expect(labels).toContain('Rust')
  })

  /**
   * Past a few dozen nodes every label drawn is a label overlapping another, so
   * only the hubs keep theirs. The threshold is the whole feature — a graph of
   * two hundred fully labelled pages is unreadable.
   */
  it('rations them on a large graph', () => {
    const many = Array.from({ length: 80 }, (_, index) => ({
      slug: `page-${index}`,
      title: `Page ${index}`,
      exists: true,
      inbound: index < 3 ? 20 : 0,
      outbound: 0,
      tags: [],
      distance: null,
    }))
    const { container } = draw({ nodes: many, edges: [], matched: 80 })

    const labels = [...container.querySelectorAll('text')]
    expect(labels.length).toBeGreaterThan(0)
    expect(labels.length).toBeLessThan(many.length)
    // The best-connected keep theirs.
    expect(labels.map((text) => text.textContent)).toContain('Page 0')
  })

  it('shortens a title long enough to cover its neighbours', () => {
    expect(truncate('Rust')).toBe('Rust')
    expect(truncate('A title that runs on well past what fits')).toHaveLength(26)
    expect(truncate('A title that runs on well past what fits')).toMatch(/…$/)
  })
})
