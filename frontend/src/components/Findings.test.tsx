import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import type { FindingView, ProseReport } from '../api/client'

const api = vi.hoisted(() => ({ checkProse: vi.fn() }))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

const { default: Findings, byteToIndex, indexToByte } = await import('./Findings')

function finding(over: Partial<FindingView> = {}): FindingView {
  return {
    rule: 'echo',
    severity: 'warn',
    span: { start: 4, end: 8 },
    quote: 'late',
    message: 'late repeated within 12 words',
    receipt: { token: 'late', first: 4, second: 40, distance: 11, within: 40 },
    ...over,
  }
}

function report(over: Partial<ProseReport> = {}): ProseReport {
  return {
    analyzer: 'prose/v1',
    rules_digest: 'sha256:9f2bcd0011223344',
    rules: 5,
    offsets: 'page',
    findings: [finding()],
    errors: 0,
    warnings: 1,
    truncated: false,
    ...over,
  }
}

/** The strip is closed on mount, so most cases start by opening it. */
function open(container: HTMLElement) {
  const toggle = container.querySelector('button[aria-expanded]') as HTMLButtonElement
  fireEvent.click(toggle)
  return toggle
}

// A block body, deliberately. `mockResolvedValue` returns the mock, and Vitest
// treats a function returned from `beforeEach` as a teardown and calls it, so
// the concise form quietly invokes `checkProse()` with no arguments after every
// case, and the next case starts with a request nothing made.
beforeEach(() => {
  api.checkProse.mockResolvedValue(report())
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  // Whether the strip was open is remembered across mounts, so a case that
  // opened it would otherwise decide what the next case starts with.
  window.localStorage.clear()
})

describe('the findings strip', () => {
  /**
   * The preview's rule, and it is not pedantry: this wiki counts its own API
   * calls and puts the numbers on its dashboard, so a request for a pane nobody
   * is looking at shows up there as usage that never happened.
   */
  it('issues no request while it is closed', async () => {
    render(() => <Findings content="The ferry was late." />)

    await Promise.resolve()
    expect(api.checkProse).not.toHaveBeenCalled()
  })

  it('checks once when it is opened, without waiting for a pause in typing', async () => {
    const { container } = render(() => <Findings content="The ferry was late." />)

    open(container)

    await waitFor(() => expect(api.checkProse).toHaveBeenCalledTimes(1))
    expect(api.checkProse).toHaveBeenCalledWith({ content: 'The ferry was late.' })
  })

  it('shows what a rule found and the text it fired on', async () => {
    const { container, findByText } = render(() => <Findings content="body" />)
    open(container)

    expect(await findByText('late repeated within 12 words')).toBeTruthy()
    expect(await findByText('late')).toBeTruthy()
  })

  /**
   * A quote is cut from the page source, so it is page content, and page content
   * is what agents write. This is the rule `Snippet.tsx` exists to keep.
   */
  it('renders a quote as text and never as markup', async () => {
    api.checkProse.mockResolvedValue(
      report({
        findings: [finding({ quote: '<img src=x onerror="alert(1)">' })],
      }),
    )

    const { container } = render(() => <Findings content="body" />)
    open(container)

    await waitFor(() => expect(container.querySelector('li p')).toBeTruthy())
    expect(container.querySelector('img')).toBeNull()
    expect(container.querySelector('li p')?.textContent).toBe(
      '<img src=x onerror="alert(1)">',
    )
  })

  /**
   * No findings from no rules is not a clean page. A wiki that has never written
   * a rules file is the ordinary case, and saying "nothing to report" there
   * would be claiming an answer that was never asked for.
   */
  it('says a wiki has no rules rather than calling the page clean', async () => {
    api.checkProse.mockResolvedValue(report({ findings: [], warnings: 0, rules: 0 }))
    const { container, findByText } = render(() => <Findings content="body" />)
    open(container)

    expect(await findByText(/No rules yet/)).toBeTruthy()
  })

  it('says nothing to report when rules ran and found none', async () => {
    api.checkProse.mockResolvedValue(report({ findings: [], warnings: 0, rules: 5 }))
    const { container, findByText } = render(() => <Findings content="body" />)
    open(container)

    expect(await findByText('Nothing to report.')).toBeTruthy()
  })

  it('reveals a finding in the editor when it is clicked', async () => {
    const onReveal = vi.fn()
    const { container, findByText } = render(() => (
      <Findings content="body" onReveal={onReveal} />
    ))
    open(container)

    fireEvent.click(await findByText('late repeated within 12 words'))

    expect(onReveal).toHaveBeenCalledWith(expect.objectContaining({ rule: 'echo' }))
  })

  it('says when the list was cut short rather than pretending to be whole', async () => {
    api.checkProse.mockResolvedValue(report({ truncated: true }))
    const { container, findByText } = render(() => <Findings content="body" />)
    open(container)

    expect(await findByText(/cut short/)).toBeTruthy()
  })
})

/**
 * Spans are bytes, and `setSelectionRange` wants UTF-16 code units. Getting this
 * wrong selects the wrong words on any page with an accent in it, and drifts
 * further out the longer the page runs.
 */
describe('byte offsets as string positions', () => {
  it('is the identity over ASCII', () => {
    expect(byteToIndex('the ferry was late', 4)).toBe(4)
    expect(byteToIndex('the ferry was late', 0)).toBe(0)
  })

  it('counts a two-byte character as one position', () => {
    // `café` is five bytes and four units, so what follows it is offset by one.
    const text = 'café was late'
    expect(text.indexOf('was')).toBe(5)
    expect(byteToIndex(text, 6)).toBe(5)
  })

  it('counts an astral character as four bytes and two positions', () => {
    const text = '🚢 was late'
    expect(byteToIndex(text, 4)).toBe(2)
    expect(byteToIndex(text, 5)).toBe(3)
  })

  it('rounds an offset inside a character forward to its start', () => {
    // Nothing the analyzer produces lands here; this is the only sane answer if
    // anything ever does.
    expect(byteToIndex('café', 4)).toBe(4)
  })

  it('never runs past the end of the text', () => {
    expect(byteToIndex('café', 99)).toBe(4)
  })
})

/**
 * The other direction, which is what a cursor becomes on its way to a split. The
 * server cuts a file at a byte offset, so a caller that sent a string index
 * would cut somewhere else, and further out the longer the page runs.
 */
describe('string positions as byte offsets', () => {
  it('is the identity over ASCII', () => {
    expect(indexToByte('the ferry was late', 4)).toBe(4)
    expect(indexToByte('the ferry was late', 0)).toBe(0)
  })

  it('counts a two-byte character as two bytes', () => {
    const text = 'café was late'
    expect(indexToByte(text, 5)).toBe(6)
  })

  it('counts an astral character as two positions and four bytes', () => {
    expect(indexToByte('🚢 was late', 2)).toBe(4)
  })

  /**
   * Forward, past the whole character, which is the safe direction: the result
   * is still an offset the server will accept rather than one that cuts an emoji
   * in half.
   */
  it('rounds a position inside a surrogate pair forward', () => {
    expect(indexToByte('🚢 was late', 1)).toBe(4)
  })

  it('never runs past the end of the text', () => {
    expect(indexToByte('café', 99)).toBe(5)
    expect(indexToByte('café', -3)).toBe(0)
  })

  /** Round-tripping any boundary has to land back where it started. */
  it('undoes byteToIndex over the same text', () => {
    const text = '# Café\n\n## Later 🚢\n\nMore.\n'
    for (let index = 0; index <= text.length; index += 1) {
      expect(byteToIndex(text, indexToByte(text, index))).toBe(
        // A position inside a surrogate pair rounds forward to the pair's end,
        // which is the only boundary there is between those two units.
        index === text.indexOf('🚢') + 1 ? index + 1 : index,
      )
    }
  })
})
