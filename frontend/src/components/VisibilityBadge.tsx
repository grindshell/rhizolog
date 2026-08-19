import { Show } from 'solid-js'
import type { Username, Visibility } from '../api/client'
import { sessionState } from '../api/session'

/**
 * Says who can read a page, when that is worth saying.
 *
 * Deliberately silent in the two cases where it would be noise. On a wiki with
 * no accounts nothing is restricted from anybody, and on an ordinary `internal`
 * page the answer is the same as it is for almost every other page — a badge on
 * everything is a badge nobody reads, which is precisely how the one that means
 * *anyone on the internet* would stop being noticed.
 */
export default function VisibilityBadge(props: {
  visibility: Visibility
  owner?: Username | null
  readers?: Username[]
}) {
  const worthSaying = (): boolean =>
    (sessionState()?.authentication_required ?? false) && props.visibility !== 'internal'

  return (
    <Show when={worthSaying()}>
      <span class={`badge badge-sm ${style(props.visibility)}`} title={explain(props)}>
        {props.visibility}
      </span>
    </Show>
  )
}

/**
 * `public` is a warning, not a status.
 *
 * It is the only rung that can reach somebody who has not signed in, and the
 * only one where picking it by mistake is a disclosure rather than an
 * inconvenience — so it is the one that is coloured to be noticed.
 */
function style(visibility: Visibility): string {
  switch (visibility) {
    case 'public':
      return 'badge-warning'
    case 'private':
      return 'badge-neutral'
    case 'restricted':
      return 'badge-outline'
    default:
      return 'badge-ghost'
  }
}

function explain(props: {
  visibility: Visibility
  owner?: Username | null
  readers?: Username[]
}): string {
  const owner = props.owner ? ` Owned by ${props.owner}.` : ''

  switch (props.visibility) {
    case 'public':
      return `Readable without signing in, if this instance serves anonymous readers.${owner}`
    case 'restricted': {
      const readers = props.readers?.length
        ? props.readers.join(', ')
        : 'nobody but the owner'
      return `Readable by ${readers}.${owner}`
    }
    case 'private':
      return `Readable by its owner alone.${owner}`
    default:
      return 'Readable by any account on this wiki.'
  }
}
