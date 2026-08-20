import { For, Show, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { pageHref } from '../api/client'
import { pins as store } from '../api/pins'
import type { PinStore } from '../api/pins'

/**
 * The pins dropdown in the top bar.
 *
 * The point of a pin is one click from anywhere, so this lives in the app
 * shell and never unmounts. It takes its store as an optional prop purely so
 * tests can hand it a fresh one; the app always uses the shared singleton.
 */
export default function PinsMenu(props: { store?: PinStore }) {
  const pins = () => props.store ?? store
  const [failure, setFailure] = createSignal<string>()

  /**
   * daisyUI dropdowns are open for as long as something inside them has focus,
   * and a client-side navigation moves no focus — so without this, following a
   * pin leaves the menu hanging open over the page it just took you to.
   */
  const close = () => {
    const focused = document.activeElement
    if (focused instanceof HTMLElement) focused.blur()
  }

  const unpin = async (slug: string) => {
    setFailure(undefined)
    try {
      await pins().unpin(slug)
    } catch {
      // The list is the thing that is wrong, so say so and re-read it rather
      // than leaving a stale entry that will not go away.
      setFailure('Could not unpin that page.')
      pins().refresh()
    }
  }

  return (
    <div class="dropdown dropdown-end">
      <div
        tabindex="0"
        role="button"
        class="btn btn-ghost btn-sm px-2"
        aria-label="Pinned pages"
      >
        {/*
          The glyph is always there and the word is not, because on a 375px
          screen this button shares a bar with timers, capture and an account,
          and every one of those has to survive. The `aria-label` carries the
          name at any width.
        */}
        <svg
          class="size-4"
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M9 4h6l-1 6 4 3v2H6v-2l4-3z" />
          <path d="M12 15v5" />
        </svg>
        <span class="hidden sm:inline">Pins</span>
        <Show when={pins().pins().length > 0}>
          <span class="badge badge-ghost badge-sm">{pins().pins().length}</span>
        </Show>
      </div>

      <ul
        tabindex="0"
        class="dropdown-content menu menu-sm bg-base-100 rounded-box z-10 mt-2 w-80 gap-1 p-2 shadow"
      >
        <Show when={failure()}>
          {(message) => (
            <li class="menu-title text-error px-3 py-1 text-xs">{message()}</li>
          )}
        </Show>

        <For
          each={pins().pins()}
          fallback={
            <li class="px-3 py-2 text-sm opacity-60">
              <Show when={!pins().error()} fallback={<span>Could not load pins.</span>}>
                Nothing pinned yet. Open a page and press Pin to keep it here.
              </Show>
            </li>
          }
        >
          {(pin) => (
            <li>
              {/*
                Two controls in one row, so the row itself is not the link —
                daisyUI's `menu` would otherwise style the wrapper as the
                clickable element and swallow the unpin button's own hit area.
              */}
              <div class="flex items-center gap-2 p-0">
                <A
                  class="flex min-w-0 flex-1 flex-col items-start gap-0 px-3 py-2"
                  href={pageHref(pin.slug)}
                  onClick={close}
                >
                  <span class="w-full truncate">
                    {pin.title}
                    <Show when={!pin.exists}>
                      <span class="badge badge-warning badge-xs ml-2">missing</span>
                    </Show>
                  </span>
                  <span class="w-full truncate font-mono text-xs opacity-60">{pin.slug}</span>
                </A>
                <button
                  class="btn btn-ghost btn-xs"
                  aria-label={`Unpin ${pin.slug}`}
                  title="Unpin"
                  onClick={() => void unpin(pin.slug)}
                >
                  ✕
                </button>
              </div>
            </li>
          )}
        </For>
      </ul>
    </div>
  )
}
