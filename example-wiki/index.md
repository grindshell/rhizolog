---
tags:
  - meta
---

# Example wiki

Sixteen pages, a week of tracked time and a week of writing, arranged to show
what Rhizolog does with them. Seven of the pages are a short book, because a
manuscript is a thing a wiki full of notes cannot demonstrate. Run the server
against this directory and the dashboard reports two orphans and two wanted
pages — all four on purpose.

Nothing here is special. It is markdown in a directory; delete the whole thing
and point `RHIZOLOG_ROOT` at your own notes.

**Reading it changes nothing.** The word log under `.rhizolog/words/` already
holds a line for every page, and every one of those lines agrees with the page it
describes, so a scan finds nothing to record. That is the same property that
makes deleting `.rhizolog/index.db` free, demonstrated on a wiki you can look at.
Starting a timer here is the exception, and it does write a file: see the note at
the end of the time log below.

## Start here

- [[notes/rust/async]] — nested slugs, and a link to a page nobody has written
- [[notes/rhizome]] — where the name comes from
- [[notes/deleuze]] — and where *that* comes from
- [[book]] — a manuscript, with a gap, a repeat and a bad entry in its contents

## Two ways to read a slug

Open [[notes/rust/async]] and there are two ways to follow the `rust` in its
slug. Three of the pages here exist to show that they are not the same way.

The breadcrumb walks the tree. `rust` there means `notes/rust`, and asks for
what is at or under it: [[notes/rust]] itself, plus `async` and `pinning`. It
does not return [[notes/rustlings]], which only starts with the same characters.

The badge beside the tags walks nothing. `/rust` there means *any* directory
called `rust`, and this wiki has two — so it also returns
[[scratch/rust/from-a-talk]], which the breadcrumb cannot reach from here at
all.

Both are correct, and they are separate controls because they answer different
questions. The second one is what [[notes/rhizome]] argues for, arriving as a
filter rather than as a metaphor.

## The orphans

`scratch/inbox` exists too, but nothing links to it. That is what makes it an
orphan, and why the dashboard counts it: in a wiki that branches, the pages you
cannot reach are the ones you forget you wrote.

This page is the other orphan, which is worth knowing before you go looking for
the bug. An entry point has nothing above it to link to it, so the front page of
a wiki is almost always orphaned. The statistic is still doing its job — it just
cannot tell the difference between a page nobody reaches and a page nobody needs
to reach *from inside*.

`scratch/inbox` has an hour and a half tracked against it and is still an
orphan, which is also right: nobody has linked to it, you have just been working
on it.

None of the book's seven pages is an orphan, though only one of them is linked
from anywhere. A `contents:` entry counts as a reference, so a chapter has a
parent even where no wikilink points at it. Without that rule, writing a book
would fill this statistic with its own chapters and the number would stop being
worth reading.

## The manuscript

[[book]] is a short book in seven pages, and the only fiction here. It exists
because a contents page is the one thing a wiki of notes cannot show you. Order
lives in frontmatter so that reflowing a paragraph cannot reorder a book, and the
price of that is real: open `book.md` raw and you get a YAML list rather than a
clickable index. The Manuscript panel on `/pages/book` is what pays it back, and
it is the only place the spine is drawn.

`GET /api/compile?root=book` should return **606 words in 10 sections**, seven
assembled and three not:

| # | Section | Depth | Words | Status |
|---|---|---|---|---|
| 1 | `book/one` | 1 | 25 | included |
| 2 | `book/one/opening` | 2 | 103 | included |
| 3 | `book/one/the-ferry` | 2 | 188 | included |
| 4 | `book/appendix` | 2 | 81 | included |
| 5 | `book/two` | 1 | 29 | included |
| 6 | `book/two/the-crossing` | 2 | 0 | wanted |
| 7 | `book/two/the-return` | 2 | 137 | included |
| 8 | `book/appendix` | 2 | 0 | duplicate |
| 9 | `../one/the-ferry` | 2 | 0 | invalid |

The root's own 43 words are section zero, which the panel drops because it is the
page you are already reading.

The three that are not assembled are why the book is shaped the way it is:

- **A chapter nobody has written.** `book/two/the-crossing` holds its position
  rather than being skipped, so the manuscript says where the missing chapter was
  going to go. Write the page and it fills with nothing to reindex. It is also
  the second of the two wanted pages this wiki reports, beside the one
  [[notes/rust/async]] links to: naming a page and not writing it is the same
  statement whether it was said in a wikilink or in a contents list, and a
  chapter you have outlined is if anything the more deliberate of the two.
- **A page in two places.** `book/appendix` is listed under both parts, because a
  timetable belongs with the outward leg and the return equally. The second
  position reports `duplicate`, which is the manifest working rather than
  complaining. It is not a cycle, and the status is deliberately not named after
  one, because this shape is far commoner than a loop.
- **An entry that is not a slug.** `../one/the-ferry` is refused by `Slug` itself
  rather than resolved against anything, and it costs the page nothing: the entry
  is `invalid` in the manifest and `book/two` stays readable everywhere else. A
  typo in a list of chapters must not take the page holding the book together out
  of every listing. Every entry is a slug from the wiki root, so the spelling that
  works is `book/one/the-ferry`, which Part One already assembles anyway.

  It is the one thing here the manifest reports and nothing else does. It is not
  drawn in the graph and it is not a third wanted page, because a wanted page is
  somewhere the dashboard suggests you write, and no wiki should be invited to
  write `../one/the-ferry`.

`?assembled=1` on the same page renders the whole thing, with headings shifted by
depth: the book's `#`, each part's `##`, each chapter's `###`. Nothing is
inserted, so a part contributes only what it wrote, which is a heading and an
epigraph. `book/one/opening` is written with a setext heading, `Opening` over a
row of `=`, and the compiled document is where you can see what happens to it:
there is no marker to shift, so it comes out as `###` with its own line kept
verbatim.

The target is 2,000 words against 606, so the panel reads 30 per cent. That is a
figure and nothing else: no streak, nothing that congratulates you, and no change
of tone when it goes up. The due date is fixed like every other date here, and is
not a deadline anybody is keeping.

## The time log

`.rhizolog/times/` holds **18 entries in 5 groups, 25 h 05 m**, none of them
running. `/api/time-groups` should say exactly:

| Group | Entries | Total |
|---|---|---|
| Deep work | 6 | 13 h 00 m |
| Writing | 4 | 5 h 15 m |
| Reading | 3 | 4 h 00 m |
| Pairing | 1 | 1 h 30 m |
| Email | 4 | 1 h 20 m |

Four of them exist to show something that is easy to get wrong:

- **Two overlapping timers**, on Tuesday 4 August. `Pairing` runs inside
  `Deep work`, and the totals count that hour and a half twice — on purpose.
  The totals are of *recorded* time, not of wall clock.
- **A session past midnight**, Wednesday 22:30 to Thursday 00:45. It is split
  between the two days rather than filed under the one it started in, and it
  lights three cells of the heat map rather than one.
- **Time against a page nobody has written.** Two and a quarter hours are
  tracked against `notes/rust/streams`, the same wanted page
  [[notes/rust/async]] links to. It is badged with a `?`, and it will attach
  itself properly the moment somebody writes that page.
- **Notes worth searching.** Search the log for `poll loop` and two entries come
  back with the matching sentence excerpted. That search is `?q=` on the log
  itself, not `/api/search` — time entries are deliberately not in the page
  index.

### The dates are fixed, so the windows will not be

The entries run from 30 July to 6 August 2026 and they stay there. Whenever you
are reading this, Today and This week are almost certainly empty, and eventually
This year will be too. That is a property of a committed fixture, not a bug.

To see the numbers as they were meant to look, ask for that moment directly:

```
/api/time-stats?at=2026-08-06T18:00:00Z&offset=0
```

which should answer 4 h 20 m for the day, 20 h 35 m for the week, 22 h 05 m for
the month and the full 25 h 05 m for the year.

Times in the files are UTC, and the dashboard cuts its days and its heat map in
*your* offset — so the entries sit in the working day only if you read them from
UTC, and slide earlier or later otherwise.

**Do not start a timer while `RHIZOLOG_ROOT` points here.** It writes a new file
into this directory and the totals above stop being true. `git status
example-wiki` afterwards says whether it happened.

## The word log

`.rhizolog/words/` holds **26 lines across two months**, pinned to the same week
as the time entries. Six of them are the startup scan finding pages that were
already there; the rest are a week of writing, most of it the book, plus a rename
and a delete.

An observation records **words added and words removed**, never their difference.
That is the whole reason the feature exists: an assistant rewriting two thousand
words into nineteen hundred is not "minus one hundred", and knowing *who*
produced the minus one hundred does not recover either figure.

Ask for the moment the log was written for:

```
/api/word-stats?at=2026-08-06T18:00:00Z&offset=0
```

which should answer **1787 added, 234 removed** across 18 observations and 15
pages, split by the tool that made each write:

| Tool | Added | Removed |
|---|---|---|
| file | 910 | 26 |
| web | 551 | 48 |
| claude-code | 326 | 160 |

Five of the lines exist to show something that is easy to get wrong:

- **A rewrite that wrote nothing.** On 6 August `scratch/rust/from-a-talk` is
  `added 24, removed 24`. A net figure would call that day empty; it was a
  morning's work.
- **A rewrite that took more away than it put back.** `book/one/the-ferry` was
  212 words on 4 August and is 188 now, and the line between them says
  `added 96, removed 120` with `claude-code` against it. The net is minus
  twenty-four. This is the case the whole feature exists for: knowing *who*
  produced the minus twenty-four recovers neither the ninety-six written nor the
  hundred and twenty cut, and the chart draws both.
- **A page that arrived from somewhere else.** `notes/rust/async` was
  `notes/async` until 3 August, and the `moved` line names both slugs. History is
  never rewritten, so the series before the move is still there under the old
  slug, and following the chain is what keeps it one series.
- **A page that no longer exists.** `scratch/old-notes` was written on 2 August
  and deleted on the 6th. Its 48 words are still in the totals, because they were
  written and deleting the file does not unwrite them. The marker is what stops a
  page later written at that slug from continuing this one's series.
- **Bookkeeping is not writing.** The six `baseline` lines carry `added 0,
  removed 0`. Without them, pointing the server at an existing wiki would report
  the whole thing as written on a Tuesday; counting them would do the same.

Every line ends with the page's own word count after the observation, which is
what makes the log checkable rather than believable. `scratch/inbox` is 60 words
on 30 July and 85 on 5 August, and the line between them says `added 30,
removed 5`.

## The prose rules

`.rhizolog/prose.toml` holds five rules, one of each kind `prose/v1` has. They
are rules somebody wrote down: no model, no network, and nothing here is a second
opinion about your voice.

`GET /api/prose/rules` reports them with their defaults filled in and a digest
over the lot, so an assistant handed a finding can reproduce it without reading
the file. `GET /api/prose?slug=index` runs them over this page and should answer
**10 errors and 69 warnings**: an em dash for each of the first, and a word used
twice inside eight of another for each of the second.

The first rule is the one this project holds itself to, and it is written with
TOML's `\u2014` escape rather than the character, because a rules file is largely
a list of things somebody is trying not to write. It fires on the prose of this
wiki and not on the fenced examples in it, which is the difference between a page
that uses a character and a page that documents one.

The warning count is worth sitting with, because it is the rule working rather
than the rule misfiring. `echo` is a lexical count with no stemming and no
stop-word list, `ignore` is the only lever it has, and a long page of
documentation repeats its own vocabulary constantly. The shorter pages here are
quiet by comparison: [[notes/rhizome]] answers 4 warnings and [[notes/deleuze]]
none, though each of them still trips the em dash rule once. Widen `ignore`
before lowering `within`, or the repeats worth seeing go with the rest.

The last rule, `names`, is the only one nobody wrote and the only one carrying an
`allow` list. It looks for a single name spelled two ways, and it is the one with
real false positives, so it gets the escape hatch and the other four do not. On
this page it finds nothing, which is what the two filters behind it exist to make
possible: without them it would report `Then` as a misspelling of `Them`.

### The rule that only fires on the whole book

Ask about the assembled manuscript instead:

```
/api/prose?slug=book&compiled=true
```

which should answer **1 error and 11 warnings**, and two of those warnings are
the ones worth the trip. The ferryman is `Marren` three times in
[[book/one/the-ferry]] and `Maren` twice in [[book/two/the-return]], and `names`
reports every occurrence of the rarer spelling: `Maren (2) beside Marren (3)`.

**No single page reports it.** Each chapter is internally consistent, and a rule
about spelling needs both spellings in front of it at once. That is what
`compiled=true` is for, and it is not a convenience: two of the five rules are
cross-page questions by nature, since a word echoed over a section break is just
as invisible from inside one chapter. Offsets in that report index the assembled
document rather than any page's source, which the response says in its `offsets`
field, and each finding still names the `slug` it fell in.

The one error is an em dash in [[book/two/the-return]], which is the same rule
firing on the same character as above. A finding is your own rule on your own
text, so there is nothing to dismiss and nowhere to dismiss it to: either the
sentence changes or the rule does.
