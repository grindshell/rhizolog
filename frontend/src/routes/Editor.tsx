import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js'
import { useBeforeLeave, useNavigate, useParams, useSearchParams } from '@solidjs/router'
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
import { ErrorNotice } from '../components/Async'
import Markdown from '../components/Markdown'

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
      }

      const target = editing()
      const page = target
        ? await replacePage(target, body)
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

            <label class="form-control">
              <div class="label">
                <span class="label-text">Body</span>
                <span class="label-text-alt opacity-60">
                  markdown, <code>[[wikilinks]]</code> included
                </span>
              </div>
              <textarea
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

/** Split a comma-separated field into tags, dropping blanks and duplicates. */
function parseTags(raw: string): string[] {
  const tags = new Set<string>()
  for (const tag of raw.split(',')) {
    const trimmed = tag.trim()
    if (trimmed) tags.add(trimmed)
  }
  return [...tags]
}

function firstParam(value: string | string[] | undefined): string | undefined {
  return Array.isArray(value) ? value[0] : value
}
