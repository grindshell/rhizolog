/**
 * The pinned pages, shared by everything that shows them.
 *
 * Two places read this and both must agree instantly: the dropdown in the top
 * bar, and the pin toggle on a page. Giving each its own resource would mean
 * pinning a page from the toggle left the dropdown a request behind, so there
 * is one store and both subscribe to it.
 *
 * Writes update the local list from what the server returned rather than
 * refetching. The server is the authority on order and on `pinned_at`, and it
 * hands both back, so a round trip would only add latency to a menu whose whole
 * job is to be quick.
 */
import { createResource, createRoot } from 'solid-js'
import { listPins, pinPage, unpinPage } from './client'
import type { PinView } from './client'

export interface PinStore {
  /** The pinned pages, oldest first. Empty while loading or after a failure. */
  pins: () => PinView[]
  /** How many pins the server allows, once it has said. */
  limit: () => number | undefined
  /** True once the list is at the server's limit. */
  full: () => boolean
  loading: () => boolean
  /** Whatever went wrong fetching the list, if anything. */
  error: () => unknown
  isPinned: (slug: string) => boolean
  /** Pin a page. Throws `ApiError` — callers show it where they asked. */
  pin: (slug: string) => Promise<void>
  unpin: (slug: string) => Promise<void>
  toggle: (slug: string) => Promise<void>
  /** Re-read from the server. */
  refresh: () => void
}

/**
 * Build a store. Exported for tests, which need a fresh one per case rather
 * than the module-wide singleton below.
 */
export function createPinStore(): PinStore {
  // Wrapped rather than passed directly: a single-argument `createResource`
  // calls its fetcher with the source value, which here is `true`, and
  // `listPins` would take that for an `AbortSignal`.
  const [resource, { mutate, refetch }] = createResource(() => listPins())

  /**
   * The last response, or undefined if there has not been a good one.
   *
   * `latest` rather than the resource call, so a refetch does not blank the
   * menu currently on screen — but guarded, because `latest` *rethrows* when
   * the most recent fetch failed. Unguarded, a backend that is merely down
   * would throw out of the app shell's navbar and take every page with it.
   * `error()` reports it and the dropdown says so in words instead.
   */
  const snapshot = () => (resource.error ? undefined : resource.latest)

  const pins = () => snapshot()?.pins ?? []
  const limit = () => snapshot()?.limit

  const replace = (next: PinView[]) => {
    const current = snapshot()
    mutate(current ? { ...current, pins: next } : { pins: next, limit: next.length })
  }

  const pin = async (slug: string) => {
    const pinned = await pinPage(slug)
    // The server is idempotent, so this may be a page that is already in the
    // list. Replacing in place keeps its position; only a genuinely new pin
    // goes on the end, which is where the server put it.
    const existing = pins().findIndex((entry) => entry.slug === pinned.slug)
    replace(
      existing >= 0
        ? pins().map((entry, at) => (at === existing ? pinned : entry))
        : [...pins(), pinned],
    )
  }

  const unpin = async (slug: string) => {
    await unpinPage(slug)
    replace(pins().filter((entry) => entry.slug !== slug))
  }

  const isPinned = (slug: string) => pins().some((entry) => entry.slug === slug)

  return {
    pins,
    limit,
    full: () => {
      const cap = limit()
      return cap !== undefined && pins().length >= cap
    },
    loading: () => resource.loading,
    error: () => resource.error,
    isPinned,
    pin,
    unpin,
    toggle: (slug: string) => (isPinned(slug) ? unpin(slug) : pin(slug)),
    refresh: () => void refetch(),
  }
}

/**
 * The app's store.
 *
 * `createRoot` because this lives outside any component: the resource is a
 * reactive computation, and one created without an owner would warn and never
 * be disposed. It is deliberately never disposed — it lasts as long as the tab.
 */
export const pins: PinStore = createRoot(createPinStore)
