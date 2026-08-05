import { createResource, For } from 'solid-js'
import { A } from '@solidjs/router'
import { health, stats } from '../api/client'
import { Async, Json } from '../components/Async'
import { encodeSlug } from '../api/client'

/**
 * Placeholder dashboard. It really does call `/api/health` and `/api/stats`;
 * the point of the scaffold is that the typed client works end to end, not
 * that this looks like anything. M7 builds the real thing.
 */
export default function Dashboard() {
  const [healthRes] = createResource(() => health())
  const [statsRes] = createResource(() => stats())

  return (
    <div class="flex flex-col gap-6">
      <section>
        <h1 class="mb-3 text-2xl font-semibold">Dashboard</h1>
        <Async resource={healthRes}>
          {(h) => (
            <div class="stats stats-vertical sm:stats-horizontal shadow">
              <div class="stat">
                <div class="stat-title">Status</div>
                <div class="stat-value text-success text-2xl">{h.status}</div>
                <div class="stat-desc">v{h.version}</div>
              </div>
              <div class="stat">
                <div class="stat-title">Pages indexed</div>
                <div class="stat-value text-2xl">{h.pages}</div>
                <div class="stat-desc">{h.last_indexed ?? 'never scanned'}</div>
              </div>
              <div class="stat">
                <div class="stat-title">Wiki root</div>
                <div class="stat-desc break-all">{h.wiki_root}</div>
              </div>
            </div>
          )}
        </Async>
      </section>

      <section>
        <h2 class="mb-3 text-xl font-semibold">Graph</h2>
        <Async resource={statsRes}>
          {(s) => (
            <div class="flex flex-col gap-4">
              <div class="stats stats-vertical sm:stats-horizontal shadow">
                <div class="stat">
                  <div class="stat-title">Pages</div>
                  <div class="stat-value text-2xl">{s.pages}</div>
                </div>
                <div class="stat">
                  <div class="stat-title">Tags</div>
                  <div class="stat-value text-2xl">{s.tags}</div>
                </div>
                <div class="stat">
                  <div class="stat-title">Orphans</div>
                  <div class="stat-value text-2xl">{s.orphan_count}</div>
                </div>
                <div class="stat">
                  <div class="stat-title">Wanted</div>
                  <div class="stat-value text-2xl">{s.wanted_count}</div>
                </div>
              </div>

              <div class="card bg-base-100 shadow">
                <div class="card-body">
                  <h3 class="card-title text-base">Most linked</h3>
                  <ul class="list-disc pl-5 text-sm">
                    <For each={s.most_linked} fallback={<li class="list-none opacity-60">none</li>}>
                      {(page) => (
                        <li>
                          <A class="link" href={`/pages/${encodeSlug(page.slug)}`}>
                            {page.title}
                          </A>{' '}
                          <span class="opacity-60">({page.referrers})</span>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              </div>

              <details class="collapse-arrow collapse bg-base-100 shadow">
                <summary class="collapse-title text-sm font-medium">Raw /api/stats</summary>
                <div class="collapse-content">
                  <Json value={s} />
                </div>
              </details>
            </div>
          )}
        </Async>
      </section>
    </div>
  )
}
