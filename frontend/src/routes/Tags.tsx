import { createResource, For } from 'solid-js'
import { A } from '@solidjs/router'
import { tags } from '../api/client'
import { Async } from '../components/Async'

/** Every tag with its page count, linking into the filtered page list. */
export default function Tags() {
  const [result] = createResource(() => tags())

  return (
    <div class="flex flex-col gap-4">
      <h1 class="text-2xl font-semibold">Tags</h1>
      <Async resource={result}>
        {(data) => (
          <div class="flex flex-wrap gap-2">
            <For
              each={data.tags}
              fallback={<div class="opacity-60">No tags in the wiki yet.</div>}
            >
              {(tag) => (
                <A
                  class="badge badge-lg badge-outline gap-2"
                  href={`/pages?tag=${encodeURIComponent(tag.tag)}`}
                >
                  {tag.tag}
                  <span class="opacity-60">{tag.pages}</span>
                </A>
              )}
            </For>
          </div>
        )}
      </Async>
    </div>
  )
}
