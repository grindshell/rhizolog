# Drafting

Status: **plan**. Nothing here is built. When it is, this page becomes the record
the way [Long-form writing](long-form.md) did, with a section per phase naming
every place the code departed from what is written here.

[Long-form writing](long-form.md) got a manuscript as far as existing: it can be
assembled, counted, and held to rules. What it cannot do is tell you anything
about a chapter without opening it. A book of forty pages has forty word counts
and forty titles, and no answer at all to the two questions somebody drafting
actually asks:

> What is this chapter supposed to do, and is it done?

Scrivener answers both with per-document metadata: a synopsis on an index card,
and a status. That is the gap this plan closes, plus two smaller ones next to it:
a chapter's own target, which the manuscript already half-supports, and a way to
keep a page in the spine and out of the book.

Everything here follows [Architecture](architecture.md): files are authoritative,
the index is derived, and a field somebody mistyped must not cost them the page.

## Goals

- Say what a chapter is for, in the author's words, without reading its prose.
- Say what stage a chapter is at, and let the manuscript summarise that.
- Give a chapter a target of its own, measured the same way the book's is.
- Keep a page in the spine and out of the compiled document, in position.

## Not goals

- **Anything that computes a stage.** A chapter is drafted when the author says
  so. Deriving it from word counts, edit recency or the word log would be the
  machine having an opinion about somebody's progress, which is the same thing
  [Idea Inbox](idea-inbox.md) refuses when it declines to move a thread's
  lifecycle on its own.
- **A rolled-up stage.** A part whose chapters are half revised is not
  "in progress"; it is whatever its own frontmatter says. The panel counts
  stages across a manuscript, which is a summary rather than an invention.
- **A synopsis derived from the prose.** See below; this is the substantial
  decision on this page.
- **Custom metadata fields.** Scrivener has them and they are the feature that
  turns a schema into a database. `tags` already carries whatever a wiki wants to
  say about a page, and a second free-form store beside it would be two answers
  to one question.
- **Pacing, reordering and splitting.** All three are real gaps, named in the
  survey this plan came out of, and none is in it. Pacing is arithmetic over
  `target`, `due` and the word log and needs no fields, so it is cheaper after
  this lands than before. Reorder and split are edits to the spine rather than
  facts about a chapter.
- **Colour as data.** Scrivener's Label is a colour with a name. Here a stage is
  a word, and the dashboard decides how to paint it. A palette in frontmatter is
  a document about a display.

## The fields

Four, all optional, all doing nothing on a wiki that does not use them, exactly
as `target`, `due` and `contents` do:

```yaml
---
title: The Ferry
synopsis: >-
  He misses the crossing and decides not to mind. First time the narrator
  chooses to be late for something.
stage: drafted
target: 3,000
compile: false
---
```

### `synopsis` is authored, and never inferred

A string. Blank lines are allowed and YAML has three ways to write one; all of
them parse, because this is a plain field and the parser is `serde_yaml_ng`.

**It is not derived from the body, and no fallback fills it in.** That is the
decision on this page most worth arguing, because Rhizolog already has a
fallback chain for `title` (frontmatter, then the first heading, then the slug)
and the obvious symmetry is to fall back to the first paragraph.

The symmetry is false. A title is a **name**, and every page has one whether or
not it says so, which is why inferring it is a service. A synopsis is a **claim
about what the chapter does**, and no page has one until somebody makes it. The
first paragraph of a chapter is prose that belongs to the book, addressed to a
reader who is inside the story; a synopsis is addressed to the author, from
outside it. Filling one from the other produces a card that is confidently
wrong, and worse, one that silently changes meaning every time the opening line
is revised.

Compile already settled this exact question in the same direction:

> A page with no heading contributes no heading, because the manifest records the
> boundary and inventing a title would be writing words the author did not.

A page with no synopsis has no synopsis. The card says so, and an empty card is a
chapter nobody has decided about yet, which is worth seeing.

**It is plain text, not markdown.** A synopsis is a card rather than prose: it is
shown at a glance, in a grid, at small sizes, and the one thing it must never do
is arrive as HTML. Page content is what agents write, which is the rule
`Snippet.tsx` exists to keep, and the cheapest way to keep it here is for there
to be nothing to render. It is not compiled, not counted in `words` (frontmatter
never is) and not checked by `prose/v1` (frontmatter is not checked in v1).

### `stage`, which wanted to be called `status`

A string. `status` is the obvious name, it is what Scrivener calls it, and it
loses to a collision: `SectionView.status` already means what compile did with an
entry, and it is `included`, `wanted`, `invalid`, `duplicate` or `unreadable`.
The manifest is precisely where a draft stage is most useful, so the two names
would meet in the one response that needs both. This project has renamed a thing
for less: `times.ended` is not `end` because `end` closes a `case` in SQLite.

So `stage`, which is also the better word: to-do, drafted, revised and final are
stages, and a stage is something a chapter passes through rather than a condition
it is in.

**The vocabulary is not fixed.** Four names are *known* and get a colour in the
dashboard, and anything else is shown as itself:

| Stage | What it means |
|---|---|
| `todo` | Named and not written |
| `drafted` | Words exist |
| `revised` | Been through at least once |
| `final` | Done unless something changes |

A writer whose process has `with-beta-readers` in it should not have to argue
with a schema, and this is a wiki whose whole stance is that these are your
notes. That makes `stage` one of the lenient fields rather than one of the strict
ones, which is a distinction L0 already drew and stated:

- `target` is **parsed**, and a bad value is refused, because a negative target
  is not a small one and reading it as zero would report a page as finished.
- `due` is **kept as written**, because a value that is not a date names no day,
  exactly as an `owner` that is not a username names nobody.

`stage` is `due`'s kind. It names no known stage, it is shown as typed, and it
does not take the page down with it. Compared case-insensitively for grouping and
colouring, stored as the author wrote it, which is the split `prose/v1`'s
`consistent` rule already uses on tokens.

Over the API it is strict at the boundary in the one way that matters: a `stage`
that is not a string is a `400`. Rhizolog is unforgiving at the API and lenient
about a file somebody typed, which is the same split slugs already get.

### `target` on a leaf earns the recursive definition

This was left open on the long-form plan page. Settled: **one rule, recursive,
no second concept.** `target` is measured against the compiled total from the
page carrying it, which on a leaf is its own words and on a contents page is the
whole work below it.

It was open because nobody had needed it on a leaf, and the answer turns out not
to need a change at all. It needs the manifest to report it, which is the work:
today a chapter's own `target` is invisible from the book that assembles it, so
the definition was already right and simply had no reader.

One consequence has to be handled rather than stated, and it is the reason this
is not a one-line change. A section's `words` in the manifest is its **own body**,
so on a part page it is the epigraph and nothing else. Comparing that against a
`target` that means the whole part would draw every part page at two per cent
forever. So the manifest gains a second number: see below.

### `compile: false` keeps a page in the spine and out of the book

Default true, so the field only ever appears as `compile: false` and the ordinary
page never carries it.

The case for it is position. A cut scene, an outline for a part, a page of notes
that belongs between chapters three and four: removing the entry from `contents:`
does say "not in the book", and it also throws away **where it went**, which is
the one thing the contents list knows and a wikilink does not. This wiki's pitch
is that it keeps unfinished thoughts, and a scene you cut but have not decided
about is exactly one.

Its manifest status is `excluded`, and it is a sixth status rather than an
absence, for the reason all five others are:

> A section with any status other than `included` still occupies its position in
> the list.

**Excluding a contents page excludes everything under it.** This is the rule with
a real alternative, and Scrivener takes the other one: there, excluding a folder
drops the folder's own text and its children compile according to their own
setting. That is wrong here, and the reason is headings. A part contributes its
heading and its epigraph and nothing else, so excluding only its body would leave
its chapters in the document with the part's heading gone, silently promoting
them under the previous part. A document that quietly restructures itself is the
same failure as a manuscript that quietly stops being the book, which compile's
limits already refuse to produce.

Two consequences worth stating because they are load-bearing rather than
incidental:

- **An excluded page is excluded from the book, not from the wiki.** It still has
  a `page_parts` row, so it is not an orphan, and the graph still draws the line.
  The spine is what the wiki knows about structure; `compile` is what this
  document is.
- **Excluded words do not count.** `target` measures the compiled total, and an
  excluded page is not compiled, so cutting a chapter moves the book's progress
  down. That is correct and it is the number moving for the right reason.

It interacts with `duplicate` better than expected. An appendix listed under two
parts, one of them excluded, is emitted once under the part that includes it and
is not a `duplicate` anywhere, because it was never emitted twice. No special
case: the rule is that `duplicate` means already emitted, and nothing excluded
was.

## What the manifest gains

Per section, four fields:

| Field | Why |
|---|---|
| `synopsis` | The card. Absent when the page has none, and absent for anything not `included`, exactly as `title` is |
| `stage` | Likewise |
| `target` | The page's own, from its frontmatter |
| `subtree` | This section's words plus everything emitted beneath it |

`subtree` is the one the plan would have missed. It is what `target` compares
against, and it exists because `words` is a section's own body and a part page's
own body is an epigraph. On a leaf the two are equal. A `duplicate` or `excluded`
section contributes nothing to any ancestor's `subtree`, because it contributed
nothing to the document, so the number always describes what a reader would
actually get.

The walk already knows the tree, so this costs an accumulation on the way back up
and no second traversal.

## What the dashboard becomes

- **The Manuscript panel gains columns.** Stage as a badge, the synopsis as one
  clamped line, and a small bar wherever a section has a `target` of its own. The
  panel is still the flat list the manifest is, because position is what compile
  promises.
- **A stage summary above it**, counted across the manifest: "6 drafted, 2
  revised, 1 to do". A count, on the same terms the words chart is on. No
  encouragement, no completion percentage dressed as an achievement.
- **A card view**, which is what a synopsis makes possible and is the corkboard
  without the part of the corkboard that stores coordinates. Freeform arrangement
  is not in this plan: order lives in frontmatter precisely so that nothing about
  a display can reorder a book, and an x and a y per card would be exactly that
  in a different coat.
- **The editor grows controls for all four**, and round-trips all four whether or
  not it shows them. The rule is already written down twice and was broken once:
  a `PUT` that leaves a field out clears it, and L4 found the editor silently
  unmaking manuscripts for four phases because nothing rendered the damage.

Synopsis renders as **text**, never through `innerHTML`.

## API surface

No new endpoints. Fields, and two query parameters:

| Where | What |
|---|---|
| `GET /api/pages` | `synopsis` and `stage` on each row; `?stage=` to filter, `?sort=stage` |
| `GET /api/pages/{slug}` | Both, plus `compile` |
| `POST`, `PUT`, `PATCH` | All four, with `PUT` clearing what it omits and `PATCH` distinguishing absent from null |
| `GET /api/compile` | The four manifest fields above |

`?stage=` is a filter on a string the author chose, so it matches exactly and
case-insensitively, and an unknown value returns nothing rather than an error:
asking for a stage nobody uses is a question with an empty answer, not a mistake.

## Index schema

Schema version 14: `pages.synopsis` and `pages.stage`, both nullable, filled at
index time from the frontmatter. They are columns because the listing returns
them and filters on them, and reading forty files to draw one table is the thing
the index exists to avoid.

`compile` gets **no column**. Nothing queries it: the compile walk fetches each
page from the store and already has its frontmatter, and the panel compiles
anyway. A column nothing reads is worse than no column, which is the argument
`idea_terms` was deliberately left out of version 8 for.

An index written before this bump has neither, so both come back empty rather
than wrong, and one scan fills them in. That puts it with version 10 rather than
version 7: stale, not leaking.

## Phases

### D0: the fields

`synopsis`, `stage` and `compile` on `Frontmatter`; the two columns at schema 14;
all four through create, replace and patch; `?stage=` and `?sort=stage`.

Done when a stage nobody has heard of survives a round trip and appears in a
listing; when a `synopsis` holding a colon, a blank line and a trailing space
comes back byte for byte; when `compile: false` round-trips and an absent
`compile` is not written into the file; when a `PUT` that omits all four clears
all four and one that sends them back does not; and when a schema rebuild
produces identical rows.

### D1: the manifest

`synopsis`, `stage`, `target` and `subtree` per section. The `excluded` status,
the subtree exclusion rule, and excluded words dropping out of every total.

Done when a part's `subtree` equals the sum of what was emitted beneath it and
its `words` is still its epigraph; when excluding a part removes its chapters
from the document and from the count while leaving all of them in the manifest in
position; when an appendix under an excluded part and an included one is
`included` once and `duplicate` nowhere; when an excluded chapter is still not an
orphan and still has its edge in the graph; and when compiling twice is still
byte-identical.

### D2: the panel, the cards and the editor

Columns, the stage summary, the card view, and four controls in the editor that
round-trip whether or not they are shown.

Done when a page carrying only a `stage` opens the manuscript block in the
editor; when saving a chapter from the dashboard leaves its synopsis, stage,
target and compile flag exactly as they were; when a synopsis containing markup
renders as characters; and when a manuscript with no stages anywhere shows no
summary rather than a row of zeroes.

### D3: documentation closure

[Architecture](architecture.md), [API design](api-design.md),
[The dashboard](dashboard.md), `AGENTS.md`, `TODO.md`, and this page from plan to
record.

**The fixture is part of this phase, not an afterthought.** `example-wiki/book`
is seven pages and would gain a synopsis and a stage on each, one `compile: false`
page, and a chapter with a `target` of its own. That moves the page count if the
excluded page is new, and it moves `index.md`'s own word count either way, so the
word log's `index` baseline is rebased the way `AGENTS.md` describes. Every
number `index.md` states is checked against a running server before this is
called done.

## Open questions

- **Does a synopsis want to be searchable?** It is the natural way to find "the
  chapter where they cross", and `pages_fts` indexes slug, title and body today.
  Adding a fourth column moves `FTS_BODY_COLUMN`, which `snippet()` indexes by
  position, so it is a real change rather than a line. Deferred, not declined.
- **Should `stage` have a wiki-wide summary?** `/api/stats` counts orphans, wanted
  pages and tags. Stages across the whole wiki is a different question from stages
  across one manuscript, and it is not obvious anybody is asking it.
- **What happens to a stage when a page is promoted from Idea Inbox?** Promotion
  writes an ordinary page through the ordinary API. `todo` would be defensible and
  so would nothing, and nothing is the smaller claim.
