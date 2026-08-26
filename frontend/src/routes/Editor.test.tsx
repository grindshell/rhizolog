import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Route, Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { PageView } from '../api/client'

const api = vi.hoisted(() => ({
  getPage: vi.fn(),
  replacePage: vi.fn(),
  createPage: vi.fn(),
  renderMarkdown: vi.fn(),
  splitPage: vi.fn(),
  mergePages: vi.fn(),
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

const { default: Editor, directoryOf, opening, parseDue, parseLines, parseTarget } =
  await import('./Editor')

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
    // What an absent `compile:` means, which is what every page here is.
    compile: true,
    ...overrides,
  }
}

/**
 * The form's fields.
 *
 * The three text inputs are in layout order; the body is found by its
 * placeholder rather than by being the first `textarea`. It is not: a page that
 * assembles others grows a contents textarea above it, so the positional version
 * would quietly type a manuscript's chapter list into the wrong box.
 */
function fields(container: HTMLElement) {
  const inputs = container.querySelectorAll('input')
  return {
    slug: inputs[0] as HTMLInputElement,
    title: inputs[1] as HTMLInputElement,
    tags: inputs[2] as HTMLInputElement,
    body: container.querySelector(
      'textarea[placeholder^="# Heading"]',
    ) as HTMLTextAreaElement,
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
  api.splitPage.mockResolvedValue({
    head: page(),
    tail: page({ slug: 'notes/rust/later' }),
    repaired: [],
  })
  api.mergePages.mockResolvedValue({
    page: page({ slug: 'notes/rust/pinning' }),
    removed: 'notes/rust/async',
    repaired: [],
  })
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

/**
 * Saving is a `PUT`, so a field left out is a field cleared. An editor that did
 * not send these back would unmake a book on the first save of any page in it,
 * which is the same failure that once handed pages to the wrong owner.
 */
describe('the manuscript fields survive a save', () => {
  const manuscript = () =>
    page({
      slug: 'book',
      contents: ['book/one/opening', 'book/one/the-ferry'],
      target: 90000,
      due: '2027-03-01T00:00:00Z',
    })

  async function saveLoaded(container: HTMLElement, getByText: (text: string) => HTMLElement) {
    await waitFor(() => expect(fields(container).slug.value).toBe('book'))
    getByText('Save').click()
    await waitFor(() => expect(api.replacePage).toHaveBeenCalled())
    return api.replacePage.mock.calls[0]?.[1] as {
      contents: string[] | null
      target: number | null
      due: string | null
    }
  }

  it('sends the contents list back untouched', async () => {
    api.getPage.mockResolvedValue(manuscript())
    const { container, getByText } = openEditor('/edit/book')

    const body = await saveLoaded(container, getByText)

    expect(body.contents).toEqual(['book/one/opening', 'book/one/the-ferry'])
    expect(body.target).toBe(90000)
    expect(body.due).toBe('2027-03-01T00:00:00Z')
  })

  /**
   * Absent and empty are different values, and the API keeps them apart:
   * absent is an ordinary page, `[]` is a manuscript with nothing in it yet.
   */
  it('keeps an empty contents list empty rather than clearing it', async () => {
    api.getPage.mockResolvedValue(page({ slug: 'book', contents: [] }))
    const { container, getByText } = openEditor('/edit/book')

    const body = await saveLoaded(container, getByText)

    expect(body.contents).toEqual([])
    expect(body.target).toBeNull()
    expect(body.due).toBeNull()
  })

  it('sends null for a page that assembles nothing', async () => {
    api.getPage.mockResolvedValue(page({ slug: 'book' }))
    const { container, getByText } = openEditor('/edit/book')

    const body = await saveLoaded(container, getByText)

    expect(body.contents).toBeNull()
  })
})

describe('the drafting fields survive a save', () => {
  async function saveLoaded(container: HTMLElement, getByText: (text: string) => HTMLElement) {
    await waitFor(() => expect(fields(container).slug.value).toBe('book/one/the-ferry'))
    getByText('Save').click()
    await waitFor(() => expect(api.replacePage).toHaveBeenCalled())
    return api.replacePage.mock.calls[0]?.[1] as {
      synopsis: string | null
      stage: string | null
      target: number | null
      compile: boolean
    }
  }

  /**
   * The rule written down twice and broken once. A `PUT` replaces every field,
   * so an editor that shows a chapter and saves it must hand all four back
   * whether or not anybody touched them.
   */
  it('sends all four back untouched', async () => {
    api.getPage.mockResolvedValue(
      page({
        slug: 'book/one/the-ferry',
        synopsis: 'He misses the crossing.\n\nAnd decides not to mind.',
        stage: 'with-beta-readers',
        target: 3000,
        compile: false,
      }),
    )
    const { container, getByText } = openEditor('/edit/book/one/the-ferry')

    const body = await saveLoaded(container, getByText)

    expect(body.synopsis).toBe('He misses the crossing.\n\nAnd decides not to mind.')
    expect(body.stage).toBe('with-beta-readers')
    expect(body.target).toBe(3000)
    expect(body.compile).toBe(false)
  })

  /**
   * An ordinary page saved from the editor must not grow any of them. `true` is
   * what an absent `compile:` means, so sending it back writes nothing.
   */
  it('sends nothing for a page that says none of them', async () => {
    api.getPage.mockResolvedValue(page({ slug: 'book/one/the-ferry' }))
    const { container, getByText } = openEditor('/edit/book/one/the-ferry')

    const body = await saveLoaded(container, getByText)

    expect(body.synopsis).toBeNull()
    expect(body.stage).toBeNull()
    expect(body.compile).toBe(true)
  })

  /** A stage of spaces is a field nobody filled in, not a stage called "  ". */
  it('sends null for a stage that is only whitespace', async () => {
    api.getPage.mockResolvedValue(
      page({ slug: 'book/one/the-ferry', stage: '   ', synopsis: '  ' }),
    )
    const { container, getByText } = openEditor('/edit/book/one/the-ferry')

    const body = await saveLoaded(container, getByText)

    expect(body.stage).toBeNull()
    expect(body.synopsis).toBeNull()
  })
})

describe('the manuscript block', () => {
  const block = (container: HTMLElement) =>
    container.querySelector('details') as HTMLDetailsElement

  /**
   * Most pages are not manuscripts, so the block is collapsed by default. A page
   * that carries any one of the fields is one, and a stage on its own is the
   * case a check for `contents`, `target` and `due` would have missed.
   */
  it('opens for a page carrying only a stage', async () => {
    api.getPage.mockResolvedValue(page({ slug: 'book/one/the-ferry', stage: 'drafted' }))
    const { container } = openEditor('/edit/book/one/the-ferry')

    await waitFor(() => expect(block(container).open).toBe(true))
    expect(block(container).textContent).toContain('drafted')
  })

  it('opens for a page carrying only a synopsis, and one kept out of the book', async () => {
    api.getPage.mockResolvedValue(
      page({ slug: 'book/one/the-ferry', synopsis: 'He misses the crossing.' }),
    )
    const { container } = openEditor('/edit/book/one/the-ferry')
    await waitFor(() => expect(block(container).open).toBe(true))
    cleanup()

    api.getPage.mockResolvedValue(page({ slug: 'book/one/cut', compile: false }))
    const cut = openEditor('/edit/book/one/cut')
    await waitFor(() => expect(block(cut.container).open).toBe(true))
    expect(block(cut.container).textContent).toContain('not compiled')
  })

  it('stays shut for an ordinary page', async () => {
    api.getPage.mockResolvedValue(page({ slug: 'notes/rust/async' }))
    const { container } = openEditor('/edit/notes/rust/async')

    await waitFor(() => expect(fields(container).slug.value).toBe('notes/rust/async'))
    expect(block(container).open).toBe(false)
  })
})

describe('splitting and merging', () => {
  /** A chapter with an accent in it, so the byte arithmetic has something to do. */
  const CHAPTER = '# Café\n\n## Later\n\nMore.\n'
  /** Where `## Later` starts: index 8 in the string, byte 9 in the file. */
  const LATER = CHAPTER.indexOf('## Later')

  const divide = (getByText: (text: string) => HTMLElement) =>
    getByText('Split and merge').closest('details') as HTMLDetailsElement

  function placeCursor(body: HTMLTextAreaElement, at: number) {
    body.setSelectionRange(at, at)
    body.dispatchEvent(new Event('select', { bubbles: true }))
  }

  async function openChapter(overrides: Partial<PageView> = {}) {
    api.getPage.mockResolvedValue(
      page({ slug: 'notes/rust/async', content: CHAPTER, ...overrides }),
    )
    const view = openEditor('/edit/notes/rust/async')
    await waitFor(() => expect(fields(view.container).body.value).toBe(CHAPTER))
    return view
  }

  /**
   * The one piece of arithmetic here that is invisible when it is wrong. A page
   * with an accent in it has more bytes than characters, and a cursor handed
   * over raw would cut the file somewhere else, further out the longer the page
   * runs.
   */
  it('sends the cursor as a byte offset rather than a string index', async () => {
    const { container, getByText } = await openChapter()

    placeCursor(fields(container).body, LATER)
    getByText('Split here').click()

    await waitFor(() => expect(api.splitPage).toHaveBeenCalled())
    const [request] = api.splitPage.mock.calls[0] as [
      { from: string; at: number; to: string; title: string | null },
    ]
    expect(request.from).toBe('notes/rust/async')
    expect(LATER).toBe(8)
    expect(request.at).toBe(9)
    // The directory is a head start; the name is the caller's, and an empty
    // title means the new page takes the heading it opens with.
    expect(request.to).toBe('notes/rust/')
    expect(request.title).toBeNull()
  })

  /** What a split would make, shown rather than described. */
  it('shows the line the new page would start with', async () => {
    const { container, getByText } = await openChapter()

    placeCursor(fields(container).body, LATER)

    await waitFor(() => expect(divide(getByText).textContent).toContain('## Later'))
  })

  /**
   * The half that needs a person is the new one: no synopsis, no stage and no
   * target on it. The half left behind is finished and saved.
   */
  it('lands in the editor for the page it made', async () => {
    const { container, getByText } = await openChapter()

    placeCursor(fields(container).body, LATER)
    getByText('Split here').click()

    await waitFor(() =>
      expect(api.getPage.mock.calls.map(([slug]) => slug)).toContain('notes/rust/later'),
    )
  })

  /**
   * Both act on the file the server holds, so an offset into a body with
   * unsaved edits in it would cut a page that is not the one being cut. The
   * same rule Rename follows, for the same reason.
   */
  it('refuses both while there are unsaved changes', async () => {
    const { container, getByText } = await openChapter()

    placeCursor(fields(container).body, LATER)
    type(fields(container).title, 'Later')

    expect((getByText('Split here') as HTMLButtonElement).disabled).toBe(true)
    expect((getByText('Merge and delete this page') as HTMLButtonElement).disabled).toBe(
      true,
    )
  })

  /** Nothing on one side of the cut is not a split; it is a rename. */
  it('refuses a cursor with nothing on one side of it', async () => {
    const { container, getByText } = await openChapter()

    for (const at of [0, CHAPTER.length]) {
      placeCursor(fields(container).body, at)
      await waitFor(() =>
        expect((getByText('Split here') as HTMLButtonElement).disabled).toBe(true),
      )
    }
  })

  /**
   * Refused by the server, and said here rather than left to be discovered by
   * pressing a button that fails.
   */
  it('says why neither will touch a page that assembles others', async () => {
    const { getByText } = await openChapter({ contents: ['notes/rust/async/one'] })

    expect(divide(getByText).textContent).toContain('This page assembles others')
    expect(() => getByText('Split here')).toThrow()
  })

  it('merges after a confirmation, and goes to the page that grew', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    const { container, getByText, findByTestId } = await openChapter()

    type(
      container.querySelector(
        'input[placeholder="book/one/the-ferry"]',
      ) as HTMLInputElement,
      'notes/rust/pinning',
    )
    getByText('Merge and delete this page').click()

    await waitFor(() => expect(api.mergePages).toHaveBeenCalled())
    expect(api.mergePages.mock.calls[0]?.[0]).toEqual({
      from: 'notes/rust/async',
      into: 'notes/rust/pinning',
    })
    // The page being edited is gone, so this is a `replace` and Back does not
    // return to a 404.
    expect(await findByTestId('elsewhere')).toBeTruthy()
    expect(confirm).toHaveBeenCalled()
  })

  it('does nothing when the confirmation is declined', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false)
    const { container, getByText } = await openChapter()

    type(
      container.querySelector(
        'input[placeholder="book/one/the-ferry"]',
      ) as HTMLInputElement,
      'notes/rust/pinning',
    )
    getByText('Merge and delete this page').click()

    expect(api.mergePages).not.toHaveBeenCalled()
  })

  it('has neither on a page that does not exist yet', () => {
    const { queryByText } = openEditor('/new')

    expect(queryByText('Split and merge')).toBeNull()
  })

  it('reads the directory a page sits in, and nothing more', () => {
    expect(directoryOf('book/one/the-ferry')).toBe('book/one/')
    expect(directoryOf('index')).toBe('')
  })

  it('finds the first line with anything on it', () => {
    expect(opening('One.\n\n\n## Two\n', 4)).toBe('## Two')
    expect(opening('One.\n', 0)).toBe('One.')
    expect(opening('One.\n\n  \n', 5)).toBe('')
    expect(opening('One.\n', -3)).toBe('One.')
  })
})

describe('parsing the manuscript fields', () => {
  it('reads a target, and refuses one that is not a whole count', () => {
    expect(parseTarget(' 90000 ')).toBe(90000)
    expect(parseTarget('')).toBeNull()
    // A negative target is not a small one, and reading `9.5` as nine would be
    // inventing a number nobody typed.
    expect(parseTarget('-1')).toBeNull()
    expect(parseTarget('9.5')).toBeNull()
    expect(parseTarget('lots')).toBeNull()
  })

  /** Midnight UTC, which is what a bare date in a file already means. */
  it('sends a day as the instant the API wants', () => {
    expect(parseDue('2027-03-01')).toBe('2027-03-01T00:00:00Z')
    expect(parseDue('')).toBeNull()
  })

  /**
   * Duplicates are kept, unlike a tag list. A contents list is positions, and an
   * appendix under two parts is a real thing the manifest reports as a
   * `duplicate` in its second position rather than an error.
   */
  it('splits a contents list by line and keeps repeats', () => {
    expect(parseLines('book/one\n\n  book/two  \nbook/one\n')).toEqual([
      'book/one',
      'book/two',
      'book/one',
    ])
    expect(parseLines('')).toEqual([])
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
