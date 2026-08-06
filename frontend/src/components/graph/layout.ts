/**
 * A force-directed layout, written out rather than pulled in.
 *
 * This is the same trade the time charts made: a graph library would be a
 * dependency, a bundle, and a second way for the dashboard to fail, in exchange
 * for features a single-user wiki's link graph does not need. What it does need
 * is one property no library offers by default, and which the rest of this file
 * exists to guarantee.
 *
 * ## The layout is deterministic
 *
 * Every published force layout seeds its nodes with `Math.random()`. That makes
 * the same wiki draw a different picture every time you open the screen, and it
 * makes the picture worthless for the thing a graph is actually good at:
 * recognising shapes. If `notes/rust` sat top-left with a fan under it
 * yesterday, it should sit there today, so that what changed is the wiki and not
 * the seed.
 *
 * So positions are seeded from a hash of the slug, laid out on a phyllotaxis
 * spiral, and every force below is a pure function of the graph. Nothing here
 * reads a clock or a random number. Two consequences worth knowing:
 *
 * - Renaming a page moves it, because its seed is its slug. That is honest —
 *   a rename *is* a different node, and the wiki's own stats treat it that way.
 * - The layout can be asserted in a test, which is why {@link runLayout} is
 *   exported separately from anything that draws.
 *
 * ## It settles before it is drawn
 *
 * `runLayout` runs to a fixed tick budget and returns finished positions rather
 * than animating. An animation would be prettier for three seconds and then be
 * over; what remains afterwards is the same picture, arrived at more slowly and
 * with a frame loop still installed. The budget is scaled down for large graphs
 * because repulsion is O(n²) — see {@link ticksFor}.
 */

/** What the layout needs from a node. Anything else is the renderer's. */
export interface LayoutInput {
  slug: string
  /** Links at this node, wiki-wide. Heavier nodes resist being pushed around. */
  degree: number
}

export interface LayoutLink {
  source: string
  target: string
}

export interface Positioned {
  slug: string
  x: number
  y: number
}

export interface LayoutState {
  nodes: PhysicsNode[]
  /** Index pairs, resolved once so the tick loop never looks a slug up. */
  links: [number, number][]
}

interface PhysicsNode {
  slug: string
  x: number
  y: number
  vx: number
  vy: number
  /** Resistance to being moved: `1 / (1 + degree)`. */
  inverseMass: number
}

/** The square the layout is drawn into. Arbitrary, and scaled to fit on screen. */
export const EXTENT = 1000

const CENTER = EXTENT / 2

/** How hard nodes push each other apart. */
const REPULSION = 26000
/** Below this the repulsion term is clamped, so coincident nodes do not explode. */
const MIN_DISTANCE = 12
/** How hard a link pulls its two ends together. */
const SPRING = 0.012
/**
 * The length a link is happy at.
 *
 * Generous on purpose. These are the same units the renderer draws discs in,
 * and a hub's disc is 20 of them across — so a rest length of, say, 90 settles
 * a busy wiki into a picture where two nodes nearly touch and the arrow between
 * them has about ten units to exist in. The edges have to be longer than the
 * things they connect or the direction they point in is unreadable, which is
 * most of what the drawing is for.
 */
const REST_LENGTH = 150
/** A pull toward the middle, which is the only thing holding orphans in frame. */
const GRAVITY = 0.008
/** Velocity retained between ticks. Lower settles sooner and overshoots less. */
const DAMPING = 0.82
/** No node may cross more than this in one tick, whatever the forces say. */
const MAX_STEP = 40

/**
 * FNV-1a over the slug.
 *
 * Any stable hash would do. This one is four lines, has no dependency, and
 * scatters similar slugs — `notes/rust/async` and `notes/rust/pinning` want
 * different seeds, or every directory would start life as a single point.
 */
export function hashSlug(slug: string): number {
  let hash = 0x811c9dc5
  for (let index = 0; index < slug.length; index++) {
    hash ^= slug.charCodeAt(index)
    hash = Math.imul(hash, 0x01000193)
  }
  return hash >>> 0
}

/**
 * Seed positions on a phyllotaxis spiral — the sunflower-seed arrangement.
 *
 * Points are placed at the golden angle from each other, which spreads them
 * evenly with no clumping and no lattice. Which point a node gets is decided by
 * its hash, so it depends on the slug alone and not on where the node happened
 * to appear in the response.
 */
function seedPosition(slug: string, count: number): { x: number; y: number } {
  const hash = hashSlug(slug)
  const index = hash % Math.max(count, 1)
  // The offset within the ring keeps two nodes that collide on `index` apart.
  const angle = index * 2.399963229728653 + (hash % 997) / 997
  const radius = (EXTENT / 2.6) * Math.sqrt((index + 0.5) / Math.max(count, 1))

  return {
    x: CENTER + radius * Math.cos(angle),
    y: CENTER + radius * Math.sin(angle),
  }
}

/** Positions before any force has been applied. */
export function seedLayout(nodes: LayoutInput[], links: LayoutLink[]): LayoutState {
  const index = new Map(nodes.map((node, position) => [node.slug, position]))

  return {
    nodes: nodes.map((node) => ({
      slug: node.slug,
      ...seedPosition(node.slug, nodes.length),
      vx: 0,
      vy: 0,
      inverseMass: 1 / (1 + node.degree),
    })),
    links: links.flatMap((link) => {
      const source = index.get(link.source)
      const target = index.get(link.target)
      // An edge with an end nobody drew is not a line, it is a bug in the
      // caller. Dropping it here keeps the tick loop free of guards.
      return source === undefined || target === undefined || source === target
        ? []
        : [[source, target] as [number, number]]
    }),
  }
}

/**
 * One tick of the simulation.
 *
 * `alpha` scales every displacement, and the caller decays it toward zero — the
 * standard trick that lets a layout move decisively at first and stop fidgeting
 * at the end.
 */
export function step(state: LayoutState, alpha: number): void {
  const { nodes, links } = state

  for (const node of nodes) {
    node.vx *= DAMPING
    node.vy *= DAMPING
  }

  // Repulsion, every pair. O(n²), which is why `ticksFor` shrinks the budget
  // as the graph grows: a Barnes-Hut tree is the fix if a wiki ever needs it,
  // and would be several hundred lines to save milliseconds here.
  for (let a = 0; a < nodes.length; a++) {
    for (let b = a + 1; b < nodes.length; b++) {
      const first = nodes[a]!
      const second = nodes[b]!
      let dx = second.x - first.x
      let dy = second.y - first.y
      let distance = Math.hypot(dx, dy)

      if (distance < MIN_DISTANCE) {
        // Two nodes exactly on top of each other have no direction to separate
        // in, and `dx / 0` is NaN, which would poison the whole layout. Push
        // them apart along a direction derived from their order instead.
        dx = distance === 0 ? (a - b) || 1 : dx
        dy = distance === 0 ? 1 : dy
        distance = MIN_DISTANCE
      }

      const force = (REPULSION / (distance * distance)) * alpha
      const ux = (dx / distance) * force
      const uy = (dy / distance) * force

      first.vx -= ux * first.inverseMass
      first.vy -= uy * first.inverseMass
      second.vx += ux * second.inverseMass
      second.vy += uy * second.inverseMass
    }
  }

  // Springs along the links.
  for (const [a, b] of links) {
    const first = nodes[a]!
    const second = nodes[b]!
    const dx = second.x - first.x
    const dy = second.y - first.y
    const distance = Math.max(Math.hypot(dx, dy), MIN_DISTANCE)
    const force = (distance - REST_LENGTH) * SPRING * alpha
    const ux = (dx / distance) * force
    const uy = (dy / distance) * force

    first.vx += ux * first.inverseMass
    first.vy += uy * first.inverseMass
    second.vx -= ux * second.inverseMass
    second.vy -= uy * second.inverseMass
  }

  for (const node of nodes) {
    // Without this an orphan — no links, only repulsion — accelerates out of
    // the frame forever, and the visible graph shrinks to a dot in the middle.
    node.vx += (CENTER - node.x) * GRAVITY * alpha
    node.vy += (CENTER - node.y) * GRAVITY * alpha

    const speed = Math.hypot(node.vx, node.vy)
    if (speed > MAX_STEP) {
      node.vx = (node.vx / speed) * MAX_STEP
      node.vy = (node.vy / speed) * MAX_STEP
    }

    node.x += node.vx
    node.y += node.vy
  }
}

/**
 * How long to simulate, given how many nodes there are.
 *
 * A tick costs O(n²), so a fixed budget would be instant on a small wiki and a
 * visible freeze on a large one. These numbers keep the whole settle inside a
 * frame or two on the graphs the endpoint's own limit allows.
 */
export function ticksFor(count: number): number {
  if (count <= 120) return 320
  if (count <= 400) return 180
  return 90
}

/**
 * Settle a graph and return where everything landed.
 *
 * Pure: the same nodes and links always produce the same coordinates, whatever
 * order they arrive in and whatever else has happened in the session.
 */
export function runLayout(
  nodes: LayoutInput[],
  links: LayoutLink[],
  ticks = ticksFor(nodes.length),
): Positioned[] {
  const state = seedLayout(nodes, links)

  for (let tick = 0; tick < ticks; tick++) {
    // Linear decay to zero. Geometric decay never quite arrives, and a layout
    // that is still creeping when it is drawn is one that draws differently
    // depending on how fast the machine is.
    step(state, 1 - tick / ticks)
  }

  return state.nodes.map(({ slug, x, y }) => ({ slug, x, y }))
}

export interface Viewport {
  minX: number
  minY: number
  width: number
  height: number
}

/**
 * The box that holds every node, with room for the labels hanging off them.
 *
 * Computed from the settled positions rather than assumed to be {@link EXTENT}:
 * a graph of three nodes occupies a corner of the square, and drawing the whole
 * square would show it as three specks.
 */
export function boundsOf(positions: Positioned[], padding = 80): Viewport {
  if (positions.length === 0) {
    return { minX: 0, minY: 0, width: EXTENT, height: EXTENT }
  }

  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const { x, y } of positions) {
    minX = Math.min(minX, x)
    minY = Math.min(minY, y)
    maxX = Math.max(maxX, x)
    maxY = Math.max(maxY, y)
  }

  return {
    minX: minX - padding,
    minY: minY - padding,
    // A single node has zero extent in both directions, and a `viewBox` of
    // width 0 renders nothing at all.
    width: Math.max(maxX - minX + padding * 2, 1),
    height: Math.max(maxY - minY + padding * 2, 1),
  }
}
