import type { RouteSectionProps } from '@solidjs/router'
import { A } from '@solidjs/router'

/**
 * The app shell. Deliberately thin — M7 owns the real dashboard chrome.
 * The daisyUI classes here (`navbar`, `menu`, `btn`, `badge`) are what prove
 * Tailwind + daisyUI are actually compiled into the bundle.
 */
export default function Layout(props: RouteSectionProps) {
  return (
    <div class="min-h-screen bg-base-200">
      <div class="navbar bg-base-100 shadow-sm">
        <div class="flex-1">
          <A href="/" class="btn btn-ghost text-xl">
            Rhizowiki
          </A>
          <span class="badge badge-outline badge-sm ml-2">scaffold</span>
        </div>
        <nav class="flex-none">
          <ul class="menu menu-horizontal px-1">
            <li>
              <A href="/" end>
                Dashboard
              </A>
            </li>
            <li>
              <A href="/pages">Pages</A>
            </li>
            <li>
              <A href="/tags">Tags</A>
            </li>
            <li>
              <a href="/swagger-ui" target="_blank" rel="noreferrer">
                API docs
              </a>
            </li>
          </ul>
        </nav>
      </div>

      <main class="mx-auto max-w-5xl p-6">{props.children}</main>
    </div>
  )
}
