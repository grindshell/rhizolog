import { For, Show, createSignal } from 'solid-js'
import type { RouteSectionProps } from '@solidjs/router'
import { A } from '@solidjs/router'
import AccountMenu from './AccountMenu'
import PinsMenu from './PinsMenu'
import TimerMenu from './TimerMenu'

/** One destination in the navigation, wherever the navigation is being drawn. */
interface Destination {
  href: string
  label: string
  /** True for a route this app owns; false for something the backend serves. */
  internal: boolean
  /** Only for `/`, which every other href is a prefix match against. */
  end?: boolean
}

/**
 * Everything the top bar leads to, in one list.
 *
 * Written once because it is rendered twice: horizontally on a wide screen and
 * inside a menu on a narrow one. Two copies of this list would drift, and the
 * one that drifts is always the one only phones see.
 */
const DESTINATIONS: Destination[] = [
  { href: '/', label: 'Dashboard', internal: true, end: true },
  { href: '/inbox', label: 'Inbox', internal: true },
  { href: '/ideas', label: 'Ideas', internal: true },
  { href: '/pages', label: 'Pages', internal: true },
  { href: '/tags', label: 'Tags', internal: true },
  { href: '/graph', label: 'Graph', internal: true },
  { href: '/times', label: 'Time', internal: true },
  // Swagger UI is served by the backend rather than by this app, so it has to
  // be a real navigation rather than a client-side one.
  { href: '/swagger-ui', label: 'API docs', internal: false },
]

/**
 * daisyUI dropdowns stay open for as long as something inside them has focus,
 * and a client-side navigation moves no focus. Without this, following a link
 * leaves the menu hanging over the page it took you to.
 */
function close() {
  const focused = document.activeElement
  if (focused instanceof HTMLElement) focused.blur()
}

function NavLinks(props: { onNavigate?: () => void }) {
  return (
    <For each={DESTINATIONS}>
      {(destination) => (
        <li>
          <Show
            when={destination.internal}
            fallback={
              <a href={destination.href} target="_blank" rel="noreferrer">
                {destination.label}
              </a>
            }
          >
            <A href={destination.href} end={destination.end} onClick={props.onNavigate}>
              {destination.label}
            </A>
          </Show>
        </li>
      )}
    </For>
  )
}

/**
 * Capture and New page, behind one control.
 *
 * They are the two things this app makes, and on a phone they cannot both be
 * buttons in a bar that also has to hold timers, pins and an account. Capture
 * comes first because it is the one you reach for while walking: a thought
 * costs one text field, and a page costs a slug, a title and a decision.
 */
function NewMenu() {
  return (
    <div class="dropdown dropdown-end">
      <div tabindex="0" role="button" class="btn btn-primary btn-sm" aria-label="Create">
        New
      </div>
      <ul
        tabindex="0"
        class="dropdown-content menu bg-base-100 rounded-box z-10 mt-2 w-56 p-2 shadow"
      >
        <li>
          {/*
            `?capture=1` rather than plain `/inbox`, because this is the
            explicit capture action and the one that should land in the text
            field. Opening the inbox to read it should not put a keyboard over
            half a phone screen.
          */}
          <A href="/inbox?capture=1" onClick={close}>
            Capture a thought
            <span class="text-xs opacity-60">one field, no title</span>
          </A>
        </li>
        <li>
          <A href="/new" onClick={close}>
            New page
            <span class="text-xs opacity-60">slug, title, markdown</span>
          </A>
        </li>
      </ul>
    </div>
  )
}

/** The app shell: navigation, and the width everything else is read at. */
export default function Layout(props: RouteSectionProps) {
  const [open, setOpen] = createSignal(false)

  return (
    <div class="min-h-screen bg-base-200">
      {/*
        `flex-nowrap` and a shrinkable brand, so the bar never wraps into two
        rows or pushes the account menu off the right of a 375px screen. What
        gives is the wordmark, which is the one thing there that is decoration.
      */}
      <div class="navbar bg-base-100 flex-nowrap gap-1 px-2 shadow-sm sm:px-4">
        <div class="dropdown lg:hidden">
          <div
            tabindex="0"
            role="button"
            class="btn btn-ghost btn-sm px-2"
            aria-label="Navigation"
            aria-expanded={open()}
            onFocus={() => setOpen(true)}
            onBlur={() => setOpen(false)}
          >
            <svg
              class="size-5"
              xmlns="http://www.w3.org/2000/svg"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              aria-hidden="true"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M4 6h16M4 12h16M4 18h16"
              />
            </svg>
          </div>
          <ul
            tabindex="0"
            class="dropdown-content menu bg-base-100 rounded-box z-10 mt-2 w-56 p-2 shadow"
          >
            <NavLinks onNavigate={close} />
          </ul>
        </div>

        <A href="/" class="btn btn-ghost min-w-0 shrink px-2 text-lg">
          <span class="truncate">Rhizolog</span>
        </A>

        <ul class="menu menu-horizontal hidden px-1 lg:flex">
          <NavLinks />
        </ul>

        {/*
          `ml-auto` rather than a spacer, and `flex-none` so this cluster keeps
          its width while the wordmark loses its. A timer left running overnight
          is the most expensive thing this bar can fail to show, which is why it
          is first and why none of these collapse into the menu.
        */}
        <nav class="ml-auto flex flex-none items-center gap-1">
          <TimerMenu />
          <PinsMenu />
          <NewMenu />
          {/*
            Renders nothing on a wiki with no accounts, which is the ordinary
            local case, so the single-user dashboard does not grow a menu
            telling it that it is nobody.
          */}
          <AccountMenu />
        </nav>
      </div>

      <main class="mx-auto max-w-6xl p-4 sm:p-6">{props.children}</main>
    </div>
  )
}
