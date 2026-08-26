# Long-form writing

Status: **built**, L0 through L4. This page was the implementation plan and is
now the record: the reasoning is kept as it was written, and what each phase
actually turned out to be is appended under it. Where the two disagree, the code
is what runs and the plan is why it was expected to be otherwise.

Every place the code departed from the plan is named in one of the five "What L*n*
turned out to be" sections at the end. Nothing above them has been quietly
corrected to match, because a plan edited into agreement with its outcome is a
plan that never taught anybody anything.

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
  thousand words into nineteen hundred, so the figure has to record words added
  and words removed, and who did each. A signed total is the thing this feature
  exists to replace, and it is worth checking any draft of the design against
  that sentence, because the first one failed it.
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

- **The spine has to be indexed**, or every chapter in the wiki is an orphan and
  `/api/stats` fills up with them. See below for where it goes and why it is not
  a row in `links`.
- **The Manuscript panel and `?assembled=1` become the rendering of the spine**,
  since nothing else is one. See [Dashboard](#dashboard).

### A part is its own table, not a kind of link

The first plan said a `contents:` entry became a row in `links` with a new `kind`
of `part`, plus an `ordinal` column. That does not fit the table it was going
into. `links` is keyed `(src_slug, target, kind)`, so **one parent cannot list
the same child twice**, and an appendix under two parts is exactly the case the
manifest has a `duplicate` status for. Making `ordinal` part of that key would
work and would also change what a row *is* for every other kind: `[[a]]` written
twice in a page is one row today, and it should stay one row, because collapsing
repeats is what makes a backlink panel readable.

So:

```sql
page_parts(src_slug, ordinal, target)   -- derived; primary key (src_slug, ordinal)
```

Position is the identity, which is what makes both meaningful things
representable: order, and the same child appearing twice. `target` is a slug as
written and resolved by joining `pages` at read time, exactly as `links.target`
is, so a chapter written later fills its gap with nothing to reindex.

This is structurally the `time_pages` decision and it lands the other way round
on the one question that matters. Time links stay out of the graph because a page
collects hundreds of them and they would swamp its backlinks. A page has exactly
one parent, so part edges are edges the graph *wants*: **the orphan query and the
graph both union `page_parts` in**, and that is a deliberate, stated change to
two queries rather than a change to the identity of every link row in the wiki.

Order could instead come from re-parsing the root at compile time, which compile
could afford since it reads bodies from disk anyway. The index wins because three
things want the tree and only one of them wants the bodies: compile, the target
rollup, and the dashboard panel.

### Absent and empty are different, and a `PUT` can tell them apart

`contents:` is `Option<Vec<String>>`. **Absent** means a leaf page. **`[]`** means
a contents page with nothing in it yet, which is what a book looks like on the day
it is started, and the Manuscript panel should say so rather than showing nothing
at all.

That distinction only survives if writes preserve it, and this wiki has already
been bitten here. [The dashboard](dashboard.md) records the rule:

> Saving is a `PUT`, so the editor sends the owner back whether or not it shows
> it. A field left out of a `PUT` is a field cleared.

So a `PUT` that omits `contents` clears the list, a `PUT` with `[]` sets an empty
one, and the editor has to round-trip the field whether or not it renders a
control for it. An editor that dropped it would silently unmake a manuscript on
the first save of any chapter page, which is the same failure that handed pages
to the wrong owner.

### Every entry is a slug from the wiki root

`book/one/opening`, never `opening`, `./opening` or `../two/opening`. There is no
relative form and no basename fallback, wherever the page holding the list sits.

This is the same answer a wikilink already gets, for the reasons
[Architecture](architecture.md) gives under "Resolution is exact, and it is a
query", and the same answer a time entry's `pages:` list already gets. A
structured list of slugs in frontmatter has one spelling in this wiki and this is
not the place to invent a second. The verbosity is a cost that page already
accepted out loud:

> The cost is verbosity: you write `[[notes/rust/async]]`, not `[[async]]`. The
> editor can offer completion for that; it cannot un-break a link that
> retargeted itself.

Three things make it more clearly right here than it is for wikilinks:

- **A relative entry would be resolved against a position that moves.**
  `POST /api/move` deliberately does not rewrite inbound links, because a move
  turning them into wanted pages is visible in the stats. A relative `contents:`
  entry would not go wanted: it would resolve against the contents page's new
  directory and quietly assemble a different book.
- **It would need a second code path that builds a page path**, since resolving
  `..` has to happen before validation. `Slug` is the only thing allowed to do
  that, and [Architecture](architecture.md) calls slug validation
  security-critical for exactly this reason. Markdown links are resolved
  relatively, and note what that resolution is allowed to do when it climbs out
  of the wiki: **drop the link entirely**. That is a fine answer for a reference
  and an unacceptable one for a chapter.
- **The manifest reports slugs**, so a relative list would be written one way and
  read back another, and every reader would be doing the translation by hand.

The failures are worth knowing because they are not all the same:

- `../two/opening` and `./opening` are refused by `Slug::parse` itself, which has
  a `RelativeSegment` variant and the error code `slug_relative_segment`. They
  land as `invalid` in the manifest and name their own problem.
- `opening` is **not** an error. It is a perfectly good slug for a top-level page,
  so it resolves to `opening`, finds nothing, and appears as a `wanted` gap at its
  position. That is the one relative-looking form that fails quietly, and it fails
  the way a missing chapter does: visible, in place, and fixed by writing the slug
  out in full.

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
- **A page already emitted is `duplicate`**, and the manifest says so rather than
  deduplicating quietly. That is how a cycle ends, but the status is not called
  after one, because the commoner case is not a cycle at all: an appendix listed
  under two parts is a diamond, and a name that said `cycle` would send somebody
  looking for a loop that is not there.
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

### Limits are numbers, and a limit refuses rather than truncates

`compile_depth_exceeded` was named as an error with no maximum behind it, which
is a limit in the same state as no limit at all. Three of them, and the numbers
matter less than that they exist and are written down:

| Limit | Value | Why |
|---|---|---|
| Depth | 16 | A book is title, part, chapter, scene. Sixteen is far past any real structure and near enough to catch a mistake |
| Sections | 2,000 | A chapter per section, in a work nobody has written |
| Output | 8 MiB | Roughly a million words, which is several books |

**Exceeding one is a refusal, never a truncation.** `compile_too_large`, naming
which limit was hit and the slug it was hit at. A truncated manuscript is the
worst possible output here: it is a complete-looking document that silently stops
being the book, and the reader most likely to be handed one is an assistant that
will then reason about an ending that is not there. The refusal is a single
error, so this is one place where the manifest is not returned, and the error
naming the slug is what makes that survivable.

Depth is the one worth having despite cycle detection already terminating the
walk, because a mistake that is not a cycle can still be deep: a chain of
sixty contents pages each holding the next terminates fine and is not a book.

`?style=` counts toward the byte limit. It is prepended to the output, so a
preamble that pushed a compile over the edge and was then not counted would be a
limit that is not one.

### The manifest is the reason this is not a blob

Per section: `slug`, `title`, `depth`, `words`, `offset`, `length`, and a
`status` of `included`, `wanted`, `invalid`, `duplicate` or `unreadable`.
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

### It measures churn, because net change is the thing this feature exists to fix

An earlier draft of this page recorded one signed `delta` per observation, which
is a straight contradiction of the paragraph at the top arguing that a net count
is broken. An assistant rewriting two thousand words into nineteen hundred
produces `delta = -100`, and knowing *who* produced the -100 does not recover the
1,900 written or the 2,000 removed. The actor split answers a different question
from the one the example asks, and shipping both would have looked like an
answer.

So an observation records **`added` and `removed`**, in words, and `delta` is
arithmetic over them rather than a stored value. The rewrite above is
`added 1900, removed 2000`, which is what a day of revision actually looks like
and is the number the chart should be drawing.

Both come from diffing the previous body against the new one, and the previous
body is already there to diff against: `Index::upsert` writes `page.body` into
`pages_fts`, keyed by the `pages` row's own rowid, and it does that *after* it
has everything it needs to read the old row. Nothing new has to be stored, which
is the only reason this is affordable on every save.

**The diff runs over extracted text, not over the markdown.** What is in
`pages_fts` is the raw body, so both sides go through the same extraction the
word counter uses before anything is counted. Otherwise wrapping a paragraph in a
block quote, or reflowing it, would report words added and removed that nobody
wrote. That is the second thing hanging off `prose/text.rs` having one answer to
what counts as text.

### The log is authored, and the index over it is derived

```text
<wiki root>/
  .rhizolog/
    words/
      2026-08.log        # NOT derived; the only copy
```

Putting this in `index.db` was the first plan and it was wrong. The durable half
survives a schema bump, which is what that plan leaned on, but it does not
survive `rm index.db`, and every document in this repository tells the reader
that deleting the database costs one scan. `index/schema.rs` says it in the
schema itself: *the files are the log; this is only an index over them, and
deleting the database loses nothing.* A writing history is unreconstructable, so
it is exactly the primary data [Time tracking](time-tracking.md) refused to put
there:

> A year of tracked time is primary data, the thing you would be most upset to
> lose, and putting it somewhere the architecture actively encourages you to
> delete would be indefensible.

Moving it to disk also removes two problems the durable version had rather than
just relocating them: `page_words` becomes an ordinary derived table rebuilt from
the log, so it is no longer the one durable table that grows without bound, and
changing its shape stops needing a real migration.

**One file per month, one line per observation**, which is a departure from the
one-file-per-record shape `times/` and `ideas/` use, and the reason is frequency.
A time entry is a document somebody may open and correct; a word observation is a
machine's reading, never edited, arriving every time a file is saved. A file per
save would be thousands of files a month, and the `YYYY-MM` directory that makes
the time log survivable would not save it.

Tab-separated, because a slug may contain a space and may never contain a control
character, which is what makes a tab a safe delimiter and a space not:

```text
2026-08-25T14:25:30.123456789Z <TAB> book/one/the-ferry <TAB> claude-code <TAB> tim <TAB> observed <TAB> 1900 <TAB> 2000 <TAB> 41230
```

`at`, `slug`, `actor`, `account`, `kind`, `added`, `removed`, `total`, and a
ninth field the built version needed: see
[What L3 turned out to be](#what-l3-turned-out-to-be). `account` is empty on a
wiki with no accounts, which is the same thing `owner` does on a capture. It is a separate field from `actor` because they answer different
questions: which tool made the write, and which person it was made as. Bucketing
happens
**at read time from an `offset`**, exactly as `/api/time-stats` does and for the
same reason: an instant is an instant, and "how much did I write today" is a
question about a wall clock, so storing a precomputed day would bake in a guess
about where the writer was.

Two more things to get right:

- **A page seen for the first time is a baseline**, `added 0, removed 0` with its
  `total`. Otherwise importing an existing wiki reports the whole thing as
  written on Tuesday.
- **`total` is the check.** It is the page's own count after the observation, so
  a line can be verified against the page rather than believed, and a missed
  observation shows up as a discontinuity rather than quietly skewing the series.

**Deriving all of this from git was considered and rejected.** `git log
--numstat` would give it free on a committed wiki, but it measures commits, and
nobody commits per save. It would report a week of work as one Friday.

### An observation is not a save, and the log has to say so

`page_words` cannot honestly promise one row per save, and the earlier draft
promised it:

- **The watcher debounces at 500 ms and collapses a burst into one batch**
  (`watcher.rs`, "Why events are debounced"), because an editor saving through a
  temporary file and a rename arrives as several events. Two saves eight seconds
  apart are two observations; two saves inside the window are one.
- **Edits made while the server was down** are one observation at the next
  startup scan, however many saves they were.
- **Only an API write is genuinely one observation per save**, because it
  reindexes synchronously.

The diff is still correct in every one of those cases, since it compares the last
indexed body against what is on disk now. What it loses is resolution in time,
and intermediate churn inside a window is invisible: save, delete a paragraph,
put it back, and the observation correctly says nothing changed. That is a
limitation of watching a filesystem rather than a bug, and `GET /api/word-stats`
should carry `resolution: "observed"` so a caller is not invited to read it as
keystroke history.

### Ownership, visibility, and what happens to a slug

**The log is wiki-wide state, like the time log**, and it gets the same rule for
the same reason: the slug is the observation's own content and stays, and it is
the page **title** that is filtered by audience, so an observation against a page
the caller cannot read reports no title. Hiding the row would be hiding somebody's
own working history from them. Aggregate totals include those pages, ranked under
the slug, because the words really were written. See
[Time tracking](time-tracking.md), "A time link is not a link".

**Actor labels are wiki-wide too.** They name tools rather than people, and a
label is already a claim rather than a proof, so treating one as a secret would
be protecting nothing.

**`/api/word-stats` refuses an anonymous caller**, including under
`RHIZOLOG_ANONYMOUS_READ`. A writing history is working state, not published
content, and the variable exists to publish pages marked `public`.

A page's life leaves records in the same log rather than editing it, because it
is append-only:

- **A move writes a `moved` record** naming both slugs. History is never
  rewritten; a reader follows the chain, which is what lets a chapter renamed
  halfway through a book keep one continuous series.
- **A delete writes a `deleted` record.** The history outlives the page, because
  the words were written and deleting the file does not unwrite them.
- **A new page at a reused slug starts a fresh series** after that `deleted`
  record. Without the marker the two would be one series and the chart would show
  a page losing forty thousand words and gaining them back.

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

On a wiki with accounts, the observation records the label **and** the account
that supplied it, in two separate fields, so a label can never be used to claim
another account: the account comes from the session and is not something a header
can set. Named API tokens, already in [`TODO.md`](../TODO.md) for other reasons,
are what would eventually make the *label* attested rather than asserted, and
nothing here depends on them.

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

### The rules file

```toml
[[rule]]
id       = "no-em-dash"
kind     = "forbid"
severity = "error"
literals = ["\u2014"]
message  = "em dash"

[[rule]]
id       = "tells"
kind     = "phrase"
severity = "warn"
phrases  = ["it is not just", "a testament to", "delve into"]

[[rule]]
id       = "echo"
kind     = "echo"
severity = "warn"
within   = 40
ignore   = ["the", "and", "a", "of", "to"]

[[rule]]
id       = "uniformity"
kind     = "uniformity"
severity = "warn"
run      = 5
spread   = 3

[[rule]]
id       = "names"
kind     = "consistent"
severity = "warn"
distance = 1
length   = 4
allow    = ["Kaltenbrunner", "Kaltenbruner"]
```

The first rule is written with TOML's `\uXXXX` escape rather than the character
itself, because this repository may not contain one. That is not a workaround:
a rules file is going to be full of things somebody is trying not to write, so
the parser has to accept the escape and the documentation has to demonstrate it.

`id` is required and unique; a duplicate is `prose_rules_invalid` rather than a
last-one-wins. `severity` is `error` or `warn` and carries no behaviour, since
nothing here blocks a save; it is what the dashboard sorts and colours by.
`message` is optional and replaces what the rule would have said for itself. It
does **not** default to the rule's `id`, which is what this page said before L2
and which would have printed the id twice: see
[What L2 turned out to be](#what-l2-turned-out-to-be).

### Tokenization, shared with the word counter

Every rule that talks about words uses one tokenizer, and it is the one
`tfidf/v1` already defines in [Idea Inbox](idea-inbox.md):

1. Take the text extracted from the AST (see [Word counts](#word-counts)).
2. Lowercase with Rust's Unicode lowercase conversion.
3. Split at characters for which `char::is_alphanumeric` is false.
4. Keep non-empty tokens in source order, each with its byte span in the source.

Steps two and three swap places in the built version, and step four is why. See
[What L2 turned out to be](#what-l2-turned-out-to-be).

No stemming and no stop-word list, for the reason that page already gives: every
signal shown appears literally in text the writer wrote. So `delve` does not
match `delved`, and a rule that wants both lists both. The `ignore` list on
`echo` is not a stop-word list smuggled back in: it is authored, it is per-rule,
and it defaults to empty.

Case folding therefore applies everywhere except `consistent`, which is the one
rule about spelling and reads the token as written.

### What each rule computes

- **`forbid`**: a literal substring search over the extracted text, not over
  tokens, so it can name a single character. This repository's own em dash rule
  is this one and `AGENTS.md` is its first fixture. The span is the match.
- **`phrase`**: a **token sequence** match, not a substring, so `delve into` does
  not fire inside a word and punctuation between the tokens does not defeat it.
  The span runs from the first token's start to the last token's end. Shipped as
  a starter file rather than compiled in, because a built-in list of banned
  phrases is a claim about taste and taste is the thing being defended.
- **`echo`**: two occurrences of the same token, neither in `ignore`, whose token
  indices differ by at most `within`. It reports the **second** occurrence, spans
  both, and a run of three produces two findings (first-second, second-third)
  rather than one.
- **`uniformity`**: a run of at least `run` consecutive sentences whose word
  counts all lie within `spread` of the run's mean. Reports the longest such run
  and does not also report the shorter runs inside it. The receipt carries every
  sentence's length and the mean.
- **`consistent`**: two tokens, each appearing at least twice in the compiled
  text, each beginning with an uppercase character, at Damerau-Levenshtein
  distance at most `distance`, where neither is in `allow`. Reports every
  occurrence of the rarer spelling. The receipt carries both spellings and both
  counts. It is the one rule nobody wrote and the one with real false positives,
  which is why it is also the only one with an `allow` list. Two further filters
  turned out to be necessary before it said anything true at all, and they are
  the substantial change L2 made to this section:
  [What L2 turned out to be](#what-l2-turned-out-to-be).

Two rules may fire on overlapping spans and both are reported: suppressing one
would mean ranking rules against each other, and the author wrote them all.
Findings are ordered by start offset, then by rule `id`, so two runs over the
same text produce the same list in the same order.

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
  "rules_digest": "sha256:9f2b...",
  "rule": "echo",
  "severity": "warn",
  "slug": "book/one/the-ferry",
  "span": { "start": 1840, "end": 1908 },
  "quote": "the ferry was late, and being late was the only thing it had ever been",
  "message": "late repeated within 12 words",
  "receipt": { "token": "late", "first": 1840, "second": 1889, "distance": 11, "within": 40 }
}
```

The example is `echo` rather than the em dash rule for a reason worth noticing:
a finding has to quote what it fired on, and this file is not allowed to contain
one. A rule whose own documentation cannot demonstrate it is a small preview of
what `forbid` is like to write about.

Spans are **byte offsets into the page source**, not into rendered HTML, because
the editor is a textarea over the source and a finding you cannot find is not a
finding.

**Every finding carries a `receipt`**, which is the numbers the rule actually
compared: the two token positions and the distance for `echo`, the sentence
lengths and the mean for `uniformity`, both spellings and both counts for
`consistent`. `forbid` and `phrase` carry the matched literal. This is the
`tfidf/v1` rule applied here, and without it "late repeated within 12 words" is a
sentence asking to be believed rather than an arithmetic anybody can check.

Changing any rule's arithmetic, the tokenizer, or the sentence splitter is a
version bump to `prose/v2` and a note on this page, exactly as `tfidf/v1` is
governed.

### The rules have to be readable over HTTP, or the promise is false

This page's goals say the assistant reads the manuscript **and the rules** the
same way the dashboard does. `.rhizolog/prose.toml` is outside the page API and
outside the wiki walker, so as first written that was not true of the rules: a
remote caller could receive findings and had no way to see what produced them.

`GET /api/prose/rules` returns the **normalized** ruleset: every rule with its
`id`, `kind`, `severity`, resolved options and defaults filled in, plus the
`rules_digest` that findings quote. Normalized rather than the file's bytes,
because a caller wanting to reproduce a finding needs the values the analyzer
used, and TOML has more than one way to write most of them.

The digest is what ties the two together. A finding and a ruleset that disagree
on it were produced from different rules, which is otherwise an invisible way for
an assistant to be confidently wrong about why something fired.

Reading the rules needs the same authentication every other `/api` route does. It
is not writable through the API in this plan: the file is authored configuration,
editing it is a text edit, and a second way to write it would be a second place
for it to be wrong.

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
| `GET` | `/api/prose` | Findings over `slug`, or over what it assembles with `compiled` |
| `GET` | `/api/prose/rules` | The normalized ruleset and its digest |

Plus fields rather than endpoints: `words` on the page listing and read, `target`,
`due` and compiled `progress` on a page that carries them.

`/api/prose` takes its page as a query parameter for the catch-all reason given
above. `POST /api/prose` is the editor's path and matches `/api/render`, which
already takes markdown and returns something derived from it. `/api/prose/rules`
is a fixed segment under it and cannot collide with anything, since `/api/prose`
takes no path parameter at all.

Errors keep the standard envelope: `compile_root_not_found`, `compile_too_large`,
`prose_rules_invalid`. A missing rules file is not an error at `POST /api/prose`
(there is nothing to check against, and a wiki that has never written rules is
the ordinary case), but it is worth distinguishing from a rules file that will
not parse, which is a mistake somebody just made and wants to hear about.
`GET /api/prose/rules` on a wiki with no rules file answers an empty ruleset
rather than `404`: no rules is a state the wiki is genuinely in, and it is the
answer a caller asking what the rules are should get. A fourth code,
`prose_rules_missing`, was named here and then left unimplemented, because those
two sentences between them rule out every place it could fire. See
[What L2 turned out to be](#what-l2-turned-out-to-be).

## Backend module seams

```text
backend/src/
  compile.rs         # assembly, heading shift, repeats, the manifest
  prose/
    mod.rs           # the rules file, its parsing, normalization and digest
    rules.rs         # the five rule kinds, each a pure function
    text.rs          # extraction from the AST, sentence splitting, word counting
  words/
    mod.rs           # the observation record and the monthly log on disk
    diff.rs          # added and removed, from two bodies
  index/
    words.rs         # the words column, page_parts, page_words, the series
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
  button. Gaps, invalid entries and duplicates are shown as themselves rather
  than omitted. This panel is not a nicety. Since the order moved into frontmatter,
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

**Built.** See [What L0 turned out to be](#what-l0-turned-out-to-be).

Add the `words` column and the AST word counter; `target`, `due` and `contents`
to frontmatter, the last as an `Option<Vec<String>>`; `?sort=words` and prefix
totals.

Done when a prefix rollup equals the sum of the pages under it, a code fence
changes no count, a `contents:` entry that is not a valid slug leaves the page
readable in every listing, an absent list and an empty one survive a `PUT`
round-trip as different values, and a schema-version rebuild produces identical
numbers.

### L1: compile and the manifest

**Built.** See [What L1 turned out to be](#what-l1-turned-out-to-be).

Add `page_parts`, and union it into the orphan and graph queries. Implement the
assembly, the heading shift, gap and duplicate reporting, the three limits, the
three formats, and the audience predicate.

Done when compiling a fixture book twice is byte-identical, every included
section's bytes appear exactly once at the offset the manifest claims, a wanted
page holds its position rather than being skipped, a repeat terminates the walk
and is reported as `duplicate` rather than dropped, a relative entry is refused
by `Slug` rather than resolved against anything, a page listed twice under one
parent keeps both positions in the manifest, a chapter listed in `contents:` is
no longer an orphan in `/api/stats`, and each of the three limits refuses with
`compile_too_large` naming the slug rather than returning a short document.

### L2: `prose/v1`

**Built.** See [What L2 turned out to be](#what-l2-turned-out-to-be).

The rules file, its normalization and digest, the five rule kinds, all three
endpoints.

Done when every finding can be reconstructed from its own `receipt` and
`GET /api/prose/rules` **without reading the rules file or the implementation**,
which is the remote case and the one the earlier draft did not support; and when
this repository's own em dash rule is expressed in `prose.toml` and fires on
`AGENTS.md`'s three deliberate specimens exactly where it should.

### L3: actor and the word log

**Built.** See [What L3 turned out to be](#what-l3-turned-out-to-be).

The header, the actor and account on every write path including the watcher,
`.rhizolog/words/`, the derived `page_words`, `GET /api/word-stats`.

Done when a startup scan, an API write with a label, an API write without one and
an external edit produce four distinguishable records; when a rewrite reports
`added` and `removed` rather than only their difference; when a first sighting is
a baseline; when a move keeps one continuous series and a delete followed by a
new page at the same slug does not; and when **deleting `index.db` and restarting
reproduces the whole series**, which is the property that made this a log on disk
rather than rows in the database.

L3 had no dependency on L2 and could have swapped places with it. It did not, and
the one thing that argues it should have is that a word log is worth what it has
accumulated: every week it was not running is a week missing from a chart.

### L4: dashboard and documentation closure

**Built.** See [What L4 turned out to be](#what-l4-turned-out-to-be).

The panel, the assembled view, the findings strip, the chart. Then update
[Architecture](architecture.md), [API design](api-design.md),
[The dashboard](dashboard.md), `AGENTS.md` and `TODO.md` from what was actually
built, and change this page's status from plan to record, naming every place the
code departed from it.

**`.rhizolog/words/` makes a fourth authored tree**, so every place that
enumerates the three has to gain it: the table in
[Architecture](architecture.md), the paragraph in `AGENTS.md`, and the `README`.
It is authored and **not** secret, so it goes with `times/` and `ideas/` rather
than with `users/`: back it up, commit it. All three were updated in L3 rather
than left for here, because the tree exists now and documentation that is wrong
about what is on disk is worse than documentation that is late.

The gitignore was going to need no change, "because it already names `index.db`
rather than the directory". That was true and beside the point, and it took one
line to fix and a `git status` to notice. See
[What L3 turned out to be](#what-l3-turned-out-to-be).

## What L0 turned out to be

`markdown::count_words` is the counter, `pages.words` is the column at schema
version 10, and `target`, `due` and `contents` are on `Frontmatter`. Four things
about it differ from what this page said above, and one is a rule the plan had
not thought to state.

### The counter is in `markdown.rs`, not in `prose/text.rs`

The module list puts text extraction in `prose/text.rs` and says it is shared
with the word counter on purpose. That sharing still stands and the location does
not: `markdown.rs` is already the module that owns walking comrak's AST, it
already has `options()` and a `text_of` for link labels, and a second module
holding one function would have split that knowledge in two before there was a
second reader. When `prose/v1` lands, `prose/text.rs` holds sentence splitting
and calls in here for extraction.

### Raw HTML divides, and only half of it was in the plan

The plan says raw HTML is not counted. That is one rule where the code needs two,
and the difference is what the reader sees:

- A raw HTML **block** takes its contents with it. The renderer drops the whole
  thing, so none of it reaches the page.
- An **inline** tag does not. `<span>` is dropped and the words it wraps are
  still rendered, still read, and therefore still words.

`markup_is_not_words_but_the_words_inside_it_are` is the test, and it was written
after the easy version of the rule failed it. Counting a `<span>`'s contents as
nothing would have made the number disagree with the page.

Two smaller rules arrived the same way. Literals are concatenated **verbatim**
rather than joined with a separator, because comrak splits `un*believable*` and
`hello *world*` into two nodes each and only the source says which was one word.
And a bare `[[notes/rust/async]]` counts as **one** word, since the slug is what
the page displays.

### `target` gets the bare-date treatment, and `due` gets `owner`'s

The plan named the fields and not what happens when somebody mistypes one. Both
answers already existed in this codebase and they are not the same answer:

- **`target` is parsed**, through a `frontmatter::word_count` deserialiser that
  accepts `90,000`, `90_000` and `90 000` as well as `90000`, for the reason
  `created` accepts a bare date. `-1`, `9.5` and `lots` are refused, because a
  negative target is not a small one and reading it as zero would report a page
  as finished.
- **`due` is kept as written** and read by an accessor. A value that is not a
  date names no day, exactly as an `owner` that is not a username names nobody,
  and the field stays in the file so the mistake is visible rather than silently
  dropped on the next write. This is the softer of the two because a due date is
  a note to the author rather than something the code computes with.

Over the API `due` is a full timestamp and a bad one is a `400`: strict at the
boundary, lenient for a file somebody typed. That is the same split slugs already
get, and it means the API can be unforgiving without a hand-written page paying
for it.

### The prefix total is a field on the listing

`GET /api/pages` gained `words` beside `total`, summed over the **whole filtered
set** rather than the page of results, from the same query that produces the
count. Summing the returned rows instead would make a manuscript's length depend
on the caller's `limit`.

Measured against `example-wiki/`, which is the first real prose it has seen:
`?prefix=notes` reports 711 words across six pages, and the six add up.

## What L1 turned out to be

`compile.rs` is the assembly, `api/compile.rs` is the seam to HTTP, and
`page_parts` is the spine at schema version 11. Six things differ from what this
page said above, and one of them is a bug this project had no guard against.

### The graph waits for L4; the orphan count did not

The plan says L1 unions `page_parts` into "the orphan and graph queries". Only
the first is built. Orphans are a **number being wrong**: without the union,
every chapter of every book is reported as unreferenced and one of the two
figures `/api/stats` exists for fills up with them. Drawing part edges is a
**display decision** with real questions attached (does a part edge look like a
wikilink? does it bow the same way?), and answering those before the Manuscript
panel exists would be guessing. It moves to L4, where the panel is.

One function, `graph::referenced`, holds the union, because the orphan count and
the orphan list both ask it and two spellings of one rule drift.

`visible_part` is deliberately simpler than `visible_link`: it checks only that
the parent is visible. The second half of `visible_link` exists to stop a link to
a private page appearing as a *wanted* page in a drawing, and nothing here draws
anything.

### The walk reaches storage through a trait, not a pure function

The module seams say `compile.rs` is a pure function of stated inputs that never
touches the disk. The assembly is; the **walk** cannot be, because what it asks
for next depends on what the last page said, and pages come off a disk.

So `compile::Pages` is a trait with one async `fetch`, implemented over the store
in `api/compile.rs` and over a `HashMap` in the tests. Every interesting case
here is a shape of tree rather than a shape of disk, so the tests build wikis in
memory: a diamond, a two-page loop, a chain past the depth limit, a chapter
nobody has written. That is the same testability the plan wanted, bought with a
trait instead of a function signature.

The audience check lives in the impl rather than in the walk, which is what makes
"a page you cannot read" and "a page nobody has written" the same answer without
the assembly having to know that rule exists.

### Two heading rules the plan did not have

Shifting by depth is easy to state and has two edges:

- **Levels clamp at six.** Markdown has no `#######`, so a heading pushed past
  six stops there rather than silently becoming a paragraph that starts with
  hashes. A deep book loses hierarchy at the bottom; the alternative loses the
  heading.
- **A setext heading becomes an ATX one.** `Opening` over `=======` has no marker
  to shift, so there is nothing to change without changing the spelling. Its
  source lines are kept verbatim after the new marker, so emphasis and links
  inside a heading survive. This happens only in the compiled output.

Both go through comrak rather than a scan for `#`, so a hash inside a fence is
not a heading. Everything that is not a heading line is passed through byte for
byte, which is what lets the manifest promise a section's bytes are its own.

### A separator is added, never trimmed

Sections are joined with a blank line, added only where the previous section did
not already end in one. Nothing is trimmed from a body, so "byte-identical" and
"appears exactly once at the offset the manifest claims" both survive: what
changed is what sits *between* sections, and the offsets account for it.

### Too large is a 413, not a 400

The plan named `compile_too_large` and not its status. Nothing about the request
is malformed, so a `400` would send the caller looking for a typo rather than at
the size of what they asked for. The details carry the limit, its ceiling and the
slug it was reached at, since the slug is the one part a caller cannot work out
for themselves.

### A dangling `$ref` got as far as a working endpoint

`format` was first written as a `ToSchema` enum on the query struct. That
compiles, serves, passes every test, and produces an OpenAPI document with a
`$ref` to a component that was never registered, because utoipa registers schemas
reachable from bodies and responses and `IntoParams` is neither.

The failure is silent from Rust and total from TypeScript: `pnpm gen:api` refuses
the **whole document**, so the frontend's types cannot be regenerated at all.
It was caught by running the command and reading its exit code, having missed it
once by piping the output away.

Two things came out of it. `format` is now a plain string parsed in the handler,
which is what `sort` and `order` already do and which also gets a caller who
names a bad format the list of good ones. And
`every_reference_in_the_spec_resolves` walks the served document and asserts
every local reference resolves, so the next one fails in `cargo test` rather than
three commands later.

### Verified against hand-written files

A scratch book of three files, none of them written through the API: a root with
`target: 90,000` and a chapter that does not exist, a part naming
`../etc/passwd`, and a chapter whose heading is setext. Compiling it produced one
document with `#`, `##` and `###` nesting, the setext heading converted, both bad
entries reported in position at the right depths, offsets that index into the
returned bytes, and an orphan list holding only the root.

## What L2 turned out to be

`prose/{mod,rules,text}.rs` is the analyzer, `api/prose.rs` is the seam to HTTP,
and `markdown::extract` is the piece neither this page nor L0 had thought of.
Eight things differ from what is written above, and two of them are the rule
`consistent` needed in order to be usable at all.

### Spans survive because nothing is extracted

The plan says a finding's spans are byte offsets into the page source, and says
separately that the tokenizer takes "the text extracted from the AST". Those two
sentences are in tension, and the second one loses: an offset into a
concatenation of literals is an offset into a string nobody has.

So there is no concatenation. `markdown::extract` returns the body **byte for
byte** with every byte that is not prose replaced by a space, newlines kept as
newlines. Code, raw HTML, image alt text, footnote markers, link targets and
every piece of markdown punctuation in between are simply blanked. Every offset
into the result is already an offset into the body, with no mapping in between to
get wrong. `every_token_indexes_the_source_it_came_from` is the test, and the
integration test reproduces a finding from its own receipt against the page as
`GET /api/pages` returns it.

That rests on comrak filling in **inline** sourcepos, which it does, and on its
columns being **byte** offsets within their line, which they are. Both were
checked against the parser before anything was built on them, because the whole
design falls over if either is false.

It disagrees with `count_words` in exactly one narrow place, and the difference
is stated in both modules: markup *inside* a word. `un*believable*` is one word
to the counter, which follows the literals, and two tokens here, because the `*`
between them is blanked. The decision that matters, which nodes are prose at all,
is one `match` that both walks call, so the two cannot drift about whether a
footnote is prose.

Case folding moved for the same reason. `tfidf/v1` lowercases the whole string
before splitting; Rust's lowercase conversion can change a string's length, so
doing that here would move every offset after the first such character. The split
happens over the text as written and each token is folded on its own. `forbid`,
which searches the text rather than the tokens, folds **ASCII case only**, which
is the most that can be done without lowercasing the haystack.

### `?compiled=true`, not `?root=`

The plan gives `GET /api/prose` a `slug` **or** a `root`, exactly one of which
must be present. That shape needs an error for "you gave me neither" and another
for "you gave me both", neither of which says anything useful. `slug` is required
and `compiled` is a flag: findings over that page, or over everything it
assembles. It reads the way `?render=true` and `?assembled=1` already read, and a
caller does not have to know the word root.

`compiled` is not a convenience. Two of the five rules are cross-page questions by
nature: a name spelled two ways in two chapters, and a word echoed across a
section break, are invisible to anything reading one page at a time.
`a_repeat_across_a_chapter_break_is_only_visible_compiled` is the test.

It moves what the offsets mean, so the response says which it gave: `offsets` is
`page` or `document`. Compiled offsets index the assembled document rather than
any one page's source, because the heading shift moves bytes inside a section.
Each finding carries the `slug` it fell in, which is what the manifest was for,
and a finding that straddles a chapter break belongs to the section it starts in.

### `consistent` needed two filters nobody had thought of

Run against this repository's own documents, the rule as specified reported
almost nothing but false positives: `If` beside `It`, `They` beside `The`, `L0`
beside `L1`, `UTF` beside `UTC`. Seventy-three findings on this page alone, none
of them a name. `allow` is the plan's answer and it is the wrong shape for this:
every wiki would have to enumerate the same list of English sentence openers
before the rule said anything.

Two filters, and both are about what a name *is* rather than about taste:

- **Capitalised somewhere a capital was not already forced.** A proper noun is
  capitalised in the middle of a sentence too. `If` and `It` are not two
  spellings of one name, they are two ordinary words that only ever start one.
  `text::opens_a_sentence` is deliberately conservative, since every case it gets
  wrong costs a candidate rather than inventing one.
- **At least `length` characters**, a new option defaulting to 4. What survived
  the first filter was initialisms and labels, which are one edit apart on
  purpose. A wiki with a three-letter name lowers the number.

Together they take `consistent` from seventy-three findings on this page to
**zero**, and from five on `AGENTS.md` to zero, while
`consistent_reports_every_occurrence_of_the_rarer_spelling` still catches
`Kaltenbruner`. `allow` stays for what is left, which is what it was always for.

### `message` says what the rule found, not what the rule is called

The plan says `message` defaults to the rule's `id`. It cannot: a finding already
carries `rule`, so defaulting to the id would print the same string twice and
throw away the one sentence the rule had to offer. This page's own example proves
it, since the `echo` rule there has `id = "echo"` and the example finding says
"late repeated within 12 words".

So each rule describes its own findings, and a `message` in the file replaces
that description rather than filling a gap. `forbid` and `phrase` name the literal
they matched; the other three say the arithmetic. Nothing is lost when an author
overrides one, because the receipt carries the numbers either way.

### Rhythm is a property of a paragraph

`uniformity` reads sentences only from paragraphs, and a sentence never crosses
from one paragraph into the next. Headings, table cells and list items contribute
none.

A list is uniform by construction. Measuring one means firing on every bulleted
list in the wiki, which is a rule nobody leaves switched on. A run may still span
consecutive paragraphs, because five one-sentence paragraphs of the same length
is the same tell as five sentences of the same length in one.

Sentence splitting is the hazard the plan said it was, and its known failure is
pinned by a test rather than left accidental: `Dr. Kaltenbrunner` splits wrongly
and always will without a list of abbreviations.

### A findings list is capped, and says so

`MAX_FINDINGS` is 1,000. A `forbid` rule naming one common letter would otherwise
return a finding per occurrence across a whole book.

This is the opposite of the compile limits and deliberately so. A truncated
manuscript is dangerous because it looks complete; a truncated findings list
carries `truncated: true` and is therefore not pretending to be anything. Nothing
in this repository's own documents comes close: the worst is 348.

### `prose_rules_missing` has nowhere to be

The plan lists it as an error and then, two paragraphs later, rules out both
places it could fire: a missing rules file is not an error at `POST /api/prose`,
and `GET /api/prose/rules` answers an empty ruleset rather than a `404`. It is not
implemented, because an error code with no site is a promise to callers that
nothing can keep.

`prose_rules_invalid` is a **422**, which is the status a page whose frontmatter
will not parse already gets, and for the same reason: the request is well formed
and the authored file it reaches for is not. Its details name the file as well as
the reason, since the one thing a remote caller cannot do is go and look.

A `toml` dependency came with all this, parsing only. A rules file is largely a
list of things somebody is trying not to write, and TOML's `\uXXXX` escape is how
you name a character your repository may not contain. The parser strips a leading
byte order mark, because PowerShell's `Out-File` writes one and a BOM has already
cost this project a real bug.

### Verified against this repository's own documents

A scratch wiki holding copies of `AGENTS.md`, this page and
[Architecture](architecture.md), and a `prose.toml` with all five rules in it.
Three things came out of it:

- **The em dash rule fires exactly where it should.** On a page with the
  character in prose, in inline code and in a fence, it reports **one** finding,
  the prose one. On `AGENTS.md` it reports **none**, which is the right answer
  and a slightly surprising one: that file's single em dash is inside backticks,
  which is exactly what makes it a specimen rather than prose.
- **`consistent` reports nothing false**, after the two filters above.
- **`echo` is as noisy as its configuration lets it be.** At `within = 12` with
  an eleven-word `ignore` list it found 63 repeats in `AGENTS.md` and 348 here.
  That is the rule working: it is a lexical count, `ignore` is the only lever,
  and the plan's example value of 40 is far more aggressive than it looks. Worth
  knowing before the dashboard shows anybody a number.

And one finding that is about this repository rather than about the code:
**the knowledge base carries 353 em dashes**, across thirteen pages, all of them
in prose. The house rule in `AGENTS.md` has been enforced on new writing and never
applied backwards. Fixing that is a prose edit across thirteen files and not part
of this phase, but the linter found it in one request, which is the first time
anything here has paid for itself.

### There is no starter rules file, and there is nowhere for one

The plan says the `phrase` list ships "as a starter file rather than compiled in".
Nothing shipped, because a `prose.toml` belongs to a wiki and this repository is
not one: `backend/wiki/` is gitignored and `example-wiki/` is a fixture whose
contents the documentation makes claims about. The starter is the example under
[The rules file](#the-rules-file), and finding it a home is part of L4's
documentation closure.

## What L3 turned out to be

`words/{mod,diff,stats}.rs` is the log, `index/words.rs` is the table folded from
it, and `page_words` is at schema version 12. Seven things differ from what this
page said above, and one of them is a line in `.gitignore` that would have thrown
the whole feature away.

### `*.log` very nearly ate the word log

This page said the gitignore needed no change, "because it already names
`index.db` rather than the directory, which is the decision that keeps paying".
That was true and beside the point. `.gitignore` also carries a plain `*.log`
under **Editor / OS noise**, and a pattern with no slash in it matches at any
depth, so `.rhizolog/words/2026-08.log` was ignored by a rule written years
before there was anything to ignore.

A tree the documentation tells you to commit, silently untracked, discovered by
creating the file and running `git status` rather than by reading the file.
`!**/.rhizolog/words/*.log` undoes it, and the comment beside it says what would
break the negation: an ignored `.rhizolog/` directory, which git would not
complain about.

### A first sighting means two different things

The plan says a page seen for the first time is a baseline, so that importing an
existing wiki does not report the whole thing as written on a Tuesday. True, and
it is only half the cases. A page nobody has a record of looks **identical**
whether the server has just been pointed at a wiki full of them or somebody has
just created one: no previous body, no previous total. `Index::upsert` cannot
tell them apart, because the difference is not in the page.

It is in who is asking. So `upsert` reports the facts and the caller reads them:
`WordChange::as_written` turns a baseline into words that have just arrived, and
the startup scan is the one caller that does not use it. Without that, the first
draft of every page would have been recorded as nothing at all.

The watcher counts as a live write, with one known cost: restoring a backup file
by file would be reported as writing it. A change on that scale arrives as a
directory event and goes through the scan instead, which is what made this the
right way round.

### The diff counts words, not arrangement

`added` and `removed` come from comparing two **multisets** of words. A word in
the new text that the old text had no copy of is added, and the reverse is
removed. Two consequences, and the plan should have said both:

- **A shared word is not written twice.** Rewriting a sentence and keeping half
  its vocabulary reports less than the whole sentence, which is why this page's
  illustrative "added 1900, removed 2000" is a ceiling rather than a figure.
- **A pure reordering reports nothing.** A day spent restructuring a chapter
  without writing a new word is a day the log says nothing happened on. The
  alternative is a sequence diff, which calls a moved paragraph both added and
  removed and whose longest common subsequence over prose is padded out by `the`
  and `and` anyway. This version is one somebody can check by hand.

It also makes the log's own check exact: `added - removed` is always the change
in the page's count. That is what `total` is for, and it is what makes deleting
the index free.

One thing the plan had no way to anticipate: **punctuation had to be trimmed off
a word before comparing**. The counter splits on whitespace, so finishing a
sentence turns `late.` into `late` and would otherwise report one word removed
and two added where one was written. Interior punctuation stays, so `it's` and
`rust-lang` are each one word.

### Five kinds, not three

`observed`, `moved` and `deleted` were named. Two more were needed:

- **`baseline`**, which the plan describes without giving it a name. It has to
  be one, because "has this slug been seen" is what the next write is decided by.
- **`net`**, which is the one case a lost index really costs something. The
  previous body went with `pages_fts`, so when the database was deleted *and* a
  file changed before the next start, the difference between the two totals is
  all there is. It is recorded under its own name rather than dressed up as a
  churn, because the whole feature exists to stop a net figure being passed off
  as one.

Only `observed` and `net` count as writing. A baseline, a move and a delete are
bookkeeping, and counting them would put a page's whole length into the day
somebody first pointed the server at it.

A `deleted` record carries **`removed: 0`**. The words were written and deleting
the file does not unwrite them; the marker exists so that the next page written
at that slug starts a series of its own. A `moved` record closes the slug it left
as well as opening the one it arrived at, which the plan did not say and which is
what stops a new page at a vacated slug inheriting a total.

### The line has a ninth field

`at`, `slug`, `actor`, `account`, `kind`, `added`, `removed`, `total`, and
**`from`**, which is the slug a page arrived from and is empty on everything but
a `moved` record. The plan called for a move to name both slugs and gave the
format eight columns to do it in.

A line with **more** than nine fields is read as the nine it understands, so a
log written by a later version stays readable by this one. An append-only file
cannot be migrated, so that has to be true from the first version.

### `api`, and the header is refused rather than trimmed

The plan names `web`, `file` and "whatever a caller supplied", and leaves the
default unstated. There are four: `scan` for the startup scan and
`POST /api/reindex`, `file` for the watcher, `api` for a write that named no
tool, and the label itself for one that did. `web` is not special; it is what the
dashboard's own client sends, which is one line in `frontend/src/api/client.ts`.

A label that could not be written into a tab-separated line is a **400**,
`invalid_actor`, and the error does not echo it back. It was just refused for
holding something unprintable, and putting it in a JSON error would be putting it
somewhere else it does not belong.

### The log is rebuilt wholesale, and is not in the sync report

`sync` reads the whole log and replaces `page_words` on every run, **before** the
page scan. The order is load-bearing: the scan asks what each page's last
recorded total was, and on a database that has just been deleted the answer is
nothing at all until the log has been read back in, which would report every page
in the wiki as written today.

It is not one of `SyncReport`'s trees. The other five are compared file by file
and counted as scanned, indexed, unchanged, removed and failed; this one is read
and replaced, and three of those five fields would be zero for reasons that mean
nothing. Reporting it belongs with L4's dashboard work, where there is somewhere
to put it.

### Verified against a real wiki, including the property that made it a file

A scratch wiki with two pages written before the server ever saw it, then: an
assistant's create, an unlabelled revision, an edit in an editor, a move and a
delete. The log came back exactly as designed, with the arithmetic checkable by
hand: a 12-word draft, then `added 5, removed 4, total 13` for a rewrite that
kept most of its vocabulary, and 12 minus 4 plus 5 is 13.

Then the test that made this a log on disk rather than rows in the database:
**stop, delete `index.db`, restart.** The file was byte for byte identical
afterwards and `/api/word-stats` returned the same series, split the same way by
tool. Nothing was recorded, because every page's count still matched the log,
which is the whole job `total` was given.

One thing that run showed and no test would have: an edit made in the same moment
as a directory appearing is attributed to `scan` rather than `file`. The
directory forces a rescan, the rescan picks the edit up, and a rescan genuinely
does not know when the change happened. That is what `scan` means, and it is a
limit of watching a filesystem rather than a fault.

## What L4 turned out to be

`components/{Manuscript,Findings,WordStats}.tsx` are the three new screens'
worth, `?assembled=1` is the fourth, and the spine is drawn. Eight things differ
from what this page said above, and two of them are bugs this phase found rather
than features it built.

### The editor was clearing `contents` on every save

The plan states the rule under
[Absent and empty are different](#absent-and-empty-are-different-and-a-put-can-tell-them-apart)
and states it correctly: a field left out of a `PUT` is a field cleared, so the
editor has to round-trip `contents` whether or not it renders a control for it.
It did not. From L0 until L4 the dashboard's Save sent `title`, `tags`,
`content`, `visibility`, `owner` and `readers`, and **any page opened in the
editor and saved lost its contents list, its target and its due date.**

Nothing caught it because nothing could: L0 through L3 were backend phases, the
manuscript panel that would have shown the damage did not exist yet, and the
first page anybody would have opened in the editor was a chapter rather than the
contents page holding the book together.

It is the same failure that once handed pages to the wrong owner, which the
dashboard page already records, with a different field. Three tests now pin it,
and the one that matters is the empty list: `[]` has to come back as `[]` rather
than as no list, because those are the two states the API is careful to keep
apart.

### `contents` needs two controls, because a textarea can only say one thing

A list of slugs is a textarea, one per line. Absent and empty are different
values and an empty textarea can only be one of them, so a checkbox says whether
the page assembles others at all and the textarea holds the list. Unchecked sends
`null`; checked and empty sends `[]`.

Duplicates are **kept**, unlike the tag and reader fields it sits beside, which
both drop them. A contents list is positions rather than a set, and an appendix
under two parts is the case the manifest has a `duplicate` status for.

### Progress is arithmetic, not a field

The API surface says "compiled `progress` on a page that carries them", and that
field does not exist. It cannot cheaply: `target` is measured against the
**compiled** total, so filling it in on `GET /api/pages/{slug}` would mean
assembling the whole book on every page read, including every read of every
chapter.

The panel pays one compile when it is shown and divides. That is the same walk
`?assembled=1` does, and a manifest-only endpoint was considered and skipped: the
walk is where the cost is, the assembly is concatenation, and a second code path
for the same tree would be a second answer about what the book is.

### A part edge does not look like a wikilink, and bows the same way

The two questions L1 deferred, answered. Drawn in the second colour and heavier
at rest, with its own arrowhead and a key that appears only on a wiki that has
one, because a link is one page mentioning another and a part is one page being
*inside* another.

It bows exactly as a link does, and for the link's own reason rather than out of
uniformity: two contents pages can each name the other, and drawn straight those
two lines would be one with an arrowhead buried under the other. A page both
linked to and assembled by one parent is a single line carrying both, so that
case never arises.

Three things the plan had not thought about came with it. **A walk crosses the
spine**, or the one page a chapter is certain to be connected to would be the one
page its neighbourhood could not reach. **A degree counts a `contents:` entry**,
or a book with sixty chapters and no links would be drawn the size of a leaf.
And **an entry that is not a valid slug is not drawn**: `page_parts.target` holds
the entry as written, so `../etc/passwd` is in the table, and drawing it would
advertise a path as a page worth writing. It stays where it belongs, `invalid` in
the manifest.

`visible_part` grew the second half of `visible_link` at the same time, which it
had deliberately gone without. That second condition exists to stop a link to a
private page appearing as a **wanted** page, named by a slug that is usually its
title, and it became necessary the moment these rows started being drawn. It
costs nothing where the rows were already used: the orphan query asks about a
page the caller can already see, so the condition is trivially true there.

### A report says how many rules ran

A new field on every `/api/prose` response, and the strip needed it before it
could say anything honest. No findings from no rules is **not** a clean page, and
a wiki that has never written a `prose.toml` is the ordinary case rather than a
fault, so the two silences have to be tellable apart. The alternative the client
reached for first was comparing the digest against the digest of an empty
ruleset, which is a constant copied out of the server into the browser: exactly
the sort of thing that is right on the day it is written and rots invisibly.

### The word log is in the sync report, and is not shaped like the rest

`SyncReport` gained a sixth field that is not a `SyncCounts`. The other five
trees are compared file by file, so `scanned`, `indexed`, `unchanged`, `removed`
and `failed` each mean something about them; the word log is read whole and the
table over it replaced, so three of those five would be zero for reasons that
mean nothing, and zeroes that mean nothing read as news.

What it carries instead is what the log **held when it was read** and how many
lines would not parse, plus whether it could be read at all. That last one is the
field worth acting on and the dashboard does: a rebuild that could not read
`.rhizolog/words/` says so, because it is the one tree here with no other copy.

`changed_anything` deliberately does not consult it. The log is replaced on every
run, so it would answer yes every time a wiki had any history at all, and the
question that is asked is whether anything *changed*.

### Spans are bytes and a textarea indexes UTF-16

Clicking a finding selects it in the editor, which is the whole reason the spans
are offsets into the page source rather than positions in rendered HTML. It is
also the first place those offsets meet JavaScript, where one `é` is two bytes
and one code unit and one emoji is four bytes and two. Handing a byte offset
straight to `setSelectionRange` selects the wrong words on any page with an
accent in it, and does it further and further out as the page goes on.

`byteToIndex` walks code points accumulating UTF-8 lengths. An offset landing
inside a character rounds forward to its start, which cannot happen for a span
the analyzer produced and is the only sane answer if it ever does.

### The starter rules file found a home, and it was the fixture

L2 recorded that nothing shipped and that finding it a home was L4's job. It is
`example-wiki/.rhizolog/prose.toml`: a rules file belongs to a wiki, this
repository has exactly one committed wiki, and the README already tells people to
point `RHIZOLOG_ROOT` at it. Five rules, one of each kind, with the em dash rule
written using TOML's escape because that is the demonstration.

The fixture gained a word log at the same time, and that was a bug rather than a
flourish. **L3 had quietly made the example wiki unreadable without changing
it**: the startup scan wrote nine baselines into `example-wiki/.rhizolog/words/`,
so merely running the server against the fixture dirtied it, and `AGENTS.md` was
still saying that pointing `RHIZOLOG_ROOT` there to look was fine. Eighteen
committed lines fix it properly rather than by warning: every page's count
already agrees with the log, so the scan finds nothing to record and the files
come back byte for byte identical. The property that made this a log on disk,
demonstrated on a wiki anybody can look at.

`example-wiki/index.md` states what both should report, exactly, as it already
did for the time log. Which makes its own word count load-bearing, because the
log's last line for `index` has to match it: editing that page means moving the
**baseline**, which carries no churn and so changes no total on the chart. The
note is in `AGENTS.md`.

### Verified against the fixture

Not a scratch wiki this time. `/api/word-stats?at=2026-08-06T18:00:00Z&offset=0`
answers 1,061 added and 114 removed across 10 observations and 8 pages, split
`file` 417, `web` 414, `claude-code` 230; `/api/prose?slug=index` answers 9
errors and 36 warnings; the orphan count, the wanted count and the time totals are
what they were. Then the whole thing again after deleting `index.db`, with the
two log files hashed before and after: identical.

Those five figures are the fixture as L4 left it, and the book below moved every
one of them. The current numbers are in `example-wiki/index.md`, which is the one
place they are meant to be read from.

One number in that set is worth keeping. `echo` at `within = 12` was the plan's
neighbourhood and it reported **60 warnings on a single page**. At `within = 8`
with a three-line `ignore` list it reports 36 on the same page and four or fewer
on every other page in the wiki. The rule is a lexical count with no stemming and
no stop-word list anywhere in the analyzer, so `ignore` is the only lever there
is, and a long page of documentation repeats its own vocabulary constantly. Widen
the list before lowering the number, or the repeats worth seeing go with the rest.

## The fixture gained a manuscript

L4 shipped with the Manuscript panel and `?assembled=1` built and nothing in
`example-wiki/` to point them at, which `TODO.md` recorded as the two screens the
fixture could not demonstrate. `example-wiki/book` is that manuscript: seven
pages, two parts, and ten sections in its manifest.

It is the only fiction in the fixture, and that is deliberate. Every other page
there is a note explaining what it demonstrates, which works for a wiki and does
not work here: what a compile has to be shown assembling is prose, and a chapter
that spends its body describing chapter structure would demonstrate the panel by
not being the thing the panel is for. So the pages are prose and every claim
about them lives in `index.md`, which is where the fixture already keeps its
claims.

Three of the ten sections are deliberately not assembled, one for each way that
happens:

- `book/two/the-crossing` is **wanted**, a chapter nobody has written, holding
  its position so the manuscript says where it was going to go.
- `book/appendix` is listed under both parts, because a ferry timetable belongs
  with the outward leg and the return equally, and its second position is
  **duplicate**. That is the diamond this status was named for rather than a
  cycle, demonstrated instead of asserted.
- `../one/the-ferry` is **invalid**, refused by `Slug` itself. The page holding
  it is still perfectly readable, which is the whole point of reading `contents:`
  as strings and parsing them at compile time.

`book/one/opening` carries a setext heading, so the compiled document is where
you can watch it become an ATX one at `###`. The root's `target: 2,000` is
written with the comma the L0 deserialiser accepts.

### The `names` rule finally has something true to find

`GET /api/prose?slug=book&compiled=true` reports the ferryman as `Maren` twice
beside `Marren` three times, and **no single page reports it**: each chapter is
internally consistent and the finding exists only across the section break. Up to
now `?compiled=true` was argued for and tested and had no worked example in the
fixture; `consistent` in particular had only ever been demonstrated by the false
positives it had to be taught not to make.

The chapter that carries the rarer spelling says out loud that the name is
written two ways and that nobody on the pier would settle it, which is the fixture
being a fixture: the finding and the reason for it are both in the wiki.

### A contents gap was not a wanted page, and the fixture is what found it

Writing the book surfaced an asymmetry nobody had had a reason to look at.
`referenced()` unions `page_parts`, so a chapter is not an orphan. `wanted_count`
did not, so `book/two/the-crossing` left `/api/stats` reporting one wanted page,
which was the wikilink to `notes/rust/streams` and nothing else.

The graph did not agree with that. A `contents:` entry pointing at nothing
becomes a node with `exists: false`, which the dashboard draws exactly as it
draws a wanted page, so the drawing showed two and the card beside it said one.

The first instinct was to leave it and explain it, on the reading that a wanted
page is a branch gestured at in prose while a gap in a manuscript is a hole the
manifest already reports in position. That does not survive looking at what
`referenced()` is. Orphans and wanted pages are the same phenomenon from either
end, which is written in the dashboard component's own doc comment: pages nothing
names, and names with nothing behind them. L1 merged the two kinds of naming on
one side of that sentence and left the other side alone, so the sentence had been
false since the day manuscripts existed. Splitting the mirror of a question you
have already merged is not two answers to two questions, it is one answer and one
oversight.

And on the merits, a chapter somebody put in a contents list is the strongest
statement this wiki has that a page ought to exist. It is more deliberate than a
wikilink, not less. A wanted list that omits it is answering something narrower
than its name.

So `named_but_unwritten()` is the union, and it sits next to `referenced()`
because they are one rule read in two directions. `links.wanted` stays link-only
and says so in its own description: it lives inside the link totals and a
contents entry is not a link.

### The rule about what counts as a slug became a column

Unioning the spine into a query that **names** pages made an existing bit of
caution load-bearing. `page_parts.target` is stored as written, because the
manifest has to report a mistyped chapter as `invalid` in its own position, so
`../etc/passwd` is genuinely in that table. The graph filtered it out with a
`Slug::parse` call in the loader. A wanted page is named on the dashboard as
somewhere to write, so a wanted query that forgot the same filter would have
advertised a path as a page somebody should create.

The filter could have been copied. It was made a property of the row instead:
`page_parts.is_slug`, decided by `Slug::parse` when the row is written, at schema
version 13. `visible_part()` checks it, so every reader gets it for free and the
graph's own call is gone. Slug validation is called security-critical in
[Architecture](architecture.md) precisely because it is the sort of rule that
gets spelled twice and drifts, and one query's private caution is exactly that
shape.

Not rebuilding this bump is the interesting failure rather than a stale row,
which puts it with version 7 rather than with the rest: an index written before
the column has nothing in it, and the entry that is not a slug goes on the
dashboard.

### What it cost, which was what was predicted

Nine pages became sixteen. The word log gained eight lines, seven of them the
book being written across the same week the time entries are pinned to, and one
of them a revision by `claude-code` that removed 120 words and added 96: the
motivating example of this whole page, on a page anybody can open. Every total in
`index.md` moved, and the `index` baseline was rebased from 1,040 to 1,909, which
is the manoeuvre `AGENTS.md` describes and the reason it is described there.

The orphan and wanted counts did not move, and that was worth arranging rather
than accepting: the book is linked from `index.md` so its root is not a third
orphan, and a contents gap is not a wikilink so it is not a second wanted page.
The property that reading the fixture writes nothing survived, which it only does
because every one of the seven new pages has a log line whose total already
matches it.

## Test strategy

### Pure unit tests

- Word counting: code fences, inline code, wikilink display text, tables,
  footnotes, an empty page, a page that is only a code block.
- Assembly: a page with no `contents:`, an empty list, a list naming a page that
  has its own list, a body followed by contents in that order, and a `contents:`
  entry that is a URL, a `..` path or an empty string.
- **Every relative spelling of an entry**, from a page that is nested rather than
  at the root: `./x` and `../x` are `invalid`, a bare basename resolves from the
  root and goes `wanted`, and none of the three ever reaches a page beside the
  one holding the list.
- **That a wikilink in a chapter's prose is never a section**, including one
  written on a page that also has a `contents:` list, which is the whole rule and
  the one a future refactor is most likely to break.
- Heading shift at depth zero, one and three, and a page with no heading.
- Repeats, both shapes: a page containing itself, a two-page loop, and an
  appendix listed under two parts, which is `duplicate` without being a cycle.
- Each rule kind against a fixture with the expected spans, including a finding
  whose quote contains a multi-byte character, since spans are bytes.
- Two rules firing on overlapping spans, both reported, in the documented order.
- Rule normalization: defaults filled in, and two spellings of the same rule
  producing the same digest.
- Sentence splitting, including the abbreviation case that is known to fail, so
  that its behaviour is pinned rather than accidental.
- The word diff: a pure rewrite (high `added`, high `removed`, `delta` near
  zero), an append, a deletion, and no change at all.
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
- A page listed twice under one parent produces two `page_parts` rows, and moving
  it to a third position leaves exactly the positions the file names.
- An external edit is attributed to `file`, and a labelled write to its label.
- **The word series survives deleting `index.db`**, not merely a schema-version
  rebuild. The earlier design passed the second and failed the first.
- A move keeps one series across both slugs; a delete and a new page at the same
  slug are two.
- On a wiki with accounts, an observation against an unreadable page reports its
  slug and no title, which is the time log's rule and not a new one.
- `GET /api/word-stats` refuses an anonymous caller even under
  `RHIZOLOG_ANONYMOUS_READ`.
- A finding's `rules_digest` matches what `GET /api/prose/rules` reports, and a
  rules file edited between the two calls makes them disagree rather than
  silently reconciling.
- A rules file that will not parse is reported as itself, an absent one is not an
  error at `POST /api/prose`, and `GET /api/prose/rules` answers an empty ruleset
  rather than `404`.
- Each compile limit refuses rather than truncating, and the error names the slug.
- Every error uses the standard envelope; every operation id is unique.

### Frontend tests

- A collapsed findings strip issues no request, matching the preview's rule.
- A finding's quote renders as text and produces no element from hostile content.
- The manuscript panel shows gaps, invalid entries and duplicates rather than
  hiding them.
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
definition is buying consistency nobody needs. Answered in
[Drafting](drafting.md), and the answer is that the definition was right and had
no reader: a chapter's own target is invisible from the book assembling it until
the manifest reports it. Nothing about the rule changes.

**Whether the word log wants pruning**, and what would be safe to prune. It is
authored data now, so the answer is not "delete the old rows"; it is closer to
what a rotated log does, and nothing about it is urgent at a few hundred
kilobytes a year.

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
