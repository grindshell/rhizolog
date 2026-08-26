import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js'
import { checkProse } from '../api/client'
import type { FindingView, ProseReport } from '../api/client'
import { ErrorNotice } from './Async'

/** How long typing has to pause before the rules are run again. */
const CHECK_DELAY_MS = 400

const OPEN_KEY = 'rhizolog:editor-findings'

/**
 * Whether the strip was left open.
 *
 * A working preference rather than a property of the page, exactly like the
 * editor's layout: somebody who closed it to write does not want it back on the
 * next page they open.
 */
function storedOpen(): boolean {
  try {
    return window.localStorage.getItem(OPEN_KEY) === 'open'
  } catch {
    // Storage can be unavailable: private mode, a blocked origin. Remembering
    // whether a strip was open is not worth failing the editor over.
    return false
  }
}

function rememberOpen(open: boolean) {
  try {
    window.localStorage.setItem(OPEN_KEY, open ? 'open' : 'closed')
  } catch {
    // As above.
  }
}

/**
 * What your own rules have to say about what you are writing.
 *
 * `prose/v1` is voice defence rather than a grammar checker, and everything
 * about this strip follows from that. There is no dismissal control, because a
 * finding is your own rule firing on your own text: if it fires where it should
 * not, the rule is wrong, and the fix is one edit to `.rhizolog/prose.toml`
 * rather than a store of things to ignore.
 *
 * Every finding shows the text it fired on and the arithmetic behind it. A rule
 * that reported a problem without quoting it would be asking to be believed.
 *
 * The quote is **page content**, which is to say it is whatever an agent wrote,
 * so it is rendered as text and never as HTML. That is the rule `Snippet.tsx`
 * exists to keep and it applies here for the same reason.
 */
export default function Findings(props: {
  content: string
  /** Reveal a finding in the editor. Byte offsets; see {@link byteToIndex}. */
  onReveal?: (finding: FindingView) => void
}) {
  const [open, setOpen] = createSignal(storedOpen())

  const toggle = () => {
    const next = !open()
    setOpen(next)
    rememberOpen(next)
  }

  /**
   * The draft being checked, or `null` when there is nothing to check.
   *
   * Solid skips the fetcher for a null source, so `null` is what makes a closed
   * strip cost no `POST /api/prose` at all: not one per pause in typing, and
   * not the one at mount either. This is the preview's rule, and it is not
   * pedantry: this wiki counts its own API usage and puts the numbers on its
   * dashboard, so a request nobody asked for shows up there.
   */
  const [checking, setChecking] = createSignal<string | null>(null)

  /** Whether the strip is coming back rather than keeping up. */
  let wasClosed = true

  createEffect(() => {
    // Returning before `content` is read is what stops a closed strip debouncing
    // at all: the effect then depends only on whether it is open.
    if (!open()) {
      wasClosed = true
      setChecking(null)
      return
    }

    const draft = props.content

    // Re-opening skips the debounce. The debounce waits for a pause in typing,
    // and nobody is typing: they clicked.
    if (wasClosed) {
      wasClosed = false
      setChecking(draft)
      return
    }

    const timer = setTimeout(() => setChecking(draft), CHECK_DELAY_MS)
    onCleanup(() => clearTimeout(timer))
  })

  const [report] = createResource(checking, (content) => checkProse({ content }))

  /** The last good answer, so the strip does not blank on every keystroke. */
  const latest = (): ProseReport | undefined =>
    report.error ? undefined : report.latest

  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body gap-2 p-4">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <button
            class="btn btn-ghost btn-sm justify-start px-1"
            aria-expanded={open()}
            aria-controls="prose-findings"
            onClick={toggle}
          >
            <span class="text-base">Prose</span>
            <Show when={open() && report.loading}>
              <span class="loading loading-spinner loading-xs" />
            </Show>
            <Show when={latest()}>
              {(found) => (
                <>
                  <Show when={found().errors > 0}>
                    <span class="badge badge-error badge-sm">{found().errors}</span>
                  </Show>
                  <Show when={found().warnings > 0}>
                    <span class="badge badge-warning badge-sm">{found().warnings}</span>
                  </Show>
                  <Show when={found().findings.length === 0}>
                    <span class="badge badge-ghost badge-sm">clean</span>
                  </Show>
                </>
              )}
            </Show>
          </button>

          <Show when={open() && latest()}>
            {(found) => (
              // The digest is what ties a finding to the rules that produced it.
              // Shown rather than hidden because the whole contract here is that
              // a finding can be reproduced, and two reports that disagree on
              // this came from different rules.
              <span
                class="font-mono text-xs opacity-40"
                title={`Ruleset ${found().rules_digest} · ${found().analyzer}`}
              >
                {found().rules_digest.slice(0, 14)}
              </span>
            )}
          </Show>
        </div>

        <Show when={open()}>
          <div id="prose-findings">
            <Show when={report.error} fallback={null}>
              <ErrorNotice error={report.error} />
            </Show>

            <Show when={latest()}>
              {(found) => (
                <>
                  <ul class="flex flex-col gap-2">
                    <For
                      each={found().findings}
                      fallback={
                        <li class="text-sm opacity-60">
                          {/*
                            A wiki with no rules file is the ordinary case, not
                            an error, so the empty state has to say which of the
                            two silences this is. No findings from no rules is
                            not a clean page, and saying "nothing to report"
                            there would be the strip claiming an answer it never
                            asked for.
                          */}
                          <Show
                            when={found().rules > 0}
                            fallback="No rules yet. Write .rhizolog/prose.toml to hold this page to rules of your own."
                          >
                            Nothing to report.
                          </Show>
                        </li>
                      }
                    >
                      {(finding) => (
                        <Finding finding={finding} onReveal={props.onReveal} />
                      )}
                    </For>
                  </ul>

                  <Show when={found().truncated}>
                    <p class="pt-2 text-xs opacity-60">
                      The list was cut short. A rule matching something very
                      common returns a finding per occurrence, and this says so
                      rather than pretending to be the whole answer.
                    </p>
                  </Show>
                </>
              )}
            </Show>
          </div>
        </Show>
      </div>
    </section>
  )
}

function Finding(props: {
  finding: FindingView
  onReveal?: (finding: FindingView) => void
}) {
  const severity = () => props.finding.severity

  return (
    <li class="border-base-300 border-l-2 pl-3" classList={severityClass(severity())}>
      <button
        class="w-full text-left"
        onClick={() => props.onReveal?.(props.finding)}
        title="Select this in the editor"
      >
        <div class="flex flex-wrap items-baseline gap-2">
          <span class="badge badge-xs" classList={badgeClass(severity())}>
            {props.finding.rule}
          </span>
          <span class="text-sm">{props.finding.message}</span>
        </div>
        {/*
          Rendered as text. The backend cuts this from the source at exactly the
          span, so it is page content and page content is what agents write.
        */}
        <p class="mt-1 font-mono text-xs break-words opacity-70">
          {props.finding.quote}
        </p>
      </button>
    </li>
  )
}

function severityClass(severity: string): Record<string, boolean> {
  return {
    'border-error': severity === 'error',
    'border-warning': severity !== 'error',
  }
}

function badgeClass(severity: string): Record<string, boolean> {
  return {
    'badge-error': severity === 'error',
    'badge-warning': severity !== 'error',
  }
}

/**
 * A byte offset, as a position in a JavaScript string.
 *
 * Spans are byte offsets into the page source, which is the right answer for an
 * editor over a textarea and the wrong shape for `setSelectionRange`: JavaScript
 * indexes UTF-16 code units, so one `é` is two bytes and one unit, and one emoji
 * is four bytes and two units. Handing a byte offset straight to the textarea
 * silently selects the wrong words on any page with an accent in it, and does it
 * further and further out as the page goes on.
 *
 * An offset that lands inside a character rounds forward to its start, which
 * cannot happen for a span the analyzer produced and is the only sane answer if
 * it ever does.
 */
export function byteToIndex(text: string, offset: number): number {
  if (offset <= 0) return 0

  let bytes = 0
  let index = 0

  for (const character of text) {
    if (bytes >= offset) break
    bytes += utf8Length(character.codePointAt(0) ?? 0)
    index += character.length
  }

  return index
}

function utf8Length(codePoint: number): number {
  if (codePoint < 0x80) return 1
  if (codePoint < 0x800) return 2
  if (codePoint < 0x10000) return 3
  return 4
}
