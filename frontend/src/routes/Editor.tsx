import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js'
import { useBeforeLeave, useNavigate, useParams, useSearchParams } from '@solidjs/router'
import type { Visibility } from '../api/client'
import {
  ApiError,
  createPage,
  decodeSlug,
  deletePage,
  editHref,
  getPage,
  movePage,
  pageHref,
  renderMarkdown,
  replacePage,
} from '../api/client'
import { sessionState } from '../api/session'
import { ErrorNotice } from '../components/Async'
import Findings, { byteToIndex } from '../components/Findings'
import Markdown from '../components/Markdown'

/**
 * The visibility ladder, in the order it narrows.
 *
 * The hints matter more than the labels. "Public" is the one somebody can pick
 * meaning "everyone here" and get "everyone at all", so it says which it is —
 * and says the second half too, since a public page is only reachable by a
 * stranger on an instance that has opted into serving them.
 */
const VISIBILITIES: { value: Visibility; label: string; hint: string }[] = [
  {
    value: 'public',
    label: 'Public — anyone, signed in or not',
    hint: 'Readable without an account, if this instance serves anonymous readers. Otherwise the same as internal.',
  },
  {
    value: 'internal',
    label: 'Internal — any account on this wiki',
    hint: 'The default. Every page with no visibility set means this.',
  },
  {
    value: 'restricted',
    label: 'Restricted — named accounts only',
    hint: 'The accounts you list below, plus the owner.',
  },
  {
    value: 'private',
    label: 'Private — only me',
    hint: 'The owner alone. Not even an owner of this instance can read it.',
  },
]

/** How long typing has to pause before the preview is re-rendered. */
const PREVIEW_DELAY_MS = 300

/** The three ways the editor can divide its space. */
export type Layout = 'editor' | 'split' | 'preview'

const LAYOUTS: { value: Layout; label: string; title: string }[] = [
  { value: 'editor', label: 'Editor', title: 'Collapse the preview' },
  { value: 'split', label: 'Split', title: 'Show the editor and the preview' },
  { value: 'preview', label: 'Preview', title: 'Maximise the preview' },
]

const LAYOUT_KEY = 'rhizolog:editor-layout'

/**
 * The layout the user last chose.
 *
 * Remembered because it is a working preference, not a property of the page:
 * somebody who collapsed the preview to write does not want it back on the next
 * page they open.
 */
function storedLayout(): Layout {
  try {
    const stored = window.localStorage.getItem(LAYOUT_KEY)
    if (LAYOUTS.some((option) => option.value === stored)) return stored as Layout
  } catch {
    // Storage can be unavailable — private mode, a blocked origin. Remembering
    // a pane arrangement is not worth failing the editor over.
  }
  return 'split'
}

function rememberLayout(layout: Layout) {
  try {
    window.localStorage.setItem(LAYOUT_KEY, layout)
  } catch {
    // As above.
  }
}

/**
 * Write a page.
 *
 * One component serves two routes: `/new` creates, `/edit/*slug` edits. They
 * differ only in whether there is a page to load first and whether the slug is
 * still up for grabs, which is not enough to justify two of everything.
 *
 * The preview is rendered by the server, on a debounce. That is the whole
 * reason the MVP can get away with a plain `textarea` and no editor library:
 * the expensive, opinionated part of a markdown editor is the rendering, and
 * the server already does it — correctly, including wikilinks, which a
 * client-side renderer would have no way to know about.
 */
export default function Editor() {
  const params = useParams<{ slug?: string }>()
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()

  /**
   * The page being edited, or `undefined` when creating one.
   *
   * `@solidjs/router` hands back the raw path, so this decodes — see the note
   * on `decodeSlug`.
   */
  const editing = (): string | undefined =>
    params.slug ? decodeSlug(params.slug) : undefined

  const [slug, setSlug] = createSignal(firstParam(searchParams.slug) ?? '')
  const [title, setTitle] = createSignal('')
  const [tags, setTags] = createSignal('')
  const [content, setContent] = createSignal('')
  const [visibility, setVisibility] = createSignal<Visibility>('internal')
  const [readers, setReaders] = createSignal('')
  /**
   * The page's owner, carried through a save rather than edited.
   *
   * Saving goes through `PUT`, which replaces every field — so a page loaded
   * and saved without this would come back owned by whoever pressed the button.
   * That is right for a page being created and wrong for one being edited by
   * somebody the owner shared it with.
   */
  const [owner, setOwner] = createSignal<string | undefined>()
  /**
   * The three manuscript fields, and they are here whether or not the page is
   * one.
   *
   * Saving is a `PUT`, so a field left out is a field cleared, and an editor
   * that dropped these would unmake a book on the first save of any page in it.
   * That is the same failure that once handed pages to the wrong owner, which is
   * why `owner` above is carried the same way.
   *
   * `contents` needs two pieces of state rather than one, because absent and
   * empty are different values and the API keeps them apart: **absent** is an
   * ordinary page, **`[]`** is a manuscript with nothing in it yet, which is
   * what a book looks like on the day it is started. A textarea alone can only
   * say one of those.
   */
  const [target, setTarget] = createSignal('')
  const [due, setDue] = createSignal('')
  const [assembles, setAssembles] = createSignal(false)
  const [contents, setContents] = createSignal('')
  /** What the server called the page when it was loaded, for the placeholder. */
  const [inheritedTitle, setInheritedTitle] = createSignal('')
  const [dirty, setDirty] = createSignal(false)
  const [busy, setBusy] = createSignal(false)
  const [failure, setFailure] = createSignal<unknown>()

  const [loaded] = createResource(editing, (target) => getPage(target))

  createEffect(() => {
    const page = loaded()
    if (!page) return

    setSlug(page.slug)
    // A derived title is deliberately left out of the field. Putting it there
    // and saving would write it into the frontmatter, and the heading it came
    // from would never update the title again — see `title_derived` in the API.
    setTitle(page.title_derived ? '' : page.title)
    setInheritedTitle(page.title)
    setTags(page.tags.join(', '))
    setContent(page.content)
    setVisibility(page.visibility)
    setReaders((page.readers ?? []).join(', '))
    setOwner(page.owner ?? undefined)
    setTarget(page.target === undefined || page.target === null ? '' : String(page.target))
    // The first ten characters of an RFC 3339 timestamp in UTC, which is the
    // day. A due date is a day rather than an instant, and reading it in the
    // browser's zone would show the day before to anybody west of Greenwich.
    setDue(page.due ? page.due.slice(0, 10) : '')
    setAssembles(page.contents !== undefined && page.contents !== null)
    setContents((page.contents ?? []).join('\n'))
    setDirty(false)
    setFailure(undefined)
  })

  /* -------------------------------------------------------------- layout -- */

  const [layout, setLayoutSignal] = createSignal<Layout>(storedLayout())
  const setLayout = (next: Layout) => {
    setLayoutSignal(next)
    rememberLayout(next)
  }

  const showEditor = () => layout() !== 'preview'
  const showPreview = () => layout() !== 'editor'

  /* ------------------------------------------------------------- preview -- */

  /**
   * What the preview is showing, or `null` when there is nothing to show.
   *
   * Solid skips the fetcher for a null source, so `null` is what makes a
   * collapsed preview cost no `POST /api/render` at all — not one per pause in
   * typing, and not the one at mount either. Requests for a pane nobody is
   * looking at are not free here: this wiki counts its own API usage and puts
   * the numbers on its dashboard.
   */
  const [previewOf, setPreviewOf] = createSignal<Draft | null>(null)

  /**
   * Whether the preview is coming back rather than keeping up.
   *
   * A plain latch, not state anything renders: it only decides whether the next
   * draft waits for the debounce.
   */
  let previewWasHidden = true

  createEffect(() => {
    // Returning before `content` is read is what stops a collapsed preview
    // debouncing at all: the effect then depends only on the layout.
    if (!showPreview()) {
      previewWasHidden = true
      setPreviewOf(null)
      return
    }

    const draft: Draft = { content: content(), slug: slug().trim() || undefined }

    // Re-opening skips the debounce. The debounce waits for a pause in typing,
    // and nobody is typing — they clicked.
    if (previewWasHidden) {
      previewWasHidden = false
      setPreviewOf(draft)
      return
    }

    const timer = setTimeout(() => setPreviewOf(draft), PREVIEW_DELAY_MS)
    onCleanup(() => clearTimeout(timer))
  })

  // The source is the draft signal alone, deliberately — not
  // `showPreview() && previewOf()`. Solid settles pure computations before user
  // effects, so a source that read the layout directly would see it flip to
  // visible while `previewOf` still held the draft from before the pane was
  // collapsed, and fetch that. The effect above owns the transition instead, so
  // there is one write and one render, with the right content.
  const [preview] = createResource(previewOf, render)

  /* ------------------------------------------------------------ findings -- */

  let body: HTMLTextAreaElement | undefined

  /**
   * Put the cursor on what a rule fired on.
   *
   * This is the whole reason spans are byte offsets into the page **source**
   * rather than positions in rendered HTML: a finding you cannot find is not a
   * finding. Bytes are not JavaScript string indices, so they go through
   * `byteToIndex` on the way; handing them over raw would select the wrong words
   * on any page with an accent in it, and further out the longer the page ran.
   */
  const reveal = (finding: { span: { start: number; end: number } }) => {
    if (!body) return
    const text = content()
    body.focus()
    body.setSelectionRange(
      byteToIndex(text, finding.span.start),
      byteToIndex(text, finding.span.end),
    )
  }

  /* ------------------------------------------------------------- actions -- */

  const save = async () => {
    if (busy()) return
    setBusy(true)
    setFailure(undefined)

    try {
      const body = {
        // Empty means "derive it", which is a null title rather than an empty
        // one. The API distinguishes the two.
        title: title().trim() || null,
        tags: parseTags(tags()),
        content: content(),
        visibility: visibility(),
        // Sent every time because saving is a `PUT`: a field left out is a
        // field cleared, and clearing the owner of a page somebody shared with
        // you would take it away from them.
        owner: owner(),
        readers: parseList(readers()),
        // The same rule, for the same reason. `null` rather than omitted so it
        // is a request rather than a silence, which is also what makes clearing
        // one of these possible at all.
        target: parseTarget(target()),
        due: parseDue(due()),
        contents: assembles() ? parseLines(contents()) : null,
      }

      // `existing` rather than `target`, which now names a word count.
      const existing = editing()
      const page = existing
        ? await replacePage(existing, body)
        : await createPage({ slug: slug().trim(), ...body })

      // Cleared before navigating, or the guard below would ask to discard the
      // changes that were just saved.
      setDirty(false)
      navigate(pageHref(page.slug))
    } catch (error) {
      setFailure(error)
    } finally {
      setBusy(false)
    }
  }

  const remove = async () => {
    const target = editing()
    if (!target || busy()) return
    if (!window.confirm(`Delete ${target}? Its file is removed from the wiki.`)) return

    setBusy(true)
    setFailure(undefined)
    try {
      await deletePage(target)
      setDirty(false)
      navigate('/pages')
    } catch (error) {
      setFailure(error)
    } finally {
      setBusy(false)
    }
  }

  /** True when the slug field has been pointed somewhere else. */
  const renaming = () => {
    const target = editing()
    return target !== undefined && slug().trim() !== '' && slug().trim() !== target
  }

  const rename = async () => {
    const target = editing()
    if (!target || busy()) return

    setBusy(true)
    setFailure(undefined)
    try {
      const page = await movePage({ from: target, to: slug().trim() })
      setDirty(false)
      // Replace rather than push: the old slug is gone, so leaving it in the
      // history would put a 404 behind the back button.
      navigate(editHref(page.slug), { replace: true })
    } catch (error) {
      setFailure(error)
    } finally {
      setBusy(false)
    }
  }

  /* --------------------------------------------------------------- guards -- */

  useBeforeLeave((event) => {
    if (!dirty() || event.defaultPrevented) return
    event.preventDefault()
    if (window.confirm('Discard unsaved changes?')) event.retry(true)
  })

  const onBeforeUnload = (event: BeforeUnloadEvent) => {
    if (!dirty()) return
    // Closing the tab is the one exit `useBeforeLeave` cannot see.
    event.preventDefault()
    event.returnValue = ''
  }

  const onKeyDown = (event: KeyboardEvent) => {
    if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 's') return
    event.preventDefault()
    void save()
  }

  window.addEventListener('beforeunload', onBeforeUnload)
  window.addEventListener('keydown', onKeyDown)
  onCleanup(() => {
    window.removeEventListener('beforeunload', onBeforeUnload)
    window.removeEventListener('keydown', onKeyDown)
  })

  /* ------------------------------------------------------------------ ui -- */

  const canSave = () => !busy() && (editing() !== undefined || slug().trim() !== '')

  /** Whether this page carries any of the three, and so is worth opening. */
  const isManuscript = () => assembles() || target().trim() !== '' || due() !== ''

  return (
    <div class="flex flex-col gap-4">
      <header class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 class="text-2xl font-semibold">
            {editing() ? 'Edit page' : 'New page'}
          </h1>
          <Show when={dirty()}>
            <span class="text-sm opacity-60">Unsaved changes</span>
          </Show>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          {/*
            Three states rather than one collapse toggle: "give the editor the
            room" and "give the preview the room" are both things you want, and
            a single button that cycled through them would make you guess which
            way it goes.
          */}
          <div class="join" role="group" aria-label="Editor layout">
            <For each={LAYOUTS}>
              {(option) => (
                <button
                  class="btn join-item btn-sm"
                  classList={{ 'btn-active': layout() === option.value }}
                  aria-pressed={layout() === option.value}
                  title={option.title}
                  onClick={() => setLayout(option.value)}
                >
                  {option.label}
                </button>
              )}
            </For>
          </div>

          <Show when={editing()}>
            {(target) => (
              <>
                <a class="btn btn-ghost btn-sm" href={pageHref(target())}>
                  View
                </a>
                <button
                  class="btn btn-error btn-outline btn-sm"
                  onClick={() => void remove()}
                  disabled={busy()}
                >
                  Delete
                </button>
              </>
            )}
          </Show>
          <button
            class="btn btn-primary btn-sm"
            onClick={() => void save()}
            disabled={!canSave()}
          >
            <Show when={busy()}>
              <span class="loading loading-spinner loading-xs" />
            </Show>
            Save
          </button>
        </div>
      </header>

      <Show when={loaded.error}>
        <ErrorNotice error={loaded.error} />
      </Show>
      <Show when={failure()}>
        <ErrorNotice error={failure()} />
      </Show>

      <div class="grid gap-4" classList={{ 'lg:grid-cols-2': layout() === 'split' }}>
        <Show when={showEditor()}>
        <section class="card bg-base-100 shadow">
          <div class="card-body gap-3">
            <label class="form-control">
              <div class="label">
                <span class="label-text">Slug</span>
                <span class="label-text-alt opacity-60">
                  {editing() ? 'change it to rename' : 'e.g. notes/rust/async'}
                </span>
              </div>
              <div class="join">
                <input
                  class="input input-bordered join-item w-full font-mono"
                  value={slug()}
                  placeholder="notes/rust/async"
                  // Deliberately does not mark the form dirty. Renaming is
                  // disabled while there are unsaved edits, so if typing a new
                  // slug counted as one, the Rename button would disable itself
                  // the moment it appeared and could never be clicked.
                  onInput={(event) => setSlug(event.currentTarget.value)}
                />
                <Show when={renaming()}>
                  <button
                    class="btn btn-outline join-item"
                    onClick={() => void rename()}
                    // A move operates on the file, not on what is in the
                    // textarea, so renaming with edits pending would drop them.
                    disabled={busy() || dirty()}
                    title={dirty() ? 'Save your changes first' : 'Move this page'}
                  >
                    Rename
                  </button>
                </Show>
              </div>
            </label>

            <label class="form-control">
              <div class="label">
                <span class="label-text">Title</span>
                <span class="label-text-alt opacity-60">
                  empty follows the first heading
                </span>
              </div>
              <input
                class="input input-bordered w-full"
                value={title()}
                placeholder={inheritedTitle() || 'Derived from the body'}
                onInput={(event) => {
                  setTitle(event.currentTarget.value)
                  setDirty(true)
                }}
              />
            </label>

            <label class="form-control">
              <div class="label">
                <span class="label-text">Tags</span>
                <span class="label-text-alt opacity-60">comma separated</span>
              </div>
              <input
                class="input input-bordered w-full"
                value={tags()}
                placeholder="theory, deleuze"
                onInput={(event) => {
                  setTags(event.currentTarget.value)
                  setDirty(true)
                }}
              />
            </label>

            {/*
              Only shown on a wiki that has accounts. On one that does not there
              is nobody to keep a page from, the field does nothing, and a
              control that does nothing is worse than no control.
            */}
            <Show when={sessionState()?.authentication_required}>
              <label class="form-control">
                <div class="label">
                  <span class="label-text">Who can read this</span>
                </div>
                <select
                  class="select select-bordered w-full"
                  value={visibility()}
                  onChange={(event) => {
                    setVisibility(event.currentTarget.value as Visibility)
                    setDirty(true)
                  }}
                >
                  <For each={VISIBILITIES}>
                    {(option) => <option value={option.value}>{option.label}</option>}
                  </For>
                </select>
                <div class="label">
                  <span class="label-text-alt opacity-60">
                    {VISIBILITIES.find((option) => option.value === visibility())?.hint}
                  </span>
                </div>
              </label>

              <Show when={visibility() === 'restricted'}>
                <label class="form-control">
                  <div class="label">
                    <span class="label-text">Readers</span>
                    <span class="label-text-alt opacity-60">
                      account names, comma separated
                    </span>
                  </div>
                  <input
                    class="input input-bordered w-full"
                    value={readers()}
                    placeholder="alice, bob"
                    onInput={(event) => {
                      setReaders(event.currentTarget.value)
                      setDirty(true)
                    }}
                  />
                  <div class="label">
                    <span class="label-text-alt opacity-60">
                      The owner{owner() ? ` (${owner()})` : ''} can always read it.
                    </span>
                  </div>
                </label>
              </Show>
            </Show>

            {/*
              Collapsed by default, because most pages are not manuscripts and a
              form that asks every page for a word target is a form that reads as
              a project tracker. The three fields are round-tripped whether or
              not this is ever opened; see the signals above.
            */}
            <details class="collapse-arrow border-base-300 collapse border" open={isManuscript()}>
              <summary class="collapse-title px-4 py-2 text-sm font-medium">
                Manuscript
                <Show when={isManuscript()}>
                  <span class="badge badge-ghost badge-sm ml-2">
                    {assembles() ? `${parseLines(contents()).length} parts` : 'target'}
                  </span>
                </Show>
              </summary>
              <div class="collapse-content flex flex-col gap-3">
                <div class="grid gap-3 sm:grid-cols-2">
                  <label class="form-control">
                    <div class="label">
                      <span class="label-text">Target</span>
                      <span class="label-text-alt opacity-60">words</span>
                    </div>
                    <input
                      class="input input-bordered w-full"
                      type="number"
                      min="0"
                      value={target()}
                      placeholder="90000"
                      onInput={(event) => {
                        setTarget(event.currentTarget.value)
                        setDirty(true)
                      }}
                    />
                    <div class="label">
                      <span class="label-text-alt opacity-60">
                        Measured against the compiled total, so on a page that
                        assembles others it is the whole book.
                      </span>
                    </div>
                  </label>

                  <label class="form-control">
                    <div class="label">
                      <span class="label-text">Due</span>
                      <span class="label-text-alt opacity-60">a day, not a time</span>
                    </div>
                    <input
                      class="input input-bordered w-full"
                      type="date"
                      value={due()}
                      onInput={(event) => {
                        setDue(event.currentTarget.value)
                        setDirty(true)
                      }}
                    />
                  </label>
                </div>

                <label class="label cursor-pointer justify-start gap-3">
                  <input
                    type="checkbox"
                    class="checkbox checkbox-sm"
                    checked={assembles()}
                    onChange={(event) => {
                      setAssembles(event.currentTarget.checked)
                      setDirty(true)
                    }}
                  />
                  <span class="label-text">This page assembles others</span>
                </label>

                <Show when={assembles()}>
                  <label class="form-control">
                    <div class="label">
                      <span class="label-text">Contents</span>
                      <span class="label-text-alt opacity-60">one slug per line, in order</span>
                    </div>
                    <textarea
                      class="textarea textarea-bordered h-32 w-full resize-y font-mono text-sm"
                      value={contents()}
                      placeholder="book/one/opening&#10;book/one/the-ferry&#10;book/two"
                      onInput={(event) => {
                        setContents(event.currentTarget.value)
                        setDirty(true)
                      }}
                    />
                    <div class="label">
                      <span class="label-text-alt opacity-60">
                        Slugs from the wiki root, never relative. A chapter
                        nobody has written yet is a gap in the manuscript rather
                        than an error, and it fills itself in when the page
                        appears.
                      </span>
                    </div>
                  </label>
                </Show>
              </div>
            </details>

            <label class="form-control">
              <div class="label">
                <span class="label-text">Body</span>
                <span class="label-text-alt opacity-60">
                  markdown, <code>[[wikilinks]]</code> included
                </span>
              </div>
              <textarea
                ref={body}
                class="textarea textarea-bordered editor-pane w-full resize-y font-mono text-sm"
                classList={{ 'editor-pane-solo': layout() === 'editor' }}
                value={content()}
                placeholder="# Heading&#10;&#10;Link to another page with [[notes/rhizome]]."
                onInput={(event) => {
                  setContent(event.currentTarget.value)
                  setDirty(true)
                }}
              />
            </label>

            {/*
              Under the textarea rather than beside the preview, because it is
              about the text you are typing and not about what it will look
              like. It runs your own rules; there is nothing here from a model.
            */}
            <Findings content={content()} onReveal={reveal} />
          </div>
        </section>
        </Show>

        <Show when={showPreview()}>
        <section class="card bg-base-100 shadow">
          <div class="card-body">
            <h2 class="card-title text-base">
              Preview
              <Show when={preview.loading}>
                <span class="loading loading-spinner loading-xs" />
              </Show>
            </h2>
            <Show
              when={!preview.error}
              fallback={<ErrorNotice error={preview.error} />}
            >
              {/*
                `preview.latest` rather than `preview()`: keeping the last good
                render on screen while the next one is in flight stops the pane
                blanking on every pause in typing.
              */}
              <Markdown
                class={
                  'prose prose-sm dark:prose-invert editor-pane max-w-none' +
                  (layout() === 'preview' ? ' editor-pane-solo' : '')
                }
                html={preview.latest?.html ?? ''}
              />
            </Show>
          </div>
        </section>
        </Show>
      </div>
    </div>
  )
}

interface Draft {
  content: string
  slug: string | undefined
}

/**
 * Render a draft, tolerating a slug that is still being typed.
 *
 * Half of `notes/rust/` is not a valid slug, and the preview should not turn
 * into an error box because of a keystroke. Dropping the slug only changes how
 * relative markdown links resolve; wikilinks are absolute and unaffected.
 */
async function render(draft: Draft) {
  try {
    return await renderMarkdown(draft)
  } catch (error) {
    if (draft.slug && error instanceof ApiError && error.code === 'invalid_request_body') {
      return await renderMarkdown({ content: draft.content })
    }
    throw error
  }
}

/**
 * A word target, or `null` to clear it.
 *
 * `null` rather than `undefined`, because a field left out of a `PUT` and a
 * field set to null mean the same thing to the API and only one of them says so.
 * Anything that is not a whole number is sent as no target: a negative target is
 * not a small one, and the backend refuses it either way.
 */
export function parseTarget(raw: string): number | null {
  const trimmed = raw.trim()
  if (trimmed === '') return null

  const value = Number(trimmed)
  return Number.isInteger(value) && value >= 0 ? value : null
}

/**
 * A day, as the instant the API wants.
 *
 * `<input type="date">` gives `YYYY-MM-DD` and the API takes a full timestamp,
 * because this is JSON and a client has a clock. Midnight **UTC** is what a bare
 * date in a file means, so that is what this sends: reading the day back in the
 * browser's own zone and re-encoding it would move a due date by a day for most
 * of the world.
 */
export function parseDue(raw: string): string | null {
  return raw ? `${raw}T00:00:00Z` : null
}

/** Split a field written one entry per line, dropping blanks. */
export function parseLines(raw: string): string[] {
  return (
    raw
      .split('\n')
      .map((line) => line.trim())
      // Duplicates are **kept**, unlike a tag list. A contents list is
      // positions, and an appendix listed under two parts is a real thing that
      // the manifest reports as a `duplicate` in its second position rather
      // than an error.
      .filter((line) => line !== '')
  )
}

/** Split a comma-separated field, dropping blanks and duplicates. */
function parseList(raw: string): string[] {
  const values = new Set<string>()
  for (const value of raw.split(',')) {
    const trimmed = value.trim()
    if (trimmed) values.add(trimmed)
  }
  return [...values]
}

/** Tags, which are the same shape as a reader list and always have been. */
const parseTags = parseList

function firstParam(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
