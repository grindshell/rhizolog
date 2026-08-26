import { createSignal, onCleanup } from 'solid-js'
import type { Accessor } from 'solid-js'
import type { SectionView } from '../api/client'
import { entry, lands, sameEntry } from './spine'
import type { Entry } from './spine'

/**
 * Dragging a row onto another, by pointer.
 *
 * Pointer events rather than HTML5 drag and drop, and the reason is a phone. A
 * native drag is a mouse gesture: there is no touch equivalent of it, so the
 * first version of this worked on a desktop and nowhere else. One pointer
 * stream covers a mouse, a pen and a finger, and this file is the price of
 * that, since everything a native drag did for free has to be done here.
 *
 * What it buys back is worth the trade twice over. **The gesture is ours, so it
 * can be driven.** A native drag cannot be started by any event a script
 * dispatches, which left the whole of it provable only by hand; these handlers
 * read a coordinate and do their own arithmetic, so a test that dispatches a
 * pointer is running the same code a finger does. And the drag starts exactly
 * where it is told to, rather than wherever the nearest draggable ancestor
 * happens to be, which is what put a question over the move buttons.
 */

/**
 * How far a pointer travels before a press becomes a drag.
 *
 * A press that never reaches it is a press: nothing lifts, nothing dims, and
 * the pointer going back up does nothing at all.
 */
const THRESHOLD = 6

/**
 * How far past a row's edge still counts as being over that row.
 *
 * The rows are a few pixels apart, and a gap that belonged to neither of them
 * would put the mark out every time a pointer crossed one.
 */
const REACH = 8

/** How near the edge of the window a held drag starts scrolling it. */
const EDGE = 64

/** The most it scrolls in one frame, reached at the very edge. */
const SPEED = 14

/**
 * Which row a pointer is over, by its vertical position alone.
 *
 * Horizontal position is deliberately not consulted. A finger or a cursor that
 * wanders off the side of the list is still plainly pointing at a row, and a hit
 * test that lost the target there would make the gesture feel like it had
 * dropped something. It also makes this arithmetic rather than a DOM lookup,
 * which is why it can be tested at all.
 */
export function rowAt(
  spans: readonly { top: number; bottom: number }[],
  y: number,
): number | undefined {
  let nearest: number | undefined
  let gap = Infinity

  for (let i = 0; i < spans.length; i++) {
    const span = spans[i]
    if (!span) continue
    if (y >= span.top && y <= span.bottom) return i
    const away = y < span.top ? span.top - y : y - span.bottom
    if (away < gap) {
      gap = away
      nearest = i
    }
  }

  return gap <= REACH ? nearest : undefined
}

/** A drag in progress, as the rows drawing themselves need to see it. */
export interface RowDrag {
  /** The section in hand, or nothing while no drag is on. */
  lifted: Accessor<SectionView | undefined>
  /** The entry a release would move it to, or nothing where it would not. */
  landing: Accessor<Entry | undefined>
  /** Begin a press that may become a drag. */
  grab: (section: SectionView, event: PointerEvent) => void
}

/**
 * The gesture, wired to a list of rows and the sections they draw.
 *
 * `rows` and `sections` are read at the moment they are needed rather than
 * captured, because a drag outlives nothing but is asked about a list that the
 * panel owns.
 */
export function createRowDrag(source: {
  rows: () => readonly HTMLElement[]
  sections: () => readonly SectionView[]
  busy: () => boolean
  onDrop: (what: SectionView, to: number) => void
}): RowDrag {
  const [lifted, setLifted] = createSignal<SectionView>()
  // Compared by what it is rather than by identity: a pointer moving inside one
  // row asks this question every few milliseconds, and a fresh object each time
  // would redraw every row in the book for a mark that has not moved.
  const [landing, setLanding] = createSignal<Entry | undefined>(undefined, {
    equals: (before, after) => before === after || sameEntry(before, after),
  })

  /** The section pressed, which is not yet the section lifted. */
  let held: SectionView | undefined
  /** Which pointer this drag belongs to, so a second one is ignored. */
  let pointer: number | undefined
  let from = 0
  let at = 0
  let frame: number | undefined
  let toward = 0

  const mark = (y: number) => {
    const what = lifted()
    if (!what) return
    const rows = source.rows()
    const index = rowAt(
      rows.map((row) => row.getBoundingClientRect()),
      y,
    )
    const onto = index === undefined ? undefined : source.sections()[index]
    setLanding(onto && lands(entry(what), onto) ? entry(onto) : undefined)
  }

  /**
   * Scrolling the window while the drag is held near the top or bottom of it.
   *
   * A native drag did this and a pointer drag has to be told. Without it the
   * only chapters reachable are the ones already on screen, which on a phone is
   * about four.
   */
  const steer = (y: number) => {
    const above = y - EDGE
    const below = y - (window.innerHeight - EDGE)
    toward = above < 0 ? above : below > 0 ? below : 0
    if (toward === 0 || frame !== undefined) return
    frame = requestAnimationFrame(scroll)
  }

  const scroll = () => {
    if (toward === 0) {
      frame = undefined
      return
    }
    window.scrollBy(0, Math.max(-1, Math.min(1, toward / EDGE)) * SPEED)
    // The rows moved and the pointer did not, so the row under it is not the
    // one that was there a frame ago.
    mark(at)
    frame = requestAnimationFrame(scroll)
  }

  const halt = () => {
    toward = 0
    if (frame !== undefined) cancelAnimationFrame(frame)
    frame = undefined
  }

  const track = (event: PointerEvent) => {
    if (event.pointerId !== pointer) return
    at = event.clientY
    if (!lifted()) {
      if (Math.abs(at - from) < THRESHOLD) return
      setLifted(held)
    }
    mark(at)
    steer(at)
  }

  /**
   * Letting go, which moves the entry to wherever the mark is.
   *
   * The mark rather than a fresh hit test, so that what was on screen when the
   * pointer came up is what gets written. There is no arrangement of the two
   * that can disagree, because there is only one of them.
   */
  const drop = (event: PointerEvent) => {
    if (event.pointerId !== pointer) return
    const what = lifted()
    const spot = landing()
    release()
    if (what && spot) source.onDrop(what, spot.ordinal)
  }

  const cancel = (event: PointerEvent) => {
    if (event.pointerId === pointer) release()
  }

  const abandon = (event: KeyboardEvent) => {
    if (event.key === 'Escape') release()
  }

  const release = () => {
    held = undefined
    pointer = undefined
    halt()
    setLifted(undefined)
    setLanding(undefined)
    window.removeEventListener('pointermove', track)
    window.removeEventListener('pointerup', drop)
    window.removeEventListener('pointercancel', cancel)
    window.removeEventListener('keydown', abandon)
  }

  /**
   * On the window rather than on the row, because a drag leaves the row it
   * started on immediately and by design. A finger is captured to its target
   * anyway; a mouse is not, and would otherwise stop reporting the moment it
   * crossed into the next chapter.
   */
  const grab = (section: SectionView, event: PointerEvent) => {
    if (source.busy() || pointer !== undefined) return
    // Or the press starts a text selection on its way to becoming a drag.
    event.preventDefault()
    held = section
    pointer = event.pointerId
    from = event.clientY
    at = from
    window.addEventListener('pointermove', track)
    window.addEventListener('pointerup', drop)
    window.addEventListener('pointercancel', cancel)
    window.addEventListener('keydown', abandon)
  }

  // A panel torn down mid-drag would otherwise leave four listeners and a frame
  // loop running against a component that is gone.
  onCleanup(release)

  return { lifted, landing, grab }
}
