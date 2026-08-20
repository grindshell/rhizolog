import { For, Show, createResource, createSignal } from 'solid-js'
import { A } from '@solidjs/router'
import {
  captureCandidates,
  connectCapture,
  createIdea,
  ideaHref,
  rejectCandidate,
  rejectCapturePair,
} from '../api/client'
import type { CandidateView, CaptureView } from '../api/client'
import { ErrorNotice } from './Async'

/**
 * What one capture might belong with, and the arithmetic that says so.
 *
 * Nothing here connects anything on its own. The analyzer proposes and the
 * person disposes, which is the whole reason the numbers are on screen at all:
 * a suggestion you cannot check is a suggestion you have to trust.
 *
 * Fetched separately from the capture that produced it, and deliberately: the
 * capture is already saved by the time this mounts, so nothing the analyzer
 * does can lose it.
 */
export default function Candidates(props: {
  capture: CaptureView
  /**
   * Called when a decision was written, so the screen around this can re-read.
   *
   * Deliberately carries nothing. The four decisions here answer with three
   * different shapes, two of them an `IdeaView` and one a `CaptureView`, and a
   * parameter that had to be cast to be useful would be a parameter asserting
   * something it does not know. Whoever needs the record re-reads it.
   */
  onChanged?: () => void
}) {
  const [reloads, setReloads] = createSignal(0)
  const [failure, setFailure] = createSignal<unknown>()
  const [naming, setNaming] = createSignal<string>()

  const [found, { refetch }] = createResource(
    () => ({ id: props.capture.id, reloads: reloads() }),
    ({ id }) => captureCandidates(id),
  )

  /**
   * `latest` rather than the resource call, and guarded, because reading a
   * resource that failed *rethrows* the failure. Unguarded, an analyzer that is
   * merely behind would throw out of this component and take the capture form
   * above it down, which is exactly the coupling the two requests exist to
   * avoid. Using `latest` also keeps the suggestions on screen while a decision
   * is being written rather than blanking them.
   */
  const shown = () => (found.error ? undefined : found.latest)

  const guard = async (work: () => Promise<unknown>) => {
    setFailure(undefined)
    try {
      await work()
      setNaming(undefined)
      setReloads((count) => count + 1)
      props.onChanged?.()
    } catch (error) {
      setFailure(error)
    }
  }

  return (
    <div class="flex flex-col gap-3">
      <Show when={failure()}>
        <ErrorNotice error={failure()} />
      </Show>

      <Show when={found.loading}>
        <div class="flex items-center gap-3 text-sm opacity-60">
          <span class="loading loading-spinner loading-xs" />
          Looking for anything like this...
        </div>
      </Show>

      {/*
        A failed analysis stays inside this panel. The capture is on disk and
        `503 idea_analysis_unavailable` means the index is behind rather than
        that anything is lost, so the offer is to rebuild it rather than an
        apology.
      */}
      <Show when={found.error}>
        <div class="flex flex-col gap-2">
          <ErrorNotice error={found.error} />
          <button class="btn btn-sm self-start" onClick={() => void refetch()}>
            Try again
          </button>
        </div>
      </Show>

      <Show when={shown()}>
        {(data) => (
          <>
            <Show
              when={data().candidates.length > 0}
              fallback={
                <p class="text-sm opacity-60">
                  <Show
                    when={data().terms > 0}
                    fallback="There are no words in this to match on yet."
                  >
                    Nothing else of yours is close enough to suggest. That is the
                    ordinary answer for a thought you have had once.
                  </Show>
                </p>
              }
            >
              <ul class="flex flex-col gap-3">
                <For each={data().candidates}>
                  {(candidate) => (
                    <li>
                      <Candidate
                        candidate={candidate}
                        threshold={data().threshold}
                        naming={naming()}
                        onName={setNaming}
                        onConnect={() =>
                          void guard(() =>
                            connectCapture(candidate.idea!.id, props.capture.id),
                          )
                        }
                        onCreate={(name) =>
                          void guard(() =>
                            createIdea({
                              name,
                              // Oldest first, so the thread reads in the order
                              // the thoughts arrived rather than in the order
                              // the analyzer happened to surface them.
                              captures: [candidate.capture!.id, props.capture.id].sort(),
                            }),
                          )
                        }
                        onReject={() =>
                          void guard(() =>
                            candidate.kind === 'idea'
                              ? rejectCandidate(candidate.idea!.id, props.capture.id)
                              : rejectCapturePair(
                                  props.capture.id,
                                  candidate.capture!.id,
                                ),
                          )
                        }
                      />
                    </li>
                  )}
                </For>
              </ul>
            </Show>

            <StartIdea
              onStart={(name) =>
                void guard(() =>
                  createIdea({ name, captures: [props.capture.id] }),
                )
              }
            />

            {/*
              What the numbers above are, said once. The analyzer version is
              here rather than hidden because it is the thing that changes what
              any of them mean.
            */}
            <p class="text-xs opacity-50">
              {data().analyzer} over {data().corpus}{' '}
              {data().corpus === 1 ? 'capture' : 'captures'} of yours, {data().terms}{' '}
              {data().terms === 1 ? 'term' : 'terms'} in this one, suggesting at{' '}
              {data().threshold} and above.
            </p>
          </>
        )}
      </Show>
    </div>
  )
}

/** One suggestion, with every number that produced it. */
function Candidate(props: {
  candidate: CandidateView
  threshold: number
  naming: string | undefined
  onName: (key: string | undefined) => void
  onConnect: () => void
  onCreate: (name: string) => void
  onReject: () => void
}) {
  const key = () => props.candidate.idea?.id ?? props.candidate.capture?.id ?? ''
  const [open, setOpen] = createSignal(false)

  return (
    <div class="rounded-box border border-base-300 bg-base-100 p-3">
      <div class="flex flex-wrap items-start justify-between gap-2">
        <div class="min-w-0 flex-1">
          <Show
            when={props.candidate.kind === 'idea'}
            fallback={
              <>
                <span class="badge badge-ghost badge-sm">another capture</span>
                <p class="pt-1 text-sm whitespace-pre-wrap">
                  {props.candidate.capture?.text}
                </p>
              </>
            }
          >
            <span class="badge badge-ghost badge-sm">idea</span>
            <A class="link pl-2 font-medium" href={ideaHref(props.candidate.idea!.id)}>
              {props.candidate.idea!.name}
            </A>
            <span class="pl-2 text-xs opacity-60">
              {props.candidate.idea!.captures} so far
            </span>
          </Show>
        </div>

        <div class="text-right">
          <div class="font-mono text-lg">{props.candidate.similarity}</div>
          {/*
            Said in words every time. It is how alike the words are, and a
            number on its own reads as a probability that the thoughts belong
            together, which is not what it measures.
          */}
          <div class="text-xs opacity-60">lexical similarity</div>
        </div>
      </div>

      <div class="flex flex-wrap gap-1 pt-2">
        <For each={props.candidate.signals}>
          {(signal) => (
            <span class="badge badge-outline badge-sm font-mono">{signal.term}</span>
          )}
        </For>
      </div>

      <div class="flex flex-wrap items-center gap-2 pt-3">
        <Show
          when={props.candidate.kind === 'idea'}
          fallback={
            <Show
              when={props.naming === key()}
              fallback={
                <button
                  class="btn btn-primary btn-sm"
                  onClick={() => props.onName(key())}
                >
                  Connect
                </button>
              }
            >
              <NameForm
                label="Name the idea these two share"
                onSubmit={props.onCreate}
                onCancel={() => props.onName(undefined)}
              />
            </Show>
          }
        >
          <button class="btn btn-primary btn-sm" onClick={props.onConnect}>
            Connect
          </button>
        </Show>

        <Show when={props.naming !== key()}>
          <button
            class="btn btn-ghost btn-sm"
            title="Do not suggest this again"
            onClick={props.onReject}
          >
            Not this
          </button>
          <button class="btn btn-ghost btn-sm" onClick={() => setOpen(!open())}>
            {open() ? 'Hide the arithmetic' : 'Why?'}
          </button>
        </Show>
      </div>

      <Show when={open()}>
        <Arithmetic candidate={props.candidate} />
      </Show>
    </div>
  )
}

/**
 * The sum the score is.
 *
 * Both vectors are unit length, so the similarity is a dot product and a dot
 * product is a sum of per-term products. These rows are the terms of that sum,
 * which is why they can be added up rather than merely believed.
 */
function Arithmetic(props: { candidate: CandidateView }) {
  return (
    <div class="overflow-x-auto pt-3">
      <table class="table table-xs">
        <thead>
          <tr>
            <th>Shared term</th>
            <th class="text-right">In how many</th>
            <th class="text-right">Here</th>
            <th class="text-right">There</th>
            <th class="text-right">Contributes</th>
          </tr>
        </thead>
        <tbody>
          <For each={props.candidate.signals}>
            {(signal) => (
              <tr>
                <td class="font-mono">{signal.term}</td>
                <td class="text-right">{signal.documents}</td>
                <td class="text-right font-mono">{signal.capture_weight}</td>
                <td class="text-right font-mono">{signal.target_weight}</td>
                <td class="text-right font-mono">{signal.contribution}</td>
              </tr>
            )}
          </For>
        </tbody>
        <tfoot>
          <tr>
            <th colSpan={4} class="text-right">
              These add up to
            </th>
            <th class="text-right font-mono">{props.candidate.explained}</th>
          </tr>
          <Show when={props.candidate.explained !== props.candidate.similarity}>
            <tr>
              <td colSpan={5} class="text-xs font-normal opacity-60">
                Five signals are shown and more than five were shared, so this is
                less than the {props.candidate.similarity} above rather than
                disagreeing with it.
              </td>
            </tr>
          </Show>
        </tfoot>
      </table>
    </div>
  )
}

/** Start a thread from this capture alone, whatever the analyzer thinks. */
function StartIdea(props: { onStart: (name: string) => void }) {
  const [open, setOpen] = createSignal(false)

  return (
    <Show
      when={open()}
      fallback={
        <button class="btn btn-ghost btn-sm self-start" onClick={() => setOpen(true)}>
          Start a new idea from this
        </button>
      }
    >
      <NameForm
        label="What is this idea called?"
        onSubmit={(name) => {
          setOpen(false)
          props.onStart(name)
        }}
        onCancel={() => setOpen(false)}
      />
    </Show>
  )
}

/**
 * Ask for a name, because Rhizolog never invents one.
 *
 * A generated name would be the first automatic claim in this feature that
 * nothing could check, and every other one comes with its arithmetic attached.
 */
function NameForm(props: {
  label: string
  onSubmit: (name: string) => void
  onCancel: () => void
}) {
  const [name, setName] = createSignal('')

  const submit = (event: Event) => {
    event.preventDefault()
    const trimmed = name().trim()
    if (!trimmed) return
    props.onSubmit(trimmed)
  }

  return (
    <form class="flex flex-wrap items-end gap-2" onSubmit={submit}>
      <label class="form-control min-w-48 flex-1">
        <span class="label-text text-xs opacity-60">{props.label}</span>
        <input
          class="input input-bordered input-sm w-full"
          aria-label="Idea name"
          autofocus
          value={name()}
          onInput={(event) => setName(event.currentTarget.value)}
        />
      </label>
      <button class="btn btn-primary btn-sm" type="submit" disabled={!name().trim()}>
        Create
      </button>
      <button class="btn btn-ghost btn-sm" type="button" onClick={props.onCancel}>
        Cancel
      </button>
    </form>
  )
}
