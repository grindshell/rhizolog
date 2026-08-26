# Splitting and merging

Status: **built**. A record rather than a plan: this was the last of the three
gaps [Drafting](drafting.md) named and left, after [Pacing](pacing.md) and
[Reordering the spine](reordering.md), and `TODO.md` had already said what it
was for.

> Splitting a page at an offset and repairing the parent's contents list is
> mechanical, error-prone by hand, and exactly what an API should do.

Doing it by hand is four operations that have to agree: read the chapter, cut the
body at the right place, write two files, and edit the list that named the first
so it names both, in the right order. Getting the last one wrong is a book with a
chapter missing, and nothing tells you.

## Goals

- Cut a page in two at an offset, and put the second half in the spine where the
  text was.
- Fold a page into another and take it out of every list that named it.
- Never lose an entry a compile could not resolve.
- Never report words as written or unwritten when nothing was.

## Not goals

- **Splitting at every heading at once.** Outlining a chapter into six is a
  different feature and a much more opinionated one. Six requests do it.
- **Rewriting inbound links.** A move already leaves them alone, on the grounds
  that a link is something another page said and is not ours to rewrite. A page
  that was merged away becomes a wanted page, which is exactly what `/api/stats`
  is for.
- **Undo.** A wiki directory is very likely a git repository, which is the
  answer [Architecture](architecture.md) already gives for history.

## The API

Two endpoints, both outside the slug namespace for the reason `/api/move` is
there: `matchit` requires a catch-all to be the final segment, so nothing can
follow a slug, and a literal `/api/pages/split` would shadow any page actually
slugged `split`.

| Endpoint | Body | Answers |
|---|---|---|
| `POST /api/split` | `from`, `at`, `to`, `title?` | `head`, `tail`, `repaired` |
| `POST /api/merge` | `from`, `into` | `page`, `removed`, `repaired` |

`repaired` is every `contents:` list that was rewritten and what each one says
now. A receipt rather than an acknowledgement: the caller asked about one page
and two other files changed, so the response says which and to what.

### The offset is in bytes

Not characters, not lines. It is the unit a `prose/v1` span already uses, and
those two numbers are about the same text: a finding quotes a span of the page
source, and an offset saying where a scene ends is the same kind of number. A
client that can reveal a finding in a textarea can already produce one of these,
and the dashboard does exactly that, through the inverse of the same function.

An offset that lands inside a character is refused rather than rounded, and the
refusal carries the body's **length**. That is the one thing the caller cannot
work out for itself: the offset was computed from a body it may no longer be
holding.

An offset with nothing on one side of it is refused too. That is a rename or a
blank page, and both have endpoints already.

### What the second half inherits

| Carried | Not carried |
|---|---|
| `tags`, `visibility`, `owner`, `readers`, `stage`, `due`, `compile` | `synopsis`, `target` |

The visibility is the one that would be a defect rather than a surprise:
splitting a private page must never leave half of it readable by somebody the
whole of it was not.

The two that are dropped are dropped for the reason
[Drafting](drafting.md) refuses to derive a synopsis at all. A synopsis is a
**claim about what a chapter does**, and the half cut off one is not that
chapter; an empty card is a chapter nobody has decided about yet, which is worth
seeing. A target is a **quantity**: copying it would double what the book is
aiming at, and halving it would be arithmetic nobody did.

`due` goes the other way from `target` and the difference is what each one is. A
date is not divisible. Both halves are due the same day, and saying so is not a
calculation.

A merge is simpler: **the destination keeps its own frontmatter entirely.** Its
target still says what it said, now over more words, which is a thing for its
author to decide about rather than for two numbers to be added together behind
them. The source's frontmatter goes with the source.

### Neither will touch a page that assembles others

The same rule read in two directions, and it is the decision on this page most
worth arguing.

**A merge moves text to where the caller said.** Both slugs are in the request.
If that moves a paragraph past four chapters, that is what was asked for.

**A split moves the second half to where the first half is**, which is a position
it has to *derive*. On a page with chapters under it, that position is after
every one of them: the epigraph of a part would be cut in two and the second half
would compile after the whole part. That is a document quietly restructuring
itself, which is what compile's limits already refuse to produce and what
`Status::Excluded` exists to stop a cut part doing to its chapters.

The alternative for a split was to put the tail **first in the source's own
contents list**, which compiles in the right place. It loses on headings: the
tail would be one level deeper than the text it was cut from, so a scene would
come back as a chapter. Two wrong answers is a refusal.

Merging away a page that assembles others is refused for the plainer reason that
its chapters would be named by nothing.

So both are `409 page_assembles_others`, a conflict rather than a bad request,
because the request is well formed and it is the state of the page that refuses
it. Editing the two lists by hand is how to mean either.

## Repairing the lists

Every `contents:` list naming the page is rewritten, in every parent. `page_parts`
is what finds them, matching the entry as **the string somebody typed**: an entry
with a stray space in it is a different entry and is left alone rather than
quietly corrected on its owner's behalf.

That is also what makes a gap, a repeat and a typo survive a repair. They are
things somebody wrote, and they are what a compile can say least about, which is
the promise [Reordering the spine](reordering.md) already makes.

**A list that already names the destination is left alone.** Splitting into a
chapter somebody outlined and never wrote is filling their gap; a second entry
for it would be a `duplicate` for them to clean up. Where they put it is where it
stays, because the position is theirs and this has no better one to offer.

A parent the caller cannot read is passed over without a word. That is the
silence a compile already gives a chapter it may not fetch, and reporting the
skip would answer "does a page you cannot see list this one" for the price of one
request.

None of it is observed in the word log. The body is not touched, so nothing was
written and the total the log checks a page against does not move. It is still
indexed, because `page_parts` is what the manifest and the orphan count are read
from.

## What the word log needed, and would have got wrong

This is the part that would have been a defect rather than a gap.

Left to the ordinary path, a split would record the first half losing two
thousand words and the second half gaining them, on a day somebody moved a
cursor. A merge would record five hundred words written that were written last
month. That is the signed net this whole feature exists to refuse, wearing a
different coat, and it would have gone into the chart.

So two kinds, `split` and `merged`, on the terms `moved` was already on:

> A marker rather than a churn: nothing was written.

Both are zero added and zero removed, so `is_work` is false and the chart does
not move. Both carry the **total**, which is the point of writing them at all:
`total` is what every slug's series is checked against, and without a line the
next startup scan would find two files disagreeing with the log and report the
difference as a `net`. The same wrong number with a worse label on it.

**Neither closes a series**, and that is the difference from `moved`. A page that
was split is still that page and its history runs straight through; the half cut
off it begins one, naming where it came from so a reader can follow a chapter
across the cut. The page that was merged away is closed by its own `deleted`
marker, which the delete writes anyway.

The rule the SQL already had turns out to be exactly right, which is worth
knowing: `last_total` vacates a slug on `kind = 'deleted'` at it or
`kind = 'moved'` **from** it, and it names the kind rather than testing for a
second slug. A `split` line carrying `from` therefore does not close the series
at the page it names. A test says so, because the next person to add a marker
with two slugs in it will have to know.

## In the editor, not in the panel

[Reordering the spine](reordering.md) went in the Manuscript panel because order
is what the panel draws. This does not, and the reason is the cursor.

A split needs an offset into a body, and the only thing that knows where a
chapter should stop being one chapter is the person reading it. The editor is
where the body is. Merge went with it rather than into the panel, so that the two
halves of one idea are in one place, and because "this page should not be its own
page" is a thought you have while looking at the page.

What the block shows is what the split would make, rather than a number: the
first line with anything on it after the cursor. On a chapter cut at a heading
that is also the title the new page will take, which is why the title field can
be left empty and says so. The slug field is prefilled with the directory the
page sits in and no further: where its siblings live is a fact, and what the new
one is called is the half nothing here knows.

Both are disabled while there are unsaved changes, which is Rename's rule and
Rename's reason: they act on the file the server holds, so an offset into a body
with edits pending would cut a page that is not the one being cut. A page that
assembles others gets the sentence instead of the controls, rather than a button
that fails when it is pressed.

A split lands in the editor for the page it made, because that is the half that
needs a person: no synopsis, no stage, no target, and a title it inherited from a
heading. The half left behind is finished and saved, and Back returns to it.

### The layout toggle's middle button is called "Both" now

It was "Split", meaning panes. Two buttons a hand apart, one saying Split and
meaning panes and the other saying Split and meaning the page, is a question
nobody should have to answer.

The stored **value** is still `split`, because it is what is in somebody's
`localStorage` and renaming it would silently reset the pane arrangement of
everyone who had ever chosen one.

## What is not done

- **Splitting from the Manuscript panel.** There is no cursor there. A control
  that split a chapter at its first heading would be a guess at where the seam
  is, which is the thing this whole feature exists to let somebody decide.
- **Merging a chapter into its neighbour in one gesture.** The panel knows which
  entry precedes which, and the editor does not, so this is the one thing a panel
  control would add. It needs a spine on screen and a page to act on at the same
  time, which is a layout question rather than an API one.
- **Splitting a page that assembles others.** Refused, above, and it stays
  refused until somebody says what should happen to the chapters.
- **The list written back is as old as the read that found it.** Same window as
  [Reordering the spine](reordering.md), same reason it is named rather than
  closed: conditional writes are a thing this API does not have anywhere and
  should not grow in one corner.
