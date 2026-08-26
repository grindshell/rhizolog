/**
 * Drafting stages: what the four known ones look like, and how a set of them is
 * counted.
 *
 * **The vocabulary is not fixed.** Four names are known and get a colour;
 * anything else is shown as itself, in an outline. A writer whose process has
 * `with-beta-readers` in it should not have to argue with a schema, and this is
 * a wiki whose whole stance is that these are your notes.
 *
 * Which is why the colours live here rather than in frontmatter. Scrivener's
 * Label is a colour with a name; here a stage is a word and the dashboard
 * decides how to paint it, because a palette in a page's frontmatter would be a
 * document about a display. See `knowledge-base/drafting.md`.
 */

import { For, Show } from 'solid-js'

/**
 * The four the dashboard knows, in the order a chapter passes through them.
 *
 * An order rather than a set, because it is what the summary reads in: left to
 * right is the work moving.
 */
export const KNOWN_STAGES = ['todo', 'drafted', 'revised', 'final'] as const

/**
 * How two stages are compared when they are being grouped or coloured.
 *
 * The same fold the backend applies for `?stage=` and `?sort=stage`: trimmed and
 * lowercased, so `Drafted` and `drafted` are one stage while the file keeps
 * whichever was typed. ASCII case, matching SQLite's `lower()`, so the two
 * cannot disagree about a stage somebody wrote in Turkish.
 */
export function stageKey(raw: string): string {
  return raw.trim().replace(/[A-Z]/g, (letter) => letter.toLowerCase())
}

const COLOURS: Record<string, string> = {
  todo: 'badge-ghost',
  drafted: 'badge-info',
  revised: 'badge-accent',
  final: 'badge-success',
}

/**
 * `todo` reads as a filename and `to do` reads as English.
 *
 * Only ever applied to a stage this dashboard knows. Anything else is the
 * author's own word and is printed as they wrote it, because relabelling
 * somebody's vocabulary is the schema argument by another route.
 */
export function stageLabel(stage: string): string {
  return stageKey(stage) === 'todo' ? 'to do' : stage
}

/** One stage badge, in the author's own spelling. */
export function StageBadge(props: { stage: string; class?: string }) {
  return (
    <span
      class={`badge badge-sm shrink-0 ${COLOURS[stageKey(props.stage)] ?? 'badge-outline'} ${props.class ?? ''}`}
      title={`Stage: ${props.stage}`}
    >
      {stageLabel(props.stage)}
    </span>
  )
}

export interface StageCount {
  /** The stage as it will be shown: the known spelling, or the first one met. */
  stage: string
  count: number
}

/**
 * How many sections sit at each stage.
 *
 * Known stages first, in lifecycle order, then anything else in the order it was
 * met. Sections with no stage are not a bucket: an unstaged chapter is one
 * nobody has said anything about, and counting it as a stage would be inventing
 * the opinion this feature refuses to derive.
 */
export function countStages(sections: readonly { stage?: string | null }[]): StageCount[] {
  const counts = new Map<string, StageCount>()

  for (const section of sections) {
    const raw = section.stage
    if (!raw || raw.trim() === '') continue

    const key = stageKey(raw)
    const seen = counts.get(key)
    if (seen) {
      seen.count += 1
      continue
    }
    // A known stage is shown in its canonical spelling, so `Drafted` and
    // `drafted` do not become two entries with one of the spellings winning by
    // arriving first. An unknown one has no canonical spelling to prefer.
    counts.set(key, {
      stage: (KNOWN_STAGES as readonly string[]).includes(key) ? key : raw.trim(),
      count: 1,
    })
  }

  const known = KNOWN_STAGES.map((stage) => counts.get(stage)).filter(
    (entry): entry is StageCount => entry !== undefined,
  )
  const rest = [...counts.entries()]
    .filter(([key]) => !(KNOWN_STAGES as readonly string[]).includes(key))
    .map(([, entry]) => entry)

  return [...known, ...rest]
}

/**
 * The stage summary: a count, on the same terms the words chart is on.
 *
 * No completion percentage, no encouragement, and nothing that changes tone when
 * the number goes up. It is also a summary rather than an invention: a part whose
 * chapters are half revised is not "in progress", it is whatever its own
 * frontmatter says, and nothing here rolls one up.
 *
 * A manuscript with no stages anywhere renders nothing at all. A row of zeroes
 * would be four claims about a book nobody has staged.
 */
export default function StageSummary(props: {
  sections: readonly { stage?: string | null }[]
}) {
  const counts = () => countStages(props.sections)

  return (
    <Show when={counts().length > 0}>
      <div class="flex flex-wrap items-center gap-2" role="list" aria-label="Stages">
        <For each={counts()}>
          {(entry) => (
            <span class="flex items-baseline gap-1 text-xs" role="listitem">
              <span class="font-mono">{entry.count}</span>
              <StageBadge stage={entry.stage} />
            </span>
          )}
        </For>
      </div>
    </Show>
  )
}
