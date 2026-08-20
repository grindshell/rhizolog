import { For, Show, createResource, createSignal } from 'solid-js'
import { A, useParams } from '@solidjs/router'
import {
  affirmIdea,
  disconnectCapture,
  getIdea,
  ideaReceipt,
  patchIdea,
  reconsiderCandidate,
  reopenIdea,
  retireIdea,
} from '../api/client'
import type { CaptureView, IdeaView, ReceiptResponse } from '../api/client'
import { Async, ErrorNotice } from '../components/Async'
import { formatDate } from './PageDetail'

/**
 * One thread, and the receipt for what Rhizolog says about it.
 *
 * The receipt is the centrepiece rather than a debug drawer. Every automatic
 * claim this product makes is on this screen, and every one of them is next to
 * the authored records it was derived from: the sentence first, the arithmetic
 * under it, and each piece of evidence a link to the capture it came from.
 */
export default function IdeaDetail() {
  const params = useParams()
  const [reloads, setReloads] = createSignal(0)
  const [failure, setFailure] = createSignal<unknown>()

  // `?? ''` because the router types a path parameter as optional, though this
  // component only exists on a path that matched `:id`.
  const key = () => ({ id: params.id ?? '', reloads: reloads() })

  const [idea] = createResource(key, ({ id }) => getIdea(id))
  // Two requests rather than one fat response: the receipt is a different
  // question, it takes an `at` the rest of this screen does not, and a receipt
  // that failed to load must leave the thread itself readable.
  const [receipt] = createResource(key, ({ id }) => ideaReceipt(id))

  const reload = () => setReloads((count) => count + 1)

  const guard = async (work: () => Promise<unknown>) => {
    setFailure(undefined)
    try {
      await work()
      reload()
    } catch (error) {
      setFailure(error)
    }
  }

  return (
    <div class="flex flex-col gap-6">
      <Show when={failure()}>
        <ErrorNotice error={failure()} />
      </Show>

      <Async resource={idea}>
        {(thread) => (
          <>
            <Header
              idea={thread}
              onRename={(name) => void guard(() => patchIdea(thread.id, { name }))}
              onNote={(note) => void guard(() => patchIdea(thread.id, { note }))}
              onAffirm={() => void guard(() => affirmIdea(thread.id))}
              onRetire={() => void guard(() => retireIdea(thread.id))}
              onReopen={() => void guard(() => reopenIdea(thread.id))}
            />

            <section id="receipt" class="card bg-base-100 shadow">
              <div class="card-body gap-4">
                <h2 class="card-title text-base">Why it says that</h2>
                <Show
                  when={!receipt.error}
                  fallback={
                    <div class="flex flex-col gap-2">
                      <ErrorNotice error={receipt.error} />
                      <p class="text-xs opacity-60">
                        The thread above is unaffected. Only the explanation could not
                        be worked out.
                      </p>
                    </div>
                  }
                >
                  <Async resource={receipt}>
                    {(data) => <Receipt receipt={data} captures={thread.captures} />}
                  </Async>
                </Show>
              </div>
            </section>

            <Captures
              idea={thread}
              onDisconnect={(capture) =>
                void guard(() => disconnectCapture(thread.id, capture))
              }
            />

            <Show when={thread.rejected.length > 0}>
              <section class="card bg-base-100 shadow">
                <div class="card-body gap-2">
                  <h2 class="card-title text-base">
                    Turned down
                    <span class="badge badge-ghost badge-sm">
                      {thread.rejected.length}
                    </span>
                  </h2>
                  <p class="text-xs opacity-60">
                    Captures the analyzer suggested and you said no to. Reconsidering
                    makes one eligible again; it does not connect it.
                  </p>
                  <ul class="flex flex-col divide-y divide-base-200">
                    <For each={thread.rejected}>
                      {(capture) => (
                        <li class="flex flex-wrap items-center justify-between gap-2 py-2">
                          <span class="font-mono text-xs break-all">{capture}</span>
                          <button
                            class="btn btn-ghost btn-xs"
                            onClick={() =>
                              void guard(() =>
                                reconsiderCandidate(thread.id, capture),
                              )
                            }
                          >
                            Reconsider
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              </section>
            </Show>
          </>
        )}
      </Async>
    </div>
  )
}

/** The name, where the rules put it, and everything reversible you can do. */
function Header(props: {
  idea: IdeaView
  onRename: (name: string) => void
  onNote: (note: string) => void
  onAffirm: () => void
  onRetire: () => void
  onReopen: () => void
}) {
  const [renaming, setRenaming] = createSignal(false)
  const [editing, setEditing] = createSignal(false)
  const [name, setName] = createSignal(props.idea.name)
  const [note, setNote] = createSignal(props.idea.note)

  return (
    <header class="flex flex-col gap-3">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0 flex-1">
          <Show
            when={renaming()}
            fallback={
              <h1 class="text-2xl font-semibold break-words">{props.idea.name}</h1>
            }
          >
            <form
              class="flex flex-wrap items-end gap-2"
              onSubmit={(event) => {
                event.preventDefault()
                if (!name().trim()) return
                setRenaming(false)
                props.onRename(name().trim())
              }}
            >
              <input
                class="input input-bordered min-w-56 flex-1"
                aria-label="Idea name"
                value={name()}
                onInput={(event) => setName(event.currentTarget.value)}
              />
              <button class="btn btn-primary btn-sm" type="submit">
                Save
              </button>
              <button
                class="btn btn-ghost btn-sm"
                type="button"
                onClick={() => {
                  setName(props.idea.name)
                  setRenaming(false)
                }}
              >
                Cancel
              </button>
            </form>
          </Show>

          <div class="flex flex-wrap items-center gap-2 pt-2">
            <Show
              when={props.idea.state}
              fallback={<span class="badge badge-warning">no evidence left</span>}
            >
              {(state) => <span class="badge badge-outline">{state()}</span>}
            </Show>
            <Show
              when={props.idea.momentum !== null && props.idea.momentum !== undefined}
            >
              <a class="badge badge-ghost font-mono" href="#receipt">
                momentum {props.idea.momentum}
              </a>
            </Show>
            <Show when={props.idea.promoted_to}>
              {(slug) => (
                <A class="badge badge-success" href={`/pages/${slug()}`}>
                  became {slug()}
                </A>
              )}
            </Show>
            <Show when={props.idea.dismissed}>
              {(at) => (
                <span
                  class="badge badge-ghost"
                  title="Rediscovery will leave it alone for thirty days from then"
                >
                  dismissed {formatDate(at())}
                </span>
              )}
            </Show>
          </div>
        </div>

        <div class="flex flex-wrap gap-2">
          <button class="btn btn-ghost btn-sm" onClick={() => setRenaming(!renaming())}>
            Rename
          </button>
          {/*
            Affirming is not idempotent and should not be. Saying you are still
            interested twice is two moments at which you were, and both of them
            happened.
          */}
          <Show when={!props.idea.retired}>
            <button class="btn btn-sm" onClick={props.onAffirm}>
              Still interested
            </button>
          </Show>
          <Show
            when={props.idea.retired}
            fallback={
              <button class="btn btn-ghost btn-sm" onClick={props.onRetire}>
                Retire
              </button>
            }
          >
            <button class="btn btn-sm" onClick={props.onReopen}>
              Reopen
            </button>
          </Show>
        </div>
      </div>

      <Show
        when={editing()}
        fallback={
          <Show
            when={props.idea.note.trim()}
            fallback={
              <button
                class="btn btn-ghost btn-xs self-start"
                onClick={() => setEditing(true)}
              >
                Add a note
              </button>
            }
          >
            <p
              class="cursor-text text-sm whitespace-pre-wrap opacity-80"
              onClick={() => setEditing(true)}
            >
              {props.idea.note}
            </p>
          </Show>
        }
      >
        <form
          class="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            setEditing(false)
            props.onNote(note())
          }}
        >
          <textarea
            class="textarea textarea-bordered min-h-24 font-mono text-sm"
            aria-label="Working notes"
            value={note()}
            onInput={(event) => setNote(event.currentTarget.value)}
          />
          <div class="flex gap-2">
            <button class="btn btn-primary btn-sm" type="submit">
              Save the note
            </button>
            <button
              class="btn btn-ghost btn-sm"
              type="button"
              onClick={() => {
                setNote(props.idea.note)
                setEditing(false)
              }}
            >
              Cancel
            </button>
          </div>
        </form>
      </Show>
    </header>
  )
}

/**
 * The receipt: the sentences, then the numbers, then what they were counted
 * from.
 *
 * In that order on purpose. Somebody who wants the answer gets it in the first
 * line, somebody who does not believe it gets the arithmetic in the second, and
 * somebody who wants to check the arithmetic gets the authored records in the
 * third. None of it is behind a toggle.
 */
function Receipt(props: { receipt: ReceiptResponse; captures: CaptureView[] }) {
  const text = (id: string) =>
    props.captures.find((capture) => capture.id === id)?.text ?? ''

  return (
    <div class="flex flex-col gap-4">
      <div class="flex flex-col gap-1">
        <For each={props.receipt.explanation}>
          {(sentence) => <p class="text-sm">{sentence}</p>}
        </For>
      </div>

      <Show
        when={props.receipt.components}
        fallback={
          <div role="alert" class="alert alert-warning">
            <div>
              <div class="font-semibold">
                No state and no score, because the evidence is gone.
              </div>
              <div class="text-sm">
                {props.receipt.missing.length} of its captures cannot be read. Deriving
                a lifecycle from files that are not there is the one thing this must
                never do.
              </div>
            </div>
          </div>
        }
      >
        {(parts) => (
          <div class="overflow-x-auto">
            <table class="table table-sm">
              <tbody>
                <Line
                  name="Connected captures"
                  value={parts().total}
                  rule="Archived ones count. Deleted ones do not, because nothing authored is left."
                />
                <Line
                  name="In the last 14 days"
                  value={parts().recent_14}
                  rule={`Captured at or after ${formatDate(props.receipt.boundaries.recent_14)}.`}
                />
                <Line
                  name="In the last 30 days"
                  value={parts().recent_30}
                  rule={`Captured at or after ${formatDate(props.receipt.boundaries.recent_30)}.`}
                />
                <Line name="Base" value={parts().base} rule="min(connected, 4)." />
                <Line
                  name="Recency"
                  value={parts().recency}
                  rule="2 for three or more in 14 days, 1 for anything in 30, otherwise 0."
                />
                <Line
                  name="Affirmation"
                  value={parts().affirmation}
                  rule="1 for saying you are still interested, or reopening it, inside 30 days."
                />
                <tr class="font-semibold">
                  <td>Momentum</td>
                  <td class="text-right font-mono">{parts().momentum}</td>
                  <td class="text-xs font-normal opacity-60">
                    base + recency + affirmation, capped at 10. In this ruleset the
                    parts cannot exceed 7.
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        )}
      </Show>

      <div class="grid gap-4 lg:grid-cols-2">
        <div>
          <h3 class="pb-1 text-sm font-semibold">Captures counted</h3>
          <ul class="flex flex-col gap-1 text-sm">
            <For
              each={props.receipt.captures}
              fallback={<li class="opacity-60">None.</li>}
            >
              {(counted) => (
                <li class="flex flex-wrap items-baseline justify-between gap-2">
                  <a class="link min-w-0 flex-1 truncate" href={`#capture-${counted.id}`}>
                    {text(counted.id) || counted.id}
                  </a>
                  <span class="text-xs opacity-60">
                    {formatDate(counted.created)}
                    <Show when={counted.within_14_days}>
                      <span class="badge badge-ghost badge-xs ml-1">14d</span>
                    </Show>
                    <Show when={counted.within_30_days && !counted.within_14_days}>
                      <span class="badge badge-ghost badge-xs ml-1">30d</span>
                    </Show>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </div>

        <div>
          <h3 class="pb-1 text-sm font-semibold">Decisions counted</h3>
          <ul class="flex flex-col gap-1 text-sm">
            <For
              each={props.receipt.affirmations}
              fallback={
                <li class="opacity-60">
                  No affirmations or reopenings. One inside thirty days is worth a
                  point.
                </li>
              }
            >
              {(event) => (
                <li class="flex flex-wrap items-baseline justify-between gap-2">
                  <span class="font-mono text-xs">{event.kind}</span>
                  <span class="text-xs opacity-60">
                    {formatDate(event.created)}
                    <Show
                      when={event.within_30_days}
                      fallback={
                        <span class="badge badge-ghost badge-xs ml-1">not counted</span>
                      }
                    >
                      <span class="badge badge-ghost badge-xs ml-1">30d</span>
                    </Show>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </div>

      <Show when={props.receipt.missing.length > 0}>
        <div class="text-sm">
          <h3 class="pb-1 font-semibold">Evidence that is gone</h3>
          <ul class="flex flex-col gap-1">
            <For each={props.receipt.missing}>
              {(id) => <li class="font-mono text-xs break-all">{id}</li>}
            </For>
          </ul>
        </div>
      </Show>

      <p class="text-xs opacity-50">
        {props.receipt.ruleset} as of {formatDate(props.receipt.computed_at)} · dormant
        at {formatDate(props.receipt.boundaries.dormant)} or earlier
        <Show when={props.receipt.last_signal}>
          {(signal) => <> · last signal {formatDate(signal())}</>}
        </Show>
      </p>
    </div>
  )
}

/** One line of the arithmetic: what it is, what it came to, and the rule. */
function Line(props: { name: string; value: number; rule: string }) {
  return (
    <tr>
      <td>{props.name}</td>
      <td class="text-right font-mono">{props.value}</td>
      <td class="text-xs opacity-60">{props.rule}</td>
    </tr>
  )
}

/** What the thread holds, oldest first, which is the order it was thought in. */
function Captures(props: { idea: IdeaView; onDisconnect: (capture: string) => void }) {
  return (
    <section class="card bg-base-100 shadow">
      <div class="card-body gap-2">
        <h2 class="card-title text-base">
          Captures
          <span class="badge badge-ghost badge-sm">{props.idea.captures.length}</span>
        </h2>

        <ul class="flex flex-col divide-y divide-base-200">
          <For
            each={props.idea.captures}
            fallback={
              <li class="py-3 text-sm opacity-60">
                Nothing connected. Connect a capture from the inbox, or retire the
                thread.
              </li>
            }
          >
            {(capture) => (
              // The anchor the receipt's evidence rows point at, so "which
              // capture was that" is one click rather than a search.
              <li id={`capture-${capture.id}`} class="flex flex-col gap-1 py-3">
                <p class="text-sm whitespace-pre-wrap">{capture.text}</p>
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <span class="text-xs opacity-60">
                    {formatDate(capture.created)}
                    <Show when={capture.archived}>
                      <span class="badge badge-ghost badge-xs ml-2">archived</span>
                    </Show>
                  </span>
                  <button
                    class="btn btn-ghost btn-xs"
                    aria-label="Disconnect this capture"
                    onClick={() => props.onDisconnect(capture.id)}
                  >
                    Disconnect
                  </button>
                </div>
              </li>
            )}
          </For>
        </ul>

        <Show when={props.idea.missing.length > 0}>
          <p class="text-xs opacity-60">
            {props.idea.missing.length} connected{' '}
            {props.idea.missing.length === 1 ? 'capture is' : 'captures are'} named in
            this thread but no longer on disk. They are listed in the receipt.
          </p>
        </Show>
      </div>
    </section>
  )
}
