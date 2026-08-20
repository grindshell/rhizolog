import { afterEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, render, waitFor } from '@solidjs/testing-library'

const api = vi.hoisted(() => ({
  session: vi.fn(),
  listPins: vi.fn(),
  listTimes: vi.fn(),
}))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

/*
  Answered before `Layout` is imported, because the session, pins and timer
  stores are module-level `createRoot`s that fetch the moment they are
  constructed. `vi.resetModules()` would be the other way to get a fresh one per
  test, and it cannot be used here: it would hand this module's `Layout` a
  second copy of `@solidjs/router`, and the `A` inside it would then be looking
  for a router this file's `MemoryRouter` is not.
*/
api.session.mockResolvedValue({
  authentication_required: true,
  authenticated: true,
  user: {
    username: 'tim',
    display_name: 'Tim',
    role: 'owner',
    created: '2026-01-01T00:00:00Z',
  },
})
api.listPins.mockResolvedValue({ pins: [], limit: 50 })
api.listTimes.mockResolvedValue({ times: [], total: 0, limit: 50, offset: 0 })

const { default: Layout } = await import('./Layout')

async function shell() {
  const history = createMemoryHistory()
  history.set({ value: '/' })

  const screen = render(() => (
    <MemoryRouter history={history} root={Layout}>
      <Route path="/" component={() => <p>the dashboard</p>} />
    </MemoryRouter>
  ))

  await waitFor(() => expect(screen.getByText('the dashboard')).toBeTruthy())
  return screen
}

const hrefs = (container: HTMLElement) =>
  [...container.querySelectorAll('a')].map((link) => link.getAttribute('href'))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the app shell', () => {
  /**
   * Every destination twice: once in the horizontal bar a wide screen shows and
   * once in the menu a narrow one does. Rendering both and letting CSS choose is
   * what keeps the phone's navigation from being a second, shorter list that
   * quietly loses a route.
   */
  it('offers every destination in both navigations', async () => {
    const { container } = await shell()
    const links = hrefs(container)

    for (const href of ['/inbox', '/ideas', '/pages', '/tags', '/graph', '/times']) {
      expect(links.filter((link) => link === href)).toHaveLength(2)
    }
    // Three for the dashboard: both navigations, plus the wordmark.
    expect(links.filter((link) => link === '/')).toHaveLength(3)
  })

  it('reaches the idea inbox and the ideas list', async () => {
    const { getAllByText } = await shell()

    expect(getAllByText('Inbox').length).toBeGreaterThan(0)
    expect(getAllByText('Ideas').length).toBeGreaterThan(0)
  })

  /** The narrow-width menu, which is what everything above collapses into. */
  it('has a navigation menu of its own for narrow screens', async () => {
    const { getByLabelText } = await shell()

    const menu = getByLabelText('Navigation')
    const list = menu.parentElement!.querySelector('ul')!
    expect(hrefs(list as unknown as HTMLElement)).toContain('/inbox')
  })

  /**
   * Capture and New page behind one control, because on a phone they cannot both
   * be buttons in a bar that also has to hold timers, pins and an account.
   * Capture comes first: it is the one you reach for while walking.
   */
  it('puts capture and new page behind one Create control', async () => {
    const { container, getByLabelText, getByText } = await shell()

    const create = getByLabelText('Create')
    const menu = create.parentElement!.querySelector('ul')!
    expect(hrefs(menu as unknown as HTMLElement)).toEqual(['/inbox?capture=1', '/new'])

    expect(getByText('Capture a thought')).toBeTruthy()
    // And no standalone New button beside it, which is what this replaced.
    expect(hrefs(container).filter((href) => href === '/new')).toHaveLength(1)
  })

  /**
   * A timer left running overnight is the most expensive thing this bar can fail
   * to show, so none of these collapse into the menu at any width.
   */
  it('keeps timers, pins and the account reachable', async () => {
    const { getByLabelText, getByText } = await shell()

    expect(getByLabelText('Running timers')).toBeTruthy()
    expect(getByLabelText('Pinned pages')).toBeTruthy()
    expect(getByText('Tim')).toBeTruthy()
  })

  /** Swagger UI is the backend's, so it has to be a real navigation. */
  it('leaves the API docs a plain link out of the app', async () => {
    const { container } = await shell()
    const docs = container.querySelector('a[href="/swagger-ui"]')!

    expect(docs.getAttribute('target')).toBe('_blank')
    expect(docs.getAttribute('rel')).toBe('noreferrer')
  })
})
