import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js'
import type { GraphResponse } from '../../api/client'
import { EXTENT, boundsOf, runLayout } from './layout'
import type { Positioned, Viewport } from './layout'

/**
 * The link graph, drawn.
 *
 * One SVG, laid out by `./layout`, with no canvas and no graph library. See that
 * file for why the positions are deterministic; this file is about what is on
 * top of them.
 *
 * ## Three things are drawn differently and each means something
 *
 * - A **page** is a filled disc, sized by how many links meet at it wiki-wide.
 * - A **wanted page** is a hollow dashed ring. It is not an error state and not
 *   a warning about the wiki being broken — it is a branch somebody gestured at,
 *   and being able to see where those cluster is most of why this screen exists.
 * - The **root** of a walk gets a second ring, because "which one did I ask
 *   about" is the first question anyone has about a neighbourhood.
 *
 * ## A part edge does not look like a link, and bows the same way
 *
 * The two questions the plan left for this screen, answered. A `contents:` entry
 * is drawn in the second colour and heavier, because it is a different kind of
 * claim: a link is one page mentioning another, and a part is one page *being
 * inside* another. Drawing them alike would hide the one relationship in a wiki
 * that has an order to it.
 *
 * It bows exactly as a link does, and for the same reason rather than out of
 * uniformity. Two pages can each name the other in their contents lists, which
 * is a cycle compile ends by reporting the repeat as a `duplicate`, and drawn
 * straight those two lines would be one with an arrowhead buried under the
 * other. A page both linked to and assembled by the same parent is one line
 * carrying both, so the two never overlap.
 *
 * ## Labels are rationed
 *
 * Every node labelled is a wall of overlapping text at any real size, and no
 * node labelled is an abstract painting. So: everything is labelled while the
 * graph is small, and past that only the hubs — plus whatever is hovered or
 * selected and its immediate neighbours, which is the set you are asking about
 * the moment you point at something.
 */

/** Past this, only hubs and the current neighbourhood get labels. */
const LABEL_EVERYTHING_BELOW = 45
/** How many of the best-connected nodes stay labelled after that. */
const LABELLED_HUBS = 12

const MIN_RADIUS = 5
const MAX_RADIUS = 20

/** How far a link bows out, as a fraction of its length. */
const BOW = 0.12
/** Room left at the pointy end for the arrowhead. */
const ARROW_CLEARANCE = 7

const MIN_ZOOM_SPAN = EXTENT / 40
const MAX_ZOOM_SPAN = EXTENT * 4

/** How far a pointer must travel, in pixels, before a press becomes a pan. */
const DRAG_THRESHOLD = 4

interface Drawn {
  slug: string
  title: string
  exists: boolean
  x: number
  y: number
  radius: number
  degree: number
}

export default function LinkGraph(props: {
  graph: GraphResponse
  /** The slug a walk started from, drawn with a second ring. */
  root?: string
  selected?: string
  onSelect: (slug: string | undefined) => void
  /** Opening a node — a double click, or a click on an already-selected one. */
  onOpen: (slug: string) => void
}) {
  let frame: HTMLDivElement | undefined
  let canvas: SVGSVGElement | undefined

  const [size, setSize] = createSignal({ width: 960, height: 600 })
  const [zoom, setZoom] = createSignal(1)
  const [pan, setPan] = createSignal({ x: 0, y: 0 })
  const [hovered, setHovered] = createSignal<string>()

  const positions = createMemo<Positioned[]>(() =>
    runLayout(
      props.graph.nodes.map((node) => ({
        slug: node.slug,
        degree: node.inbound + node.outbound,
      })),
      props.graph.edges,
    ),
  )

  const nodes = createMemo<Drawn[]>(() => {
    const placed = new Map(positions().map((position) => [position.slug, position]))
    const degrees = props.graph.nodes.map((node) => node.inbound + node.outbound)
    const busiest = Math.max(1, ...degrees)

    return props.graph.nodes.flatMap((node) => {
      const position = placed.get(node.slug)
      if (!position) return []
      const degree = node.inbound + node.outbound
      return [
        {
          slug: node.slug,
          title: node.title,
          exists: node.exists,
          x: position.x,
          y: position.y,
          // Square-rooted, because area is what the eye compares: scaling the
          // radius linearly makes a hub with ten links look four times the
          // page with five rather than twice it.
          radius:
            MIN_RADIUS + (MAX_RADIUS - MIN_RADIUS) * Math.sqrt(degree / busiest),
          degree,
        },
      ]
    })
  })

  const byslug = createMemo(() => new Map(nodes().map((node) => [node.slug, node])))

  /** Both ends of every edge, so a hover can dim everything else. */
  const neighbours = createMemo(() => {
    const map = new Map<string, Set<string>>()
    const touch = (a: string, b: string) => {
      const existing = map.get(a)
      if (existing) existing.add(b)
      else map.set(a, new Set([b]))
    }
    for (const edge of props.graph.edges) {
      touch(edge.source, edge.target)
      touch(edge.target, edge.source)
    }
    return map
  })

  /** What the reader is currently asking about: hover wins over selection. */
  const focus = () => hovered() ?? props.selected

  const inFocus = (slug: string): boolean => {
    const current = focus()
    if (!current) return true
    return current === slug || (neighbours().get(current)?.has(slug) ?? false)
  }

  const labelled = createMemo(() => {
    const all = nodes()
    if (all.length <= LABEL_EVERYTHING_BELOW) return new Set(all.map((node) => node.slug))

    const hubs = [...all]
      .sort((a, b) => b.degree - a.degree || a.slug.localeCompare(b.slug))
      .slice(0, LABELLED_HUBS)
      .map((node) => node.slug)

    return new Set(hubs)
  })

  const showsLabel = (slug: string): boolean => {
    const current = focus()
    if (current) return current === slug || (neighbours().get(current)?.has(slug) ?? false)
    return labelled().has(slug)
  }

  /**
   * The `viewBox`, matched to the container's aspect ratio.
   *
   * Letting `preserveAspectRatio` absorb the mismatch instead would leave the
   * mapping from a pointer's position to a graph coordinate non-linear, and
   * zooming toward the cursor would drift.
   */
  const view = createMemo<Viewport>(() => {
    const bounds = boundsOf(positions())
    const aspect = size().width / Math.max(size().height, 1)

    let width = bounds.width
    let height = bounds.height
    if (width / height < aspect) width = height * aspect
    else height = width / aspect

    const scale = 1 / zoom()
    const centreX = bounds.minX + bounds.width / 2 + pan().x
    const centreY = bounds.minY + bounds.height / 2 + pan().y

    return {
      minX: centreX - (width * scale) / 2,
      minY: centreY - (height * scale) / 2,
      width: width * scale,
      height: height * scale,
    }
  })

  const measure = () => {
    if (!frame) return
    const { clientWidth, clientHeight } = frame
    // jsdom reports zeroes for everything unlaid-out; the default is a sane
    // desktop shape and only the pointer mapping cares.
    if (clientWidth > 0 && clientHeight > 0) {
      setSize({ width: clientWidth, height: clientHeight })
    }
  }

  onMount(() => {
    measure()
    window.addEventListener('resize', measure)
    onCleanup(() => window.removeEventListener('resize', measure))
  })

  /** A client point in graph coordinates. Linear, because `view` matches the box. */
  const toGraph = (clientX: number, clientY: number) => {
    const box = canvas?.getBoundingClientRect()
    const current = view()
    if (!box || box.width === 0 || box.height === 0) {
      return { x: current.minX + current.width / 2, y: current.minY + current.height / 2 }
    }
    return {
      x: current.minX + ((clientX - box.left) / box.width) * current.width,
      y: current.minY + ((clientY - box.top) / box.height) * current.height,
    }
  }

  const onWheel = (event: WheelEvent) => {
    event.preventDefault()
    const before = toGraph(event.clientX, event.clientY)
    const next = clampZoom(zoom() * Math.exp(-event.deltaY / 500))
    setZoom(next)
    // Pan so the point under the cursor stays under it: without this, zooming
    // walks whatever you were looking at off the screen.
    const after = toGraph(event.clientX, event.clientY)
    setPan((current) => ({
      x: current.x + (before.x - after.x),
      y: current.y + (before.y - after.y),
    }))
  }

  const clampZoom = (value: number) => {
    const bounds = boundsOf(positions())
    const span = Math.max(bounds.width, bounds.height)
    return Math.min(
      Math.max(value, span / MAX_ZOOM_SPAN),
      span / MIN_ZOOM_SPAN,
    )
  }

  /**
   * Panning, which may not begin until the pointer has actually gone somewhere.
   *
   * The obvious version — capture the pointer on `pointerdown` and pan from
   * there — silently breaks every click on the graph, and does it in a way no
   * test here can see. A captured pointer retargets the `click` and `dblclick`
   * that follow it to the **capture element**, so they arrive at the canvas
   * instead of the node that was pressed: the node's handler never runs, and
   * the canvas reads the click as one on empty space and clears the selection.
   * jsdom does not implement that retargeting, so it passes under the tests and
   * fails in every browser.
   *
   * Waiting for real movement fixes it and is the better behaviour anyway — a
   * press that never moved is a click, not a pan of zero pixels.
   */
  let press: { clientX: number; clientY: number } | undefined
  let panning = false
  /** Set by a pan, and consumed by the click that inevitably follows it. */
  let panned = false

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return
    press = { clientX: event.clientX, clientY: event.clientY }
    panned = false
  }

  const onPointerMove = (event: PointerEvent) => {
    if (!press) return

    if (!panning) {
      const travelled = Math.hypot(event.clientX - press.clientX, event.clientY - press.clientY)
      if (travelled < DRAG_THRESHOLD) return
      panning = true
      panned = true
      // Safe now: the gesture is a pan, so there is no click left to break, and
      // capture is what keeps it working when the pointer leaves the canvas.
      canvas?.setPointerCapture?.(event.pointerId)
    }

    // Both ends converted against the same viewport, so the delta is exact even
    // though panning moves the viewport out from under it.
    const from = toGraph(press.clientX, press.clientY)
    const to = toGraph(event.clientX, event.clientY)
    setPan((current) => ({
      x: current.x + (from.x - to.x),
      y: current.y + (from.y - to.y),
    }))
    press = { clientX: event.clientX, clientY: event.clientY }
  }

  const endDrag = (event: PointerEvent) => {
    if (panning) canvas?.releasePointerCapture?.(event.pointerId)
    press = undefined
    panning = false
  }

  const reset = () => {
    setZoom(1)
    setPan({ x: 0, y: 0 })
  }

  const pick = (slug: string) => {
    if (props.selected === slug) props.onOpen(slug)
    else props.onSelect(slug)
  }

  return (
    <div class="relative" ref={frame}>
      <svg
        ref={canvas}
        class="h-[68vh] w-full touch-none rounded-box bg-base-100 shadow select-none"
        viewBox={`${view().minX} ${view().minY} ${view().width} ${view().height}`}
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label={`Link graph: ${props.graph.nodes.length} pages, ${props.graph.edges.length} links`}
        onWheel={onWheel}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onClick={(event) => {
          // A pan ends in a click, and it is not one: the pointer stopped over
          // whatever happened to be under it when the drag finished.
          if (panned) {
            panned = false
            return
          }
          // A click that reached the canvas rather than a node is a click on
          // nothing, and clears the selection.
          if (event.target === canvas) props.onSelect(undefined)
        }}
      >
        <defs>
          <marker
            id="graph-arrow"
            viewBox="0 0 8 8"
            refX="7"
            refY="4"
            markerWidth="6"
            markerHeight="6"
            markerUnits="userSpaceOnUse"
            orient="auto"
          >
            <path d="M0 0 L8 4 L0 8 z" class="fill-base-content/40" />
          </marker>
          <marker
            id="graph-arrow-lit"
            viewBox="0 0 8 8"
            refX="7"
            refY="4"
            markerWidth="7"
            markerHeight="7"
            markerUnits="userSpaceOnUse"
            orient="auto"
          >
            <path d="M0 0 L8 4 L0 8 z" class="fill-primary" />
          </marker>
          {/*
            One marker for a part edge in both states. The stroke weight and the
            dimming carry the emphasis; a second arrowhead colour for the same
            relationship would be saying it twice.
          */}
          <marker
            id="graph-arrow-part"
            viewBox="0 0 8 8"
            refX="7"
            refY="4"
            markerWidth="7"
            markerHeight="7"
            markerUnits="userSpaceOnUse"
            orient="auto"
          >
            <path d="M0 0 L8 4 L0 8 z" class="fill-secondary" />
          </marker>
        </defs>

        <g>
          <For each={props.graph.edges}>
            {(edge) => {
              const geometry = () =>
                edgeGeometry(byslug().get(edge.source), byslug().get(edge.target))
              const lit = () => {
                const current = focus()
                return current === edge.source || current === edge.target
              }
              const shown = () => !focus() || lit()

              return (
                <Show when={geometry()}>
                  {(path) => (
                    <path
                      d={path()}
                      fill="none"
                      class="transition-opacity"
                      classList={{
                        'stroke-secondary': edge.part,
                        'stroke-primary': !edge.part && lit(),
                        'stroke-base-content/25': !edge.part && !lit(),
                        // A part edge that is not the one being pointed at is
                        // dimmed rather than recoloured, so the spine of a book
                        // stays legible as a spine at a glance.
                        'opacity-45': edge.part && !lit() && shown(),
                        'opacity-10': !shown(),
                      }}
                      stroke-width={strokeWidth(edge.part, lit())}
                      marker-end={arrow(edge.part, lit())}
                    />
                  )}
                </Show>
              )
            }}
          </For>
        </g>

        <g>
          <For each={nodes()}>
            {(node) => (
              <g
                class="cursor-pointer transition-opacity"
                classList={{ 'opacity-15': !inFocus(node.slug) }}
                onPointerEnter={() => setHovered(node.slug)}
                onPointerLeave={() => setHovered(undefined)}
                onClick={(event) => {
                  event.stopPropagation()
                  pick(node.slug)
                }}
                onDblClick={(event) => {
                  event.stopPropagation()
                  props.onOpen(node.slug)
                }}
              >
                <Show when={props.root === node.slug}>
                  <circle
                    cx={node.x}
                    cy={node.y}
                    r={node.radius + 5}
                    fill="none"
                    class="stroke-secondary"
                    stroke-width="2"
                  />
                </Show>

                <circle
                  cx={node.x}
                  cy={node.y}
                  r={node.radius}
                  classList={{
                    'fill-primary': node.exists,
                    'fill-base-100': !node.exists,
                    'stroke-warning': !node.exists,
                    'stroke-base-content': props.selected === node.slug,
                  }}
                  stroke-width={props.selected === node.slug ? 2.5 : 2}
                  stroke-dasharray={node.exists ? undefined : '4 3'}
                >
                  {/*
                    A `<title>` is the one tooltip that needs no code and works
                    on a graph this dense, where a hover panel would spend its
                    life covering the node underneath it.
                  */}
                  <title>
                    {node.title}
                    {node.exists ? '' : ' (wanted)'}
                  </title>
                </circle>

                <Show when={showsLabel(node.slug)}>
                  <text
                    x={node.x}
                    y={node.y - node.radius - 6}
                    text-anchor="middle"
                    class="pointer-events-none fill-base-content stroke-base-100 text-[11px]"
                    // A stroke behind the fill is what keeps a label readable
                    // where it crosses an edge, and `paint-order` is what stops
                    // that halo being painted over the letters instead.
                    stroke-width="3"
                    style={{ 'paint-order': 'stroke' }}
                  >
                    {truncate(node.title)}
                  </text>
                </Show>
              </g>
            )}
          </For>
        </g>
      </svg>

      <div class="absolute top-3 right-3 flex flex-col gap-1">
        <button
          class="btn btn-xs btn-square"
          onClick={() => setZoom((value) => clampZoom(value * 1.3))}
          title="Zoom in"
          aria-label="Zoom in"
        >
          +
        </button>
        <button
          class="btn btn-xs btn-square"
          onClick={() => setZoom((value) => clampZoom(value / 1.3))}
          title="Zoom out"
          aria-label="Zoom out"
        >
          &minus;
        </button>
        <button class="btn btn-xs" onClick={reset} title="Fit the whole graph">
          Fit
        </button>
      </div>

      {/*
        `pointer-events-none`: it floats over the canvas, and a key at the
        bottom-left of the frame must not be a place where nodes stop being
        clickable. The zoom buttons above it are controls and keep theirs.
      */}
      <div class="pointer-events-none absolute bottom-3 left-3 flex flex-wrap items-center gap-3 rounded-box bg-base-100/80 px-3 py-2 text-xs">
        <span class="flex items-center gap-1.5">
          <span class="inline-block size-3 rounded-full bg-primary" />
          page
        </span>
        <span class="flex items-center gap-1.5">
          <span class="inline-block size-3 rounded-full border-2 border-dashed border-warning" />
          wanted
        </span>
        {/*
          Only when there is one to explain. A wiki with no manuscripts in it
          should not carry a key to a thing it has none of.
        */}
        <Show when={props.graph.edges.some((edge) => edge.part)}>
          <span class="flex items-center gap-1.5">
            <span class="bg-secondary inline-block h-0.5 w-4" />
            part of
          </span>
        </Show>
        <Show when={props.root}>
          <span class="flex items-center gap-1.5">
            <span class="inline-block size-3 rounded-full border-2 border-secondary" />
            root
          </span>
        </Show>
        <span class="opacity-60">drag to pan · scroll to zoom · click a node</span>
      </div>
    </div>
  )
}

/**
 * How heavy a line is: what kind of edge it is, and whether it is being asked
 * about.
 *
 * A part edge is heavier than a link at rest, because it is the relationship
 * that gives a branch of the wiki an order and there are far fewer of them.
 */
export function strokeWidth(part: boolean, lit: boolean): number {
  if (part) return lit ? 2.6 : 1.8
  return lit ? 2 : 1.2
}

/** Which arrowhead a line ends in. */
export function arrow(part: boolean, lit: boolean): string {
  if (part) return 'url(#graph-arrow-part)'
  return lit ? 'url(#graph-arrow-lit)' : 'url(#graph-arrow)'
}

/**
 * The path for one directed edge, bowed to the left of the direction of travel.
 *
 * The bow is what makes a mutual link readable. Drawn straight, `a → b` and
 * `b → a` are the same line with one arrowhead buried under the other, so the
 * commonest and most interesting relationship in a wiki — two pages that both
 * point at each other — would be indistinguishable from a one-way link.
 *
 * Both ends are pulled back to the rim of their node so an arrowhead lands
 * against the circle rather than inside it.
 */
export function edgeGeometry(
  source: Drawn | undefined,
  target: Drawn | undefined,
): string | undefined {
  if (!source || !target || source.slug === target.slug) return undefined

  const dx = target.x - source.x
  const dy = target.y - source.y
  const distance = Math.hypot(dx, dy)
  if (distance === 0) return undefined

  const bow = distance * BOW
  const controlX = (source.x + target.x) / 2 - (dy / distance) * bow
  const controlY = (source.y + target.y) / 2 + (dx / distance) * bow

  const start = along(controlX - source.x, controlY - source.y, source, source.radius)
  const end = along(controlX - target.x, controlY - target.y, target, target.radius + ARROW_CLEARANCE)

  return `M ${round(start.x)} ${round(start.y)} Q ${round(controlX)} ${round(controlY)} ${round(end.x)} ${round(end.y)}`
}

/** A point `gap` out from a node's centre, toward the curve's control point. */
function along(dx: number, dy: number, node: Drawn, gap: number) {
  const length = Math.hypot(dx, dy) || 1
  return { x: node.x + (dx / length) * gap, y: node.y + (dy / length) * gap }
}

function round(value: number): number {
  return Math.round(value * 10) / 10
}

/** Titles are prose and some of them are sentences. */
export function truncate(title: string, limit = 26): string {
  return title.length <= limit ? title : `${title.slice(0, limit - 1)}…`
}
