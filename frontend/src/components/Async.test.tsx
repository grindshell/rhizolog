import { afterEach, describe, expect, it, vi } from 'vitest'
import { createResource, createSignal } from 'solid-js'
import { cleanup, render, waitFor } from '@solidjs/testing-library'
import { Async } from './Async'

/** A promise somebody else decides the fate of. */
function deferred<T>() {
  let settle!: (value: T) => void
  let fail!: (reason: unknown) => void
  const promise = new Promise<T>((resolve, reject) => {
    settle = resolve
    fail = reject
  })
  // Attached now so a rejection that nothing is awaiting yet is not an unhandled
  // one; the resource picks the same promise up immediately afterwards.
  promise.catch(() => {})
  return { promise, settle, fail }
}

/**
 * A screen with one resource in it, and a handle to make it fetch again.
 *
 * The resource has to be created under an owner, so it lives in a component
 * rather than in the test body.
 */
function screenOver(fetcher: () => Promise<string>) {
  let again!: () => void

  const result = render(() => {
    const [attempt, setAttempt] = createSignal(0)
    const [data] = createResource(attempt, () => fetcher())
    again = () => setAttempt((count) => count + 1)
    return <Async resource={data}>{(value) => <p>{value}</p>}</Async>
  })

  return { ...result, again }
}

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('waiting for a resource', () => {
  it('shows a spinner while there is nothing to show yet', async () => {
    const first = deferred<string>()
    const screen = screenOver(() => first.promise)

    expect(screen.getByText('Loading...')).toBeTruthy()

    first.settle('the answer')
    await waitFor(() => expect(screen.getByText('the answer')).toBeTruthy())
    expect(screen.queryByText('Loading...')).toBeNull()
  })

  /**
   * The reason this component was changed. Every screen that can change
   * something re-reads afterwards, and blanking the whole route to a spinner
   * each time makes what you were reading flash out of existence for as long as
   * the round trip takes.
   */
  it('keeps the answer it has while fetching the next one', async () => {
    const first = deferred<string>()
    const second = deferred<string>()
    const answers = [first, second]
    let asked = 0

    const screen = screenOver(() => answers[asked++]!.promise)

    first.settle('the answer')
    await waitFor(() => expect(screen.getByText('the answer')).toBeTruthy())

    screen.again()
    await waitFor(() => expect(screen.getByText('Refreshing...')).toBeTruthy())
    // Still there, rather than a spinner where it used to be.
    expect(screen.getByText('the answer')).toBeTruthy()

    second.settle('a newer answer')
    await waitFor(() => expect(screen.getByText('a newer answer')).toBeTruthy())
    expect(screen.queryByText('Refreshing...')).toBeNull()
  })

  /**
   * A failed refresh shows the failure rather than what it used to know. Stale
   * data with nothing saying so reads as though the thing you just did worked.
   */
  it('replaces the answer when refreshing it fails', async () => {
    const first = deferred<string>()
    const second = deferred<string>()
    const answers = [first, second]
    let asked = 0

    const screen = screenOver(() => answers[asked++]!.promise)

    first.settle('the answer')
    await waitFor(() => expect(screen.getByText('the answer')).toBeTruthy())

    screen.again()
    second.fail(new Error('the index is unreadable'))

    await waitFor(() =>
      expect(screen.getByText(/the index is unreadable/)).toBeTruthy(),
    )
    expect(screen.queryByText('the answer')).toBeNull()
  })

  /**
   * Reading a Solid resource that failed *rethrows*, so the guard inside this
   * component is the only thing between a backend that is down and an exception
   * thrown out of a route. It is invisible in the source, which is why it is
   * here.
   */
  it('reports a first failure rather than throwing out of the route', async () => {
    const first = deferred<string>()
    const screen = screenOver(() => first.promise)

    first.fail(new Error('could not reach the server'))

    await waitFor(() =>
      expect(screen.getByText(/could not reach the server/)).toBeTruthy(),
    )
    expect(screen.getByRole('alert')).toBeTruthy()
  })
})
