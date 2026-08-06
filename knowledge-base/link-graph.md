# Drawing the link graph

`GET /api/graph` and the `/graph` screen: the wiki as a picture rather than a
count. See [Architecture](architecture.md) for how links are extracted and
stored, [API design](api-design.md) for the endpoint surface this joins, and
[The dashboard](dashboard.md) for the rest of the UI.

## Why a drawing at all

The dashboard already counts the graph — links, resolved, wanted, orphans,
most-linked — and those numbers are the right answer to "is the wiki healthy".
They are the wrong answer to "what does it look like". "Six orphans" does not
tell you that all six are in `scratch/`; a cluster hanging off the side of the
picture with nothing joining it to the rest tells you at a glance, and that is
the only thing a drawing does better than a list.

The premise of this project is that knowledge branches chaotically. A screen
that shows the branching is not decoration on that idea; it is the first place
the idea is visible.

## What is a node

Pages, **including ones nobody has written**. A wanted page is the far end of a
link somebody wrote, which is precisely a branch gestured at, and a graph that
drew only what exists would show a tidy wiki that isn't there.

Time is not in the picture at all, for the reason
[Time tracking](time-tracking.md) gives: a page collects a time entry every time
a timer starts, so those edges are counted in hundreds where links are counted in
ones. Mixed in they would be the graph.

## Three rules decide what comes back

`tag`, `prefix` and `root` all narrow the view and all intersect. What makes
them consistent is stating the rules in this order:

1. **The filters select pages.** `tag` and `prefix` mean exactly what they mean
   on `GET /api/pages`; `root` plus `depth` narrows to a neighbourhood.
2. **An edge is returned when both of its ends survived.** So a filtered branch
   shows its internal structure and not its ragged half-edges.
3. **A wanted page is not a page.** It has no row anywhere — no tags, no path on
   disk, nothing a filter could ask about — so no filter can apply to it. It is
   returned wherever a link in the view reaches it.

Rule 3 is the one that looks wrong and isn't. Under `?tag=rust`, a want has no
tags and would vanish; under `?prefix=notes`, a want at `code/…` sits outside the
path. Dropping it in either case makes the view claim that every link in it lands
somewhere, which is the one thing this wiki's whole model says is not true.

The single exception is the walk: with a `root`, a want must also be within
`depth` hops, or `depth` would be a promise broken at the edges of the picture.

## The walk goes both ways

`?root=` follows links in **either** direction, for the same reason
`GET /api/links/{slug}` returns `outbound` and `inbound` from one call: a page's
neighbourhood is what it points at *and* what points at it. A walk that only
followed arrows forward would answer "what does this page reach", which is a
much less interesting question than "what is this page among".

It is a recursive CTE over `links`, and `union` rather than `union all` is what
makes a cycle terminate — it drops rows already produced. A node can still be
reached at two distances, so the outer query takes the smaller.

The root need not name a page that exists. A wanted page has a neighbourhood and
it is exactly the set of pages waiting on it, which is the most useful thing you
can see before deciding to write it.

## Degrees count the whole wiki, not the view

A node's `inbound` and `outbound` are wiki-wide even inside a filter. Two
reasons, and the second is the good one:

- A hub keeps looking like a hub when you filter, so the picture does not
  reshuffle its own emphasis every time you narrow it.
- **The difference is the signal.** One line arrives at `notes/rust` in a
  `?prefix=notes/rust` view and the node says two pages link to it — which says
  this branch is attached to something outside what you asked for. The detail
  panel prints both numbers side by side for exactly that.

Anything the client could compute from `edges` is left to the client. This is
the number it cannot.

## Truncation drops leaves, not hubs

`limit` caps **pages** (wanted pages hang off the survivors and are not counted
against it). When it bites, the best-connected survive: a graph cut down to its
least connected pages is a scatter of dots that says nothing at all, which is a
worse answer than a partial picture that still has a shape.

## The layout is deterministic, and that is the point

`frontend/src/components/graph/layout.ts` is a force-directed layout written out
rather than pulled in — the same trade the time charts made, and for the same
reasons. But it is not just a small d3-force: it deliberately lacks the one thing
every published force layout has.

**Nothing here reads a clock or a random number.** Seeds come from an FNV-1a hash
of the slug, placed on a phyllotaxis spiral; every force is a pure function of
the graph. The same wiki therefore draws the same picture every time it is
opened.

That matters more than it sounds. A graph is good at one thing — being
recognised — and a random seed destroys it: you cannot tell whether the shape
changed because the wiki changed or because the layout rolled differently.
Determinism turns the screen into something you can compare against yesterday.

Two consequences worth knowing:

- **Renaming a page moves it**, because its seed is its slug. That is honest: a
  rename produces a different node, and the wiki's own stats already treat it
  that way — inbound links become wants.
- **The layout is testable**, which is why `runLayout` is exported separately
  from anything that draws. `layout.test.ts` asserts the same graph twice, and
  the same graph with its nodes in reverse order, produce identical coordinates.

### It settles before it is drawn

`runLayout` runs to a fixed tick budget and returns finished positions. There is
no animation loop. An animation is prettier for three seconds and then over, and
what remains is the same picture arrived at more slowly with a frame loop still
installed — and, with a deterministic seed, an animation that plays the same way
every visit is a cutscene.

Repulsion is O(n²) per tick, so `ticksFor` scales the budget down as the graph
grows. Barnes-Hut is the fix if a wiki ever needs it, and would be several
hundred lines to save milliseconds at the sizes `limit` allows.

### Two constants are load-bearing

- **Rest length is generous (150) relative to the node discs (5–20 radius).**
  The first version used 90 and settled the example wiki into a picture where
  linked nodes nearly touched and the arrow between them had about ten units to
  exist in. Edges have to be longer than the things they connect, or the
  direction they point in — most of what the drawing has to say — is unreadable.
- **Gravity exists at all.** An orphan has no spring holding it and only
  repulsion pushing it, so without a pull toward the centre it accelerates out of
  frame forever and everything else collapses to a dot in a huge empty box.
  Orphans are exactly what this screen is for, so this is not an edge case.

## Drawing decisions

**Edges bow.** A mutual link drawn straight is one line with one arrowhead
buried under the other, so the commonest and most interesting relationship in a
wiki would be indistinguishable from a one-way link. Every edge curves to the
left of its direction of travel, which turns a mutual pair into two arcs.

**Labels are rationed.** Everything is labelled while the graph is small; past
about forty-five nodes only the hubs keep theirs, plus whatever is hovered or
selected and its immediate neighbours — the set you are asking about the moment
you point at something. Every label drawn beyond that is a label overlapping
another.

**Three appearances mean three things**: a filled disc is a page, a dashed
hollow ring is a wanted page, and a second ring marks the root of a walk. Discs
are sized by the square root of degree, because area is what the eye compares —
scaling the radius linearly makes ten links look four times five rather than
twice it.

**The `viewBox` is matched to the container's aspect ratio** rather than left to
`preserveAspectRatio`. Otherwise the mapping from a pointer position to a graph
coordinate is not linear, and zoom-toward-the-cursor drifts.

## Deliberately not in the first version

- **Dragging a node.** It would mean keeping the simulation alive after the
  screen settles, which is the animation loop this design just spent its budget
  avoiding. Pan, zoom, hover and select cover exploring; nothing here needs the
  reader to arrange the graph by hand.
- **Clustering by tag or directory.** The forces know only about links. Colouring
  or grouping by tag is a plausible next step and would need a legend, a palette,
  and an answer for pages with four tags.
- **Time on the graph.** Not a rendering decision — see above, and
  [Time tracking](time-tracking.md).
