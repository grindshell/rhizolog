import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Route, Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { PageView } from '../api/client'

/**
 * What a page's own header says about its drafting: the stage badge beside the
 * title, and the synopsis above the prose.
 *
 * A file of its own rather than cases in `PageDetail.test.ts`, which tests two
 * pure functions and mounts nothing. Rendering this route needs the page, its
 * links and the pin store stubbed, and that scaffolding has no business sitting
 * on top of a test about grouping link panels.
 */
const api = vi.hoisted(() => ({
  getPage: vi.fn(),
  pageLinks: vi.fn(),
  compilePages: vi.fn(),
  // The header reads the session through `VisibilityBadge`. Left real it would
  // fire a `fetch` at module load, which in jsdom has no origin to resolve
  // `/api/auth/session` against.
  session: vi.fn(async () => ({
    authentication_required: false,
    authenticated: true,
    user: null,
  })),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

const { default: PageDetail } = await import('./PageDetail')

function page(over: Partial<PageView> = {}): PageView {
  return {
    slug: 'book/one/the-ferry',
    title: 'The Ferry',
    title_derived: false,
    tags: ['book'],
    created: '2026-08-05T14:00:00Z',
    updated: '2026-08-05T14:00:00Z',
    size: 312,
    words: 188,
    content: '# The Ferry\n\nThe ferry was late.\n',
    html: '<h1>The Ferry</h1>',
    visibility: 'internal',
    compile: true,
    ...over,
  }
}

function open(over: Partial<PageView> = {}) {
  api.getPage.mockResolvedValue(page(over))
  window.history.pushState({}, '', '/pages/book/one/the-ferry')
  return render(() => (
    <Router>
      <Route path="/pages/*slug" component={PageDetail} />
    </Router>
  ))
}

/**
 * The page's own `<h1>`, once it has loaded.
 *
 * By element rather than by text: the title is in the breadcrumb and in the
 * rendered body as well, and "find the words The Ferry" would be three matches
 * and an error rather than a question about the header.
 */
async function heading(container: HTMLElement): Promise<HTMLElement> {
  await waitFor(() => expect(container.querySelector('h1.card-title')).toBeTruthy())
  return container.querySelector('h1.card-title') as HTMLElement
}

/**
 * The synopsis card, or `null`.
 *
 * Also by element, and for a second reason: testing-library normalises
 * whitespace when it matches text, which would collapse the blank line that is
 * the whole point of one of the cases below.
 */
function synopsisCard(container: HTMLElement): HTMLElement | null {
  return container.querySelector('p.whitespace-pre-line')
}

beforeEach(() => {
  api.pageLinks.mockResolvedValue({
    slug: 'book/one/the-ferry',
    outbound: [],
    inbound: [],
    // Present and empty rather than null: the time panel reads through it and
    // hides itself on a count of zero, so a wiki nobody times looks the way it
    // did before timers existed.
    times: { entries: 0, groups: 0, seconds: 0, running: 0, recent: [] },
  })
  api.compilePages.mockResolvedValue({
    compiler: 'compile/v1',
    root: 'book/one/the-ferry',
    sections: [],
    words: 188,
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the stage badge', () => {
  it('sits beside the title when the page names a stage', async () => {
    const { container } = open({ stage: 'drafted' })

    expect((await heading(container)).textContent).toContain('drafted')
  })

  /** Shown as itself: the vocabulary is not fixed anywhere else either. */
  it('shows a stage this dashboard has never heard of', async () => {
    const { container } = open({ stage: 'with-beta-readers' })

    expect((await heading(container)).textContent).toContain('with-beta-readers')
  })

  /**
   * Most pages are not at a stage, and a badge on every page is a badge nobody
   * reads, which is the argument `VisibilityBadge` beside it already makes.
   */
  it('says nothing on a page that names no stage', async () => {
    const { container } = open()

    expect((await heading(container)).textContent?.trim()).toBe('The Ferry')
  })
})

describe('the synopsis', () => {
  const card = 'He misses the crossing.\n\nFirst time he chooses to be late.'

  it('appears above the prose, with its paragraph break kept', async () => {
    const { container } = open({ synopsis: card })

    await waitFor(() => expect(synopsisCard(container)).toBeTruthy())
    // Compared against the raw text rather than through a text query, which
    // would normalise the blank line away and pass either way.
    expect(synopsisCard(container)?.textContent).toBe(card)
  })

  /**
   * A synopsis is page content and page content is what agents write, which is
   * the rule `Snippet.tsx` exists to keep. Here it is kept by there being
   * nothing to render: the field is plain text and goes in as a text node.
   */
  it('renders markup as characters', async () => {
    const hostile = '<img src=x onerror="alert(1)"> **not bold**'
    const { container } = open({ synopsis: hostile })

    await waitFor(() => expect(synopsisCard(container)).toBeTruthy())
    expect(synopsisCard(container)?.textContent).toBe(hostile)
    expect(container.querySelector('img')).toBeNull()
    expect(container.innerHTML).toContain('&lt;img')
  })

  /**
   * Nothing derives one, so a page with a perfectly good opening paragraph still
   * has no card. An excerpt of the prose here would be a claim about the chapter
   * that nobody made.
   */
  it('is absent on a page that has none, rather than taken from the body', async () => {
    const { container } = open()

    await heading(container)
    expect(synopsisCard(container)).toBeNull()
  })

  /**
   * It describes the page, and the page is still the page when the body is being
   * shown as raw markdown.
   */
  it('stays while the source is shown', async () => {
    const { container, getByText } = open({ synopsis: card })

    await waitFor(() => expect(synopsisCard(container)).toBeTruthy())
    getByText('Source').click()

    await waitFor(() => expect(getByText('Rendered')).toBeTruthy())
    expect(synopsisCard(container)?.textContent).toBe(card)
  })
})
