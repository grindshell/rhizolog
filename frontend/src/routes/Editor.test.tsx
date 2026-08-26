import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Route, Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { PageView } from '../api/client'

const api = vi.hoisted(() => ({
  getPage: vi.fn(),
  replacePage: vi.fn(),
  createPage: vi.fn(),
  renderMarkdown: vi.fn(),
  // The editor reads the session to decide whether to show the visibility
  // control. Left real it would fire a `fetch` at module load, which in jsdom
  // has no origin to resolve `/api/auth/session` against — so every test in
  // this file would fail on a request none of them are about.
  session: vi.fn(async () => ({
    authentication_required: false,
    authenticated: true,
    user: null,
  })),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  // Only the calls are replaced. `encodeSlug`, `pageHref` and `ApiError` stay
  // real, because a test that mocked those would stop testing anything.
  return { ...actual, ...api }
})

const { default: Editor } = await import('./Editor')

function page(overrides: Partial<PageView> = {}): PageView {
  return {
    slug: 'notes/rust/async',
    title: 'Async in Rust',
    title_derived: true,
    tags: ['rust', 'async'],
    created: '2026-08-05T14:00:00Z',
    updated: '2026-08-05T14:00:00Z',
    size: 312,
    words: 8,
    content: '# Async in Rust\n\nFutures are lazy.\n',
    // What an unmarked page means, which is what every page in this file is.
    // The visibility control only appears on a wiki that has accounts, and
    // these tests run against one that does not.
    visibility: 'internal',
    ...overrides,
  }
}

/** The form's fields, in the order the editor lays them out. */
function fields(container: HTMLElement) {
  const inputs = container.querySelectorAll('input')
  return {
    slug: inputs[0] as HTMLInputElement,
    title: inputs[1] as HTMLInputElement,
    tags: inputs[2] as HTMLInputElement,
    body: container.querySelector('textarea') as HTMLTextAreaElement,
  }
}

function type(field: HTMLInputElement | HTMLTextAreaElement, value: string) {
  field.value = value
  field.dispatchEvent(new Event('input', { bubbles: true }))
}

function openEditor(path: string) {
  window.history.pushState({}, '', path)
  return render(() => (
    <Router>
      <Route path="/edit/*slug" component={Editor} />
      <Route path="/new" component={Editor} />
      {/* Where a successful save navigates to. */}
      <Route path="*" component={() => <div data-testid="elsewhere" />} />
    </Router>
  ))
}

beforeEach(() => {
  api.renderMarkdown.mockResolvedValue({ html: '' })
  api.replacePage.mockResolvedValue(page())
  api.createPage.mockResolvedValue(page())
  api.getPage.mockResolvedValue(page())
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  // The layout is remembered across mounts, so a case that changed it would
  // otherwise decide what the next case starts with.
  window.localStorage.clear()
})

describe('loading a page', () => {
  /**
   * The behaviour that keeps read-modify-write from being lossy.
   *
   * A page with no frontmatter title takes one from its first heading. Putting
   * that derived value in the field and saving would write it into the
   * frontmatter, and the heading would never update the title again — so the
   * field stays empty and shows the derived title as a placeholder instead.
   */
  it('leaves the title field empty when the title is derived', async () => {
    const { container } = openEditor('/edit/notes/rust/async')

    await waitFor(() => expect(fields(container).slug.value).toBe('notes/rust/async'))

    const { title } = fields(container)
    expect(title.value).toBe('')
    expect(title.placeholder).toBe('Async in Rust')
  })

  it('fills the title field when the title is stored', async () => {
    api.getPage.mockResolvedValue(page({ title: 'Pinning', title_derived: false }))

    const { container } = openEditor('/edit/notes/rust/pinning')

    await waitFor(() => expect(fields(container).title.value).toBe('Pinning'))
  })

  it('loads the tags and the body', async () => {
    const { container } = openEditor('/edit/notes/rust/async')

    await waitFor(() => expect(fields(container).tags.value).toBe('rust, async'))
    expect(fields(container).body.value).toBe('# Async in Rust\n\nFutures are lazy.\n')
  })

  /** `@solidjs/router` does not decode path params; the editor has to. */
  it('decodes the slug before asking for the page', async () => {
    openEditor('/edit/notes/my%20page')

    await waitFor(() => expect(api.getPage).toHaveBeenCalled())
    expect(api.getPage.mock.calls[0]?.[0]).toBe('notes/my page')
  })
})

describe('saving', () => {
  /**
   * The other half of the derived-title fix: an empty field has to travel as
   * `null`, not as `""`. The API distinguishes them — one means "derive it",
   * the other would store an empty title.
   */
  it('sends a null title when the field is empty', async () => {
    const { container, getByText } = openEditor('/edit/notes/rust/async')
    await waitFor(() => expect(fields(container).slug.value).toBe('notes/rust/async'))

    getByText('Save').click()

    await waitFor(() => expect(api.replacePage).toHaveBeenCalled())
    const [slug, body] = api.replacePage.mock.calls[0] as [string, { title: string | null }]
    expect(slug).toBe('notes/rust/async')
    expect(body.title).toBeNull()
  })

  it('sends a title that was typed', async () => {
    const { container, getByText } = openEditor('/edit/notes/rust/async')
    await waitFor(() => expect(fields(container).slug.value).toBe('notes/rust/async'))

    type(fields(container).title, '  Futures  ')
    getByText('Save').click()

    await waitFor(() => expect(api.replacePage).toHaveBeenCalled())
    const [, body] = api.replacePage.mock.calls[0] as [string, { title: string | null }]
    expect(body.title).toBe('Futures')
  })

  it('splits the tag field, dropping blanks and duplicates', async () => {
    const { container, getByText } = openEditor('/edit/notes/rust/async')
    await waitFor(() => expect(fields(container).slug.value).toBe('notes/rust/async'))

    type(fields(container).tags, 'rust,  async , , rust,')
    getByText('Save').click()

    await waitFor(() => expect(api.replacePage).toHaveBeenCalled())
    const [, body] = api.replacePage.mock.calls[0] as [string, { tags: string[] }]
    expect(body.tags).toEqual(['rust', 'async'])
  })

  it('creates rather than replaces when there is no page yet', async () => {
    const { container, getByText } = openEditor('/new')

    type(fields(container).slug, 'notes/new')
    type(fields(container).body, '# New\n')
    getByText('Save').click()

    await waitFor(() => expect(api.createPage).toHaveBeenCalled())
    expect(api.replacePage).not.toHaveBeenCalled()
    const [body] = api.createPage.mock.calls[0] as [{ slug: string; content: string }]
    expect(body.slug).toBe('notes/new')
    expect(body.content).toBe('# New\n')
  })

  /** Nothing to save it as, so the button stays out of reach. */
  it('cannot save a new page with no slug', () => {
    const { getByText } = openEditor('/new')

    expect((getByText('Save') as HTMLButtonElement).disabled).toBe(true)
  })

  it('does not read a page when creating one', () => {
    openEditor('/new')

    expect(api.getPage).not.toHaveBeenCalled()
  })
})

describe('the layout', () => {
  /** Which panes are on screen. The preview is the only `prose` block here. */
  function panes(container: HTMLElement) {
    return {
      editor: container.querySelector('textarea') !== null,
      preview: container.querySelector('.prose') !== null,
    }
  }

  /** The layout buttons, by the titles that say what each one does. */
  const CONTROLS = {
    editor: 'Collapse the preview',
    split: 'Show the editor and the preview',
    preview: 'Maximise the preview',
  }

  it('shows both panes by default', () => {
    const { container } = openEditor('/new')

    expect(panes(container)).toEqual({ editor: true, preview: true })
  })

  /**
   * Collapsing is not just hiding. The preview costs a `POST /api/render` on
   * every pause in typing, and this wiki counts its own API usage — so a pane
   * nobody is looking at must stop asking for renders entirely, including the
   * one that would otherwise fire at mount.
   */
  it('collapsing the preview leaves the editor and stops rendering', async () => {
    const { container, getByTitle } = openEditor('/new')
    await waitFor(() => expect(api.renderMarkdown).toHaveBeenCalled())
    vi.clearAllMocks()

    getByTitle(CONTROLS.editor).click()

    expect(panes(container)).toEqual({ editor: true, preview: false })
    type(fields(container).body, '# Something new\n')
    // Past the 300 ms debounce, with room to spare.
    await new Promise((resolve) => setTimeout(resolve, 400))
    expect(api.renderMarkdown).not.toHaveBeenCalled()
  })

  it('maximising the preview hides the form', () => {
    const { container, getByTitle } = openEditor('/new')

    getByTitle(CONTROLS.preview).click()

    expect(panes(container)).toEqual({ editor: false, preview: true })
  })

  it('renders again when the preview comes back', async () => {
    const { container, getByTitle } = openEditor('/new')
    getByTitle(CONTROLS.editor).click()
    type(fields(container).body, '# Written while collapsed\n')
    vi.clearAllMocks()

    getByTitle(CONTROLS.split).click()

    await waitFor(() => expect(api.renderMarkdown).toHaveBeenCalled())
    const [draft] = api.renderMarkdown.mock.calls[0] as [{ content: string }]
    expect(draft.content).toBe('# Written while collapsed\n')
  })

  /**
   * A working preference, not a property of the page: somebody who collapsed
   * the preview to write does not want it back on the next page they open.
   */
  it('remembers the choice across mounts', () => {
    const first = openEditor('/new')
    first.getByTitle(CONTROLS.preview).click()
    cleanup()

    const { container } = openEditor('/new')

    expect(panes(container)).toEqual({ editor: false, preview: true })
  })

  it('falls back to the split view when the stored layout is nonsense', () => {
    window.localStorage.setItem('rhizolog:editor-layout', 'sideways')

    const { container } = openEditor('/new')

    expect(panes(container)).toEqual({ editor: true, preview: true })
  })
})
