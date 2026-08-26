# Long-form writing

Status: **planned**. Nothing here is built. This page is the implementation
plan and the reasoning behind it; where it and the code eventually disagree, the
code is what runs and this page is why.

Rhizolog can capture a thought, turn it into a page, and say where the hours
went. What it cannot do is anything that happens after a first draft exists. A
wiki page is something you have decided. A **manuscript** is something you hand
over, on a date, at a length, in a format, and the wiki has no concept of length,
no concept of a date, and no way to turn nine pages into one document.

The writer this is for is drafting long-form work **alone, with an assistant**:
Claude, Codex or anything else reaching the same HTTP API an agent already has.
That second half is not decoration. It changes what two of the three features
have to be:

- **Compile is the context loader before it is an export.** The assistant is a
  reader of the manuscript, not only a writer into it, and today the only way to
  give it chapter nine in the light of chapter two is to paste.
- **A net word count is a broken metric** on any day an assistant rewrote two
  thousand words into nineteen hundred, so the daily figure has to record who.
- **`prose/v1` is voice defence**, not a grammar checker. Drafting alone with
  assistance, the failure is drift, and you cannot see it happening because you
  read the prose as it arrives.

The promise, in the one sentence this project makes its features fit into:

> Rhizolog assembles the manuscript, counts what is in it, and holds the prose to
> rules you wrote down.

Everything below builds on [Architecture](architecture.md), especially that files
are authoritative and SQLite is derived, and follows the discipline
[Idea Inbox](idea-inbox.md) set for anything that makes a claim: local,
deterministic, versioned, and never a number without the arithmetic and the
authored text behind it.

## Goals

- Turn a tree of pages into one document, deterministically, addressable back to
  the page and offset each part came from.
- Give a page and a manuscript a length, a target and a due date.
- Say how many words were written on a day, and by whom.
- Check prose against rules the author wrote down, in one place, with every
  finding quoting the text that produced it.
- Keep every one of those available over HTTP, so the assistant reads the
  manuscript and the rules the same way the dashboard does.

## Not goals

- A rich or WYSIWYG editor. The textarea decision in
  [The dashboard](dashboard.md) stands; an outline, a findings panel and a
  compiled preview are what people actually want when they ask for one.
- Automatic version history. See [Named milestones](#named-milestones-are-not-in-this-plan).
- `.docx` or PDF generation. Compile emits clean Markdown and HTML; pandoc is
  better at the rest than anything this repository would grow.
- Rates, invoices or billing, which [Time tracking](time-tracking.md) already
  rules out and a writing feature will pull at.
- Streaks, goals-with-encouragement or anything that congratulates you. The word
  chart is a chart, on the same terms as the hours heat map.
- Any server-side call to a model, **including inside `prose/v1`**. The whole
  value of the linter is that the rule is fixed and yours. A second machine
  opinion is the problem it exists to answer.
- Comments, editorial round-trips, submission tracking. The writer is alone.
- Span-level provenance in this plan. See
  [Provenance is per edit](#provenance-is-per-edit-not-per-sentence).

## Terms

**Manuscript:** Any page with a `target`, read as the compiled total from that
page. There is no manuscript object.

**Contents page:** A page carrying `contents: true`, whose internal links are
positions in an assembly rather than ordinary references.

**Compile:** Assembling a root page and the pages reachable through contents
pages into one document.

**Manifest:** The list compile returns beside the document: every section, in
order, with its slug, depth, word count, byte range in the output, and what
happened to it.

**Actor:** The label recording who made a write. `web`, `file`, or a name a
caller supplied.

**Finding:** One rule firing on one span, carrying the text it fired on.

## Frontmatter

Three optional fields, and they do nothing on a wiki that does not use them, in
the same way `owner` and `readers` do nothing on a wiki with no accounts:

```yaml
---
title: The Long Way Round
contents: true
target: 90000
due: 2027-03-01
---
```

- **`target`** is a word count, measured against the **compiled** total from this
  page. On a leaf page that is its own words; on a contents page it is the book.
  One rule, recursive, no second concept.
- **`due`** is a date. It is read the way `created` is: a bare date is midnight
  UTC, and a wall-clock time with no zone is refused. A due date is a day rather
  than an instant, so unlike `created` it is stored as written.
- **`contents`** turns substitution on for this page and nothing else. See below.

`due` is one field away from project management and stops there. There is no
priority, no status board and no dependency between pages.

## Word counts

`words` becomes a column on `pages`, filled at index time from the comrak AST
that link extraction already walks. One schema bump, one scan, no migration, per
[Architecture](architecture.md).

What counts is worth pinning down, because a count nobody can reproduce is a
number to argue with:

- Text inside paragraphs, headings, list items, block quotes, tables and
  footnotes.
- A wikilink's display text, not its target. `[[notes/rust/async|the async
  notes]]` is three words.
- **Not** code blocks or inline code. **Not** frontmatter. **Not** raw HTML,
  which comrak drops anyway.
- A word is a whitespace-separated run containing at least one alphanumeric
  character, over the extracted text. This disagrees slightly with what a word
  processor reports, and that is fine as long as it is stated and stable.

It gives, for free: length in the listing, `?sort=words`, and a total under any
`?prefix=`.

## Compile

```
GET /api/compile?root=book/contents&format=markdown
```

It is `/api/compile` with a query parameter rather than
`/api/pages/{slug}/compiled` for the reason that already produced `/api/move`
and `/api/prose` below: `matchit` requires a catch-all to be the final segment,
and a slug is a catch-all.

### Ordering is the links in the page, in the order they are written

A contents page is already a page: navigable, renderable, editable, greppable,
and diffable. Nothing new to create, and the table of contents cannot get out of
step with itself.

Order comes from the index, which means `links` gains an **`ordinal`** column.
The alternative is re-parsing the root at compile time, which compile could
afford since it reads bodies from disk anyway. The index wins because three
things want the tree and only one of them wants the bodies: compile, the target
rollup, and the dashboard panel. One column, one schema bump, one answer.

### Substitution, and the rule that stops it

Two conditions, and both are needed:

1. The page carries `contents: true`.
2. The link is the **entire content of a list item or a paragraph**.

Such a link is replaced, in place, by the target page's compiled body. Every
other link is left as a link.

The first condition is what keeps a chapter's prose from inlining the research
notes it cites. The second is what lets a contents page have prose in it: a part
title, an epigraph, a note to yourself, all survive compilation, and the links
between them become the sections.

This is the part of the plan most likely to be wrong, and the alternatives are
recorded in [Open questions](#open-questions) rather than discarded.

### The rest of the rules

- **Heading levels shift down by depth**, so an H1 in a chapter inserted at depth
  one becomes an H2 under the book's H1. Nothing is inserted: a page with no
  heading contributes no heading, because the manifest records the boundary and
  inventing a title would be writing words the author did not.
- **A page already emitted is skipped**, and the manifest says so. That is how
  cycles end, and it is reported rather than silently deduplicated.
- **A wanted page in the contents is a gap in the manuscript**, and the manifest
  says that too. This is exactly the stance
  [Architecture](architecture.md) already takes: an unresolved link is a branch
  somebody gestured at, and a manuscript missing a chapter should say which.
- **External links are never substituted**, whatever they are wrapped in.
- **Compile is a page-returning query**, so the audience predicate in
  `index/audience.rs` applies to every page it assembles, and an unreadable
  section is `missing` in the manifest for the same reason an unreadable page is
  a `404`. On a single-user wiki this never fires. It is stated because the rule
  is that every such query pastes it in, and a query that forgot would be the
  interesting one.

### The manifest is the reason this is not a blob

Per section: `slug`, `title`, `depth`, `words`, `offset`, `length`, and a
`status` of `included`, `missing`, `wanted`, `skipped_cycle` or `unreadable`.
Plus totals, and the analyzer-style stamp `compiler: "compile/v1"`.

Without it the output is text nothing can point into. With it, a finding over the
whole book maps back to the page that owns it, the dashboard can show per-section
counts without reading files, and an assistant that says "the ferry scene
contradicts the opening" can be made to say `book/one/the-ferry`, offset 1840.

### Formats

`markdown` by default. `html`, rendered **after** assembly so cross-page heading
levels are right and comrak sees one document. `json`, which is the manifest plus
each section's text, for a caller that wants the parts rather than the whole.

`?style=` names a page to prepend as a preamble: the rules of the work, in the
wiki, handed to the assistant in the same request as the manuscript. It is a
parameter rather than frontmatter because it is a property of who is asking, not
of the book.

## Where the words went

The hours have a heat map. The words should have the same thing beside it, on
the same terms: a chart, not a streak.

It cannot be derived, since nothing on disk records what a page used to be, so it
is durable state:

```sql
page_words(slug, at, actor, delta, total)   -- durable, beside api_usage and pins
```

One row per save, `at` in nanoseconds like every other timestamp here. Bucketing
happens **at read time from an `offset`**, exactly as `/api/time-stats` does and
for the same reason: an instant is an instant, and "how much did I write today"
is a question about a wall clock. Storing a pre-computed day would bake in a
guess about where the writer was.

Four things to get right, because this is durable and
[Architecture](architecture.md) is right that a change to a durable table needs a
real migration:

- **A page seen for the first time records a baseline, not a delta.** Otherwise
  importing an existing wiki reports the whole thing as written on Tuesday.
- **`total` is kept beside `delta`** so a row can be checked against the page
  rather than believed, and so a missed save shows up as a discontinuity instead
  of quietly skewing the series.
- **It is the first durable table that grows with use.** One row per save is
  small (a heavy year is tens of thousands of rows), and saying so now is cheaper
  than discovering it later. Pruning is deliberately not designed yet.
- **Deriving it from git was considered and rejected.** `git log --numstat` would
  give this for free on a committed wiki, but it measures commits, and nobody
  commits per save. It would report a week of work as one Friday.

`GET /api/word-stats?offset=&from=&to=` answers the series, split by actor.

## Provenance is per edit, not per sentence

Every write records an **actor**:

- `web` for the dashboard.
- `file` for an edit the watcher noticed, which is the writer in their own
  editor.
- Whatever a caller supplied, for anything else.

A caller supplies it with a request header, `X-Rhizolog-Actor`. Not a named API
token, which is what an earlier sketch of this said and which is wrong for this
wiki: tokens are a credential, credentials belong to accounts, and this wiki has
none. **An actor label is a claim, not a proof**, and on an open wiki anything
that can write can claim anything. That is fine, because the question it answers
is bookkeeping about your own tools, not security. It has to be said out loud
rather than left to be assumed.

On a wiki with accounts the label is recorded under the account that supplied it
and cannot be used to claim another. Named API tokens, already in
[`TODO.md`](../TODO.md) for other reasons, are what would eventually make a label
attested rather than asserted, and nothing here depends on them.

That is **edit granularity**. It answers "how much of today came through Claude"
and not "which sentence". Going further means diffing each save and attributing
surviving lines, which is blame for prose: genuinely useful, genuinely a
different feature, and carrying a per-page blame map as its cost. Not in this
plan.

## `prose/v1`

Rules live in `.rhizolog/prose.toml`. It is authored configuration: the only
copy, worth committing, and not secret, so it sits with `times/` and `ideas/`
rather than with `users/`.

The contract is the one `tfidf/v1` established: local, deterministic, no network,
no model, versioned, and every finding carries the text it fired on. A rule that
reported a problem without quoting it would be asking to be believed.

### Rule kinds

- **`forbid`**: literal strings or characters. This repository's own em dash rule
  is this one, and `AGENTS.md` is its first fixture.
- **`phrase`**: case-insensitive multiword. The tells. Shipped as a starter file
  rather than compiled in, because a built-in list of banned phrases is a claim
  about taste and taste is the thing being defended.
- **`echo`**: the same non-trivial word twice within N words. Catches the
  machine's habit and the author's equally.
- **`uniformity`**: a run of at least N consecutive sentences whose word counts
  all fall within k of their mean. The real tell of generated prose is not that
  sentences are long, it is that they are all the same size, and that is
  arithmetic.
- **`consistent`**: a capitalised token appearing in more than one spelling
  across the compiled manuscript, at edit distance one. The error long-form alone
  actually produces, and only findable because the corpus is indexed. It is the
  one rule with real false positives, so it takes an `allow` list in the same
  file.

Code blocks and inline code are excluded from every rule, always, from the AST
and not from configuration. A page documenting a syntax should not be flagged for
containing it, which is the same reason link extraction goes through comrak
rather than a regex. Frontmatter is not checked in v1.

Sentence splitting is a hazard and the honest answer is to state its failure:
split on `.`, `?` or `!` followed by whitespace and an uppercase letter, and
accept that abbreviations split wrongly. `uniformity` is about runs rather than
exact lengths, so a wrong split moves a number by a few words and does not invent
a finding.

### Findings

```json
{
  "analyzer": "prose/v1",
  "rule": "echo",
  "severity": "warn",
  "slug": "book/one/the-ferry",
  "span": { "start": 1840, "end": 1908 },
  "quote": "the ferry was late, and being late was the only thing it had ever been",
  "message": "late repeated within 12 words"
}
```

The example is `echo` rather than the em dash rule for a reason worth noticing:
a finding has to quote what it fired on, and this file is not allowed to contain
one. A rule whose own documentation cannot demonstrate it is a small preview of
what `forbid` is like to write about.

Spans are **byte offsets into the page source**, not into rendered HTML, because
the editor is a textarea over the source and a finding you cannot find is not a
finding.

Changing any rule's arithmetic, the tokenizer, or the sentence splitter is a
version bump to `prose/v2` and a note on this page, exactly as `tfidf/v1` is
governed.

### There is no dismissal store, and that is deliberate

Idea Inbox needs rejections because the machine proposes something about your
data and can be wrong about it. A prose finding is **your own rule firing on your
own text**. If it fires where it should not, the rule is wrong, and the fix is to
edit the rules file: a narrower pattern, or an entry in `allow`. One place,
no event store, nothing to fold.

The one rule this argument does not cover is `consistent`, which nobody wrote,
and that is exactly why it is the one with an `allow` list.

## API surface

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/compile` | Assemble from `root`; `format`, `style` |
| `GET` | `/api/word-stats` | The series, by day and by actor; `offset`, `from`, `to` |
| `POST` | `/api/prose` | Findings over a body, for the editor, like `/api/render` |
| `GET` | `/api/prose` | Findings over `slug`, or over a whole `root` |

Plus fields rather than endpoints: `words` on the page listing and read, `target`,
`due` and compiled `progress` on a page that carries them.

`/api/prose` takes `slug` and `root` as query parameters for the catch-all reason
given above. `POST /api/prose` is the editor's path and matches `/api/render`,
which already takes markdown and returns something derived from it.

Errors keep the standard envelope. At minimum: `compile_root_not_found`,
`compile_depth_exceeded`, `prose_rules_invalid`, `prose_rules_missing`.
A missing rules file is not an error at `POST /api/prose` (there is nothing to
check against, and a wiki that has never written rules is the ordinary case), but
it is worth distinguishing from a rules file that will not parse, which is a
mistake somebody just made and wants to hear about.

## Backend module seams

```text
backend/src/
  compile.rs         # assembly, substitution, heading shift, the manifest
  prose/
    mod.rs           # the rules file and its parsing
    rules.rs         # the five rule kinds, each a pure function
    text.rs          # extraction from the AST, sentence splitting, word counting
  index/
    words.rs         # the words column, page_words, the series
  api/
    compile.rs
    prose.rs
```

`prose/text.rs` is shared with the word counter on purpose: "what counts as text
in this page" has one answer, and the second copy is where the two quietly
disagree about whether a footnote is prose.

`compile.rs` and `prose/rules.rs` are pure functions of stated inputs, following
`ideas/{analysis,lifecycle}.rs`. Neither reaches the disk or the index; the
`index` and `api` layers gather what they read and hand it over whole. That is
the only reason their edge cases are testable.

Nothing here goes in `desktop/`.

## Dashboard

Almost no new routes, which is the measure of whether this fits.

- **A contents page gets a Manuscript panel** on `/pages/*slug`: the sections in
  order with their counts, the target and progress, the due date, and a Compile
  button. Gaps and cycles are shown as themselves rather than omitted.
- **`/pages/*slug?assembled=1`** renders the compiled document, which is also the
  proof-reading view.
- **The editor grows a findings strip** under the textarea, fed by
  `POST /api/prose` on the same debounce as the preview, and following the same
  rule the preview does: a collapsed strip issues no request.
- **`/` gains a words chart** beside the hours, split by actor.

Findings render as text, never through `innerHTML`. A `quote` is page content and
page content is what agents write, which is the rule `components/Snippet.tsx`
already exists to keep.

## Named milestones are not in this plan

"The draft before I let Claude rewrite chapter three" is a real want, and it is
not version history: it is a deliberate act with a name on it, which is the thing
git does not record and the author will not have committed. It was considered for
this plan and left out because the three features here do not need it and it
brings an authored store with it.

It stays a candidate, on one condition that is worth writing down now:
**deliberate and named, never automatic.** An automatic snapshot on every save is
the thing [Architecture](architecture.md) already declined, and declining it
twice by accident would be worse than declining it once on purpose.

## Implementation phases

Each ends at a reviewable state and a reasonable commit boundary.

### L0: words

Add the `words` column and the AST word counter; `target`, `due` and `contents`
to frontmatter; `?sort=words` and prefix totals.

Done when a prefix rollup equals the sum of the pages under it, a code fence
changes no count, and a schema-version rebuild produces identical numbers.

### L1: compile and the manifest

Add `ordinal` to `links`. Implement substitution, the heading shift, cycle and
gap reporting, the three formats, and the audience predicate.

Done when compiling a fixture book twice is byte-identical, every included
section's bytes appear exactly once at the offset the manifest claims, a wanted
page is reported as a gap rather than skipped, and a cycle terminates and says so.

### L2: `prose/v1`

The rules file, the five rule kinds, both endpoints.

Done when every finding can be reconstructed from the response and the rules file
without reading implementation code, and this repository's own em dash rule is
expressed in `prose.toml` and fires on `AGENTS.md`'s three deliberate specimens
exactly where it should.

### L3: actor and the word series

The header, the actor on every write path including the watcher, `page_words`,
`GET /api/word-stats`.

Done when a scan, an API write with a label, an API write without one, and an
external edit produce four distinguishable rows; when a first sighting records a
baseline and no delta; and when a schema-version bump leaves the table untouched.

L3 has no dependency on L2 and may swap places with it.

### L4: dashboard and documentation closure

The panel, the assembled view, the findings strip, the chart. Then update
[Architecture](architecture.md), [API design](api-design.md),
[The dashboard](dashboard.md), `AGENTS.md` and `TODO.md` from what was actually
built, and change this page's status from plan to record, naming every place the
code departed from it.

## Test strategy

### Pure unit tests

- Word counting: code fences, inline code, wikilink display text, tables,
  footnotes, an empty page, a page that is only a code block.
- Substitution: a link alone in a list item, a link alone in a paragraph, a link
  mid-sentence, a link in a page without `contents: true`, an external link in
  every one of those positions.
- Heading shift at depth zero, one and three, and a page with no heading.
- Cycles: a page containing itself, and a two-page loop.
- Each rule kind against a fixture with the expected spans, including a finding
  whose quote contains a multi-byte character, since spans are bytes.
- Sentence splitting, including the abbreviation case that is known to fail, so
  that its behaviour is pinned rather than accidental.
- Bucketing the word series across a local midnight and across a boundary at a
  non-zero offset, which is the bug `times::stats` already had to be written to
  avoid.

### Backend integration tests

- A compiled manuscript's manifest offsets index into the returned bytes.
- A page created after compilation turns a gap into a section with no reindex,
  which is the same self-healing property exact link resolution already has.
- Compile filters by audience on a wiki with accounts.
- An external edit is attributed to `file`, and a labelled write to its label.
- The word series survives a schema-version rebuild.
- A rules file that will not parse is reported as itself, and an absent one is
  not an error at `POST /api/prose`.
- Every error uses the standard envelope; every operation id is unique.

### Frontend tests

- A collapsed findings strip issues no request, matching the preview's rule.
- A finding's quote renders as text and produces no element from hostile content.
- The manuscript panel shows gaps and cycles rather than hiding them.
- The word chart renders with one actor, with several, and with none.

### Performance evidence

Compile is the one thing here whose cost grows with the work. Generate scratch
manuscripts of 50, 200 and 500 sections without touching `example-wiki/`, and
measure compile three times per size, keeping the minimum and every raw sample.
The existing note in [`TODO.md`](../TODO.md) about a noisy machine applies: a
single timing on this machine is not evidence.

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

Check `git status example-wiki` after any manual run and leave the fixture
unchanged.

## Open questions

**The recursion rule.** `contents: true` plus link-alone-in-a-block is a new
concept and it should beat the alternatives before L1 starts:

- **A depth limit alone.** Rejected: it inlines the research notes the moment a
  chapter's prose links to one, and the depth that is right for a book is wrong
  for a chapter.
- **Only links inside list items count.** Rejected as it stands: a contents page
  with a part title and an epigraph between its lists is ordinary, and this rule
  is invisible in the source of the page it governs.
- **An explicit `parts:` list in frontmatter.** Rejected: it is a second place
  the order lives, and it stops the contents page being a page you can read.
- **`contents: true` plus link-alone-in-a-block**, as planned above. The cost is
  that the second half is a formatting rule with meaning, which is the kind of
  thing that surprises somebody reflowing a paragraph.

**Whether `target` on a leaf page is useful**, or whether the recursive
definition is buying consistency nobody needs.

**Whether compile should have a byte ceiling.** A book is not large, and neither
is any reasonable manuscript, but nothing here bounds the output and an assistant
asking for one has a context window that does.

## Decisions to revisit only after use

- Named milestones, on the conditions above.
- Blame for prose, after edit-granularity provenance proves it is not enough.
- A focus or typewriter mode in the editor, which is cheap and which nothing here
  needs.
- Citations, where a link to a page marked as a source becomes a footnote and a
  reference list at compile time. It was part of the original sketch and was cut
  because the writer this plan is for is drafting rather than citing. The design
  hazard is recorded so it is not rediscovered: deriving a citation from the
  cited page's frontmatter makes every passing mention a citation, which is the
  mistake [Time tracking](time-tracking.md) avoided by counting only what the
  frontmatter says.
- `.docx` and PDF, only if pandoc over the compiled Markdown turns out not to be
  the answer.

None is a prerequisite.
