import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { assembledHref, compilePages, pageHref } from '../api/client'
import type { CompiledView, PageView, SectionView } from '../api/client'
import { Async } from './Async'
import StageSummary, { StageBadge } from './Stages'

/** How the sections can be laid out. */
type View = 'list' | 'cards'

const VIEWS: { value: View; label: string; title: string }[] = [
  { value: 'list', label: 'List', title: 'The spine, in order, one line each' },
  { value: 'cards', label: 'Cards', title: 'One card per section, with its synopsis' },
]

/**
 * The spine of a manuscript, rendered as something you can click.
 *
 * This panel is not a nicety. Order moved into frontmatter precisely so that a
 * formatter joining two lines could not reorder a book, and the cost of that
 * decision was stated up front: a contents page opened raw is a YAML list rather
 * than an index. This is what pays it back. Nothing else in the wiki renders the
 * spine, so anything hidden here is hidden everywhere.
 *
 * Which is why a gap, a duplicate and a mistyped entry are shown **in position**
 * rather than filtered out. A manuscript short of a chapter says where the
 * chapter was going to be, and that is the whole difference between a gap and an
 * omission.
 *
 * It costs one compile per page view, which is the same walk `GET /api/compile`
 * does for the document itself. A cheaper endpoint returning only the manifest
 * was considered and skipped: the walk is where the cost is, the assembly is
 * concatenation, and a second code path for the same tree is a second answer
 * about what the book is.
 */
export default function Manuscript(props: { page: PageView }) {
  const [compiled] = createResource(
    () => props.page.slug,
    (root) => compilePages({ root }),
  )
  /**
   * Which layout the sections get.
   *
   * The card view is the corkboard without the part of the corkboard that stores
   * coordinates. Freeform arrangement is deliberately absent: order lives in
   * frontmatter precisely so that nothing about a display can reorder a book, and
   * an x and a y per card would be exactly that in a different coat.
   */
  const [view, setView] = createSignal<View>('list')

  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body gap-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
          <h2 class="card-title text-base">Manuscript</h2>
          <div class="flex flex-wrap items-center gap-2">
            <div class="join" role="group" aria-label="Section layout">
              <For each={VIEWS}>
                {(option) => (
                  <button
                    class="btn join-item btn-sm"
                    classList={{ 'btn-active': view() === option.value }}
                    aria-pressed={view() === option.value}
                    title={option.title}
                    onClick={() => setView(option.value)}
                  >
                    {option.label}
                  </button>
                )}
              </For>
            </div>
            <A class="btn btn-ghost btn-sm" href={assembledHref(props.page.slug)}>
              Read assembled
            </A>
          </div>
        </div>

        <Async resource={compiled}>
          {(document) => (
            <Assembly page={props.page} compiled={document} view={view()} />
          )}
        </Async>
      </div>
    </section>
  )
}

function Assembly(props: { page: PageView; compiled: CompiledView; view: View }) {
  /**
   * Everything but the root's own body, which compile emits first.
   *
   * Dropped because it is the page you are already reading, and a book listing
   * itself as its own first chapter reads as a bug. Checked rather than assumed:
   * a compile given a `?style=` preamble puts that first instead, and this panel
   * should not silently swallow a section because it counted from one.
   */
  const parts = createMemo(() => {
    const sections = props.compiled.sections
    return sections[0]?.slug === props.compiled.root ? sections.slice(1) : sections
  })

  const included = createMemo(
    () => parts().filter((section) => section.status === 'included').length,
  )
  const gaps = createMemo(
    () => parts().filter((section) => section.status !== 'included').length,
  )

  /**
   * The sentence shown where there is nothing to list.
   *
   * An absent `contents:` and an empty one are different values and survive a
   * round trip as different values, so they get different sentences. A book on
   * the day it is started is the second.
   */
  const nothing = () => (
    <span class="text-sm opacity-60">
      <Show
        when={props.page.contents}
        fallback="Nothing is assembled here. Give this page a contents list to make it a manuscript."
      >
        This manuscript has no parts yet. Add slugs to its contents list.
      </Show>
    </span>
  )

  return (
    <>
      <Progress compiled={props.compiled} due={props.page.due} />

      {/*
        Counted across the sections this panel is showing, which is the manifest
        without the root's own body. A summary that counted something a reader
        cannot see is a number they have no way to check.
      */}
      <StageSummary sections={parts()} />

      <Show when={parts().length > 0} fallback={nothing()}>
        <Show
          when={props.view === 'cards'}
          fallback={
            <ul class="flex flex-col gap-1 text-sm">
              <For each={parts()}>
                {(section, position) => (
                  <Part section={section} position={position()} />
                )}
              </For>
            </ul>
          }
        >
          <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
            <For each={parts()}>
              {(section, position) => (
                <Card section={section} position={position()} />
              )}
            </For>
          </div>
        </Show>
      </Show>

      <Show when={parts().length > 0}>
        <div class="text-xs opacity-60">
          {included()} {included() === 1 ? 'section' : 'sections'}
          <Show when={gaps() > 0}>
            {' '}
            · {gaps()} not assembled
          </Show>
        </div>
      </Show>
    </>
  )
}

/**
 * One section, in its position, whatever happened to it.
 *
 * The status badge is the point of the row. `wanted` covers a slug nobody has
 * written **and** a page this reader may not see, deliberately: telling the two
 * apart would confirm that something exists at a slug somebody guessed, which is
 * `404, never 403` in the manifest's own spelling.
 */
function Part(props: { section: SectionView; position: number }) {
  const status = () => props.section.status
  const included = () => status() === 'included'

  return (
    <li class="flex flex-col gap-0.5">
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <span class="flex min-w-0 items-baseline gap-2">
          <span class="w-6 shrink-0 text-right font-mono text-xs opacity-40">
            {props.position + 1}
          </span>
          {/*
            Indented by depth, which is the only thing that says a chapter sits
            under a part rather than beside it. The list is flat because the
            manifest is: positions are what compile promises, and a tree would
            have to invent the nesting back out of them.
          */}
          <span style={{ 'padding-left': `${Math.max(0, props.section.depth - 1) * 0.75}rem` }}>
            <Show
              when={included()}
              fallback={
                <span class="font-mono text-xs break-all opacity-70">
                  {props.section.slug}
                </span>
              }
            >
              <A class="link" href={pageHref(props.section.slug)}>
                {props.section.title ?? props.section.slug}
              </A>
            </Show>
          </span>
        </span>

        <span class="flex shrink-0 items-baseline gap-2">
          <Show when={props.section.stage}>
            {(stage) => <StageBadge stage={stage()} />}
          </Show>
          <Show when={!included()}>
            <span class="badge badge-sm" classList={badgeClass(status())}>
              {status()}
            </span>
          </Show>
          <Show when={props.section.words > 0}>
            <span class="font-mono text-xs opacity-60">
              {props.section.words.toLocaleString()}
            </span>
          </Show>
        </span>
      </div>

      {/*
        Both hang off the title rather than the number, so a chapter's synopsis
        and its own progress line up under the chapter and not under the margin.
      */}
      <div style={{ 'padding-left': `${1.5 + Math.max(0, props.section.depth - 1) * 0.75}rem` }}>
        <Show when={props.section.synopsis}>
          {(synopsis) => (
            <p class="line-clamp-1 text-xs opacity-60" title={synopsis()}>
              {/*
                A text node, never `innerHTML`. A synopsis is page content and
                page content is what agents write, which is the rule
                `Snippet.tsx` exists to keep. Here it is kept by there being
                nothing to render: the field is plain text by definition.
              */}
              {synopsis()}
            </p>
          )}
        </Show>
        <SectionTarget section={props.section} />
      </div>
    </li>
  )
}

/**
 * One section, as a card.
 *
 * This is what a synopsis makes possible, and it is the corkboard without the
 * coordinates. A card with no synopsis says so rather than showing an excerpt of
 * the prose: an empty card is a chapter nobody has decided about yet, which is
 * exactly the thing worth seeing.
 */
function Card(props: { section: SectionView; position: number }) {
  const included = () => props.section.status === 'included'

  return (
    <article
      class="border-base-300 flex flex-col gap-2 rounded border p-3"
      classList={{ 'opacity-60': !included() }}
    >
      <div class="flex items-baseline justify-between gap-2">
        <span class="flex min-w-0 items-baseline gap-2">
          <span class="shrink-0 font-mono text-xs opacity-40">
            {props.position + 1}
          </span>
          <Show
            when={included()}
            fallback={
              <span class="font-mono text-xs break-all opacity-70">
                {props.section.slug}
              </span>
            }
          >
            <A class="link truncate text-sm font-medium" href={pageHref(props.section.slug)}>
              {props.section.title ?? props.section.slug}
            </A>
          </Show>
        </span>
        <Show
          when={props.section.stage}
          fallback={
            <Show when={!included()}>
              <span class="badge badge-sm" classList={badgeClass(props.section.status)}>
                {props.section.status}
              </span>
            </Show>
          }
        >
          {(stage) => <StageBadge stage={stage()} />}
        </Show>
      </div>

      {/*
        A text node, never `innerHTML`: a synopsis is page content and page
        content is what agents write. `whitespace-pre-line` is what keeps a card
        written as two paragraphs looking like two.

        The placeholder is only for a section that is actually in the document.
        A gap has no page to have a synopsis, and a cut chapter reports nothing
        about itself at all, so "No synopsis yet" there would be a sentence about
        a page rather than about the position it left behind.
      */}
      <p
        class="line-clamp-4 min-h-16 text-xs whitespace-pre-line"
        classList={{ 'opacity-40 italic': !props.section.synopsis }}
      >
        {props.section.synopsis ?? (included() ? 'No synopsis yet.' : '')}
      </p>

      <Show when={included()}>
        <div class="font-mono text-xs opacity-60">
          {props.section.words.toLocaleString()}{' '}
          {props.section.words === 1 ? 'word' : 'words'}
        </div>
      </Show>
      <SectionTarget section={props.section} />
    </article>
  )
}

/**
 * A section's own progress, wherever it names a target of its own.
 *
 * Measured against `subtree` rather than `words`, which is the whole reason the
 * manifest carries two numbers. A section's `words` is its own body, so on a part
 * page it is the epigraph and nothing else, and drawing a target against that
 * would show every part in the book at two per cent forever.
 */
function SectionTarget(props: { section: SectionView }) {
  const target = () => props.section.target ?? 0

  return (
    <Show when={target() > 0}>
      <div class="flex items-center gap-2">
        <progress
          class="progress progress-primary h-1 w-full"
          value={props.section.subtree}
          max={target()}
        />
        <span class="shrink-0 font-mono text-xs opacity-50">
          {props.section.subtree.toLocaleString()}/{target().toLocaleString()}
        </span>
      </div>
    </Show>
  )
}

/**
 * The not-included statuses that are worth different colours.
 *
 * `duplicate` is deliberately not a warning. The commonest case is not a cycle
 * at all: an appendix listed under two parts is a diamond, and the second
 * position reporting itself is the manifest working rather than complaining.
 * `excluded` is not one either: a page kept in the spine and out of the book is
 * a decision somebody made, not a fault.
 */
function badgeClass(status: string): Record<string, boolean> {
  return {
    'badge-warning': status === 'wanted',
    'badge-error': status === 'invalid' || status === 'unreadable',
    'badge-ghost': status === 'duplicate' || status === 'excluded',
  }
}

/**
 * Words against target, and the day it is due.
 *
 * Progress is arithmetic over the manifest rather than a field on the page, and
 * that is a decision rather than an omission: `target` is measured against the
 * **compiled** total, so filling a `progress` field on `GET /api/pages/{slug}`
 * would mean assembling the whole book on every page read.
 *
 * The bar is a bar and says nothing else. No streak, no encouragement, and
 * nothing that changes tone when the number goes up: the same terms the hours
 * heat map is on.
 */
function Progress(props: { compiled: CompiledView; due?: string | null }) {
  const target = () => props.compiled.target ?? 0
  const share = () =>
    target() > 0 ? Math.min(1, props.compiled.words / target()) : 0

  return (
    <div class="flex flex-col gap-1">
      <div class="flex flex-wrap items-baseline justify-between gap-2 text-sm">
        <span>
          <span class="font-mono text-lg">
            {props.compiled.words.toLocaleString()}
          </span>
          <span class="opacity-60">
            {' '}
            {props.compiled.words === 1 ? 'word' : 'words'}
            <Show when={target() > 0}>
              {' '}
              of {target().toLocaleString()}
            </Show>
          </span>
        </span>
        <Show when={props.due}>
          {(due) => (
            <span class="text-xs opacity-60">due {formatDay(due())}</span>
          )}
        </Show>
      </div>

      <Show when={target() > 0}>
        <progress
          class="progress progress-primary w-full"
          value={props.compiled.words}
          max={target()}
        />
        <div class="text-xs opacity-60">{Math.round(share() * 100)}%</div>
      </Show>
    </div>
  )
}

/**
 * A due date is a **day**, not an instant, so it is shown as one.
 *
 * It arrives as a full timestamp because this is JSON and a client has a clock,
 * and a bare `2027-03-01` in a file reads as midnight UTC. Rendering that in the
 * reader's own zone would show 28 February to anybody west of Greenwich, so the
 * day is read back in UTC and only its name is shown.
 */
export function formatDay(value: string): string {
  const at = new Date(value)
  if (Number.isNaN(at.getTime())) return value
  return at.toLocaleDateString(undefined, {
    timeZone: 'UTC',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}
