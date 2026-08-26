import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { CompiledView, PageView, SectionView } from '../api/client'

const api = vi.hoisted(() => ({ compilePages: vi.fn() }))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

const { default: Manuscript } = await import('./Manuscript')

function section(over: Partial<SectionView> = {}): SectionView {
  return {
    slug: 'book/one/opening',
    title: 'The Opening',
    depth: 1,
    words: 2180,
    // Equal to `words` on a leaf, which is what most of these fixtures are.
    subtree: 2180,
    offset: 238,
    length: 12903,
    status: 'included',
    ...over,
  }
}

function compiled(over: Partial<CompiledView> = {}): CompiledView {
  return {
    compiler: 'compile/v1',
    root: 'book',
    sections: [
      section({ slug: 'book', title: 'The Long Way Round', depth: 0, words: 40 }),
      section(),
    ],
    words: 2220,
    ...over,
  }
}

function page(over: Partial<PageView> = {}): PageView {
  return {
    slug: 'book',
    title: 'The Long Way Round',
    title_derived: false,
    tags: [],
    created: '2026-08-05T14:00:00Z',
    updated: '2026-08-05T14:00:00Z',
    size: 312,
    words: 40,
    content: '# The Long Way Round\n',
    visibility: 'internal',
    compile: true,
    contents: ['book/one/opening'],
    ...over,
  }
}

/** `<A>` needs a router around it, and a root with no routes is the least of one. */
function panel(over: Partial<PageView> = {}) {
  return render(() => (
    <Router root={() => <Manuscript page={page(over)} />}>{[]}</Router>
  ))
}

// A block body: `mockResolvedValue` returns the mock, and Vitest calls a
// function returned from `beforeEach` as a teardown.
beforeEach(() => {
  api.compilePages.mockResolvedValue(compiled())
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the manuscript panel', () => {
  it('lists the parts without repeating the page you are reading', async () => {
    const { findByText, container } = panel()

    expect(await findByText('The Opening')).toBeTruthy()
    await waitFor(() => expect(container.querySelectorAll('li').length).toBe(1))
  })

  /**
   * The whole reason this panel exists. Order moved into frontmatter, so nothing
   * else renders the spine, so anything hidden here is hidden everywhere, and a
   * manuscript short of a chapter has to say where the chapter was going.
   */
  it('shows a gap, a duplicate and a mistyped entry in position', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({ slug: 'book/one/the-ferry', title: undefined, status: 'wanted', words: 0 }),
          section({ slug: '../etc/passwd', title: undefined, status: 'invalid', words: 0 }),
          section({ slug: 'book/appendix', title: undefined, status: 'duplicate', words: 0 }),
        ],
      }),
    )

    const { findByText, container } = panel()

    expect(await findByText('wanted')).toBeTruthy()
    expect(await findByText('invalid')).toBeTruthy()
    expect(await findByText('duplicate')).toBeTruthy()

    // In position, and the slug is shown as text rather than as a link: there
    // is nothing at the end of any of them to go to.
    const rows = [...container.querySelectorAll('li')].map((row) => row.textContent ?? '')
    expect(rows[0]).toContain('book/one/the-ferry')
    expect(rows[1]).toContain('../etc/passwd')
    expect(rows[2]).toContain('book/appendix')
  })

  /**
   * Absent and empty are different values and survive a `PUT` as different
   * values, so they get different sentences. A book on the day it is started is
   * the second one, and showing nothing at all would look like a bug.
   */
  it('tells an empty contents list apart from an ordinary page', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [section({ slug: 'book', title: 'The Long Way Round', depth: 0 })],
      }),
    )

    const empty = panel({ contents: [] })
    expect(await empty.findByText(/no parts yet/)).toBeTruthy()

    cleanup()

    const leaf = panel({ contents: undefined, target: 500 })
    expect(await leaf.findByText(/Nothing is assembled here/)).toBeTruthy()
  })

  it('measures progress against the compiled total, not the page', async () => {
    api.compilePages.mockResolvedValue(compiled({ words: 45000, target: 90000 }))

    const { findByText, container } = panel({ words: 40, target: 90000 })

    expect(await findByText('45,000')).toBeTruthy()
    expect(await findByText('50%')).toBeTruthy()
    const bar = container.querySelector('progress') as HTMLProgressElement
    expect(bar.value).toBe(45000)
    expect(bar.max).toBe(90000)
  })

  /** No target is no bar, rather than a bar sitting at nothing. */
  it('draws no progress bar without a target', async () => {
    const { container, findByText } = panel()

    expect(await findByText('2,220')).toBeTruthy()
    expect(container.querySelector('progress')).toBeNull()
  })

  /**
   * A due date is a day. Read in the browser's own zone it would show the day
   * before to everybody west of Greenwich, because a bare date in a file is
   * midnight UTC.
   */
  it('shows a due date as the day it names, in UTC', async () => {
    const { findByText } = panel({ due: '2027-03-01T00:00:00Z' })

    const due = await findByText(/^due /)
    expect(due.textContent).toContain('2027')
    expect(due.textContent).toMatch(/Mar/)
    expect(due.textContent).toContain('1')
  })
})
