import { For, Show, createMemo, createResource, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import { assembledHref, compilePages, pageHref, patchPage } from '../api/client'
import type { CompiledView, PageView, SectionView } from '../api/client'
import { Async, ErrorNotice } from './Async'
import Pace, { formatDay } from './Pace'
import StageSummary, { StageBadge } from './Stages'
import { bounds, entry, lands, moved, sameEntry, spines } from './spine'
import type { Bounds, Entry } from './spine'

/**
 * How the sections are shown, and whether they can be moved.
 *
 * `reorder` is a mode rather than a layout, and it is in this group anyway: it is
 * the list with two buttons on each row, and a fourth control saying "now you may
 * edit" beside three saying "look at it this way" would be a distinction nobody
 * asked for. Being modal is the point. Order lives in frontmatter precisely so
 * that nothing incidental can move a chapter, and a mode you have to enter is
 * that argument carried into the one place that can.
 */
type View = 'list' | 'cards' | 'reorder'

const VIEWS: { value: View; label: string; title: string }[] = [
  { value: 'list', label: 'List', title: 'The spine, in order, one line each' },
  { value: 'cards', label: 'Cards', title: 'One card per section, with its synopsis' },
  {
    value: 'reorder',
    label: 'Reorder',
    title: 'Move an entry within the contents list that names it',
  },
]

/** What a row needs to offer its two buttons and be dragged. */
interface Reorder extends Bounds {
  busy: boolean
  move: (by: -1 | 1) => void
  /** This row is the one being dragged. */
  lifted: boolean
  /** A drop here would put the dragged entry in this position. */
  landing: boolean
  lift: (event: DragEvent) => void
  over: (event: DragEvent) => void
  drop: (event: DragEvent) => void
  release: () => void
}

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
  const [compiled, { refetch }] = createResource(
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
  const [moving, setMoving] = createSignal(false)
  const [failure, setFailure] = createSignal<unknown>()

  /**
   * Move one entry to a position within the list that names it, and read the
   * book back.
   *
   * A `PATCH` of the parent's `contents:` and nothing else. There is no reorder
   * endpoint, deliberately: `contents` is already patchable, a second way to say
   * the same thing would be two answers to one question, and the whole dashboard
   * is read-modify-write already, since saving in the editor replaces a page with
   * what was on screen when it opened.
   *
   * `to` is a position rather than a direction, which is what lets one function
   * serve both gestures: a button asks for the place next door and a drop asks
   * for the place it landed on, and neither of them is a different write.
   *
   * The list written back is rebuilt from the manifest this panel is showing, so
   * it is as old as the compile above it. A chapter added in your own editor
   * since then would be written back out of the spine. Refetching after the write
   * is what makes that visible rather than silent, and the honest fix is the same
   * one the editor wants and does not have.
   */
  const move = async (section: SectionView, to: number) => {
    const document = compiled.error ? undefined : compiled.latest
    const parent = section.parent
    const ordinal = section.ordinal
    if (moving() || !document || parent == null || ordinal == null) return

    const list = spines(document.sections).get(parent)
    if (!list || to < 0 || to >= list.length) return

    setMoving(true)
    setFailure(undefined)
    try {
      await patchPage(parent, { contents: moved(list, ordinal, to) })
      await refetch()
    } catch (error) {
      setFailure(error)
    } finally {
      setMoving(false)
    }
  }

  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body gap-3">
        <div class="flex flex-wrap items-baseline justify-between gap-2">
          <h2 class="card-title text-base">Manuscript</h2>
          <div class="flex flex-wrap items-center gap-2">
            <div class="join" role="group" aria-label="Section view">
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

        {/*
          A failed move, kept until the next one. It is the only write this panel
          makes, and the interesting failure is a 409 or a 401 rather than
          anything about the book.
        */}
        <Show when={failure()}>
          <ErrorNotice error={failure()} />
        </Show>

        <Async resource={compiled}>
          {(document) => (
            <Assembly
              page={props.page}
              compiled={document}
              view={view()}
              moving={moving()}
              onMove={move}
            />
          )}
        </Async>
      </div>
    </section>
  )
}

function Assembly(props: {
  page: PageView
  compiled: CompiledView
  view: View
  moving: boolean
  onMove: (section: SectionView, to: number) => void
}) {
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
   * Every contents list the manifest holds, rebuilt once for the whole panel.
   *
   * Over the **whole** manifest rather than over `parts()`, because the root's
   * own section is dropped from the list and is still the page most of these
   * entries belong to. Rebuilding a list without it would be rebuilding it
   * without one of its entries.
   */
  const lists = createMemo(() => spines(props.compiled.sections))

  /**
   * The drag, which is two signals and nothing else.
   *
   * What is being moved lives here rather than in the drag's own `dataTransfer`,
   * and that is the decision worth stating: the payload is an entry in a list
   * this panel is holding, so a drag arriving from another tab carrying the same
   * slug would mean nothing here and must not be treated as though it did. The
   * transfer is still filled in, because a drag with nothing in it is one some
   * browsers decline to start.
   */
  const [lifted, setLifted] = createSignal<SectionView>()
  // Compared by what it is rather than by identity, because `dragover` fires
  // every few tens of milliseconds over a stationary pointer and a fresh object
  // each time would redraw every row in the book for a mark that has not moved.
  const [landing, setLanding] = createSignal<Entry | undefined>(undefined, {
    equals: (before, after) => before === after || sameEntry(before, after),
  })

  const lift = (section: SectionView, event: DragEvent) => {
    if (props.moving) return
    setLifted(section)
    setLanding(undefined)
    const transfer = event.dataTransfer
    if (!transfer) return
    transfer.effectAllowed = 'move'
    transfer.setData('text/plain', section.slug)
  }

  /**
   * Over a row, which is where the rule is enforced rather than at the drop.
   *
   * A `dragover` refuses the drop unless it is cancelled, so not calling
   * `preventDefault` is how a row says no, and the pointer says so too without
   * anything here having to draw it. Every row gets this handler, including the
   * ones that refuse, so that passing over a chapter in another part clears the
   * mark left on the last one it could have used.
   */
  const over = (section: SectionView, event: DragEvent) => {
    const what = lifted()
    if (!what || !lands(entry(what), section)) {
      setLanding(undefined)
      return
    }
    event.preventDefault()
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
    setLanding(entry(section))
  }

  const drop = (section: SectionView, event: DragEvent) => {
    const what = lifted()
    const there = entry(section)
    setLifted(undefined)
    setLanding(undefined)
    if (!what || !there || !lands(entry(what), section)) return
    event.preventDefault()
    props.onMove(what, there.ordinal)
  }

  /** The end of a drag, whether it was dropped anywhere or given up on. */
  const release = () => {
    setLifted(undefined)
    setLanding(undefined)
  }

  /**
   * Whether a drag is on and this row is nowhere it could end.
   *
   * On the row rather than on its controls, which is the difference that matters
   * for the rows that have none. A section in a list this panel could not rebuild
   * gets no buttons and cannot be dragged, and it is still somewhere a drop
   * cannot go: left undimmed beside eight rows that are, it would be the one row
   * on screen claiming to accept what it will not.
   */
  const inert = (section: SectionView) => {
    const what = lifted()
    if (!what) return false
    const here = entry(section)
    return !sameEntry(here, entry(what)) && !lands(entry(what), section)
  }

  /** What a row needs to move itself, or nothing outside the reorder view. */
  const reorderable = (section: SectionView): Reorder | undefined => {
    if (props.view !== 'reorder') return undefined
    const ends = bounds(lists(), section)
    const here = entry(section)
    // `bounds` is already undefined wherever `entry` is, so the second check
    // never fires on its own. Asking it anyway is what tells the compiler the
    // ordinal is a number, without anything here asserting that it is.
    if (!ends || !here) return undefined

    const what = lifted()
    const mine = sameEntry(here, what && entry(what))

    return {
      ...ends,
      busy: props.moving,
      move: (by) => props.onMove(section, here.ordinal + by),
      lifted: mine,
      landing: sameEntry(here, landing()),
      lift: (event) => lift(section, event),
      over: (event) => over(section, event),
      drop: (event) => drop(section, event),
      release,
    }
  }

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
        Under the bar rather than beside it, because it is the same question with
        time in it: the bar says how far along, this says how fast. It costs a
        second walk of the same book, which is the one real price of putting it
        here and is only paid on a page that names a target or a day.
      */}
      <Pace page={props.page} />

      {/*
        Counted across the sections this panel is showing, which is the manifest
        without the root's own body. A summary that counted something a reader
        cannot see is a number they have no way to check.
      */}
      <StageSummary sections={parts()} />

      {/*
        Every contents list this manifest holds, rebuilt once for the whole
        panel rather than per row: it is what decides which buttons are at an
        end, and rebuilding it inside a `For` would do it once a chapter.
      */}
      <Show when={props.view === 'reorder'}>
        <p class="text-xs opacity-60">
          Drag a row onto another, or use its arrows. Each entry moves within the
          contents list that names it, so a chapter cannot leave its part and the
          rows it cannot land on dim while you drag. Editing that list by hand is
          the other way, and the only way to move one between parts.
        </p>
      </Show>

      <Show when={parts().length > 0} fallback={nothing()}>
        <Show
          when={props.view === 'cards'}
          fallback={
            <ul
              class="flex flex-col gap-1 text-sm"
              /*
                Rows mark and unmark themselves as the pointer crosses them, so
                the only place a mark could be left behind is where no row's
                handler runs: off the end of the list, and the few pixels of gap
                between two rows, which is not a place a drop can land either.

                Clearing on every `dragleave` covers both without asking which
                element the pointer went to. The obvious version of that question
                is `relatedTarget`, and it is one browsers answer differently:
                some do not populate it on a drag event at all, and a guard
                reading it would then either do nothing or do this. Whichever it
                is, a pointer still over a row it can use gets the mark back from
                that row's next `dragover`, which arrives within the frame.
              */
              onDragLeave={() => setLanding(undefined)}
            >
              <For each={parts()}>
                {(section, position) => (
                  <Part
                    section={section}
                    position={position()}
                    reorder={reorderable(section)}
                    dimmed={inert(section)}
                  />
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
function Part(props: {
  section: SectionView
  position: number
  reorder?: Reorder
  /** A drag is on and nothing about this row is part of it. */
  dimmed?: boolean
}) {
  const status = () => props.section.status
  const included = () => status() === 'included'

  /**
   * The whole row is what gets picked up, rather than the grip beside the
   * buttons.
   *
   * A grip is a small target, and the drag image a browser makes of one is the
   * glyph rather than the chapter. Carrying `draggable` on the row means the
   * thing under the pointer during the drag is the row, which is the thing being
   * moved. The price is that a row in this view cannot have its text selected,
   * which is what a mode is for.
   */
  const liftable = () =>
    props.reorder !== undefined && !props.reorder.busy

  return (
    <li
      /*
        The padding is for the ring a landing row draws, and the negative margin
        is so that paying for it costs the List view nothing: a row's text lines
        up with the progress bar and the summary above and below it, as it did
        before there was anything to draw. Padding alone indented every row in
        every view by four pixels for a mark only one view has.
      */
      class="-mx-1 flex flex-col gap-0.5 px-1 transition-opacity"
      classList={{
        'cursor-grab select-none active:cursor-grabbing': liftable(),
        'opacity-40': props.reorder?.lifted,
        'opacity-30': props.dimmed,
        'ring-primary rounded-sm ring-1': props.reorder?.landing,
      }}
      draggable={liftable()}
      onDragStart={(event) => props.reorder?.lift(event)}
      onDragOver={(event) => props.reorder?.over(event)}
      onDrop={(event) => props.reorder?.drop(event)}
      onDragEnd={() => props.reorder?.release()}
    >
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <span class="flex min-w-0 items-baseline gap-2">
          <Show when={props.reorder}>
            {(reorder) => <Handles section={props.section} reorder={reorder()} />}
          </Show>
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
              {/*
                A link is draggable of its own accord, and a drag started on one
                is a drag of the link rather than of the row. Refusing it in this
                view hands the gesture to the row underneath; outside it the
                attribute is absent and a link is a link.
              */}
              <A
                class="link"
                href={pageHref(props.section.slug)}
                draggable={props.reorder ? false : undefined}
              >
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
 * Two buttons, and a grip saying the row can also be dragged.
 *
 * The buttons came first and are still the ones that have to work. A drag has no
 * keyboard and none on a phone, so it can only ever be the second way in; these
 * are the first, and they are the same size on a row that has wrapped.
 *
 * The label names the entry rather than the direction, because a row of "Move up"
 * buttons read out one after another says nothing about which chapter each one
 * moves.
 */
function Handles(props: { section: SectionView; reorder: Reorder }) {
  const name = () => props.section.title ?? props.section.slug

  return (
    <span class="flex shrink-0 items-baseline gap-1">
      {/*
        A cue, not a control. The row carries `draggable`, so the drag starts
        anywhere on it and this only says so. Hidden from assistive technology
        deliberately: a handle nothing can grab from a keyboard would be a
        control that does not work, and the two buttons beside it are the ones
        that do.
      */}
      <span
        class="w-3 shrink-0 text-center text-xs opacity-30"
        aria-hidden="true"
      >
        ⠿
      </span>
      <span class="join shrink-0">
        <button
          class="btn join-item btn-xs"
          disabled={props.reorder.first || props.reorder.busy}
          title="Move up"
          aria-label={`Move ${name()} up`}
          onClick={() => props.reorder.move(-1)}
        >
          {/* A character rather than an icon set this project does not have. */}
          <span aria-hidden="true">↑</span>
        </button>
        <button
          class="btn join-item btn-xs"
          disabled={props.reorder.last || props.reorder.busy}
          title="Move down"
          aria-label={`Move ${name()} down`}
          onClick={() => props.reorder.move(1)}
        >
          <span aria-hidden="true">↓</span>
        </button>
      </span>
    </span>
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
