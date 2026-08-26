# Pacing

Status: **built**, `pace/v1`. This page is a record rather than a plan.
[Drafting](drafting.md) named pacing as one of three gaps it deliberately left,
with the note that it "is arithmetic over `target`, `due` and the word log and
needs no fields, so it is cheaper after this lands than before". That turned out
to be exactly right, so there was nothing to phase: it was designed and built in
one pass. Everywhere a decision was made against a real alternative, the
alternative is written down here.

[Long-form writing](long-form.md) gave a manuscript a length and a target.
[Drafting](drafting.md) gave every section its own target and a `subtree` to
measure it against. Both answer *how far along*. Neither answers the question a
deadline makes people ask:

> Am I going to finish this by then, at the rate I am actually going?

The three inputs were already on disk. `target` says how long the work should
end up. `due` says when. The word log says what was written, when, and to which
page. Nothing was missing except the division.

## Goals

- Say how many words are left and how many days there are to write them in.
- Say what the last fortnight actually came to, in the same unit.
- Project a finish date from the second, and say when there is not one.
- Show every figure with the values it was divided from.

## Not goals

- **A verdict.** No `on_track`, no "behind schedule", no colour that changes
  when a number crosses a line, no streak and no completion badge. Two rates come
  back in the same unit and the reader compares them. A tool that told somebody
  they were behind would be having an opinion about their week, which is what
  [Idea Inbox](idea-inbox.md) refuses when it declines to move a thread's
  lifecycle on its own, and what [Long-form writing](long-form.md) refuses when
  it draws a progress bar and nothing else.
- **New frontmatter.** Nothing was added and nothing needed to be.
- **A stored number.** A manuscript falls behind because days pass and nothing
  happens, so a value in a table would be an answer to a question nobody had
  asked yet. Computed on read, which also removes the background job that would
  otherwise have to keep it current. `idea-momentum/v1` made the same call for
  the same reason.
- **Notifications.** Nothing here is pushed, and nothing records that anybody was
  shown a figure. Same terms as the rediscovery card.
- **Per-section pacing.** The manifest already draws a bar per section that names
  a target of its own. A deadline belongs to the work, and a chapter with its own
  `due` is rare enough that guessing at it would be inventing a use.

## What it computes

Ten figures, and every one of them recomputable from the others in the response.
That is the gate, and it is the one `idea-momentum/v1` is held to: a reader can
check the arithmetic without reading the source.

| Figure | From |
|---|---|
| `words` | The compiled total, which is what a reader would actually get |
| `remaining` | `target - words`, signed |
| `days_remaining` | `due` and `at`, counting today |
| `required_per_day` | `remaining / days_remaining` |
| `window.net` | `added - removed` over the manuscript's pages in the window |
| `window.per_day` | `net / days` |
| `window.active_days` | Distinct local days something was written on |
| `projected_days` | `ceil(remaining / per_day)` |
| `projected_finish` | Today plus that, less the one that is today |
| `uncounted` | What was written on pages the document does not carry |

### A net is the right number here, and nowhere else

The word log deliberately never stores a difference. That is the whole reason
the feature exists, and `example-wiki/index.md` states it as plainly as it can
be stated: an assistant rewriting two thousand words into nineteen hundred is
not "minus one hundred", and knowing *who* produced the minus one hundred
recovers neither figure.

Pacing computes a net anyway. It is not a contradiction, and the line between
them is worth being precise about: **a target is a net quantity.** It says how
long the work should end up, not how much effort should go into it. Cutting two
hundred words moves you away from a target exactly as surely as writing two
hundred moves you toward it, so the number that closes the gap is the difference
and nothing else will do.

So the difference is taken at the point where it is the question, and both halves
come back beside it. `+606 (726 added, 120 removed)` is what the dashboard shows,
in that order, so the net never appears without the figures it came from.

### The rate has to be in the same currency as the remainder

`remaining` is measured against the **compiled** total: what a reader would get.
A page carrying `compile: false` is not in it, which [Drafting](drafting.md)
settled and which `example-wiki` demonstrates with a cut scene.

That decides which observations count. Only pages the document actually carries
are in the rate, because a day spent on a scene that is out of the book does not
move the compiled total, and counting it would project a finish date that never
arrives. Dividing a remainder measured one way by a rate measured another is not
a conservative estimate, it is a category error with a number on it.

**And then it says so.** The `uncounted` block reports what was written in the
same window on pages the document leaves behind, with the pages named. This is
the single most likely misreading of the two figures, and [Drafting](drafting.md)
recorded it as the most useful thing its fixture gained:

> "Excluded words do not count" is true of exactly one number, and a reader who
> generalised it would be wrong about the chart.

Reporting it beside the rate is what stops somebody concluding that a fortnight
of work vanished. It is defined by **subtraction** rather than by status: every
slug in the manifest that is not in the included set. That makes an appendix
listed under two parts count once and stay out of it without a special case, and
it correctly catches a chapter that was written and then deleted, which is a
`wanted` gap with words in the log.

### Two zones in one response, on purpose

The deadline is counted in **UTC days**. The window is cut in the **caller's
offset**. That looks like an inconsistency and is the opposite of one.

`due` names a day. A bare `2027-03-01` in a file reads as midnight UTC, and the
Manuscript panel already renders it back in UTC for the reason its own comment
gives: shown in the reader's zone it would say 28 February to anybody west of
Greenwich. Reading it in the caller's offset to count days would make "due 30
September" arrive a day early for half the planet.

Which local day an observation fell on is a different kind of question. It is
about a wall clock, which is precisely the argument `words::stats` already makes
for bucketing in Rust rather than in SQL, and the offset is the answer to it.

So the two really are asked in different zones, and both fields say which. What
they share is the day boundary itself: `end_of_local_day` moved into
`words::stats` so that the pace window and the chart's window cannot be cut in
two different places. Before this it was written once in `api::words` and about
to be written a second time here, which is exactly how two features come to
disagree about what "the last fortnight" means.

### Days remaining counts today

Due today is one day, not none. Zero is reserved for a deadline that has gone,
which is then the only thing zero ever means, and `required_per_day` is absent
rather than infinite.

### A rate of zero projects nothing

A fortnight spent cutting has a negative net, and a fortnight spent on other
pages has a net of zero. Neither projects a finish, and the field is absent
rather than carrying a date arrived at by dividing by nothing. Saying nothing is
more use than saying 1 January 1970.

A projection past a hundred years keeps its number and loses its date, because
past that the date is not a figure, it is what dividing by a rate near zero
produces. `projected_days` still says 438,000, so the reason is visible rather
than silent, and `chrono` is never asked to add a span it cannot represent.

### The window is a fortnight

Fourteen days, and `?days=` moves it, clamped to between one and four hundred
rather than refused: a window nobody thought about is a client rather than a
mistake, which is what the chart already does with one.

A fortnight is long enough that a day off does not halve the rate and short
enough to describe what somebody is doing now rather than what they did in the
spring. It is the same figure `idea-momentum/v1` calls recent, and that is a
reason rather than a coincidence: both are asking what is happening lately.

The rate divides by the **window**, not by the days somebody wrote. A projection
is against a calendar, so the rate that projects has to be over calendar days.
"What I do when I sit down" is a real and different question, and it gets
`active_days` beside the totals rather than a second rate that would silently be
the one people quoted.

## Why an endpoint rather than a flag on `/api/compile`

`GET /api/pace?root=` is its own route, and the reason is the gate rather than
the shape.

A compile is a document, served under the ordinary page rules: an anonymous
caller on a wiki published with `RHIZOLOG_ANONYMOUS_READ` can assemble a public
book. A pace is half read off the **word log**, which is working state and is
refused to a caller with no account even under that variable. Two different
answers to "who may ask this" is two endpoints. A query parameter that quietly
changes the audience of a response is right until somebody adds a caller, and
then it is a leak with a plausible commit message behind it.

`?root=` rather than `/api/pages/{slug}/pace` for the reason that already
produced `/api/move` and `/api/compile`: `matchit` requires a catch-all to be the
final segment, and a slug is a catch-all.

Two small things came with it. `Compiled` gained the root's `due` beside the
`target` it already carried, so the arithmetic does not read the page a second
time; `CompiledView` exposes it for the same reason, and it is the same kind of
thing: what the work is aiming at, and when. Neither is consulted by the
assembly.

## What the dashboard shows

A strip under the Manuscript panel's progress bar, because it is the same
question with time in it: the bar says how far along, the strip says how fast.

```
606 words of 2,000                                  due Sep 30, 2026
[============                                    ] 30%
1,394 to go · 56 days left · 24.9 a day to make it
Last 14 days · +606 (726 added, 120 removed) on 4 days · 43.3 a day
At that rate, Sep 7, 2026 (33 days)
```

The two rates end consecutive lines so they can be read against each other, and
neither is coloured. Nothing on the strip says which is bigger.

Three things about how it behaves are decisions rather than defaults:

- **It asks for nothing on a page with neither a target nor a day.** A null
  resource source is what makes that true: Solid skips the fetcher for one. The
  Manuscript panel appears on `contents`, `target` or `due`, so a page carrying
  only a contents list gets the spine and no second walk of the book.
- **It renders nothing at all when the request fails**, with no error notice. The
  one failure that is expected rather than exceptional is a 401, and on a
  published wiki the spine above is still the anonymous reader's to read.
  Anything else that could fail here fails the compile drawing that spine too,
  and that is where it is reported once. The read is guarded because reading an
  errored resource **rethrows**, which unguarded would take the panel down with
  it. That is the same guard `Async` carries.
- **A manuscript past its target says so** rather than clamping to zero and
  reading as finished. A target is a length somebody is aiming at, not a ceiling.

`formatDay` moved from `Manuscript.tsx` into `Pace.tsx` and is imported back, so
the one import between the two files points in one direction. A cycle would
probably have worked and is not worth finding out about.

## The cost

**It walks the whole book a second time.** The Manuscript panel already compiles
on every view of a page that has one, which `TODO.md` names as the first thing to
look at if a large manuscript is slow to open; a pace doubles that on a page that
also names a target or a day.

It was paid rather than avoided. The alternatives were worse: taking a compiled
total from the client means trusting a client's arithmetic, and hiding the strip
behind a click would make the one glanceable figure the feature has into
something you have to ask for. The endpoint is also the cheaper half of the two,
since the assembly is concatenation and the walk is where the cost is. It goes in
`TODO.md` beside the existing note rather than being discovered later.

## Open questions

- **Nothing paces a whole wiki.** "How much did I write this fortnight" is
  answered by `/api/word-stats`; "how much of it was the book" is answered here,
  one manuscript at a time. Somebody writing two books at once has to ask twice.
- **The strip cannot be asked about a past instant.** `?at=` is on the endpoint
  and the dashboard always sends now, so the fixture's pinned figures are
  reproducible over HTTP and not in a browser. The same is true of the hours heat
  map and the words chart, so this is a shape the dashboard has rather than a gap
  this feature left.
- **A rate over a fortnight says nothing about which fortnight.** Two weeks that
  hold one enormous day and two weeks of steady work produce the same figure, and
  `active_days` is the only thing separating them. A sparkline is the obvious
  answer and `/api/word-stats` already draws one, so the case for repeating it
  here is weak.
