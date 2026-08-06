import type { RouteSectionProps } from '@solidjs/router'
import { A } from '@solidjs/router'

/** The app shell: navigation, and the width everything else is read at. */
export default function Layout(props: RouteSectionProps) {
  return (
    <div class="min-h-screen bg-base-200">
      <div class="navbar bg-base-100 shadow-sm">
        <div class="flex-1">
          <A href="/" class="btn btn-ghost text-xl">
            Rhizolog
          </A>
        </div>
        <nav class="flex flex-none items-center gap-1">
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
              {/*
                A plain anchor, not `A`: Swagger UI is served by the backend,
                not by this app, so it must be a real navigation.
              */}
              <a href="/swagger-ui" target="_blank" rel="noreferrer">
                API docs
              </a>
            </li>
          </ul>
          <A class="btn btn-primary btn-sm" href="/new">
            New
          </A>
        </nav>
      </div>

      <main class="mx-auto max-w-6xl p-6">{props.children}</main>
    </div>
  )
}
