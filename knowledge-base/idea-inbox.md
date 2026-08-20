# Idea Inbox implementation plan

Status: in progress. **Phases I0 and I1 are built**: the authored model and
store, and the derived index that folds decisions into current state. There is
still no API, no analysis and no UI, so the feature is not usable and capture
alone is not the MVP. The phases and completion gates below remain the handoff
for the rest, and [What I0 and I1 actually built](#what-i0-and-i1-actually-built)
records where the code has departed from this page.

Idea Inbox gives Rhizolog a low-friction place to capture unfinished thoughts,
notice which ones recur, and turn a mature idea into a wiki page. A capture is
not a page, task, project or journal entry. It is working material that may
eventually become knowledge.

The product promise is deliberately narrow:

> Rhizolog notices which ideas keep coming back, and shows why it thinks so.

The analysis is local, deterministic and advisory. It may propose a
connection. It may not silently create one. Every lifecycle label and momentum
value must be accompanied by the evidence and rules that produced it.

This plan builds on [Architecture](architecture.md), especially the rule that
files are authoritative and SQLite is derived, and on
[Time tracking](time-tracking.md), which already keeps authored non-page data
under `.rhizolog/` without letting it distort the wiki's link graph.

## Goals

- Make capture take one text field and one action. Do not require a title,
  slug, tag, folder or interpretation.
- Keep unfinished captures out of page search, orphan counts, wanted pages,
  tags and the ordinary link graph.
- Detect lexical recurrence locally without an LLM or network request.
- Keep the user in control of idea membership.
- Make every automatic claim inspectable down to authored captures and named
  rules.
- Preserve the same HTTP boundary for the dashboard, desktop app, agents and a
  browser on another device.
- Give an idea a deliberate path into the wiki without turning Rhizolog into
  project-management software.

## Not goals for the first implementation

- LLM summaries, generated names or generated interpretations
- Embeddings, vector databases or approximate nearest-neighbour search
- Automatic thread membership, even for a high similarity score
- Native mobile applications, operating-system share sheets or widgets
- Push notifications, daily prompts, streaks or engagement goals
- Due dates, priorities, tasks, projects, calendars or kanban boards
- Collaboration on captures or shared idea threads
- Sentiment analysis or claims about what the user values

The responsive web dashboard is the mobile surface for this implementation.
A PWA or native shell can be considered after the capture and rediscovery loop
has proved useful.

## Terms

**Capture:** One timestamped piece of text exactly as the user supplied it.

**Idea:** A user-named thread containing one or more confirmed captures.

**Candidate:** A derived suggestion that a capture may belong to an idea. It
is rebuildable and has no authority.

**Decision event:** An authored record of an explicit action such as connect,
reject, affirm, retire or promote.

**Receipt:** The rules, intermediate values and authored evidence behind an
idea's current lifecycle state and momentum.

**Promotion:** Creating a normal wiki page from selected captures, then
recording which page the idea produced.

## Product flow

```text
capture text
    -> save the authored capture
    -> index terms locally
    -> propose up to three possible idea connections
    -> user connects or rejects each suggestion
    -> derive lifecycle state and momentum from confirmed evidence
    -> optionally promote selected captures into a wiki page
```

Saving the capture is the primary operation. Analysis happens through a
separate request, so analysis failure or slowness can never lose or reject the
text the user submitted.

## Authored storage

Idea Inbox is a third authored tree beside pages and the time log:

```text
<wiki root>/
  notes/
    rust.md
  .rhizolog/
    index.db                         # derived
    times/                           # authored
    users/                           # authored and secret
    ideas/                           # authored
      captures/
        2026-08/
          20260820T141530-123456789.md
      threads/
        20260820T142000-234567890.md
      events/
        2026-08/
          20260820T142030-345678901.md
```

The repository and user documentation must say explicitly that
`.rhizolog/ideas/` is authored data and is not safe to delete. It should be
backed up and may be committed with the wiki. Unlike `.rhizolog/users/`, it
contains no password hashes and must not be gitignored by Rhizolog.

The page walker continues to ignore the whole internal directory. Captures do
not become pages merely because they contain Markdown.

### Identifiers

Capture, idea and event ids use the same rigid, server-generated shape as time
ids:

```text
20260820T141530-123456789
```

Each public id type remains distinct in Rust and OpenAPI, but their parsers may
share a private validation helper. The shape is safe as a filename, sorts by
creation time and chooses the `YYYY-MM` directory without accepting a path from
the caller. Minting nudges the nanosecond suffix until the target filename is
free.

Do not refactor `TimeId` as part of this feature unless sharing a helper is a
small, behaviour-preserving change. A broad identifier rewrite is not a
prerequisite.

### Capture file

```markdown
---
created: 2026-08-20T14:15:30Z
owner: tim
---

Maybe dungeon quests should require finding particular seeds.
```

`owner` is absent on a wiki with no accounts. `created` is immutable through
the API. The body may be corrected through `PATCH`; git remains the history of
such edits, just as it is for pages.

Archiving is a decision event rather than a frontmatter field. An archived
capture leaves the inbox but remains part of any idea history and remains
available as lifecycle evidence.

### Idea file

```markdown
---
name: Dungeon seeds
created: 2026-08-20T14:20:00Z
owner: tim
captures:
  - 20260820T141530-123456789
---

Optional working notes about the idea.
```

The file stores identity, its user-supplied name, optional notes and a non-empty
list of seed captures. The seed list is immutable through the API and makes
idea creation one atomic authored-file write rather than a thread file followed
by several event files. It is the evidence of the initial grouping. The file
does not store momentum, lifecycle state, rejections, promotion state or the
current member list after later decisions. Those are derived by starting with
the seeds and folding decision events.

A non-retired idea must always have at least one live connected capture.
Removing its last connection is refused with `409 idea_would_be_empty`;
retiring the idea is the reversible way to set it aside.

### Decision event file

```markdown
---
kind: capture_connected
created: 2026-08-20T14:20:30Z
actor: tim
idea: 20260820T142000-234567890
capture: 20260820T141530-123456789
---
```

The first implementation needs these event kinds:

- `capture_connected`
- `capture_disconnected`
- `candidate_rejected`
- `candidate_reconsidered`
- `interest_affirmed`
- `capture_archived`
- `capture_restored`
- `capture_deleted`
- `idea_retired`
- `idea_reopened`
- `idea_promoted`
- `rediscovery_dismissed`

Event ids define fold order. An event's `created` timestamp must agree with the
timestamp encoded in its id; a hand-written file that disagrees is malformed
and reported during reconciliation. API events are append-only. Reversals
write the inverse event rather than modifying history.

Required references depend on the event kind:

| Kind | Required references |
|---|---|
| Connect, disconnect | `idea`, `capture` |
| Reject or reconsider an idea candidate | `idea`, `capture` |
| Reject or reconsider a capture pair | `capture`, `other_capture` |
| Affirm, retire, reopen, dismiss rediscovery | `idea` |
| Archive, restore, delete | `capture` |
| Promote | `idea`, `page` |

`actor` is required on a wiki with accounts and absent on an open accountless
wiki. A capture-pair event stores the two ids in lexical order so the same
pair has one canonical identity whichever capture produced the suggestion.

The latest applicable event wins when state is folded. Duplicate idempotent
API actions do not write duplicate events.

## Ownership and disclosure

Idea Inbox is personal working state in the first implementation.

- On a wiki with no accounts, captures and ideas have no owner and the one open
  user can access them.
- On a wiki with accounts, every capture, idea and event is owned by the
  authenticated account that created it.
- Idea endpoints never answer an unauthenticated caller, including when
  `RHIZOLOG_ANONYMOUS_READ` is enabled.
- An account may only read, search, modify, connect, reject or score its own
  captures and ideas. The owner role does not bypass this through the API.
- A capture may only connect to an idea with the same owner.
- An unreadable id returns the same `404` and error shape as a missing id.

There is no `public`, `internal` or `restricted` setting for captures in this
slice. Sharing happens by promoting material to a normal page, where the
existing page visibility model applies. This keeps similarity scores, shared
terms, counts and receipts from becoming indirect existence oracles for
another account's private working material.

Account deletion does not delete idea data. It follows the current private-page
precedent: the files remain on disk, and recovery requires filesystem access or
a separately designed ownership-transfer operation.

### Open records are adopted by the first account

Owner comparison is equality, so a capture written while the wiki had no
accounts belongs to no account once one exists, and its author cannot reach it
through the API any more. That is what the rule above says on its own, and it is
not what anybody wants on the day they turn authentication on: it costs somebody
their whole inbox for doing the thing the documentation told them to do.

**Decided: creating the first account stamps `owner:` onto every unowned
capture, idea and event.** The open user and the first account are the same
person, on a single-user wiki that has just been pointed at a network. Reading
the other two answers out loud settles it. Leaving the files unowned is honest
and unhelpful. Treating an unowned record as readable by every account leaks
working notes the moment a second account is added, which is exactly the
boundary the private-owner design exists to hold, and it leaks them silently.

Two things this does not decide, both for I2:

- **Where adoption runs.** There is no single moment the API controls, because
  an account can also be created by dropping a file into `.rhizolog/users/`,
  and `UserStore::count` is answered from the directory on every request so
  that this works immediately. Adoption in the create-account handler alone
  would miss it. Startup reconciliation sees both paths and is the obvious
  second half, with the condition being "there is exactly one account and there
  are unowned idea records".
- **That it is a one-way write.** It rewrites authored files, so it is a
  migration rather than a view, and it should log what it touched and be
  refused rather than guessed at if there is more than one account by the time
  it runs. Two accounts and a pile of unowned captures is a question only a
  person can answer.

I0 implements neither, and deliberately has no code path that infers an owner:
it is the phase that decides what a file means, not what a migration does.

## Derived index

Bump the derived schema version and add tables equivalent to:

```sql
idea_captures(id PK, owner, created, updated, size)
idea_captures_fts                              -- id, body
idea_threads(id PK, owner, name, created, updated, size)
idea_seed_captures(idea_id, capture_id)        -- immutable creation evidence
idea_events(id PK, owner, kind, idea_id, capture_id, other_capture_id,
            page_slug, created, updated, size)
idea_membership(idea_id, capture_id)           -- folded current state
idea_rejections(idea_id, capture_id)           -- folded current state
idea_capture_rejections(capture_id, other_capture_id)
idea_capture_state(capture_id, archived)       -- folded current state
idea_thread_state(idea_id, retired, promoted_to, last_signal)
idea_terms(capture_id, term, occurrences)      -- unigrams and bigrams
```

The exact table split may change if the queries read more clearly another way,
but these invariants may not:

- Every row is rebuildable from `.rhizolog/ideas/`.
- No idea table belongs in the durable half of `index.db`.
- Owner filtering happens in SQL before returning text, counts, terms,
  candidates or receipts.
- Removing or editing an event recomputes the affected idea's folded state.
- Removing or editing a capture recomputes its terms and every affected idea.
- A schema-version rebuild restores the same current membership and lifecycle
  inputs as an incremental update.
- Capture FTS remains separate from page and time search because its result
  shape and privacy boundary are different.

Deleting a capture removes its authored text and term rows. Its deletion event
and older decision events remain as an audit trail but expose no deleted body.
Membership folding ignores an absent capture, and receipts show a deleted
evidence reference rather than silently attributing text that no longer exists.
If an external deletion leaves a non-retired idea with no live capture, the
idea has an `evidence_missing` integrity diagnostic and no lifecycle label or
momentum until a capture is restored, another is connected, or the idea is
retired. The dashboard groups it under Needs repair. Do not manufacture a
lifecycle answer from missing evidence.

Extend `SyncReport` with separate counts for captures, ideas and events rather
than hiding all three behind the page count. Server startup logs each tree and
treats malformed idea files as non-fatal reconciliation failures, following
the page and time behaviour.

## File watching and synchronous writes

Extend the existing watcher rather than adding a second watcher.

- Add capture, idea and event targets to `Reindex::Targets`.
- Teach internal-path classification about `.rhizolog/ideas/` while continuing
  to ignore the database, endpoint file, accounts and temporary files.
- Reindex a changed file directly when its id is recoverable.
- Force a full idea rescan for directory changes or paths that cannot be
  classified safely.
- Recompute an idea after an event is added, changed or removed.
- Keep API-write echoes harmless through idempotent reindexing.

API writes follow the existing file-first sequence: write the authored file
atomically, synchronously update the derived index, then return. If indexing
fails after the file is written, return an error that says the authored write
succeeded and a reindex is required. Do not delete the authored file to make
the index appear successful.

`POST /api/captures` does not calculate candidates before returning. The UI
requests candidates after it has the successfully written capture.

## Lexical analysis version 1

The first analyzer uses no new model or service.

### Terms

1. Read the capture body as text.
2. Lowercase with Rust's Unicode lowercase conversion.
3. Split tokens at characters for which `char::is_alphanumeric` is false.
4. Keep non-empty unigrams in source order.
5. Add adjacent bigrams using a visible separator that cannot collide with a
   unigram.
6. Count occurrences. Do not stem, guess synonyms or maintain a stop-word
   list.

TF-IDF itself downweights words common across the owner's corpus. The absence
of stemming and synonym expansion is a deliberate explainability trade: every
signal shown to the user appears in text they actually wrote.

### Weighting and similarity

For one owner's `N` captures:

```text
tf(term, capture) = occurrences / total terms in that capture
idf(term) = ln((1 + N) / (1 + documents containing term)) + 1
weight = tf * idf
similarity = cosine(capture vector, idea centroid)
```

An idea centroid is the arithmetic mean of the unit vectors for its currently
connected captures. Compute candidates against every non-retired idea and every
other unthreaded capture owned by the caller for the first implementation. Here,
unthreaded means the capture belongs to no current non-retired idea. A candidate
is a discriminated union whose target is either an idea or a capture:

- Connecting to an idea appends a membership event.
- Connecting to a capture creates a user-named idea with both captures.

This is what lets an idea emerge from two similar captures before a thread
already exists. A capture may belong to more than one idea; branching is not a
classification error. Do not introduce approximate search before measurement
demonstrates a need.

Return the three highest candidates across both target kinds at or above
`0.35`. Exclude ideas to which the capture is already connected, captures that
already share an idea with it, and targets for which the current rejection
state is true. A reconsider event makes the target eligible again. Archived
captures remain eligible because rediscovering an older thought is part of the
feature.

For each candidate, return up to five shared signals ordered by their
contribution to the cosine dot product. The UI describes the number as lexical
similarity, not as a probability that the ideas are related.

The candidate response includes `analyzer: "tfidf/v1"`. Changing tokenization,
weighting, thresholds or centroid construction is an analysis-version change
and requires fixture updates plus a note on this page.

### Analysis failure

Candidate generation is derived and retryable. A missing or incomplete term
index returns a stable analysis error and offers reindexing. It must never make
capture creation fail retroactively, remove a capture, or create a connection.

## Lifecycle and momentum version 1

Lifecycle is a pure function of the folded authored state and an explicit
`at` timestamp. Store neither the label nor the score.

Run the integrity check first. An `evidence_missing` idea returns its diagnostic
and missing ids with `state` and `momentum` absent; it does not enter the rules
below until it has live evidence again or is explicitly retired.

For an idea at time `at`:

```text
total = number of currently connected captures
recent_14 = connected captures created in [at - 14 days, at]
recent_30 = connected captures created in [at - 30 days, at]

base = min(total, 4)
recency = 2 if recent_14 >= 3
          1 if recent_30 >= 1
          0 otherwise
affirmation = 1 if an interest_affirmed or idea_reopened event occurred
                in [at - 30 days, at]

momentum = min(base + recency + affirmation, 10)
```

`last_signal` is the latest of a connected capture's creation time and an
affirm, connect, reopen or promote event. State is selected in this order:

1. `Retired` when the latest retire or reopen event says retired.
2. `Dormant` when `at - last_signal` is at least 60 days.
3. `New` when exactly one capture is connected.
4. `Active` when momentum is at least 4.
5. `Recurring` otherwise.

Archived captures remain in `total` and in the chronology. Archive means
"processed or hidden from the inbox," not "this thought never happened."
Deleted capture files do not count because there is no authored evidence left.

Every lifecycle response includes:

- `ruleset: "idea-momentum/v1"`
- `computed_at`
- the state and momentum
- every component value used above
- the ids and timestamps of the captures and events used as evidence
- a short factual explanation assembled from fixed templates

The API must be able to answer with an explicit `at` query parameter. The UI
normally asks for now; tests and historical inspection can supply a timestamp.
No background scheduler is needed for dormancy because time-dependent state is
computed when read.

## API surface

Ids contain no slash, so these routes use ordinary path parameters rather than
page-style wildcards.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/captures` | Newest-first inbox; `q`, `archived`, `from`, `to`, `limit`, `offset` |
| `POST` | `/api/captures` | Save text with a server timestamp and owner |
| `GET` | `/api/captures/{id}` | Read one capture |
| `PATCH` | `/api/captures/{id}` | Correct its body |
| `DELETE` | `/api/captures/{id}` | Permanently delete after explicit confirmation |
| `POST` | `/api/captures/{id}/archive` | Append an archive event |
| `POST` | `/api/captures/{id}/restore` | Append a restore event |
| `GET` | `/api/captures/{id}/candidates` | Top explainable idea candidates |
| `GET` | `/api/ideas` | Ideas grouped or filtered by lifecycle state |
| `POST` | `/api/ideas` | Create a named idea from one or more captures |
| `GET` | `/api/ideas/{id}` | Thread, chronology, current state and receipt |
| `PATCH` | `/api/ideas/{id}` | Rename it or edit its note |
| `PUT` | `/api/ideas/{id}/captures/{capture_id}` | Idempotently connect a capture |
| `DELETE` | `/api/ideas/{id}/captures/{capture_id}` | Disconnect, refusing the last member |
| `PUT` | `/api/ideas/{id}/rejections/{capture_id}` | Idempotently reject this candidate |
| `DELETE` | `/api/ideas/{id}/rejections/{capture_id}` | Reconsider a rejected candidate |
| `PUT` | `/api/captures/{id}/rejections/{other_id}` | Idempotently reject a capture pair |
| `DELETE` | `/api/captures/{id}/rejections/{other_id}` | Reconsider a rejected capture pair |
| `POST` | `/api/ideas/{id}/affirm` | Record deliberate current interest |
| `POST` | `/api/ideas/{id}/retire` | Retire without deleting history |
| `POST` | `/api/ideas/{id}/reopen` | Reopen a retired idea |
| `POST` | `/api/ideas/{id}/dismiss` | Suppress rediscovery for 30 days |
| `GET` | `/api/ideas/{id}/receipt` | Receipt at optional `at` |
| `GET` | `/api/ideas/{id}/draft` | Markdown assembled from selected captures |
| `PUT` | `/api/ideas/{id}/promotion` | Record the existing page slug it produced |

List endpoints default to `50`, accept at most `200`, and return `total`,
`limit` and `offset`. Empty search text means no search and should be omitted by
the dashboard rather than sent as `q=`. Capture search narrows the chronological
listing and does not reorder it by relevance, following the time-log precedent.
The ideas listing accepts `state`, `integrity` and `at`; an absent `at` means
the server's current time.

Permanent capture deletion is refused with `409 capture_required_by_idea` when
it is the last live member of a non-retired idea. Otherwise it first appends
`capture_deleted`, then removes the capture file and its derived text. If the
event write fails, the file remains. The response names affected ideas without
returning any other capture bodies so the UI can explain the consequence before
refreshing them.

Use semantic operation ids such as `list_captures`, `create_capture`,
`connect_idea_capture` and `read_idea_receipt`; operation ids are global in the
generated document. Every schema and operation needs a real OpenAPI description
and example, following [API design](api-design.md).

Errors keep the standard envelope and stable codes. At minimum cover:

- `capture_not_found`
- `idea_not_found`
- `idea_would_be_empty`
- `capture_required_by_idea`
- `idea_owner_mismatch`
- `idea_already_retired`
- `idea_not_retired`
- `idea_analysis_unavailable`
- `idea_promotion_page_not_found`

The promotion endpoint does not create a page. Promotion is deliberately two
steps:

1. `GET /api/ideas/{id}/draft` returns chronological Markdown and provenance.
2. The caller creates a normal page through the existing page API.
3. `PUT /api/ideas/{id}/promotion` records that readable page slug
   idempotently.

This avoids pretending that two authored file writes and two index updates are
one transaction. If step 3 fails, the page still exists and the caller can
retry the idempotent association without losing work.

Recording the same promotion slug twice writes no second event. Recording a
different slug writes a new promotion event and makes that slug the current
`promoted_to` value while preserving the earlier association in history.

## Backend module seams

A reasonable layout is:

```text
backend/src/
  ideas/
    mod.rs             # ids and file formats
    store.rs           # atomic authored-file operations and walks
    analysis.rs        # tokenization, TF-IDF and explanations
    lifecycle.rs       # pure state and receipt calculation
  index/
    ideas.rs           # idea queries, folding, FTS and term rows
  api/
    ideas.rs           # capture and idea routes
```

Expected integration points:

- Export `IdeaStore` and the public idea types from `lib.rs`.
- Add `ideas: IdeaStore` to `AppState`.
- Open and log the store in `server::start`.
- Pass it through startup reconciliation, manual reindex and the watcher.
- Extend `SyncReport`, index clear/rebuild and manual-reindex responses.
- Keep global health counts unchanged; owner-scoped idea counts come from the
  idea endpoints and must not leak through `/api/health`.
- Register every route through `OpenApiRouter` and add an `ideas` tag.
- Regenerate `frontend/openapi.json` from `cargo run --example dump-openapi`.
- Regenerate `frontend/src/api/schema.d.ts` through `pnpm gen:api`.

Do not put `IdeaStore`, page `Store`, `Index` or any other privileged access in
`desktop/`. The desktop product continues to use the same HTTP API as a remote
browser.

## Dashboard

Add three routes:

| Route | Surface |
|---|---|
| `/inbox` | Capture, chronological inbox, candidate prompts and one rediscovery card |
| `/ideas` | Active, Recurring, New, Dormant, Retired and Needs repair ideas |
| `/ideas/:id` | Timeline, membership controls, receipt and promotion |

### Inbox

- Put the text field first and focus it on an explicit capture action.
- `Ctrl+Enter` submits on desktop; the visible button remains the mobile path.
- Keep the text in the form until the create request succeeds.
- Fetch candidates only after the create response and show their shared
  signals before Connect and Keep separate actions.
- Show captures newest first with compact Archive, Restore and Add to idea
  actions.
- Offer one deterministic rediscovery card at most. Do not build an infinite
  generated feed.

### Ideas list

- Group by lifecycle state and show capture count, momentum and last signal.
- Group `evidence_missing` ideas under Needs repair without inventing a score.
- Do not show a score without a link or control that opens its receipt.
- Keep Retired collapsed by default.
- Put filters in the URL so an Active or Dormant view is linkable.

### Idea detail

- Treat the receipt as the centrepiece, not an advanced debug drawer.
- Present the fixed explanation first and the arithmetic immediately below it.
- Show every evidence row as a link to the capture.
- Show the chronological capture timeline separately from decision events.
- Support Connect, Disconnect, Reconsider, Affirm, Retire and Reopen with the
  semantic API operations above.
- Promotion opens a page draft, asks for the ordinary page fields, creates the
  page through the existing API, then records the idempotent promotion link.

### Responsive shell

The current horizontal app navigation is too dense for a phone. Replace the
standalone New button with a compact menu containing Capture and New page, and
collapse the full navigation behind a menu at narrow widths. Capture is the
primary mobile action; page creation remains reachable and stays prominent on
the page and dashboard surfaces.

Check the whole shell, not only the new routes, at approximately 375, 768 and
desktop widths. The graph and editor may remain desktop-oriented, but navigation
must not overflow or hide account, timer and pin controls.

## Rediscovery

The MVP has in-app rediscovery only. On opening `/inbox`, select at most one
eligible idea:

- dormant and not retired
- not affirmed or dismissed in the last 30 days
- at least two connected captures

Selection must be stable for one owner and local calendar date so refreshing
does not rotate through cards. Dismiss appends `rediscovery_dismissed`; Still
interested appends `interest_affirmed`. Merely rendering the card does not
create authored data or momentum.

Do not add operating-system notifications until the in-app card demonstrates
that resurfacing is useful and its frequency has been measured.

## Implementation phases

Each phase ends with a green, reviewable repository state and is a reasonable
logical commit boundary. Stage exact paths and do not include unrelated work.

### I0: authored model and store

**Built.** See [What I0 and I1 actually built](#what-i0-and-i1-actually-built).

- Add ids, capture, idea and event parsing and serialization.
- Add atomic create, read, patch, delete and walk operations.
- Enforce owner and cross-record invariants in a domain service, not only in
  handlers.
- Cover BOM, CRLF, malformed frontmatter, wrong-month paths, collision nudging,
  temporary files and directory pruning.

Done when files round-trip, ids cannot escape their roots, and the store can be
reopened over data created through the API-facing drafts.

### I1: derived index, reconciliation and watcher

**Built.** See [What I0 and I1 actually built](#what-i0-and-i1-actually-built).

- Add schema and index operations.
- Fold decision events into current membership and state inputs.
- Add startup sync, full rebuild and targeted external-edit handling.
- Prove incremental state equals a clean rebuild after create, edit, inverse
  event and deletion mutations.

Done when deleting `index.db` and restarting produces exactly equivalent
API-visible idea state for a fixed `at` timestamp.

### I2: capture and idea API

- Add owner-scoped CRUD and decision operations.
- Add uniform errors, limits, OpenAPI schemas and unique operation ids.
- Add accountless-open and account-owned integration tests.
- Regenerate and inspect the OpenAPI outputs.

Done when an API client can capture text, create an idea, connect and reject
captures, reverse those decisions, retire and reopen the idea, and see the same
state after a clean rebuild.

### I3: explainable analysis and lifecycle

- Implement TF-IDF version 1 and shared-signal contributions.
- Implement the pure lifecycle function and receipt.
- Use small fixed corpora with exact expected candidates and arithmetic.
- Add adversarial cases: empty text, repeated one-word captures, identical
  captures, all-common terms, deleted evidence, rejected candidates, mixed
  owners and timestamps exactly on each boundary.

Done when every candidate and lifecycle claim can be reconstructed from its
response without reading implementation code.

### I4: responsive dashboard

- Add typed client functions and the three routes.
- Implement capture success and failure behaviour before candidate UI.
- Add Ideas and receipt views, then decision actions.
- Repair the narrow app shell without regressing existing routes.
- Add component tests around request composition, stale resources, errors,
  retry behaviour and preserved capture drafts.

Done when desktop and narrow-browser users can complete the full loop without
Swagger UI: capture, connect, inspect why, reject a wrong suggestion, retire,
reopen and rediscover.

### I5: promotion and documentation closure

- Add the draft and promotion endpoints.
- Create a page through the existing page contract and record the association.
- Update Architecture, API design, The dashboard, Product vision, `AGENTS.md`
  and `TODO.md` from implemented behaviour.
- Keep this page as the rationale and change its status from implementation
  plan to implemented, naming any deliberate differences.

Done when promotion preserves every source capture, creates an ordinary page
with ordinary visibility, and leaves a retryable path if recording the
promotion association fails.

## What I0 and I1 actually built

`backend/src/ideas/{mod,store,service}.rs` are the authored half:
the three file formats, the three trees on disk, and the rules. `index/ideas.rs`
is the derived half, and `index/schema.rs`, `index/sync.rs`, `watcher.rs`,
`server.rs` and `api/mod.rs` carry it into startup, reconciliation and live
pickup of external edits.

### Nothing is created until something is written

`IdeaStore::open` does not create its directories, which is a difference from
`TimeStore` and worth the paragraph it cost to find out.

`server::start` opens the stores and then watches the wiki directory. A store
that creates directories on open is therefore the server writing into the tree
it is about to watch, and Windows reports those creations *after* the watch is
established. That lands a spurious full rescan in the watcher's first debounce
window, and a rescan is not harmless there: it indexes files whose own create
events are still pending, so a file created and then deleted inside one window
correctly collapses to no event at all and the index keeps a row for a file that
is gone until the next scan. It showed up as a one-in-three flake in
`tests/watcher.rs` and took a while to stop looking like a lost event.

So the trees appear on first write. A wiki that has never captured a thought has
no `ideas/` directory to explain, back up or wonder about, which is the better
answer anyway.

### Where the code departs from the plan above

- **There is an `ideas/service.rs`**, which the module seam list did not name.
  Every rule that is not "does this file parse" lives there: text that is not
  blank, a name that is not blank, seeds that exist, and the owner check. The
  store stays unaware of who is asking, because reconciliation has to walk every
  record on a wiki with accounts without pretending to be somebody. `AppState`
  should therefore hold an `IdeaService` and reach the store through
  `IdeaService::store()`, rather than holding an `IdeaStore` directly.
- **A record that belongs to another owner is reported missing**, not
  forbidden, which is the disclosure rule above expressed in the service rather
  than in a handler. Because every record an operation touches is checked
  against the caller, "a capture may only connect to an idea with the same
  owner" holds by transitivity and is not asserted a second time. A
  hand-written file that breaks it is reconciliation's to diagnose.
- **One `RecordError` covers all three file formats**, rather than a parse
  error type per record. They are three spellings of one thing: an authored
  file under `.rhizolog/ideas/` that does not parse, skipped the same way
  whichever tree it came from.
- **An event is only its frontmatter.** Anything written after it is ignored
  rather than round-tripped. Events are append-only and no path rewrites one,
  so nothing is lost, and a second place to write a note about an idea would be
  one place too many.
- **An absent `created` is recovered from the id**, on all three record types.
  The id was minted from that instant and carries the nanoseconds, so this
  invents nothing, and it means a capture typed straight into a file with no
  frontmatter at all is still a capture.
- **An event's `created` must agree with its id to the whole second**, which is
  the rule this page already states, enforced at parse time. A file that
  disagrees is malformed rather than silently reordered.
- **An idea may be started from at most `MAX_SEEDS` captures**, currently 200,
  the same ceiling the list endpoints take. A seed list is a page of captures
  somebody selected, and one longer than a page could not have been.
- **There is no way to rewrite or delete an event, and no way to delete an
  idea.** The store offers neither, so append-only and retire-rather-than-delete
  are properties of the type rather than conventions the API layer has to
  remember. Captures can be deleted, because the API offers that.

`EventDraft::new` validates by running the *reader's* rules over the references
the draft would write, so there is one place that decides which references a
kind may carry and a draft that exists is one the next startup can parse. That
is the "never ship a writer for a format the reader refuses" rule expressed as a
constructor, and it is also what puts a capture pair in lexical order without
every call site having to remember to.

### The fold is SQL, and that is why a rebuild agrees with an update

The plan's table list is built as written, minus `idea_terms`, which belongs to
the analyzer and arrives with it: a table nothing writes yet is worse than a
second schema bump, and a bump costs one scan by design.

Every folded table is recomputed by one statement over `idea_seed_captures` and
`idea_events`, keyed on whatever just changed. There is no separate rebuild
path, so `a_rebuild_reproduces_the_folded_idea_state` is checking that one query
gives the same answer twice rather than that two algorithms agree. Each of those
statements ends in `order by id desc limit 1`, which is the latest-decision-wins
rule spelled in SQL; event ids sort chronologically as text, so ordering by id
is ordering by when the decision was taken.

Three consequences worth knowing:

- **A fold for a record that has not been indexed writes nothing** rather than
  failing. An event can be read before the thread it names, and a scan makes no
  promises about order, so this is the ordinary case rather than an error. The
  thread's own upsert folds it again.
- **`last_signal` is the only folded value that reads outside the event log**,
  because it takes the creation time of connected captures. That is why indexing
  or removing a capture recomputes every idea naming it, and why the idea tables
  are not a pure function of `idea_events` alone.
- **A rejection needs its capture to still exist**, both for an idea candidate
  and for a capture pair. A rejection is a standing instruction not to suggest
  something, and a capture that is gone cannot be suggested. The decision itself
  is not lost, because the event is still there to read.

`idea_membership` and `idea_seed_captures` deliberately carry no foreign key to
`idea_captures`. That is what makes `evidence_missing` observable: live
membership is the membership table joined to the captures, and whatever the join
drops is the evidence the idea has lost. A cascade would tidy away the symptom.

### Owner filtering is one clause, `owner is :owner`

`is` rather than `=`, and it is load-bearing. An open wiki's records have a null
owner, and in SQL `null = null` is null, which a `where` clause reads as false:
under `=` the open user would be unable to see a single thing they had written.
`is` compares nulls as equal, so the open user matches exactly the open records
and an account matches exactly its own.

### `SyncReport` counts five trees

Captures, threads and events are counted apart from each other rather than
totalled. They are read by different code and go wrong in different ways, and a
scan reporting "412 idea files" when one thread has gone missing is a number
nobody can act on. `POST /api/reindex` returns all five, and `server::start`
logs one line each.

## Test strategy

### Pure unit tests

- All three id parsers and path conversions
- Capture, idea and event file round-trips
- Event folding, inverse events and idempotence
- Tokenization, unigram and bigram counts
- TF-IDF weights, cosine similarity and contribution ordering
- Lifecycle boundaries at 14, 30 and 60 days
- Fixed factual explanation templates
- Deterministic rediscovery selection

### Backend integration tests

- API write is immediately readable and searchable
- External file create, edit and delete are picked up
- Full rebuild equals incremental state
- Candidate rejection survives restart and suppresses the same idea
- A reconsider event restores eligibility
- No account can infer another owner's ids, text, terms, counts or scores
- An accountless wiki remains open for idea writes
- Anonymous-read mode still refuses every idea endpoint
- Promotion refuses a missing or unreadable page without revealing which
- Every error uses the standard envelope
- Every OpenAPI operation id is unique and every new schema is documented

### Frontend tests

- Capture text remains after a failed request and clears only after success
- Empty capture is refused locally without a request
- Candidate responses show shared signals and never connect automatically
- Connect and reject update the visible state from server responses
- A failed resource stays inside its route and does not take down the shell
- URL filters round-trip and an empty search sends no `q`
- Receipt arithmetic and evidence links render from server data
- Retire and reopen are reversible
- Promotion retries only the missing association when the page already exists
- Mobile navigation exposes Capture, New page, timers, pins and accounts

### Performance evidence

Generate scratch corpora of 1,000, 5,000 and 10,000 captures without touching
`example-wiki/`. Measure startup reconciliation, full rebuild and candidate
query time three times per size and retain the minimum plus all raw samples.
The first release gate is roughly linear growth and an interactive candidate
request at 10,000 captures on the development machine. Record the actual
numbers before deciding whether a centroid cache or bounded candidate corpus
is necessary.

Do not use one noisy run as evidence and do not introduce approximate search
without a measured failure.

## Validation commands

Stop a running server before Cargo links the executable.

From `backend/`:

```powershell
cargo fmt --check
cargo test -p rhizolog
cargo clippy -p rhizolog --all-targets
cargo run --example dump-openapi
```

From `frontend/`:

```powershell
pnpm gen:api
pnpm typecheck
pnpm test
pnpm build
```

Then, with `frontend/dist` current, from `backend/`:

```powershell
cargo test -p rhizolog --features embed-assets
cargo test --workspace
```

Inspect the generated OpenAPI diff rather than treating generation as proof.
Click the real built UI against a scratch wiki, including a network-served
account session and narrow viewports. Check `git status example-wiki` after any
manual run and leave the fixture unchanged.

## Rollback and partial completion

Each phase must leave authored files readable by the previous completed phase.
Derived schema changes roll back by deleting `index.db` and rebuilding with the
checked-out code. Do not claim that this makes authored format changes
reversible.

Before changing an authored file format, either keep the old spelling readable
or add an explicit conversion plan. Never ship a writer for a format the next
startup cannot read.

If analysis or the dashboard is incomplete, the capture API and authored store
may still be delivered as a useful partial slice, but this page and `TODO.md`
must say exactly which phase is complete. Do not describe capture alone as the
Idea Inbox MVP: the explainable recurrence loop is the feature being tested.

## Decisions to revisit only after use

- Sentence embeddings, after real false negatives show a lexical limitation
- Shared idea threads, after the private owner boundary proves too restrictive
- Notifications, after in-app rediscovery has useful acceptance and dismissal
  rates
- Approximate nearest-neighbour search, after measured local scale requires it
- Automatic names or summaries, only with a traceable non-authoritative design
- Whether promoted ideas should remain in rediscovery, after promotion is used

None is a prerequisite for this implementation.
