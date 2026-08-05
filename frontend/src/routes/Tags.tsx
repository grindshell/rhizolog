import { createResource, For } from 'solid-js'
import { A } from '@solidjs/router'
import { tagHref, tags } from '../api/client'
import { Async } from '../components/Async'

/** Every tag with its page count, linking into the filtered page list. */
export default function Tags() {
  const [result] = createResource(() => tags())

  return (
    <div class="flex flex-col gap-4">
      <h1 class="text-2xl font-semibold">Tags</h1>
      <p class="text-sm opacity-70">
        Most-used first. Following one filters the page list.
      </p>
      <Async resource={result}>
        {(data) => (
          <div class="flex flex-wrap gap-2">
            <For
              each={data.tags}
              fallback={
                <div class="opacity-60">
                  No tags in the wiki yet — add some in a page's frontmatter.
                </div>
              }
            >
              {(tag) => (
                <A class="badge badge-lg badge-outline gap-2" href={tagHref(tag.tag)}>
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
