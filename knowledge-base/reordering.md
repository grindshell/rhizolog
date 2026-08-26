# Reordering the spine

Status: **built**. A record rather than a plan: this was one of the three gaps
[Drafting](drafting.md) named and left, and `TODO.md` had already settled the
shape as "a drag that ends in a `PATCH` of `contents`". Half of that is what got
built; the other half is argued below.

[Long-form writing](long-form.md) made a decision and named its price:

> Order lives in frontmatter so that reflowing a paragraph cannot reorder a book,
> and the price of that is real: open `book.md` raw and you get a YAML list rather
> than a clickable index. The Manuscript panel on `/pages/book` is what pays it
> back.

It paid back reading and not editing. Moving a chapter meant opening the parent
in the editor and rewriting a YAML list by hand, in a textarea, without seeing the
book. That is worse than the thing the decision was protecting against.

## Goals

- Move an entry within the list that names it, from the panel that draws it.
- Never lose an entry a compile could not resolve. A gap, a repeat and a typo are
  all things somebody wrote.
- No new endpoint, and no new frontmatter.

## Not goals

- **Moving a chapter between parts.** Two lists change, which is two writes and a
  question about what happens if the second fails. Editing the lists by hand is
  the way to do it and the panel says so out loud rather than leaving somebody to
  discover that the buttons will not do it.
- **Freeform arrangement.** Refused once already, on the card view, and the
  argument has not changed: order lives in frontmatter precisely so that nothing
  about a display can reorder a book, and an x and a y per card would be exactly
  that in a different coat.
- **Adding or removing entries.** That is writing a book rather than arranging
  one, and the editor already does it.

## The manifest says who named each entry

Two fields per section, `parent` and `ordinal`, absent together on the root and on
a `?style=` preamble because nothing named those.

They are what the panel was missing. It draws the **flat, recursive** manifest,
where `book/one` and `book/one/opening` are both rows; moving the second means
editing `book/one`, and moving the first means editing `book`. Compile knew that
and did not say it. It even said so in its own record, under D1:

> the walk never has to know who its parent is

True of `subtree`, which is a fact about what comes after. Not true of an edit,
which is a fact about which list an entry is in.

**They are kept on a section that is not `included`**, where everything else a
page would say about itself is dropped. That looks like an exception to the rule
`title` set and is not the same kind of field: `title`, `synopsis` and `stage`
describe a **page** the reader will not get, and `parent` and `ordinal` describe
the **entry**, which is exactly what is still there. A gap that could not say
which list named it would be a gap nothing could move or correct, and those are
the entries most likely to need it.

### The ordinal is an identity, not a row number

This is the one that would have been a bug. A contents list may name the same
child twice, which is what `page_parts` is keyed `(src_slug, ordinal)` for. Worse,
**a page reached down both an excluded path and an included one is walked twice**,
because `emitted` is deliberately not consulted on the excluded path (which is
what makes an appendix under one cut part and one live part come out `included`
once and `duplicate` nowhere). So its children appear in the manifest twice,
carrying the same ordinals.

Rebuilding a contents list by collecting rows in order would therefore double it,
and the write would be a book with every chapter in it twice. Rebuilding by
ordinal is correct in both cases and is what the field's doc comment tells a
client to do.

A list whose ordinals are not exactly `0..n-1` is left out of the rebuild rather
than repaired. Nothing in a successful compile produces one, since the limits
refuse rather than truncate, so this is the case that should not happen: a chapter
that cannot be moved is a far smaller problem than one that is silently dropped.

## Why the write is a `PATCH` and not an endpoint

`POST /api/reorder {parent, from, to}` was the alternative and it has one real
advantage: it never sends the list, so it cannot write back a stale one.

It loses on two counts. `contents` is already patchable, so this would be a second
way to say a thing the API can already say, and two answers to one question is
what this project renames fields to avoid. And the stale-list problem it solves is
not this feature's: **the whole dashboard is read-modify-write already.** Saving in
the editor replaces a page with what was on screen when it opened, which is a
bigger window over a bigger surface. Inventing an operation-based endpoint for one
field of one page, while the editor clobbers whole pages, would be treating the
smaller instance of a problem as though it were the only one.

So the window is real and is named rather than closed: the list written back is
rebuilt from the compile the panel is showing, so a chapter added in your own
editor since then would be written out of the spine. The panel re-reads the book
after every move, which makes that visible rather than silent. Closing it properly
means conditional writes, which is a thing this API does not have anywhere and
should not grow in one corner.

## Buttons, not a drag

`TODO.md` said "a drag that ends in a `PATCH` of `contents` is the shape", and the
shape is a pair of buttons per row.

A drag needs a keyboard alternative to be usable at all, and that alternative is a
pair of buttons, so the real choice was between buttons and buttons plus a second
way in. Buttons also work on a phone, where an HTML5 drag does not, and they are
testable in jsdom, where drag events are mocked into something that proves very
little. A drag can be laid over this later and would end in the same write.

They are labelled by the entry rather than by the direction: a column of "Move up"
buttons read out one after another says nothing about which chapter each one
moves.

## Reorder is a view, not a mode switch beside the views

The panel already had a List / Cards toggle. Reorder is a third entry in it rather
than a fourth control saying "now you may edit" beside three saying "look at it
this way".

**Being modal is the point.** Order lives in frontmatter so that nothing
incidental can move a chapter, and a mode you have to enter is that same argument
carried into the one place that can. It also keeps the default reading of a book
free of a pair of buttons on every row.

There are no reorder controls in the card view. Up and down in a grid that wraps
means something different in every column width, and the list is where position is
legible.

## What it does not gate

The controls are shown to anybody who can see the panel, and a refused write is
reported where it happens. That is not an oversight: `/pages/*slug` already carries
an Edit button on the same terms, and inventing a "may write" signal here while the
button next to it does not consult one would be two answers again. Rhizolog has no
write permission distinct from read; `TODO.md` has said so under Accounts since
accounts landed.

## What is not done

- **A drag.** See above. The write it would end in exists.
- **Moving between parts.** Two lists, two writes, and a question about the second
  failing.
- **Undo.** A wiki directory is very likely a git repository, which is the answer
  [Architecture](architecture.md) already gives for history.
- **Reordering from the card view.** Deliberate rather than pending.
