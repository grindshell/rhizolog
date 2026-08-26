# Reordering the spine

Status: **built**. A record rather than a plan: this was one of the three gaps
[Drafting](drafting.md) named and left, and `TODO.md` had already settled the
shape as "a drag that ends in a `PATCH` of `contents`". The buttons came first
and the drag was laid over them afterwards, which is argued below and is the
order rather than the delay.

[Long-form writing](long-form.md) made a decision and named its price:

> Order lives in frontmatter so that reflowing a paragraph cannot reorder a book,
> and the price of that is real: open `book.md` raw and you get a YAML list rather
> than a clickable index. The Manuscript panel on `/pages/book` is what pays it
> back.

It paid back reading and not editing. Moving a chapter meant opening the parent
in the editor and rewriting a YAML list by hand, in a textarea, without seeing the
book. That is worse than the thing the decision was protecting against.

## Goals

- Move an entry within the list that names it, from the panel that draws it, by
  dragging it or by pressing a button.
- Never lose an entry a compile could not resolve. A gap, a repeat and a typo are
  all things somebody wrote.
- No new endpoint, and no new frontmatter.

## Not goals

- **Moving a chapter between parts.** Two lists change, which is two writes and a
  question about what happens if the second fails. Editing the lists by hand is
  the way to do it and the panel says so out loud rather than leaving somebody to
  discover that the buttons will not do it.
- **Freeform arrangement.** Refused once already, on the card view, and a drag
  existing has not changed the argument: order lives in frontmatter precisely so
  that nothing about a display can reorder a book, and an x and a y per card
  would be exactly that in a different coat. What this drag moves is an entry in
  a list, and where it lands is a position in that list rather than a place on
  the screen.
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

## Buttons first, and then a drag over them

`TODO.md` said "a drag that ends in a `PATCH` of `contents` is the shape". The
buttons were built first and the drag was laid over them afterwards, which is the
order that mattered rather than a delay.

A drag has no keyboard, so whatever else it is, it can only ever be the second
way in. Building it first would have meant building the buttons anyway and
calling them the fallback; building them first meant the gesture had nothing to
prove and could be refused outright if it did not work. Both end in the same
`PATCH`, because `move` takes a **position** rather than a direction: a button
asks for the place next door and a release asks for the place it landed on, and
neither of them is a different write.

The buttons are labelled by the entry rather than by the direction: a column of
"Move up" buttons read out one after another says nothing about which chapter each
one moves.

## Pointer events, not HTML5 drag and drop

The first version of the drag was `draggable` on the row and the browser's own
drag and drop. It worked on a desktop and nowhere else, because **a native drag
is a mouse gesture**: there is no touch equivalent of it, and no amount of
attributes makes one. So the whole of it was replaced with one pointer stream,
which is a mouse, a pen and a finger without knowing or caring which.

`dragging.ts` is what that costs. Everything a native drag did for free is done
by hand there: a movement threshold, a hit test, scrolling the window near its
edges, Escape, and clearing up after a gesture the browser takes back. That is
not a small file for what it does.

Two things bought it back and either would have been enough.

**The gesture can be driven.** A native drag cannot be started by any event a
script dispatches, so the whole of the first version was provable only by hand,
and the tests said as much out loud. These handlers read a coordinate and do
their own arithmetic, so a test that dispatches a pointer runs the code a finger
runs. The drag is now checked in a fake DOM by the test suite and was checked
end to end in a real browser **as a touch pointer**, which is exactly the thing
that could not be reached before.

**The drag starts where it is told.** The source of a native drag is the nearest
draggable ancestor of whatever the pointer went down on, which meant the two move
buttons were inside a drag source and a press on one that drifted might have
begun a drag instead of firing a click. A review raised that and could not settle
it, since it needed real mouse input to reproduce. It is not a question any more:
`pointerdown` is on one element and that element is the grip.

### The grip is the handle, and the row is not

This is the reversal the change forced, and the reason is scrolling. On a phone,
a row that answered a drag would be a row you could not scroll past, and a list
of chapters nobody can scroll is worse than one nobody can reorder. `touch-action:
none` says which patch of the screen is the gesture's; it is on the grip, and the
rest of the row is still the page's to scroll.

The argument for the row carrying it was the **drag image**, since the picture a
browser makes of a grip is the glyph rather than the chapter. That argument is
gone with the native drag: there is no drag image now, and what says where a row
will land is the ring on the row it will land on, which was always the part doing
the work.

Two things fall out of the grip owning the gesture. A row's text can be selected
again, since nothing needs `user-select: none` but the grip. And the section link
is a link again, with no `draggable="false"` on it, because nothing is competing
for the gesture.

It stays hidden from assistive technology. A drag has no keyboard, so a handle
nothing can grab from one would be a control that does not work; the two buttons
beside it are the ones that do.

### What the gesture is made of

| Piece | Why |
|---|---|
| A **six pixel threshold** | A press that never travels is a press. Nothing lifts, nothing dims, and letting go is not a move nobody asked for. |
| A **hit test by vertical position alone** | A pointer wandering off the side of the list is still plainly pointing at a row. It also makes the question arithmetic over an array of spans rather than a DOM lookup, which is why it can be tested. |
| **Eight pixels of reach** past a row's edge | The rows are four pixels apart, and a gap belonging to neither of them would put the mark out every time a pointer crossed one. |
| **Listeners on the window**, not the row | A drag leaves the row it started on immediately and by design. A finger is captured to its target anyway; a mouse is not, and would stop reporting at the next chapter. |
| **Scrolling near the edges** | The native drag did this. Without it the only chapters reachable are the ones on screen, which on a phone is about four. |
| **Escape**, and `pointercancel` | Two ways out that the native drag had: one for a person who thought better of it, one for a phone deciding the gesture was really something else. |

The release moves the entry to wherever **the mark** is, rather than hit testing
again on the way up. There is no arrangement of the two that can disagree,
because there is only one of them: what was on screen when the pointer came up is
what gets written.

### A release takes a position, and most rows have none to give

Letting go over a row means taking that row's place, which is `moved(list, from,
to)` and is exactly what the buttons do one step at a time. The ring marks the
position the held row will occupy, which is true going up the list and going
down it.

The rule is the same one the buttons are on, and a drag is what makes it possible
to break: an entry moves within the list that names it. The panel draws a **flat,
recursive** manifest, so a chapter's own scenes sit between it and the next
chapter and are most of what a dragged row passes over. Letting go on one of those
reads as both "before the part" and "into the part", and answering it would be
picking one on somebody's behalf. So there is simply nowhere to land there: no
ring, and a release that writes nothing.

Every row is dimmed while a drag is on unless it is somewhere the held row could
end, which teaches the rule rather than stating it. **The dimming is on the row
and not on its controls**, and that is the difference that matters for the rows
that have none: a section in a list this panel could not rebuild gets no buttons
and cannot be dragged, and it is still nowhere a release can go. Left bright
beside eight rows that are dimmed, it would be the one row on screen claiming to
accept what it will not.

Off the end of the list there is no row at all, so the mark goes out and letting
go there is how a drag is called off by somebody with no keyboard to press
Escape on.

### What the tests say about it, which is now most of it

The pure half is in two files. `spine.ts` holds the rule about where an entry may
go, and `dragging.ts` holds `rowAt`, which answers which row a vertical
coordinate is over given the rows' spans. Both are ordinary functions with
ordinary tests.

The gesture itself is driven in jsdom. The rows are handed spans by hand, since
jsdom lays nothing out and would otherwise stack every row on the same point, and
that is the whole of the fake: **the handlers a test drives are the handlers a
finger drives**, because a pointer drag is arithmetic over coordinates rather
than something the browser does on our behalf. So the threshold, the refusal, the
marks, Escape, the guard while a write is in flight, and the `PATCH` a release
ends in are all covered.

What is left over is small and was checked against a running server on
`example-wiki/`, whose book is ten rows in three lists and is the shape the flat
manifest makes awkward. Driven as a **touch** pointer: a part picked up there
dims eight of the other nine, the ninth being its only sibling; three pixels of
travel is still a press; `pointerdown` comes back cancelled, so no selection
starts; and Escape, a release off the end and a `pointercancel` all put it down
without writing. The edge scrolling was exercised with the frame loop shimmed,
since the pane it ran in never composites: it stays still well inside the window,
scrolls the right way at a speed set by how near the edge the pointer is, and
stops both on leaving the margin and on release.

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

- **A row that follows the finger.** Nothing moves during a drag: the held row
  dims where it is and the ring says where it would land. A native drag drew a
  ghost and this draws none, which is a real difference on a phone, where the
  finger is on top of the row it is holding. It is a transform on one element and
  it is deliberate that it is not there yet, because the ring is what actually
  answers "where will this end up" and a ghost would be decoration over the top
  of it.
- **A long press to drag from anywhere on the row.** The grip is a sixteen by
  twenty target, which is smaller than the forty-four a finger wants. A long
  press on the row would be the usual answer and it needs a timer, a way to tell
  it apart from a scroll that began slowly, and a decision about what it does to
  text selection. The two buttons are the same size and are the primary control,
  so the grip is not out of step with what is beside it.
- **Landing between two rows rather than on one.** An insertion line is the
  clearer affordance and it needs a geometry this list does not have: the gaps in
  a flat, recursive manifest are between entries of different lists as often as
  not, so half of them would mean nothing. Taking a row's place is unambiguous
  everywhere.
- **Moving between parts.** Two lists, two writes, and a question about the second
  failing. A drag makes the gesture obvious and it is still the two writes.
- **Undo.** A wiki directory is very likely a git repository, which is the answer
  [Architecture](architecture.md) already gives for history.
- **Reordering from the card view.** Deliberate rather than pending.
