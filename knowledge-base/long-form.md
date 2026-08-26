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

**Contents page:** A page carrying a `contents:` list. There is no flag; holding
a list is what makes a page one.

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
target: 90000
due: 2027-03-01
contents:
  - book/one/opening
  - book/one/the-ferry
  - book/two
---
```

- **`target`** is a word count, measured against the **compiled** total from this
  page. On a leaf page that is its own words; on a contents page it is the book.
  One rule, recursive, no second concept.
- **`due`** is a date. It is read the way `created` is: a bare date is midnight
  UTC, and a wall-clock time with no zone is refused. A due date is a day rather
  than an instant, so unlike `created` it is stored as written.
- **`contents`** is the ordered list of pages this one assembles. See below.

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

### The rule, in one sentence

**A page contributes its body, then each page in its `contents:`, in order,
recursively.** Holding a list is what makes a page a contents page. There is no
flag, no depth parameter, and no convention about how a link is written.

### Structure is frontmatter, because prose is not a structured act

The alternative was to read the contents page's body: every internal link in it,
or every link standing alone in a list item or a paragraph, as a position in the
assembly. Both were planned at one point and both are recorded under
[Settled: the recursion rule](#settled-the-recursion-rule) with the rest.

[Time tracking](time-tracking.md) already decided this question in the other
direction and the argument transfers whole:

> A wikilink written inside an entry's note renders as a link and is not indexed
> as a time link. Attaching a page to an entry is a structured act, because these
> edges drive the numbers.

Membership in a manuscript drives what the manuscript *is*, which is a stronger
claim than driving a total. If a link in prose could put a page in the book, then
a see-also, a back-link to the index, or a sentence mentioning where an idea came
from would each be a chapter, and the wiki would have two kinds of link that look
identical and behave differently depending on which page they were written on.

Three consequences follow, and all three are the point rather than the price:

- **Nothing about formatting carries meaning.** Reflow a paragraph, join two
  lines, run a formatter over the file. The manuscript does not move.
- **Structure is editable by an assistant without touching prose.** "Insert a
  chapter after the ferry" is an unambiguous frontmatter edit through the
  ordinary page API. Under either body rule it is a text edit into prose, which
  is the operation you least want a machine improvising in, on the one page where
  a mistake reorders the book.
- **Body before contents gives part titles and epigraphs for free.** Prose
  belongs to a page's body and structure belongs to its frontmatter, so the two
  never compete for the same line. A book with parts is a contents page whose
  contents are contents pages, and each part's own body holds its heading and its
  epigraph, in exactly the place they belong.

### What it costs

**The contents page's body no longer shows the chapters.** Opened raw it is a
YAML list rather than a clickable index, which is a real loss for a project that
cares about the file being good on its own. It is readable there; it is just not
a link. Two things have to make up for it, and they are work rather than
objections:

- **A `contents:` entry is indexed as a link**, with a new `kind` of `part`
  beside `wiki`, `internal` and `external`. Otherwise every chapter in the wiki
  is an orphan and `/api/stats` fills up with them. This is not the
  `time_pages` case: a page collects hundreds of time links, and has exactly one
  parent, so these are edges the graph wants drawn rather than edges that would
  swamp it.
- **The Manuscript panel and `?assembled=1` become the rendering of the spine**,
  since nothing else is one. See [Dashboard](#dashboard).

`links` also gains an **`ordinal`** column, which is where the list's order is
kept. Order could instead come from re-parsing the root at compile time, which
compile could afford since it reads bodies from disk anyway; the index wins
because three things want the tree and only one of them wants the bodies:
compile, the target rollup, and the dashboard panel.

### A slug typo must not cost you the page

`contents:` entries are read as **strings** and parsed into `Slug` at compile
time, not at frontmatter time. A mistyped chapter is reported in the manifest as
`invalid` and everything else still compiles.

The alternative is what [Architecture](architecture.md) already warns about under
"A date somebody typed must not cost them the page": a value the frontmatter
parser refuses does not make a missing field, it makes a **malformed page**, and
a malformed page drops out of every listing taking its title and tags with it.
That is far too much to charge for a typo in a list of sixty chapters, and the
page it would be charged against is the one holding the book together.

### Writing the links in the body as well is harmless

Some people will, out of habit or because they want the page to render as an
index. The body is emitted verbatim, so the result is a table of contents printed
above the first chapter rather than anything wrong or doubled. It is visible
immediately, and the Manuscript panel can point out that a body link and a
`contents:` entry name the same page.

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
  A chapter written later fills the gap with no reindex, because a `contents:`
  entry is resolved by the same read-time join every wikilink is.
- **A `contents:` entry is a slug and only a slug.** A URL in that list is
  `invalid` in the manifest rather than a section, an external reference, or an
  attempt to fetch anything. Compile makes no network request, ever.
- **Compile is a page-returning query**, so the audience predicate in
  `index/audience.rs` applies to every page it assembles. A section the caller
  may not read is `wanted`, which is exactly the answer a slug with nothing
  written at it gives, and that indistinguishability is the point: it is the
  manifest's spelling of `404, never 403`. `unreadable` is kept for a page that
  will not **parse**, which is already reported to anybody who asks for it, so
  saying so here discloses nothing new and hiding it would swallow a real fault.
  This is the same split [Idea Inbox](idea-inbox.md) arrived at for promotion.
  On a single-user wiki none of it fires. It is stated because the rule is that
  every such query pastes the predicate in, and a query that forgot would be the
  interesting one.

### The manifest is the reason this is not a blob

Per section: `slug`, `title`, `depth`, `words`, `offset`, `length`, and a
`status` of `included`, `wanted`, `invalid`, `skipped_cycle` or `unreadable`.
Plus totals, and the analyzer-style stamp `compiler: "compile/v1"`.

A section with any status other than `included` still occupies its position in
the list. A manuscript that is short of a chapter says where the chapter was
going to be, which is the whole difference between a gap and an omission.

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
  compile.rs         # assembly, heading shift, cycles, the manifest
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
  button. Gaps, invalid entries and cycles are shown as themselves rather than
  omitted. This panel is not a nicety. Since the order moved into frontmatter,
  it is the only place the spine is rendered as something you can click, and it
  is what the body used to be.
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
to frontmatter, the last as a list of strings; `?sort=words` and prefix totals.

Done when a prefix rollup equals the sum of the pages under it, a code fence
changes no count, a `contents:` entry that is not a valid slug leaves the page
readable in every listing, and a schema-version rebuild produces identical
numbers.

### L1: compile and the manifest

Index `contents:` entries as `part` links with an `ordinal`. Implement the
assembly, the heading shift, cycle and gap reporting, the three formats, and the
audience predicate.

Done when compiling a fixture book twice is byte-identical, every included
section's bytes appear exactly once at the offset the manifest claims, a wanted
page holds its position rather than being skipped, a cycle terminates and says
so, and a chapter listed in `contents:` is no longer an orphan in `/api/stats`.

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
- Assembly: a page with no `contents:`, an empty list, a list naming a page that
  has its own list, a body followed by contents in that order, and a `contents:`
  entry that is a URL, a `..` path or an empty string.
- **That a wikilink in a chapter's prose is never a section**, including one
  written on a page that also has a `contents:` list, which is the whole rule and
  the one a future refactor is most likely to break.
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
- A page named in `contents:` is not an orphan, and a page merely linked from a
  chapter's prose still is if nothing else points at it.
- A `contents:` entry that is not a valid slug leaves the page in every listing
  with its title and tags intact, which is the one failure this design could
  charge somebody the whole page for.
- Compile filters by audience on a wiki with accounts, and a section hidden that
  way is indistinguishable from one nobody has written.
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

## Settled: the recursion rule

**Decided: an ordered `contents:` list in frontmatter.** The reasoning is under
[Structure is frontmatter](#structure-is-frontmatter-because-prose-is-not-a-structured-act);
what follows is the alternatives, kept so the question is not reopened from
scratch.

- **A depth limit alone**, following every internal link. Rejected: it inlines
  the research notes the moment a chapter's prose links to one, and the depth
  that is right for a book is wrong for a chapter.
- **Filesystem order**, a directory whose pages sort by slug. Rejected, and worth
  keeping because it is the obvious answer: it needs `01-opening.md`, and
  renumbering to insert a chapter renames files. This project deliberately does
  not rewrite inbound links on a move, so inserting chapter two would turn every
  reference to chapters three onward into a wanted page. It also contradicts
  [Architecture](architecture.md)'s premise that directories are for humans and
  the link graph is what gives the wiki its shape.
- **`order:` and `part_of:` on each page.** Rejected: an ordered list distributed
  across N files, where inserting between three and four means renumbering or
  fractional indices, and the parent cannot say what it holds without a query.
- **A separate manifest file** beside the page or under `.rhizolog/`. Rejected:
  a second file format, invisible in the wiki, and it gives up the one thing the
  contents page had going for it.
- **`contents: true`, and every internal link in the body is a section.** The
  runner-up, and cheap: one flag, formatting-insensitive, and the body renders as
  a browsable index everywhere with no panel needed. Rejected because a contents
  page could then never link in prose, so a see-also or a back-link to the index
  becomes a chapter, and because it leaves two kinds of link that look identical
  and behave differently depending on which page they were written on.
- **`contents: true` plus link-alone-in-a-block**, which this plan originally
  said. Rejected: it buys inline links in a contents page's prose and pays with a
  formatting convention that carries meaning, so a formatter joining two lines
  silently changes the manuscript.

The name is `contents:` rather than `parts:` or `sections:` because a book
already has a word for an ordered list of what is inside it, and because
`sections` is taken by the manifest. It carries the list rather than a boolean,
so there is one field rather than a flag and a list that could disagree.

## Open questions

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
