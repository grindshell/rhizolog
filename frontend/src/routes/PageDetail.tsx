import { createResource, For, Show } from 'solid-js'
import { A, useParams } from '@solidjs/router'
import { decodeSlug, encodeSlug, getPage, pageLinks } from '../api/client'
import { Async } from '../components/Async'

/**
 * Read one page.
 *
 * The route is `/pages/*slug`, a splat, because slugs contain `/`
 * (`notes/rust/async`). A single-segment `:slug` param would only ever match
 * top-level pages — the same trap the backend hit with `/api/pages/{*slug}`.
 *
 * `@solidjs/router` reads `location.pathname` verbatim and does not decode
 * path params, so the raw param is percent-encoded and goes through
 * `decodeSlug` before it reaches the API client.
 */
export default function PageDetail() {
  const params = useParams<{ slug: string }>()
  const slug = () => decodeSlug(params.slug ?? '')

  const [page] = createResource(slug, (s) => getPage(s))
  const [links] = createResource(slug, (s) => pageLinks(s))

  return (
    <div class="flex flex-col gap-6">
      <div class="breadcrumbs text-sm">
        <ul>
          <li>
            <A href="/pages">Pages</A>
          </li>
          <li class="font-mono">{slug()}</li>
        </ul>
      </div>

      <Async resource={page}>
        {(p) => (
          <article class="card bg-base-100 shadow">
            <div class="card-body">
              <h1 class="card-title">{p.title}</h1>
              <div class="flex flex-wrap gap-1">
                <For each={p.tags}>
                  {(tag) => (
                    <A class="badge badge-outline" href={`/pages?tag=${encodeURIComponent(tag)}`}>
                      {tag}
                    </A>
                  )}
                </For>
              </div>
              <div class="text-xs opacity-60">
                {p.size} bytes · created {p.created} · updated {p.updated}
              </div>
              {/*
                Raw markdown on purpose. Rendering (and the editor) is M7;
                the server can already do it via `?render=true`.
              */}
              <pre class="mt-2 max-h-[32rem] overflow-auto rounded bg-base-200 p-4 text-sm whitespace-pre-wrap">
                {p.content}
              </pre>
            </div>
          </article>
        )}
      </Async>

      <section class="card bg-base-100 shadow">
        <div class="card-body">
          <h2 class="card-title text-base">Links</h2>
          <Async resource={links}>
            {(l) => (
              <div class="grid gap-6 sm:grid-cols-2">
                <div>
                  <h3 class="mb-2 text-sm font-semibold opacity-70">
                    Outbound ({l.outbound.length})
                  </h3>
                  <ul class="flex flex-col gap-1 text-sm">
                    <For
                      each={l.outbound}
                      fallback={<li class="opacity-60">none</li>}
                    >
                      {(link) => (
                        <li>
                          <Show
                            when={link.kind !== 'external'}
                            fallback={
                              <span class="font-mono break-all">{link.target}</span>
                            }
                          >
                            <A class="link font-mono" href={`/pages/${encodeSlug(link.target)}`}>
                              {link.target}
                            </A>
                          </Show>
                          <span
                            class="badge badge-xs ml-2"
                            classList={{
                              'badge-warning': !link.resolved && link.kind !== 'external',
                            }}
                          >
                            {link.kind}
                            {link.resolved || link.kind === 'external' ? '' : ' · wanted'}
                          </span>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
                <div>
                  <h3 class="mb-2 text-sm font-semibold opacity-70">
                    Inbound ({l.inbound.length})
                  </h3>
                  <ul class="flex flex-col gap-1 text-sm">
                    <For each={l.inbound} fallback={<li class="opacity-60">none</li>}>
                      {(link) => (
                        <li>
                          <A class="link" href={`/pages/${encodeSlug(link.slug)}`}>
                            {link.title}
                          </A>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
                <Show when={!l.exists}>
                  <div class="alert alert-warning sm:col-span-2">
                    This page does not exist yet — it is a wanted page.
                  </div>
                </Show>
              </div>
            )}
          </Async>
        </div>
      </section>
    </div>
  )
}
