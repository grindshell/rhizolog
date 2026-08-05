import { afterEach, describe, expect, it } from 'vitest'
import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, render } from '@solidjs/testing-library'
import SlugPath from './SlugPath'

afterEach(cleanup)

/** `SlugPath` renders `<A>`, so it needs a router above it. */
function renderPath(slug: string) {
  return render(() => (
    <MemoryRouter>
      <Route path="/" component={() => <SlugPath slug={slug} />} />
    </MemoryRouter>
  ))
}

function links(container: HTMLElement) {
  return [...container.querySelectorAll('a')].map((anchor) => ({
    text: anchor.textContent,
    href: anchor.getAttribute('href'),
  }))
}

describe('a slug as a path', () => {
  /**
   * Each link is the path *through* that segment, not the segment alone. That
   * is what makes the breadcrumb hierarchical: following `rust` in
   * `notes/rust/async` stays inside `notes`.
   */
  it('links each directory to everything under it', () => {
    const { container } = renderPath('notes/rust/async')

    expect(links(container)).toEqual([
      { text: 'notes', href: '/pages?prefix=notes' },
      { text: 'rust', href: '/pages?prefix=notes%2Frust' },
    ])
  })

  it('still reads as the slug it was given', () => {
    const { container } = renderPath('notes/rust/async')
    expect(container.textContent).toBe('notes/rust/async')
  })

  /**
   * The page is not a directory it sits in, and a link to the filtered listing
   * would sit next to the title link that goes to the page itself — two
   * neighbouring links, differently destined, reading identically.
   */
  it('leaves the segment naming the page itself as text', () => {
    const { container } = renderPath('notes/rust/async')
    expect(links(container).map((link) => link.text)).not.toContain('async')
  })

  it('has nothing to link in a top-level page', () => {
    const { container } = renderPath('index')

    expect(links(container)).toEqual([])
    expect(container.textContent).toBe('index')
  })
})
