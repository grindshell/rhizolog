import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Router } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { CompiledView, PageView, SectionView } from '../api/client'

// `manuscriptPace` as well as the compile: the panel asks for a pace on any page
// that names a target or a day, and a fetcher left real would reach for an
// origin jsdom does not have.
const api = vi.hoisted(() => ({
  compilePages: vi.fn(),
  manuscriptPace: vi.fn(),
  patchPage: vi.fn(),
}))

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
  // Rejected rather than resolved, which is the case worth defaulting to: the
  // pace is refused for a caller with no account, and the spine has to draw
  // anyway. `Pace.test.tsx` is where the figures themselves are checked.
  api.manuscriptPace.mockRejectedValue(new Error('no account'))
  api.patchPage.mockResolvedValue(page())
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

describe('what a section says about itself', () => {
  const staged = (over: Partial<CompiledView> = {}) =>
    compiled({
      sections: [
        section({ slug: 'book', title: 'The Long Way Round', depth: 0, words: 40 }),
        section({
          slug: 'book/one/opening',
          title: 'The Opening',
          stage: 'drafted',
          synopsis: 'They leave, and nobody says why.',
        }),
        section({
          slug: 'book/one/the-ferry',
          title: 'The Ferry',
          stage: 'with-beta-readers',
        }),
      ],
      ...over,
    })

  it('shows the stage as a badge and the synopsis as one line', async () => {
    api.compilePages.mockResolvedValue(staged())

    const { container, findByText } = panel()

    await findByText('The Opening')
    // Scoped to the rows, because the summary above them says these words too.
    const rows = container.querySelector('ul') as HTMLElement
    expect(rows.textContent).toContain('drafted')
    // An unknown stage is shown as itself rather than corrected to one of the
    // four the dashboard knows.
    expect(rows.textContent).toContain('with-beta-readers')

    const card = await findByText('They leave, and nobody says why.')
    expect(card.className).toContain('line-clamp-1')
  })

  /**
   * A synopsis is page content and page content is what agents write, which is
   * the rule `Snippet.tsx` exists to keep. Here it is kept by there being
   * nothing to render: the field is plain text and goes in as a text node.
   */
  it('renders a synopsis containing markup as characters', async () => {
    const hostile = '<img src=x onerror="alert(1)"> **not bold**'
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({ synopsis: hostile }),
        ],
      }),
    )

    const { findByText, container } = panel()

    expect(await findByText(hostile)).toBeTruthy()
    expect(container.querySelector('img')).toBeNull()
    expect(container.innerHTML).toContain('&lt;img')
  })

  /**
   * The reason the manifest carries two counts. A part's `words` is its own body,
   * so drawing a target against it would show every part in the book at two per
   * cent forever.
   */
  it('measures a section target against its subtree', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({ slug: 'book/one', title: 'Part One', words: 6, subtree: 2500, target: 5000 }),
        ],
      }),
    )

    const { container, findByText } = panel()

    await findByText('Part One')
    const bar = container.querySelector('li progress') as HTMLProgressElement
    expect(bar.value).toBe(2500)
    expect(bar.max).toBe(5000)
    expect(await findByText('2,500/5,000')).toBeTruthy()
  })

  it('draws no section bar where a section names no target', async () => {
    api.compilePages.mockResolvedValue(staged())

    const { container, findByText } = panel()

    await findByText('The Opening')
    expect(container.querySelector('li progress')).toBeNull()
  })
})

describe('the stage summary', () => {
  it('counts the stages across the manifest, in lifecycle order', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'Book', depth: 0 }),
          section({ slug: 'a', title: 'A', stage: 'revised' }),
          section({ slug: 'b', title: 'B', stage: 'drafted' }),
          // Two spellings of one stage, which fold together and are shown in
          // the canonical one rather than whichever arrived first.
          section({ slug: 'c', title: 'C', stage: 'Drafted' }),
          section({ slug: 'd', title: 'D', stage: 'todo' }),
          section({ slug: 'e', title: 'E' }),
        ],
      }),
    )

    const { findByText, getByRole } = panel()

    await findByText('A')
    expect(getByRole('list', { name: 'Stages' }).textContent).toBe(
      '1to do2drafted1revised',
    )
  })

  /**
   * A row of zeroes would be four claims about a book nobody has staged, and an
   * unstaged chapter is not a bucket: it is one nobody has said anything about.
   */
  it('shows nothing at all where no section has a stage', async () => {
    const { container, findByText } = panel()

    await findByText('The Opening')
    expect(container.textContent).not.toContain('to do')
    expect(container.textContent).not.toContain('drafted')
  })
})

describe('the card view', () => {
  const cards = (container: HTMLElement) => container.querySelectorAll('article')

  it('swaps the list for one card per section', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({
            slug: 'book/one/opening',
            title: 'The Opening',
            synopsis: 'They leave, and nobody says why.',
            stage: 'drafted',
          }),
          section({ slug: 'book/one/the-ferry', title: 'The Ferry' }),
        ],
      }),
    )

    const { container, findByText, getByText } = panel()

    await findByText('The Opening')
    expect(cards(container).length).toBe(0)

    getByText('Cards').click()

    await waitFor(() => expect(cards(container).length).toBe(2))
    expect(container.querySelectorAll('li').length).toBe(0)
    expect(cards(container)[0]?.textContent).toContain('They leave, and nobody says why.')
    // An empty card is a chapter nobody has decided about yet, which is exactly
    // the thing worth seeing. It says so rather than showing the prose.
    expect(cards(container)[1]?.textContent).toContain('No synopsis yet.')
  })

  /**
   * A gap has no page to have a synopsis and a cut chapter reports nothing about
   * itself, so the placeholder would be a sentence about a page rather than
   * about the position it left behind.
   */
  it('says nothing about a section that is not in the document', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({ slug: 'book/cut', title: undefined, status: 'excluded', words: 0 }),
          section({ slug: 'book/nowhere', title: undefined, status: 'wanted', words: 0 }),
        ],
      }),
    )

    const { container, findByText, getByText } = panel()

    await findByText('book/cut')
    getByText('Cards').click()
    await waitFor(() => expect(cards(container).length).toBe(2))

    for (const card of cards(container)) {
      expect(card.textContent).not.toContain('No synopsis yet.')
      expect(card.textContent).not.toContain('words')
    }
    // The status is what a position without a section has to say for itself,
    // and it is still said.
    expect(cards(container)[0]?.textContent).toContain('excluded')
    expect(cards(container)[1]?.textContent).toContain('wanted')
  })
})

describe('reordering the spine', () => {
  /**
   * A book whose contents list holds four entries, two of which a compile could
   * not resolve. Both have to survive a move, because both are things somebody
   * wrote.
   */
  function book(): CompiledView {
    return compiled({
      sections: [
        section({ slug: 'book', title: 'The Long Way Round', depth: 0, words: 40 }),
        section({ slug: 'book/one', title: 'Part One', parent: 'book', ordinal: 0 }),
        section({
          slug: 'book/gone',
          title: undefined,
          status: 'wanted',
          words: 0,
          parent: 'book',
          ordinal: 1,
        }),
        section({ slug: 'book/two', title: 'Part Two', parent: 'book', ordinal: 2 }),
        section({
          slug: '../nope',
          title: undefined,
          status: 'invalid',
          words: 0,
          parent: 'book',
          ordinal: 3,
        }),
      ],
    })
  }

  async function reordering() {
    api.compilePages.mockResolvedValue(book())
    const rendered = panel()
    await rendered.findByText('Part One')
    rendered.getByText('Reorder').click()
    await waitFor(() => expect(rendered.queryByLabelText('Move Part One down')).toBeTruthy())
    return rendered
  }

  it('offers no move buttons until the mode is entered', async () => {
    api.compilePages.mockResolvedValue(book())
    const { findByText, queryByLabelText } = panel()

    await findByText('Part One')
    expect(queryByLabelText('Move Part One down')).toBeNull()
  })

  /**
   * The whole feature: the parent's list is written back with one entry moved,
   * and it is a `PATCH` of `contents` rather than anything new.
   */
  it('writes the parent list back with the entry moved', async () => {
    const { getByLabelText } = await reordering()

    getByLabelText('Move Part Two up').click()

    await waitFor(() => expect(api.patchPage).toHaveBeenCalled())
    expect(api.patchPage).toHaveBeenCalledWith('book', {
      contents: ['book/one', 'book/two', 'book/gone', '../nope'],
    })
  })

  /**
   * A gap and a typo are entries somebody wrote, and a reorder that dropped one
   * would be losing work to a button press. They are also what a compile can say
   * least about, which is why they are the ones worth pinning.
   */
  it('keeps a gap and a bad entry in the list it writes', async () => {
    const { getByLabelText } = await reordering()

    getByLabelText('Move Part One down').click()

    await waitFor(() => expect(api.patchPage).toHaveBeenCalled())
    const [, body] = api.patchPage.mock.calls[0] as [string, { contents: string[] }]
    expect(body.contents).toHaveLength(4)
    expect(body.contents).toContain('book/gone')
    expect(body.contents).toContain('../nope')
  })

  it('will not move an entry off either end of its list', async () => {
    const { getByLabelText } = await reordering()

    expect(getByLabelText('Move Part One up')).toHaveProperty('disabled', true)
    expect(getByLabelText('Move ../nope down')).toHaveProperty('disabled', true)
    expect(getByLabelText('Move Part One down')).toHaveProperty('disabled', false)
  })

  /** Reading the book back is what makes a stale list visible rather than silent. */
  it('re-reads the manuscript after a move', async () => {
    const { getByLabelText } = await reordering()

    getByLabelText('Move Part Two up').click()

    await waitFor(() => expect(api.compilePages).toHaveBeenCalledTimes(2))
  })

  /**
   * The only write this panel makes, so the only failure it has to render. A 401
   * on a published wiki is the likeliest.
   */
  it('says so when the write is refused', async () => {
    const { getByLabelText, findByRole, queryByRole } = await reordering()
    api.patchPage.mockRejectedValue(new Error('page_not_found'))

    // Or the wait below would find an alert that was already there and pass
    // whatever the click did.
    expect(queryByRole('alert')).toBeNull()
    getByLabelText('Move Part Two up').click()

    expect(await findByRole('alert')).toBeTruthy()
  })

  /**
   * A book whose second part has scenes under it, which is what makes the flat
   * manifest interesting: the rows between Part One and Part Two are in a list of
   * their own and are most of what a dragged row passes over.
   */
  function nested(): CompiledView {
    return compiled({
      sections: [
        section({ slug: 'book', title: 'The Long Way Round', depth: 0, words: 40 }),
        section({ slug: 'book/one', title: 'Part One', parent: 'book', ordinal: 0 }),
        section({
          slug: 'book/one/opening',
          title: 'The Opening',
          depth: 2,
          parent: 'book/one',
          ordinal: 0,
        }),
        section({
          slug: 'book/one/the-ferry',
          title: 'The Ferry',
          depth: 2,
          parent: 'book/one',
          ordinal: 1,
        }),
        section({ slug: 'book/two', title: 'Part Two', parent: 'book', ordinal: 1 }),
      ],
    })
  }

  /**
   * A drag event, as far as jsdom has one.
   *
   * There is no `DragEvent` and no `DataTransfer` in jsdom, so this is a
   * cancelable event with a stub stapled to it. What it proves is the wiring:
   * which rows accept a drop, and that the write a drop ends in is the write the
   * buttons make. Whether a browser picks the row up, what the pointer shows and
   * where the drop actually lands are the browser's, and none of it is visible
   * from here. That is the honest limit of testing a drag in a fake DOM, and it
   * is why the buttons are still the ones the rest of this block checks.
   */
  function drag(type: string): Event {
    const event = new Event(type, { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'dataTransfer', {
      value: { effectAllowed: '', dropEffect: '', setData: () => {} },
    })
    return event
  }

  /** The rows, in the order the panel drew them. */
  const rows = (container: HTMLElement) => [...container.querySelectorAll('li')]

  it('writes the same list back for a drop as for a button', async () => {
    const { container } = await reordering()
    const [one, , two] = rows(container)

    two?.dispatchEvent(drag('dragstart'))
    one?.dispatchEvent(drag('dragover'))
    one?.dispatchEvent(drag('drop'))

    await waitFor(() => expect(api.patchPage).toHaveBeenCalled())
    expect(api.patchPage).toHaveBeenCalledWith('book', {
      contents: ['book/two', 'book/one', 'book/gone', '../nope'],
    })
  })

  /**
   * The rule the buttons are already on, and the one a drag makes it possible to
   * break: an entry moves within the list that names it, so a chapter cannot
   * leave its part. A `dragover` refuses the drop unless it is cancelled, so a
   * row that says nothing is a row that says no.
   */
  it('will not drop a part onto a scene inside another one', async () => {
    api.compilePages.mockResolvedValue(nested())
    const rendered = panel()
    await rendered.findByText('Part One')
    rendered.getByText('Reorder').click()
    await waitFor(() => expect(rendered.queryByLabelText('Move Part One down')).toBeTruthy())

    const [one, opening, , two] = rows(rendered.container)
    two?.dispatchEvent(drag('dragstart'))

    const refused = drag('dragover')
    opening?.dispatchEvent(refused)
    expect(refused.defaultPrevented).toBe(false)

    const accepted = drag('dragover')
    one?.dispatchEvent(accepted)
    expect(accepted.defaultPrevented).toBe(true)

    opening?.dispatchEvent(drag('drop'))
    expect(api.patchPage).not.toHaveBeenCalled()
  })

  /**
   * What is being dragged is an entry in the list this panel is holding, so a
   * drop nothing here picked up is a drag from somewhere else and means nothing.
   */
  it('ignores a drop that began outside the list', async () => {
    const { container } = await reordering()
    const [one] = rows(container)

    one?.dispatchEvent(drag('drop'))

    expect(api.patchPage).not.toHaveBeenCalled()
  })

  /** The row in hand, the place it would land, and the rows it could not. */
  it('marks the row being dragged, its landing place and neither of the rest', async () => {
    api.compilePages.mockResolvedValue(nested())
    const rendered = panel()
    await rendered.findByText('Part One')
    rendered.getByText('Reorder').click()
    await waitFor(() => expect(rendered.queryByLabelText('Move Part One down')).toBeTruthy())

    const [one, opening, , two] = rows(rendered.container)
    two?.dispatchEvent(drag('dragstart'))
    one?.dispatchEvent(drag('dragover'))

    await waitFor(() => expect(one?.className).toContain('ring-primary'))
    expect(two?.className).toContain('opacity-40')
    expect(opening?.className).toContain('opacity-30')
    expect(opening?.className).not.toContain('ring-primary')

    // Every row clears the mark, including the ones that refuse, or passing over
    // a scene would leave the last part it could have used still lit.
    opening?.dispatchEvent(drag('dragover'))
    await waitFor(() => expect(one?.className).not.toContain('ring-primary'))
  })

  /**
   * A list with a hole in it is one `spines` refuses to rebuild, so nothing in
   * it gets controls. It is still nowhere a drop can go, and the dimming has to
   * say so: one bright row among eight dimmed ones is the one row on screen
   * claiming to accept what it will not.
   */
  it('dims a row that has no controls, because a drop cannot go there either', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({ slug: 'book/one', title: 'Part One', parent: 'book', ordinal: 0 }),
          section({
            slug: 'book/one/opening',
            title: 'The Opening',
            depth: 2,
            parent: 'book/one',
            ordinal: 0,
          }),
          section({
            slug: 'book/one/late',
            title: 'Late',
            depth: 2,
            parent: 'book/one',
            ordinal: 2,
          }),
          section({ slug: 'book/two', title: 'Part Two', parent: 'book', ordinal: 1 }),
        ],
      }),
    )
    const rendered = panel()
    await rendered.findByText('Part One')
    rendered.getByText('Reorder').click()
    await waitFor(() => expect(rendered.queryByLabelText('Move Part One down')).toBeTruthy())
    expect(rendered.queryByLabelText('Move The Opening down')).toBeNull()

    const [, opening, , two] = rows(rendered.container)
    two?.dispatchEvent(drag('dragstart'))

    await waitFor(() => expect(opening?.className).toContain('opacity-30'))
    expect(opening?.getAttribute('draggable')).toBe('false')
  })

  /** Outside the mode the rows are rows, and the link inside one is a link. */
  it('makes no row draggable until the mode is entered', async () => {
    api.compilePages.mockResolvedValue(book())
    const { findByText, getByText, container } = panel()

    await findByText('Part One')
    expect(rows(container)[0]?.getAttribute('draggable')).toBe('false')
    expect(container.querySelector('li a')?.getAttribute('draggable')).toBeNull()

    getByText('Reorder').click()

    await waitFor(() =>
      expect(rows(container)[0]?.getAttribute('draggable')).toBe('true'),
    )
    // A link drags itself by default, and that drag is of the link rather than
    // of the row underneath it.
    expect(container.querySelector('li a')?.getAttribute('draggable')).toBe('false')
  })

  /**
   * Nothing named the root, so there is no list to move it within, and the panel
   * drops its row anyway. This is the guard that keeps a manifest without
   * ordinals from growing buttons that would write nonsense.
   */
  it('offers nothing on a manifest that does not say who named what', async () => {
    api.compilePages.mockResolvedValue(
      compiled({
        sections: [
          section({ slug: 'book', title: 'The Long Way Round', depth: 0 }),
          section({ slug: 'book/one', title: 'Part One' }),
        ],
      }),
    )
    const { findByText, getByText, queryByLabelText } = panel()

    await findByText('Part One')
    getByText('Reorder').click()

    await waitFor(() => expect(getByText('Reorder')).toBeTruthy())
    expect(queryByLabelText('Move Part One up')).toBeNull()
  })
})
