import { afterEach, describe, expect, it } from 'vitest'
import { createSignal } from 'solid-js'
import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, render } from '@solidjs/testing-library'
import Markdown from './Markdown'

afterEach(cleanup)

/** `Markdown` navigates in-app, so it needs a router above it. */
function renderMarkdown(html: () => string) {
  return render(() => (
    <MemoryRouter>
      <Route path="/" component={() => <Markdown html={html()} />} />
    </MemoryRouter>
  ))
}

/**
 * Click `target` and report whether the app claimed the event.
 *
 * The observer is added after the component mounts, so it runs after Solid's
 * delegated handler and sees the decision that handler made. It then prevents
 * the default itself, which stops jsdom trying to perform a real navigation and
 * filling the output with warnings about it.
 */
function clickIsHandledInApp(target: Element, init: MouseEventInit = {}): boolean {
  let handled = false
  const observer = (event: Event) => {
    handled = event.defaultPrevented
    event.preventDefault()
  }

  document.addEventListener('click', observer)
  target.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, ...init }))
  document.removeEventListener('click', observer)

  return handled
}

describe('rendering server HTML', () => {
  it('shows what the server rendered', () => {
    const { container } = renderMarkdown(
      () => '<h1>Async in Rust</h1>\n<p>Futures are <em>lazy</em>.</p>',
    )

    expect(container.querySelector('h1')?.textContent).toBe('Async in Rust')
    expect(container.querySelector('em')?.textContent).toBe('lazy')
  })

  it('replaces the content when the html changes', () => {
    const [html, setHtml] = createSignal('<p>first</p>')
    const { container } = renderMarkdown(html)

    expect(container.textContent).toContain('first')

    setHtml('<p>second</p><a href="https://example.com">out</a>')

    expect(container.textContent).toContain('second')
    expect(container.textContent).not.toContain('first')
    // The anchor treatment has to be re-applied to the new content, not just
    // to whatever was there on the first render.
    expect(container.querySelector('a')?.getAttribute('target')).toBe('_blank')
  })
})

describe('links out of the wiki', () => {
  it('opens them away from the app', () => {
    const { container } = renderMarkdown(
      () => '<a href="https://rust-lang.github.io/async-book/">the async book</a>',
    )

    const anchor = container.querySelector('a')
    expect(anchor?.getAttribute('target')).toBe('_blank')
    // Without `noopener` the opened page gets a handle on this window.
    expect(anchor?.getAttribute('rel')).toBe('noopener noreferrer')
  })

  it('leaves the click to the browser', () => {
    const { container } = renderMarkdown(() => '<a href="https://example.com">out</a>')

    expect(clickIsHandledInApp(container.querySelector('a')!)).toBe(false)
  })
})

describe('links between pages', () => {
  /**
   * The server rewrites page links to root-absolute `/pages/...` precisely so
   * that recognising them here is a prefix check rather than a second copy of
   * the backend's link resolution.
   */
  it('keeps them in the app', () => {
    const { container } = renderMarkdown(
      () => '<a href="/pages/notes/rust/pinning" data-wikilink="true">pinning</a>',
    )

    const anchor = container.querySelector('a')!
    expect(anchor.getAttribute('target')).toBeNull()
    expect(clickIsHandledInApp(anchor)).toBe(true)
  })

  /**
   * A click almost never lands on the anchor itself — it lands on whatever is
   * inside it. The handler walks up from the click target to find the link.
   */
  it('handles a click on markup inside the link', () => {
    const { container } = renderMarkdown(
      () => '<p>See <a href="/pages/notes/a"><strong>a</strong></a>.</p>',
    )

    expect(clickIsHandledInApp(container.querySelector('strong')!)).toBe(true)
  })

  /** But an element that merely *contains* a link is not a link. */
  it('ignores a click beside a link rather than inside it', () => {
    const { container } = renderMarkdown(
      () => '<p>See <a href="/pages/notes/a">a</a>.</p>',
    )

    expect(clickIsHandledInApp(container.querySelector('p')!)).toBe(false)
  })

  /**
   * A modified click means "open this somewhere else". The browser already does
   * exactly the right thing with it, and intercepting would break opening a
   * page in a new tab — the one interaction a wiki reader uses constantly.
   */
  it('leaves modified clicks alone', () => {
    const { container } = renderMarkdown(() => '<a href="/pages/notes/a">a</a>')
    const anchor = container.querySelector('a')!

    expect(clickIsHandledInApp(anchor, { ctrlKey: true })).toBe(false)
    expect(clickIsHandledInApp(anchor, { metaKey: true })).toBe(false)
    expect(clickIsHandledInApp(anchor, { shiftKey: true })).toBe(false)
    expect(clickIsHandledInApp(anchor, { button: 1 })).toBe(false)
  })

  it('ignores a click that landed on no link at all', () => {
    const { container } = renderMarkdown(() => '<p>just text</p>')

    expect(clickIsHandledInApp(container.querySelector('p')!)).toBe(false)
  })
})
