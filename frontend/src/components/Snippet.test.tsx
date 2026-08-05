import { afterEach, describe, expect, it } from 'vitest'
import { cleanup, render } from '@solidjs/testing-library'
import Snippet, { split } from './Snippet'

afterEach(cleanup)

describe('splitting a snippet', () => {
  it('leaves text with no matches alone', () => {
    expect(split('futures are lazy')).toEqual([{ text: 'futures are lazy', matched: false }])
  })

  it('separates the matched runs from the rest', () => {
    expect(split('futures are <mark>lazy</mark> here')).toEqual([
      { text: 'futures are ', matched: false },
      { text: 'lazy', matched: true },
      { text: ' here', matched: false },
    ])
  })

  it('handles several matches, and matches at either end', () => {
    expect(split('<mark>a</mark> and <mark>b</mark>')).toEqual([
      { text: 'a', matched: true },
      { text: ' and ', matched: false },
      { text: 'b', matched: true },
    ])
  })

  it('handles an empty snippet', () => {
    expect(split('')).toEqual([])
  })

  /**
   * `split` walks with a global regex, whose `lastIndex` persists across calls.
   * If it were not reset, the second call would start mid-string and quietly
   * lose the first match.
   */
  it('does not carry state between calls', () => {
    const snippet = 'see <mark>this</mark> one'
    expect(split(snippet)).toEqual(split(snippet))
  })
})

describe('rendering a snippet', () => {
  it('marks the matched terms', () => {
    const { container } = render(() => <Snippet text="futures are <mark>lazy</mark>" />)

    const marks = container.querySelectorAll('mark')
    expect(marks.length).toBe(1)
    expect(marks[0]?.textContent).toBe('lazy')
    expect(container.textContent).toBe('futures are lazy')
  })

  /**
   * The reason this component exists at all.
   *
   * SQLite's `snippet()` wraps matches in `<mark>` but leaves the text around
   * them exactly as the page wrote it — unescaped. Page bodies are what agents
   * write through the API, so injecting a snippet as HTML would execute
   * whatever a page happened to contain. Every piece has to become a text node.
   */
  it('does not execute markup that came from a page body', () => {
    const hostile = `carrying <script>alert('xss')</script> and <img src=x onerror=alert(1)>`

    const { container } = render(() => <Snippet text={hostile} />)

    expect(container.querySelectorAll('script').length).toBe(0)
    expect(container.querySelectorAll('img').length).toBe(0)
    // Shown, not run: the reader sees the markup as the page's own text.
    expect(container.textContent).toBe(hostile)
  })

  /**
   * A page that literally contains `<mark>` gets a spurious highlight. That is
   * the acceptable half of the trade — the text is still text — and it is
   * pinned here so nobody "fixes" it by reaching for `innerHTML`.
   */
  it('still refuses to inject when a page contains the marker itself', () => {
    const { container } = render(() => (
      <Snippet text="the <mark><script>bad</script></mark> tag" />
    ))

    expect(container.querySelectorAll('script').length).toBe(0)
    expect(container.querySelector('mark')?.textContent).toBe('<script>bad</script>')
  })
})
