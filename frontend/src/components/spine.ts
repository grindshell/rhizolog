import type { SectionView } from '../api/client'

/**
 * Rebuilding a `contents:` list out of a compiled manifest, and moving one entry
 * in it.
 *
 * Pure, and separate from the panel that draws it, because getting this wrong
 * writes a corrupted spine into somebody's book. The manifest is the only thing
 * the dashboard has: order lives in frontmatter precisely so that nothing about a
 * display can reorder a book, and the price of that is that moving a chapter
 * means writing the whole list back.
 */

/**
 * Every contents list in a manifest, rebuilt, keyed by the page holding it.
 *
 * Keyed on each entry's `ordinal` rather than on the order rows appear or on how
 * many there are, which is what `SectionView.ordinal` says to do and is not
 * pedantry: a page listed under one excluded part and one included part is walked
 * down **both** paths, so its own children appear in the manifest twice carrying
 * the same ordinals. Counting rows there would double the list and write a book
 * with every chapter in it twice.
 *
 * A list whose ordinals are not exactly `0..n-1` is **left out** rather than
 * repaired. Nothing in a successful compile produces one, since the limits refuse
 * rather than truncate, so this is the case that should not happen: guessing at a
 * list with a hole in it would guess at somebody's book, and a chapter that
 * cannot be moved is a far smaller problem than one that is silently dropped.
 */
export function spines(sections: readonly SectionView[]): Map<string, string[]> {
  const found = new Map<string, Map<number, string>>()

  for (const section of sections) {
    const { parent, ordinal } = section
    if (parent == null || ordinal == null) continue

    let entries = found.get(parent)
    if (!entries) {
      entries = new Map()
      found.set(parent, entries)
    }
    // First writer wins. A second row for the same ordinal is the same entry
    // met down a second path, not a second entry.
    if (!entries.has(ordinal)) entries.set(ordinal, section.slug)
  }

  const lists = new Map<string, string[]>()

  for (const [parent, entries] of found) {
    const ordered = [...entries.keys()].sort((left, right) => left - right)
    if (ordered.some((ordinal, at) => ordinal !== at)) continue
    lists.set(
      parent,
      ordered.map((ordinal) => entries.get(ordinal) as string),
    )
  }

  return lists
}

/**
 * `list` with the entry at `from` taken out and put back in at `to`.
 *
 * A move rather than a swap, so it is the same function whether a chapter goes
 * one place or twenty. Out-of-range asks return the list unchanged rather than
 * throwing: the buttons that call this are disabled at the ends, and a copy of
 * what was already there is the harmless answer if one ever is not.
 */
export function moved(
  list: readonly string[],
  from: number,
  to: number,
): string[] {
  const next = [...list]
  if (from === to) return next
  if (to < 0 || to >= next.length) return next

  // The bounds check on `from` as well, phrased as a lookup: the array index is
  // typed optional here, and a guard that satisfies the compiler by asking the
  // question it is actually asking beats one that asserts the answer.
  const entry = next[from]
  if (entry === undefined) return next

  next.splice(from, 1)
  next.splice(to, 0, entry)
  return next
}

/** Which way a section can be moved inside the list that names it. */
export interface Bounds {
  first: boolean
  last: boolean
}

/**
 * Whether a section is at either end of its own list, or `undefined` when it is
 * not in one this can rebuild.
 *
 * The root is the commonest `undefined`: nothing named it, so there is no list to
 * move it within. Its own contents are moved from the pages inside them.
 */
export function bounds(
  lists: Map<string, string[]>,
  section: SectionView,
): Bounds | undefined {
  const { parent, ordinal } = section
  if (parent == null || ordinal == null) return undefined

  const list = lists.get(parent)
  if (!list || ordinal >= list.length) return undefined

  return { first: ordinal === 0, last: ordinal === list.length - 1 }
}
