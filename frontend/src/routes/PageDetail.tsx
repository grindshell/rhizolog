import { For, Match, Show, Switch, createResource, createSignal } from 'solid-js'
import { A, useParams, useSearchParams } from '@solidjs/router'
import {
  ApiError,
  compilePages,
  decodeSlug,
  editHref,
  getPage,
  graphHref,
  groupHref,
  pageHref,
  pageLinks,
  pageTimesHref,
  prefixHref,
  segmentHref,
  slugSegments,
  tagHref,
} from '../api/client'
import type { PageLinksResponse, PageView } from '../api/client'
import { pins } from '../api/pins'
import { Async, ErrorNotice } from '../components/Async'
import Duration, { formatDuration } from '../components/Duration'
import Manuscript from '../components/Manuscript'
import Markdown from '../components/Markdown'
import { StageBadge } from '../components/Stages'
import SlugPath from '../components/SlugPath'
import { PageTimerButton } from '../components/TimerMenu'
import VisibilityBadge from '../components/VisibilityBadge'

/**
 * Read one page.
 *
 * The route is `/pages/*slug`, a splat, because slugs contain `/`
 * (`notes/rust/async`). A single-segment `:slug` param would only ever match
 * top-level pages — the same trap the backend hit with `/api/pages/{*slug}`.
 *
 * `@solidjs/router` reads `location.pathname` verbatim and does not decode
 * path params, so the raw param goes through `decodeSlug` before it reaches
 * the API client.
 */
export default function PageDetail() {
  const params = useParams<{ slug: string }>()
  const [searchParams, setSearchParams] = useSearchParams()
  const slug = () => decodeSlug(params.slug ?? '')
  const [showSource, setShowSource] = createSignal(false)

  /**
   * Whether to show the whole manuscript rather than this page.
   *
   * In the URL rather than in component state, because it is a different
   * document and somebody should be able to keep the link to it. It is also the
   * proof-reading view, which is a thing you send yourself.
   */
  const assembled = () => searchParams.assembled === '1'
  const setAssembled = (on: boolean) =>
    setSearchParams({ assembled: on ? '1' : undefined }, { replace: true })
  const [pinning, setPinning] = createSignal(false)
  const [pinFailure, setPinFailure] = createSignal<unknown>()

  const togglePin = async () => {
    if (pinning()) return
    setPinning(true)
    setPinFailure(undefined)
    try {
      await pins.toggle(slug())
    } catch (error) {
      // Shown here rather than swallowed: the interesting failure is hitting
      // the pin limit, and it comes back with the limit in `details`.
      setPinFailure(error)
    } finally {
      setPinning(false)
    }
  }

  const [page] = createResource(slug, (target) => getPage(target, { render: true }))
  const [links, { refetch: refetchLinks }] = createResource(slug, (target) =>
    pageLinks(target),
  )

  /**
   * The assembled document, fetched only while it is being looked at.
   *
   * A null source is what makes that true: Solid skips the fetcher for one, so
   * an ordinary page view costs no compile at all. That matters more here than
   * it does for the editor's preview, because a compile walks a whole book.
   */
  const [document] = createResource(
    () => (assembled() ? slug() : null),
    (root) => compilePages({ root, format: 'html' }),
  )

  /**
   * A slug with no page is not an error here. Something linked to it, which is
   * how wanted pages come into being, and the useful answer is "not yet — here
   * is what is waiting for it".
   */
  const wanted = () => page.error instanceof ApiError && page.error.code === 'page_not_found'

  return (
    <div class="flex flex-col gap-6">
      {/*
        One `<li>` per segment rather than one holding the whole slug, so the
        directories a page sits in are the breadcrumb rather than decoration on
        the end of it. Each one leads to what is under it; the last is the page
        you are already reading.
      */}
      <div class="breadcrumbs text-sm">
        <ul>
          <li>
            <A href="/pages">Pages</A>
          </li>
          <For each={slugSegments(slug())}>
            {(segment) => (
              <li class="font-mono">
                <Show when={!segment.last} fallback={<span>{segment.name}</span>}>
                  <A href={prefixHref(segment.path)} title={`Pages under ${segment.path}`}>
                    {segment.name}
                  </A>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </div>

      <Switch>
        <Match when={page.loading}>
          <div class="flex items-center gap-3 py-6 text-base-content/60">
            <span class="loading loading-spinner loading-sm" />
            Loading...
          </div>
        </Match>

        <Match when={wanted()}>
          <div class="card bg-base-100 shadow">
            <div class="card-body items-start">
              <h1 class="card-title">
                <span class="font-mono">{slug()}</span>
                <span class="badge badge-warning">wanted</span>
              </h1>
              <p class="opacity-70">
                Nothing is written here yet. Pages linked to but never written
                show up as wanted — the link starts working the moment the page
                exists, with no reindex.
              </p>
              <div class="flex flex-wrap gap-2">
                <A class="btn btn-primary btn-sm" href={`/new?slug=${encodeURIComponent(slug())}`}>
                  Write this page
                </A>
                {/*
                  A wanted page has a neighbourhood too, and it is the whole
                  reason to write one: the pages already reaching for it.
                */}
                <A class="btn btn-sm" href={graphHref(slug())}>
                  What reaches for it
                </A>
              </div>
            </div>
          </div>
        </Match>

        <Match when={page.error}>
          <ErrorNotice error={page.error} />
        </Match>

        <Match when={page()}>
          {(loaded) => (
            <article class="card bg-base-100 shadow">
              <div class="card-body">
                <div class="flex flex-wrap items-start justify-between gap-3">
                  <h1 class="card-title text-2xl">
                    {loaded().title}
                    {/*
                      Only when it is worth saying. An unmarked page is
                      `internal`, which is most of them, and a badge on every
                      page is a badge nobody reads — so the one that means
                      "anyone on the internet" stays legible.
                    */}
                    <VisibilityBadge
                      visibility={loaded().visibility}
                      owner={loaded().owner}
                      readers={loaded().readers}
                    />
                    {/*
                      Beside the visibility rather than in the metadata line
                      below, because it is a fact about the page rather than
                      about its file, and because a chapter you have opened to
                      work on is the place the answer is most wanted. Silent on
                      a page that says nothing, which is most of them: a stage
                      nobody has set is not a fifth stage.
                    */}
                    <Show when={loaded().stage}>
                      {(stage) => <StageBadge stage={stage()} />}
                    </Show>
                  </h1>
                  <div class="flex items-center gap-2">
                    {/*
                      Starting a timer here rather than only on the Time screen:
                      the moment you know what you are working on is the moment
                      you are looking at it.
                    */}
                    <PageTimerButton
                      slug={loaded().slug}
                      title={loaded().title}
                      onChange={() => void refetchLinks()}
                    />
                    <button
                      class="btn btn-sm"
                      classList={{
                        'btn-ghost': !pins.isPinned(loaded().slug),
                        'btn-secondary': pins.isPinned(loaded().slug),
                      }}
                      onClick={() => void togglePin()}
                      disabled={pinning()}
                      title={
                        pins.isPinned(loaded().slug)
                          ? 'Remove this page from the Pins menu'
                          : 'Keep this page in the Pins menu'
                      }
                    >
                      {pins.isPinned(loaded().slug) ? 'Pinned' : 'Pin'}
                    </button>
                    {/*
                      The link panels below say what this page reaches in one
                      hop. This is the same question asked two hops out, where
                      a list stops being readable and a picture starts.
                    */}
                    <A
                      class="btn btn-ghost btn-sm"
                      href={graphHref(loaded().slug)}
                      title="Draw this page's neighbourhood"
                    >
                      Graph
                    </A>
                    {/*
                      Only offered where it means something. Every page compiles,
                      a leaf to itself, so a button on all of them would promise
                      a second reading that is the same reading.
                    */}
                    <Show when={isManuscript(loaded())}>
                      <button
                        class="btn btn-ghost btn-sm"
                        classList={{ 'btn-active': assembled() }}
                        aria-pressed={assembled()}
                        onClick={() => setAssembled(!assembled())}
                        title="Read this page and everything its contents list assembles"
                      >
                        Assembled
                      </button>
                    </Show>
                    <button
                      class="btn btn-ghost btn-sm"
                      onClick={() => setShowSource((shown) => !shown)}
                    >
                      {showSource() ? 'Rendered' : 'Source'}
                    </button>
                    <A class="btn btn-primary btn-sm" href={editHref(loaded().slug)}>
                      Edit
                    </A>
                  </div>
                </div>

                <Show when={pinFailure()}>
                  <ErrorNotice error={pinFailure()} />
                </Show>

                {/*
                  The other reading of the slug, next to the tags because it is
                  the same kind of thing: `/rust` collects every page in a
                  `rust` directory, wherever in the wiki that directory is,
                  which is what the breadcrumb above deliberately will not do.
                  Mono and slash-prefixed so the two kinds of badge do not read
                  as one list.
                */}
                <div class="flex flex-wrap items-center gap-1">
                  <For each={slugSegments(loaded().slug).filter((segment) => !segment.last)}>
                    {(segment) => (
                      <A
                        class="badge badge-ghost font-mono"
                        href={segmentHref(segment.name)}
                        title={`Every page in a ${segment.name} directory, anywhere`}
                      >
                        <span class="opacity-50">/</span>
                        {segment.name}
                      </A>
                    )}
                  </For>
                  <For each={loaded().tags}>
                    {(tag) => (
                      <A class="badge badge-outline" href={tagHref(tag)}>
                        {tag}
                      </A>
                    )}
                  </For>
                </div>

                <div class="text-xs opacity-60">
                  <span class="font-mono">{loaded().slug}</span> · {loaded().size} bytes ·
                  updated {formatDate(loaded().updated)}
                </div>

                {/*
                  The index card, above the prose and set apart from it.
                  A synopsis is addressed to the author from outside the story
                  and the body is addressed to a reader inside it, so the two
                  must not run together: dropped straight above the first
                  paragraph in the same weight, this would read as a standfirst
                  somebody wrote for the page rather than a note about it.
                  Hence the rule down the side and the smaller, quieter type.

                  Shown in all three readings on purpose. It describes the page,
                  which is still the page when the body is the raw source, and
                  on a root it describes the work, which is what `?assembled=1`
                  is showing.

                  Text, never `innerHTML`. A synopsis is page content and page
                  content is what agents write, which is the rule `Snippet.tsx`
                  exists to keep; here it is kept by there being nothing to
                  render, since the field is plain text by definition.
                  `whitespace-pre-line` is what keeps a card written as two
                  paragraphs looking like two.
                */}
                <Show when={loaded().synopsis}>
                  {(synopsis) => (
                    <p class="border-base-300 mt-1 border-l-2 py-0.5 pl-3 text-sm opacity-70 whitespace-pre-line">
                      {synopsis()}
                    </p>
                  )}
                </Show>

                <Show
                  when={!showSource()}
                  fallback={
                    <pre class="mt-2 overflow-auto rounded bg-base-200 p-4 text-sm whitespace-pre-wrap">
                      {loaded().content}
                    </pre>
                  }
                >
                  {/*
                    The assembled document replaces the body rather than sitting
                    beside it: it is the same prose with the chapters after it,
                    and showing both would print the first one twice.
                  */}
                  <Show
                    when={assembled()}
                    fallback={
                      <Markdown
                        class="prose dark:prose-invert mt-2 max-w-none"
                        html={loaded().html ?? ''}
                      />
                    }
                  >
                    <Async resource={document}>
                      {(compiled) => (
                        <>
                          <div class="mt-2 text-xs opacity-60">
                            {compiled.sections.length}{' '}
                            {compiled.sections.length === 1 ? 'section' : 'sections'} ·{' '}
                            {compiled.words.toLocaleString()} words ·{' '}
                            <span class="font-mono">{compiled.compiler}</span>
                          </div>
                          {/*
                            Rendered by the server after assembly, so comrak sees
                            one document and the shifted heading levels nest the
                            way the manifest says they do.
                          */}
                          <Markdown
                            class="prose dark:prose-invert mt-2 max-w-none"
                            html={compiled.content ?? ''}
                          />
                        </>
                      )}
                    </Async>
                  </Show>
                </Show>
              </div>
            </article>
          )}
        </Match>
      </Switch>

      {/*
        Under the page and above the links, because it is the page's own
        structure rather than its neighbourhood. Only on a page that has some:
        this panel is the only place the spine is rendered as something you can
        click, and on an ordinary page there is no spine to render.
      */}
      <Show when={isManuscript(page())}>
        {(loaded) => <Manuscript page={loaded()} />}
      </Show>

      <Async resource={links}>{(data) => <LinkPanels links={data} />}</Async>
    </div>
  )
}

/**
 * Whether this page is a manuscript, in the sense the plan gives the word.
 *
 * Three things make one, and each on its own is enough. A `contents:` list makes
 * a page assemble others, **including an empty one**, which is what a book looks
 * like on the day it is started and is why this checks for the field rather than
 * for entries in it. A `target` makes a page something you are working toward,
 * measured against its compiled total. A `due` makes it something with a day
 * attached.
 *
 * A leaf page carrying only a `target` is a manuscript of one section, which is
 * the recursive definition doing what it promised rather than a special case.
 */
function isManuscript(page: PageView | undefined): PageView | undefined {
  if (!page) return undefined
  const carries =
    page.contents !== undefined || page.target !== undefined || page.due !== undefined
  return carries ? page : undefined
}

/**
 * Both directions of the page's links.
 *
 * They come back from one endpoint because this is how they are read: what a
 * page points at is only half of where it sits in the wiki.
 *
 * Each side is collapsed to one row per page. The graph legitimately holds two
 * edges when something is linked both as `[[a]]` and as `[a](a.md)`, and the
 * API is right to report both — but rendering the same page twice, under the
 * same title, reads as a bug. The kinds are kept as badges so nothing is lost.
 */
function LinkPanels(props: { links: PageLinksResponse }) {
  const outbound = () => collapse(props.links.outbound, (link) => link.target)
  const inbound = () => collapse(props.links.inbound, (link) => link.slug)

  return (
    <>
    <TimePanel links={props.links} />
    <div class="grid gap-4 sm:grid-cols-2">
      <section class="card bg-base-100 shadow">
        <div class="card-body">
          <h2 class="card-title text-base">
            Links out
            <span class="badge badge-ghost badge-sm">{outbound().length}</span>
          </h2>
          <ul class="flex flex-col gap-2 text-sm">
            <For
              each={outbound()}
              fallback={<li class="opacity-60">This page links nowhere.</li>}
            >
              {({ first: link, kinds }) => (
                <li class="flex flex-wrap items-center gap-2">
                  <Show
                    when={link.kind !== 'external'}
                    fallback={
                      <a
                        class="link break-all"
                        href={link.target}
                        target="_blank"
                        rel="noopener noreferrer"
                      >
                        {link.display ?? link.target}
                      </a>
                    }
                  >
                    <A class="link" href={pageHref(link.target)}>
                      {link.title ?? link.target}
                    </A>
                    <Show when={!link.resolved}>
                      <span class="badge badge-warning badge-xs">wanted</span>
                    </Show>
                  </Show>
                  <For each={kinds}>
                    {(kind) => <span class="badge badge-ghost badge-xs">{kind}</span>}
                  </For>
                </li>
              )}
            </For>
          </ul>
        </div>
      </section>

      <section class="card bg-base-100 shadow">
        <div class="card-body">
          <h2 class="card-title text-base">
            Backlinks
            <span class="badge badge-ghost badge-sm">{inbound().length}</span>
          </h2>
          <ul class="flex flex-col gap-2 text-sm">
            <For
              each={inbound()}
              fallback={<li class="opacity-60">Nothing links here — this page is an orphan.</li>}
            >
              {({ first: link }) => (
                <li>
                  <A class="link" href={pageHref(link.slug)}>
                    {link.title}
                  </A>
                  <SlugPath class="block font-mono text-xs opacity-60" slug={link.slug} />
                </li>
              )}
            </For>
          </ul>
        </div>
      </section>
    </div>
    </>
  )
}

/**
 * The time tracked against this page.
 *
 * Not a third link panel, and deliberately shaped nothing like one. A page you
 * actually work on collects a time entry every time a timer starts, so this
 * side of the graph is counted in hundreds where the other two are counted in
 * ones — which is exactly why the API keeps them in separate tables. Rendered
 * as backlinks they would bury the backlinks; rendered as a total with a few
 * recent entries under it, they say the thing worth knowing in one line.
 *
 * Absent entirely when nothing has been tracked, so a wiki nobody times looks
 * exactly as it did before.
 */
function TimePanel(props: { links: PageLinksResponse }) {
  const times = () => props.links.times

  return (
    <Show when={times().entries > 0}>
      <section class="card bg-base-100 shadow">
        <div class="card-body gap-3">
          <div class="flex flex-wrap items-baseline justify-between gap-2">
            <h2 class="card-title text-base">
              Time on this page
              <Show when={times().running > 0}>
                <span class="badge badge-secondary badge-sm">
                  {times().running} running
                </span>
              </Show>
            </h2>
            <div class="text-sm">
              <span class="font-mono text-lg">{formatDuration(times().seconds)}</span>
              <span class="opacity-60">
                {' '}
                across {times().entries}{' '}
                {times().entries === 1 ? 'entry' : 'entries'} in {times().groups}{' '}
                {times().groups === 1 ? 'group' : 'groups'}
              </span>
            </div>
          </div>

          <ul class="flex flex-col gap-1 text-sm">
            <For each={times().recent}>
              {(entry) => (
                <li class="flex items-baseline justify-between gap-2">
                  <A class="link min-w-0 truncate" href={groupHref(entry.name)}>
                    {entry.name}
                  </A>
                  <span class="flex items-baseline gap-3 whitespace-nowrap">
                    <span class="text-xs opacity-60">{formatDate(entry.start)}</span>
                    <Duration
                      class="font-mono text-xs"
                      seconds={entry.seconds}
                      runningSince={entry.end ? null : entry.start}
                    />
                  </span>
                </li>
              )}
            </For>
          </ul>

          <Show when={times().entries > times().recent.length}>
            <A class="link text-xs" href={pageTimesHref(props.links.slug)}>
              All {times().entries} entries
            </A>
          </Show>
        </div>
      </section>
    </Show>
  )
}

/**
 * One entry per distinct key, keeping every `kind` that reached it.
 *
 * Exported for its tests. It is the whole of the fix for a page appearing twice
 * in a link panel, and the ordering it promises is load-bearing for reading.
 */
export function collapse<T extends { kind: string }>(
  links: T[],
  key: (link: T) => string,
): { first: T; kinds: string[] }[] {
  const grouped = new Map<string, { first: T; kinds: string[] }>()

  for (const link of links) {
    const existing = grouped.get(key(link))
    if (existing) {
      if (!existing.kinds.includes(link.kind)) existing.kinds.push(link.kind)
    } else {
      grouped.set(key(link), { first: link, kinds: [link.kind] })
    }
  }

  return [...grouped.values()]
}

/** Timestamps arrive as RFC 3339; show them in the reader's locale. */
export function formatDate(value: string): string {
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString()
}
