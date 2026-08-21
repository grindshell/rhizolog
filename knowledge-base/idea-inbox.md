# Idea Inbox

Status: **built**. Phases I0 through I5 are complete: the authored model and
store, the derived index that folds decisions into current state, the
owner-scoped HTTP API over both, the explainable half (`tfidf/v1` candidates and
`idea-momentum/v1` lifecycle receipts), the dashboard over the lot, and the way
out into the wiki. The whole loop can be walked in a browser at 375 pixels wide:
capture, connect, see why, reject a wrong suggestion, retire, reopen, rediscover
and promote.

This page began as the implementation plan and is now the record of one. The
phases and their completion gates are kept rather than deleted, because they say
what each slice had to prove and a later change still has to keep proving it;
[What is built so far](#what-is-built-so-far) names every place the code
departed from the plan above it. Where the two disagree, the code is what runs
and this page is why.

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

**Decided and built: creating the first account stamps `owner:` onto every
unowned capture and thread, and `actor:` onto every unowned event.** See
`backend/src/ideas/adoption.rs`. The open user and the first account are the same
person, on a single-user wiki that has just been pointed at a network. Reading
the other two answers out loud settles it. Leaving the files unowned is honest
and unhelpful. Treating an unowned record as readable by every account leaks
working notes the moment a second account is added, which is exactly the
boundary the private-owner design exists to hold, and it leaks them silently.

It runs in **two places**, because there is no single moment the API controls:
an account can also be created by dropping a file into `.rhizolog/users/`, and
`UserStore::count` is answered from the directory on every request so that this
works immediately. The create-account handler alone would miss that, and startup
alone would leave the inbox invisible between creating an account through the
API and restarting. Both, and idempotent, costs a second run that finds nothing.

It rewrites authored files, so it is a migration rather than a view. It logs
what it touched, it is written to survive being interrupted, and it refuses to
act when there is more than one account: two accounts and a pile of unowned
captures is a question only a person can answer.

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

**Built.** See [What is built so far](#what-is-built-so-far).

- Add ids, capture, idea and event parsing and serialization.
- Add atomic create, read, patch, delete and walk operations.
- Enforce owner and cross-record invariants in a domain service, not only in
  handlers.
- Cover BOM, CRLF, malformed frontmatter, wrong-month paths, collision nudging,
  temporary files and directory pruning.

Done when files round-trip, ids cannot escape their roots, and the store can be
reopened over data created through the API-facing drafts.

### I1: derived index, reconciliation and watcher

**Built.** See [What is built so far](#what-is-built-so-far).

- Add schema and index operations.
- Fold decision events into current membership and state inputs.
- Add startup sync, full rebuild and targeted external-edit handling.
- Prove incremental state equals a clean rebuild after create, edit, inverse
  event and deletion mutations.

Done when deleting `index.db` and restarting produces exactly equivalent
API-visible idea state for a fixed `at` timestamp.

### I2: capture and idea API

**Built.** See [What is built so far](#what-is-built-so-far).

- Add owner-scoped CRUD and decision operations.
- Add uniform errors, limits, OpenAPI schemas and unique operation ids.
- Add accountless-open and account-owned integration tests.
- Regenerate and inspect the OpenAPI outputs.

Done when an API client can capture text, create an idea, connect and reject
captures, reverse those decisions, retire and reopen the idea, and see the same
state after a clean rebuild.

### I3: explainable analysis and lifecycle

**Built.** See [What is built so far](#what-is-built-so-far).

- Implement TF-IDF version 1 and shared-signal contributions.
- Implement the pure lifecycle function and receipt.
- Use small fixed corpora with exact expected candidates and arithmetic.
- Add adversarial cases: empty text, repeated one-word captures, identical
  captures, all-common terms, deleted evidence, rejected candidates, mixed
  owners and timestamps exactly on each boundary.

Done when every candidate and lifecycle claim can be reconstructed from its
response without reading implementation code.

### I4: responsive dashboard

**Built.** See [What is built so far](#what-is-built-so-far).

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

**Built.** See [What is built so far](#what-is-built-so-far).

- Add the draft and promotion endpoints.
- Create a page through the existing page contract and record the association.
- Update Architecture, API design, The dashboard, Product vision, `AGENTS.md`
  and `TODO.md` from implemented behaviour.
- Keep this page as the rationale and change its status from implementation
  plan to implemented, naming any deliberate differences.

Done when promotion preserves every source capture, creates an ordinary page
with ordinary visibility, and leaves a retryable path if recording the
promotion association fails.

## What is built so far

`backend/src/ideas/{mod,store,service,adoption}.rs` are the authored half: the
three file formats, the three trees on disk, the rules, and the one migration.
`ideas/{analysis,lifecycle}.rs` are the explainable half, and both are pure
functions of stated inputs with no access to the index or the disk.
`index/ideas.rs` is the derived half and the seam between them: it gathers what
the two pure modules read and hands it over whole. `index/schema.rs`,
`index/sync.rs`, `watcher.rs` and `server.rs` carry all of it into startup,
reconciliation and live pickup of external edits, and `api/ideas.rs` is the
twenty-five endpoints over the lot.

In `frontend/`, `routes/{Inbox,Ideas,IdeaDetail}.tsx` are the three screens,
`components/Candidates.tsx` is the suggestion panel both of the first two use,
`components/Rediscovery.tsx` is the card and the pure function that chooses it,
and `api/client.ts` grew the twenty-five typed calls. `components/Layout.tsx`
is the shell, rebuilt for a phone.

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

The plan's table list is built as written. `idea_terms` arrived a version later
than the rest, with the analyzer that fills it: a table nothing writes yet is
worse than a second schema bump, and a bump costs one scan by design.

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

### Deciding is `PUT` and `DELETE`, and acting is `POST`

Connecting a capture to an idea is `PUT /api/ideas/{id}/captures/{capture}` and
disconnecting it is the `DELETE`. The state being asked for is in the URL, so a
client that repeats itself changes nothing and writes no second event. That is
the plan's "duplicate idempotent API actions do not write duplicate events",
enforced by the shape of the route rather than by a check somebody has to
remember.

Archive and restore are `POST` because they read as acts, and they are
idempotent anyway. Affirm and dismiss are `POST` and are *not* idempotent, which
is right: affirming twice is two affirmations at two times and both are real.

Retiring what is already retired is a `409` rather than a no-op. That is not
inconsistent with the paragraph above: a caller retiring twice believes the
state is something it is not, and telling it so is more use than an event nobody
asked for.

### The API's own additions to the plan

- **`Viewer::owner`** refuses an anonymous caller rather than mapping it to the
  open user. `Viewer::username` returns `None` for both, and they mean opposite
  things: an open wiki has one user and nothing to withhold, while an anonymous
  request on a wiki with accounts is nobody. The gate already refuses every idea
  route to anonymous callers and `RHIZOLOG_ANONYMOUS_READ` lists none of them;
  this is the second lock, because the first one is a list somebody could add to
  by accident.
- **`written_but_not_indexed`** is a new error code, for the case the plan
  describes: the authored file is on disk and the index would not take it. It
  says so rather than reporting a generic failure, because a caller told only
  "internal error" would retry and write the record a second time.
- **The note is read from the thread's file, not the index.** `idea_threads`
  carries no `note` column: notes are usually empty, they are needed only by the
  detail view, and the time log already sets the precedent of keeping the prose
  on disk and a flag in the index. A file that cannot be read at that instant
  costs the note rather than the request.
- **`idea_analysis_unavailable` is a `503`, not a `500`.** Nothing is broken and
  nothing is lost: the capture is on disk and the derived half is behind, which a
  reindex fixes. 503 is the status that means come back rather than something
  went wrong, and like `written_but_not_indexed` its message is put on the wire
  rather than swallowed, because the caller's next move is specific and the
  response is the only place to say what it is.
- **Every float on the wire is rounded to six decimal places**, in one place, so
  a client comparing two numbers is comparing them at the same precision.
- **An idea carries `dismissed`**, which the plan's tables do not mention. It is
  the one rediscovery input a caller cannot work out for itself, and it arrived
  with the card that needed it. See
  [The rediscovery card is chosen in the browser](#the-rediscovery-card-is-chosen-in-the-browser-and-the-server-had-to-say-one-more-thing).

### Adoption is built, and runs in both places

The handler behind `POST /api/users` runs it when it creates the first account,
which is what keeps an inbox from disappearing even for a moment, and startup
runs it after reconciliation, which is what catches an account file somebody
dropped into `.rhizolog/users/`. It is idempotent, so the second of those is
three counting queries that find nothing.

After reconciliation and never before it: the decision is made from index counts
rather than a walk of every file, and a deleted database would otherwise report
an empty inbox and adopt nothing.

`IdeaStore::adopt_event` is the one operation in the codebase that rewrites a
decision, and it exists only for this. It changes who a decision is attributed
to, never what was decided or when, so the fold is untouched and the id, which
is the fold order, does not move. It refuses an event that already names an
actor.

### The analyzer, and what 0.35 actually means

`ideas/analysis.rs` is `tfidf/v1` and it is arithmetic over one owner's captures,
exactly as written above. Three things about it are worth knowing before reading
a score.

**Bigrams roughly halve what a paraphrase can score.** A capture of six words
produces six unigrams and five bigrams, so about half of any capture's vector is
phrases. Two captures using the same words in a different order share every
unigram and no bigram, and score around 0.43;
`shuffling_the_word_order_costs_about_half_the_score` pins that. Identical text
scores 1. So 0.35 is not "a third alike": it is roughly "most of the same words,
or a good few of the same phrases", and it is deliberately hard to reach by
accident. It is also why there is no point tuning the threshold without tuning
tokenization at the same time, since they set each other's scale.

**The smoothed idf floors a ubiquitous term at 1 rather than erasing it.**
`ln((1 + N) / (1 + df)) + 1` gives exactly 1 when a term is in every capture,
where the unsmoothed form would give 0. That is the standard shape and it is the
right one here: a single-user inbox is full of the same handful of words, and a
formula that erased them would erase the signal along with the noise. The
consequence is that a corpus of one repeated thought scores near 1 against
itself, which is the honest answer to "these really do all say the same thing"
and is what the threshold and the three-candidate cap are for rather than the
weighting.

**The contributions are the score, not a summary of it.** Both vectors are unit
length, so the cosine is a dot product, and a dot product is a sum of per-term
products. The signals in a response are the actual terms of that actual sum, and
`explained` is what the listed five add up to. It is below `similarity` whenever
more than five terms were shared, and saying so is the difference between showing
the evidence and implying it is all of it. `explained` is summed from the
*rounded* contributions rather than the exact ones, so a reader adding up the
numbers in front of them arrives at the number printed beside them.

A centroid is the mean of its members' unit vectors, normalised. Normalising
matters for the same reason: without it the contributions would not sum to the
similarity, and the receipt would be approximately true.

### Lifecycle is a pure function, and the ceiling is unreachable

`ideas/lifecycle.rs` takes the folded evidence and an instant and returns the
whole receipt: state, momentum, every component, the window boundaries each was
measured against, and every capture and event that was counted with a flag saying
which window it fell in. `a_receipt_reconstructs_its_own_momentum` recomputes the
score from nothing but the response, which is the phase gate written as a test.

Two things the plan states that are worth restating as consequences:

- **`min(base + recency + affirmation, 10)` cannot reach 10.** Four plus two plus
  one is seven. The cap is implemented because the ruleset defines it and because
  a component added later must not silently change what the top of the scale
  means, but nobody should build a bar chart out of ten.
- **Dormancy is decided before the capture count**, so an idea with one capture
  and nothing since is dormant rather than new. That is the ordering the plan
  gives and it is the right way round: rediscovery looks for dormant ideas, and a
  thought from last year is exactly what it should resurface.

An `at` that comes off a query string is subtracted from, and chrono's
subtraction panics on overflow, so the boundaries saturate rather than taking the
request handler down with them.

### The listing filters on values SQL does not compute

`GET /api/ideas?state=` runs the lifecycle over every one of the owner's ideas
before anything is paginated. That is a full pass rather than a `where` clause,
and it is the deliberate trade: dormancy spelled in SQL would be a second
implementation of the rules, free to disagree with the receipt that explains
them. It also makes `total` honest under a filter, which paginating first and
filtering after cannot be.

The cost is bounded by how many threads a person names, which is tens. The same
is not true of candidates, where every vector is rebuilt per request; that one is
in `TODO.md` waiting for the plan's measurements rather than for a guess.

### `idea_terms` is the analyzer's output and nobody else's

The table holds what `analysis::counts` produced, which is why **changing
tokenization is a schema-version change even though it changes no DDL**. It is
separate from `idea_captures_fts` because the two answer different questions:
search wants to find a capture from a word somebody typed into a box, and this
wants weights, occurrence counts and bigrams, none of which fts5 offers without
reaching into its internals.

A capture whose text holds no terms at all, such as one that is only punctuation,
has no rows here and no vector. It still counts toward `N`, because it is still a
capture, and the candidates response says `terms: 0` so that "nothing to match
on" and "nothing matched" are distinguishable answers.

### `SyncReport` counts five trees

Captures, threads and events are counted apart from each other rather than
totalled. They are read by different code and go wrong in different ways, and a
scan reporting "412 idea files" when one thread has gone missing is a number
nobody can act on. `POST /api/reindex` returns all five, and `server::start`
logs one line each.

### The dashboard keeps saving and analyzing apart, because the API does

`POST /api/captures` returns before anything has looked at the text, and
`Inbox.tsx` is written so that nothing can quietly put the two back together.
The text stays in the textarea until the create request has resolved, the field
is cleared only then, and the candidates request is made from the *response*
rather than from the draft. `keeps the text when the request fails` and
`asks for candidates only after the capture is saved` are those two sentences as
tests, and they are the ones to keep if the file is ever rewritten: a form that
empties optimistically and then fails has thrown away the only copy of a thought
that existed, and the thought is the entire product.

The suggestion panel is the same component in both places it appears, and its
resource is read through a guard rather than directly. Reading a Solid resource
that failed *rethrows*, so an unguarded read of a `503 idea_analysis_unavailable`
would throw out of the panel and take the capture form above it down with it,
which is precisely the coupling the two requests exist to prevent. The panel
shows the error and an offer to try again, and the capture is on disk throughout.

### The inbox and the ideas listing re-read for different reasons

`/inbox` holds two resources answering two questions, and they are bumped by
different things. Capturing, archiving, restoring and deleting change what the
inbox holds; connecting a capture to a thread and deleting one out from under a
thread change what an idea holds. Connecting does not take a capture out of the
inbox, and archiving one cannot make a thread dormant, so a single counter behind
both meant re-reading two hundred ideas every time somebody archived a note.

Deleting is the one action that bumps both, which is the whole reason it is worth
separating rather than picking one: it removes a capture from the inbox *and* can
leave a thread with nothing live connected. What comes back names the ideas that
held it, and the screen says so, with a badge on any that now need repair. That
is what the response is shaped for: the ids and names and nothing else about
them, so the consequence can be explained without reading threads the person was
not looking at.

### The rediscovery card is chosen in the browser, and the server had to say one more thing

Selection is `chooseRediscovery` in `components/Rediscovery.tsx`: a pure function
of the eligible ideas and an instant, which sorts by id, hashes the reader's
**local** calendar date and indexes into the list. Local rather than UTC, because
"today" happens where the person is and a UTC date would change the card over
dinner east of Greenwich. Deterministic, because a card you can refresh past is a
feed, and this feature has spent its whole design avoiding being one. Nothing is
written by rendering it: dismissing and affirming are requests, and closing the
tab is neither.

**Answering the card puts rediscovery away until the page is loaded again.** That
is a second guard and it is not redundant, because both answers change the
eligible set: a dismissal makes an idea ineligible and an affirmation makes it no
longer dormant, so the pool shrinks by one and, with nothing else stopping it,
the next name comes straight up. Saying "not now" and being handed another
thought for having said it is the feed arriving by a different road. The flag
lives at module scope rather than in the component, so walking to an idea and
back is not a new day, and it is set only once the decision is written, because a
dismissal that never reached the server has suppressed nothing and taking the
card away would leave nothing to press again.

It deliberately does not survive a reload and nothing about it is recorded. An
authored note that somebody had been shown a card would be the engagement
bookkeeping the goals rule out. What that leaves is small and worth stating: after
a dismissal, a reload deals a different card. Closing it needs a memory of what
was offered today, which is worth building when there is evidence anybody reloads
to get one.

Three of the four eligibility rules are on the listing already. The fourth,
"not affirmed or dismissed in the last 30 days", turned out to be one rule and a
half. Affirmation needs no check at all: an affirmation moves `last_signal`, and
an idea whose last signal is inside sixty days is not dormant, so the dormancy
test has already made it. **Dismissal was invisible.** `rediscovery_dismissed`
was written and stored and folded into nothing, because nothing needed it until
something had to decide what to offer today.

So `IdeaSummary`, `IdeaSummaryView` and `IdeaView` gained a `dismissed`
timestamp, gathered by a third query in `idea_evidence` over `idea_events`. Three
things about that shape were deliberate:

- **Not a column on `idea_thread_state`**, which would have been a schema bump
  for a value nothing folds. It answers one question, asked once per listing, and
  a `max(created) group by idea_id` answers it.
- **Not part of `Evidence`.** The lifecycle must not be able to see it. Dismissing
  a card is a statement about the card, and a momentum that fell because somebody
  looked away from a thought would be the product arguing with them.
- **Not an eligibility flag.** The server returns when it happened and the client
  applies the thirty days, so the rule stays in one place beside the other three
  rather than being half here and half there.

`a_dismissal_is_visible_to_whoever_chooses_what_to_resurface` covers all of it,
including that momentum and state do not move when it is written.

### The shell gives up the wordmark before it gives up a control

The old top bar was a wordmark, six links, timers, pins, a New button and an
account menu in one row, which is fine at 1280 and off the side of the screen at
375. What replaced it renders the destinations **twice**, horizontally above
`lg` and inside a menu below it, from one `DESTINATIONS` array. One array because
two lists drift, and the one that drifts is always the one only phones see.

Capture and New page moved behind a single Create menu, with Capture first: it is
the one you reach for while walking, and it costs one text field where a page
costs a slug, a title and a decision. Its entry is `/inbox?capture=1` rather than
`/inbox`, because that is the explicit capture action and the one that should
land in the field. Opening the inbox to read it should not throw a keyboard over
half the screen.

What stays visible at every width is timers, pins, create and the account. A
timer left running overnight is the most expensive thing this bar can fail to
show. To make room, the two idle labels became glyphs below `sm`, the account
name truncates harder, and the wordmark is the only thing allowed to shrink,
because it is the only thing there that is decoration. Measured at 375 with an
account signed in, the bar comes to exactly the viewport width with the wordmark
still whole.

Checking the whole shell at 375, as this phase's brief says to, found three
overflows that had nothing to do with Idea Inbox and one cause between them:
**a grid item will not go narrower than its content, and daisyUI's `.label` and
`.stat-desc` are `nowrap`.** So a long Windows path in the dashboard's Wiki root
stat, the heat map inside its own `overflow-x-auto`, and the account form's
username hint each set the width of the page they were on. Three classes fixed
all three, and they are recorded here rather than in a commit message because
the next component to use `grid` will hit it again.

### The draft is a copy, and it says what it could not copy

`GET /api/ideas/{id}/draft` assembles a heading from the idea's name, the
thread's note if it has one, and every capture it still holds, oldest first,
separated by blank lines. Each block is trimmed at its ends and untouched
between them: the blank line a frontmatter block leaves behind and the file's
trailing newline are not part of what anybody wrote, and everything else is.
Nothing is summarised, reordered, deduplicated or interpreted. A draft that
improved on its sources would be the first place this product stopped keeping
its promise.

**Nothing in the markdown says where a paragraph came from.** Provenance is in
the response instead, as ids and timestamps, because the alternative is handing
somebody prose of ours to delete out of their own page. Connected captures whose
files are gone are listed in `missing`, so a draft that is short of something
says so rather than coming back quietly shorter. Archived captures are in it:
archived means processed, and dropping the older half of a thread would be the
wrong reading of both words.

**There is no capture selection, which the plan's endpoint table implies there
would be.** The caller creates the page themselves from markdown they can edit,
so any subset is a text edit away, and a server-side selector would be a second
way to do the same thing with its own error cases and its own answer to "what if
you name a capture the idea does not hold". The gate is stronger without it:
every source capture is in the draft, always.

Reading a draft writes nothing. It is a suggestion about a page that does not
exist, and there is nothing about it to record.

### Promotion is three steps, and only the last one is safe to repeat

The API refuses to pretend that creating a page and recording the association are
one operation, because they are two authored writes and two index updates and no
amount of API design makes them a transaction. What the three-step shape buys is
the failure it can recover from: if the page is written and the association is
not, the page still exists and `PUT /api/ideas/{id}/promotion` is idempotent, so
sending it again finishes the job rather than writing a second page.

The dashboard's panel is that sequence with a form around it, and it keeps
exactly one piece of state: the slug of a page it knows exists. The button reads
**Create the page and record it** until then and **Record the page** afterwards.
A `409 page_already_exists` sets the same flag, which gets the other case free:
somebody who wrote the page by hand first is offered a form that records it
rather than a refusal to work around.

**The page has to exist and be readable by the caller**, and a page that is
neither gets one answer, `404 idea_promotion_page_not_found`. Distinguishing them
would answer questions about another account's wiki for the price of guessing a
slug, which is the rule that already sends a private page's read to `404` rather
than `403`. It is checked against the file that was just read rather than against
the index, exactly as `GET /api/pages/{slug}` does, so a page whose frontmatter
changed a moment ago is not judged by what it used to say. A page that will not
*parse* is the one thing reported as itself: that read already reports a
malformed page to anybody before it consults visibility, so saying so here
discloses nothing new and hiding it would swallow a real fault.

**Nothing is consumed.** No capture is archived, disconnected or deleted, and the
idea is not retired: promotion is something that happened to a thread, not a way
of spending one, and the sources of a promoted page have to stay readable. What
does move is `last_signal`, because `idea_promoted` is one of the four events the
fold counts as a signal. That is the ruleset's decision and it is the right one:
writing the page up is the strongest evidence there is that the idea is live.

Recording the same slug twice writes no second event. Recording a different one
writes a new event, makes that slug the current `promoted_to`, and leaves the
earlier association in the log, because an idea that became one page and then
another has done both of those things.

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
