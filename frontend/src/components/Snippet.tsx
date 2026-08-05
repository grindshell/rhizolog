import { For } from 'solid-js'

/** One run of snippet text, and whether the search matched it. */
interface Part {
  text: string
  matched: boolean
}

const MARK = /<mark>([\s\S]*?)<\/mark>/g

/**
 * A search excerpt with the matched terms highlighted.
 *
 * SQLite's `snippet()` wraps matches in `<mark>` but does **not** escape the
 * text around them — that is the page body, verbatim. Assigning it as
 * `innerHTML` would run whatever a page happened to contain, and page bodies
 * are exactly what an agent writes through the API. So the marks are parsed out
 * here and every piece is emitted as a text node.
 *
 * A page that literally contains the characters `<mark>` gets a spurious
 * highlight. That is the acceptable half of the trade: the text is still shown
 * as text.
 */
export default function Snippet(props: { text: string }) {
  return (
    <For each={split(props.text)}>
      {(part) =>
        part.matched ? (
          <mark class="rounded bg-warning/40 px-0.5 text-inherit">{part.text}</mark>
        ) : (
          <>{part.text}</>
        )
      }
    </For>
  )
}

export function split(snippet: string): Part[] {
  const parts: Part[] = []
  let index = 0

  // `lastIndex` is stateful on a global regex, so the loop resets it by
  // construction: `exec` is only ever called on this one local walk.
  MARK.lastIndex = 0
  for (let match = MARK.exec(snippet); match; match = MARK.exec(snippet)) {
    if (match.index > index) {
      parts.push({ text: snippet.slice(index, match.index), matched: false })
    }
    parts.push({ text: match[1] ?? '', matched: true })
    index = match.index + match[0].length
  }

  if (index < snippet.length) {
    parts.push({ text: snippet.slice(index), matched: false })
  }

  return parts
}
