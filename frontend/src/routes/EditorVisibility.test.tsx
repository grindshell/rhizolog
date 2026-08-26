import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { PageView, SessionStatus } from '../api/client'

/**
 * The editor's visibility control, which is in a file of its own because it is
 * the one part of the editor whose behaviour depends on the session — and the
 * session store resolves once per module instance, so each case has to re-import
 * the whole thing with a fresh registry.
 */
const api = vi.hoisted(() => ({
  getPage: vi.fn(),
  replacePage: vi.fn(),
  createPage: vi.fn(),
  renderMarkdown: vi.fn(),
  session: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

function page(overrides: Partial<PageView> = {}): PageView {
  return {
    slug: 'notes/rust/async',
    title: 'Async in Rust',
    title_derived: true,
    tags: ['rust'],
    created: '2026-08-05T14:00:00Z',
    updated: '2026-08-05T14:00:00Z',
    size: 312,
    words: 3,
    content: 'Futures are lazy.\n',
    visibility: 'internal',
    compile: true,
    ...overrides,
  }
}

const CLOSED: SessionStatus = {
  authentication_required: true,
  authenticated: true,
  user: {
    username: 'tim',
    display_name: 'Tim',
    role: 'owner',
    has_password: true,
    profile: '',
    created: '2026-08-19T10:00:00Z',
    updated: '2026-08-19T10:00:00Z',
  },
}

const OPEN: SessionStatus = {
  authentication_required: false,
  authenticated: true,
  user: null,
}

async function openEditor(status: SessionStatus, loaded: PageView = page()) {
  api.session.mockResolvedValue(status)
  api.getPage.mockResolvedValue(loaded)
  api.renderMarkdown.mockResolvedValue({ html: '' })
  api.replacePage.mockResolvedValue(loaded)

  vi.resetModules()
  const { default: Editor } = await import('./Editor')
  // From the same registry as `Editor`, and that is not a detail: after
  // `resetModules` a statically imported `Router` is a *different instance* of
  // the router module, so its context is not the one `useParams` inside the
  // editor looks in. The symptom is "router primitives can be only used inside
  // a Route" on a component that plainly is.
  const { Route, Router } = await import('@solidjs/router')

  window.history.pushState({}, '', '/edit/notes/rust/async')
  const screen = render(() => (
    <Router>
      <Route path="/edit/*slug" component={Editor} />
      <Route path="*" component={() => <div data-testid="elsewhere" />} />
    </Router>
  ))

  await waitFor(() => expect(api.getPage).toHaveBeenCalled())
  return screen
}

beforeEach(() => {
  vi.clearAllMocks()
})

afterEach(() => {
  cleanup()
  window.localStorage.clear()
})

describe('the visibility control', () => {
  /**
   * On a wiki with no accounts the field does nothing, and a control that does
   * nothing is worse than no control.
   */
  it('is absent on a wiki with no accounts', async () => {
    const screen = await openEditor(OPEN)

    await waitFor(() => expect(screen.queryByText('Who can read this')).toBeNull())
    expect(screen.container.querySelector('select[class*="select"]')).toBeNull()
  })

  it('offers all four rungs once the wiki has accounts', async () => {
    const screen = await openEditor(CLOSED)

    await waitFor(() => expect(screen.getByText('Who can read this')).toBeTruthy())

    const select = screen.container.querySelector('select') as HTMLSelectElement
    const values = [...select.options].map((option) => option.value)
    expect(values).toEqual(['public', 'internal', 'restricted', 'private'])
    expect(select.value).toBe('internal')
  })

  it('starts on whatever the page already says', async () => {
    const screen = await openEditor(CLOSED, page({ visibility: 'private', owner: 'tim' }))

    await waitFor(() => {
      const select = screen.container.querySelector('select') as HTMLSelectElement
      expect(select.value).toBe('private')
    })
  })

  /** The reader list is only meaningful for one of the four. */
  it('asks for readers only while the page is restricted', async () => {
    const screen = await openEditor(CLOSED)
    await waitFor(() => expect(screen.getByText('Who can read this')).toBeTruthy())

    expect(screen.queryByText('Readers')).toBeNull()

    const select = screen.container.querySelector('select') as HTMLSelectElement
    select.value = 'restricted'
    select.dispatchEvent(new Event('change', { bubbles: true }))

    await waitFor(() => expect(screen.getByText('Readers')).toBeTruthy())
  })

  /**
   * Saving is a `PUT`, so a field left out is a field cleared. An editor that
   * dropped the owner would hand every page it touched to whoever last saved
   * it — which for a page shared with you means taking it from its owner.
   */
  it('sends the visibility, the readers and the existing owner when saving', async () => {
    const screen = await openEditor(
      CLOSED,
      page({ visibility: 'restricted', owner: 'alice', readers: ['tim'] }),
    )
    await waitFor(() => expect(screen.getByText('Who can read this')).toBeTruthy())

    const save = [...screen.container.querySelectorAll('button')].find((button) =>
      button.textContent?.includes('Save'),
    )
    save?.click()

    await waitFor(() => expect(api.replacePage).toHaveBeenCalled())
    const [, body] = api.replacePage.mock.calls[0] as [string, Record<string, unknown>]

    expect(body.visibility).toBe('restricted')
    expect(body.owner).toBe('alice')
    expect(body.readers).toEqual(['tim'])
  })
})
