import { For, Show } from 'solid-js'
import { A } from '@solidjs/router'
import { prefixHref, slugSegments } from '../api/client'

/**
 * A slug shown as its parts, with the directories as links.
 *
 * Following one narrows the listing to everything at or under that path, so
 * `rust` in `notes/rust/async` means `notes/rust` — the hierarchical reading.
 * The flat one, "every `rust` directory in the wiki", is the badge row on a
 * page; the two live apart deliberately, and the tooltips say which is which.
 *
 * The last segment is the page, not a directory holding it, so it stays text.
 * Wherever this appears the page's own title is already a link right beside it,
 * and a second link to the same page dressed as a filter would only mislead.
 */
export default function SlugPath(props: { slug: string; class?: string }) {
  return (
    <span class={props.class}>
      <For each={slugSegments(props.slug)}>
        {(segment) => (
          <Show when={!segment.last} fallback={<span>{segment.name}</span>}>
            <A
              class="link link-hover"
              href={prefixHref(segment.path)}
              title={`Pages under ${segment.path}`}
            >
              {segment.name}
            </A>
            <span class="opacity-40">/</span>
          </Show>
        )}
      </For>
    </span>
  )
}
