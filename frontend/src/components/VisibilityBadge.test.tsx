import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import type { SessionStatus, Visibility } from '../api/client'

const api = vi.hoisted(() => ({ session: vi.fn() }))

vi.mock('../api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client')>()
  return { ...actual, ...api }
})

/**
 * The session store resolves once per module instance, so each case re-imports
 * it with a fresh registry.
 */
async function badge(status: SessionStatus, visibility: Visibility, readers?: string[]) {
  api.session.mockResolvedValue(status)
  vi.resetModules()

  const { default: VisibilityBadge } = await import('./VisibilityBadge')
  const screen = render(() => (
    <VisibilityBadge visibility={visibility} owner="tim" readers={readers} />
  ))

  // The session resource has to settle before the badge decides anything.
  await waitFor(() => expect(api.session).toHaveBeenCalled())
  return screen
}

const CLOSED: SessionStatus = {
  authentication_required: true,
  authenticated: true,
  user: null,
}

const OPEN: SessionStatus = {
  authentication_required: false,
  authenticated: true,
  user: null,
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('the visibility badge', () => {
  /**
   * A badge on every page is a badge nobody reads, which is exactly how the one
   * that means *anyone on the internet* stops being noticed.
   */
  it('says nothing about an ordinary internal page', async () => {
    const screen = await badge(CLOSED, 'internal')
    await waitFor(() => expect(screen.container.querySelector('.badge')).toBeNull())
  })

  /** There is nobody to keep a page from, so the field means nothing. */
  it('says nothing at all on a wiki with no accounts', async () => {
    for (const visibility of ['public', 'restricted', 'private'] as Visibility[]) {
      const screen = await badge(OPEN, visibility)
      await waitFor(() => expect(screen.container.querySelector('.badge')).toBeNull())
      cleanup()
    }
  })

  it('marks the three that are worth marking', async () => {
    for (const visibility of ['public', 'restricted', 'private'] as Visibility[]) {
      const screen = await badge(CLOSED, visibility)
      await waitFor(() => expect(screen.getByText(visibility)).toBeTruthy())
      cleanup()
    }
  })

  /**
   * `public` is the only rung where picking it by mistake is a disclosure
   * rather than an inconvenience, so it is the one coloured to be noticed.
   */
  it('colours public as a warning and the others more quietly', async () => {
    const open = await badge(CLOSED, 'public')
    await waitFor(() =>
      expect(open.container.querySelector('.badge-warning')).toBeTruthy(),
    )
    cleanup()

    const shut = await badge(CLOSED, 'private')
    await waitFor(() => expect(shut.container.querySelector('.badge')).toBeTruthy())
    expect(shut.container.querySelector('.badge-warning')).toBeNull()
  })

  it('names the readers of a restricted page in its tooltip', async () => {
    const screen = await badge(CLOSED, 'restricted', ['alice', 'bob'])

    await waitFor(() => {
      const element = screen.container.querySelector('.badge')
      expect(element?.getAttribute('title')).toContain('alice, bob')
      expect(element?.getAttribute('title')).toContain('tim')
    })
  })

  it('says so when a restricted page has no readers but its owner', async () => {
    const screen = await badge(CLOSED, 'restricted', [])

    await waitFor(() =>
      expect(screen.container.querySelector('.badge')?.getAttribute('title')).toContain(
        'nobody but the owner',
      ),
    )
  })
})
