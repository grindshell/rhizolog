---
tags:
  - meta
---

# Example wiki

Nine pages, a week of tracked time and a week of writing, arranged to show what
Rhizolog does with them. Run the server against this directory and the dashboard
reports two orphans and one wanted page — all three on purpose.

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

`.rhizolog/words/` holds **18 lines across two months**, pinned to the same week
as the time entries. Six of them are the startup scan finding pages that were
already there; the rest are a week of writing, a rename and a delete.

An observation records **words added and words removed**, never their difference.
That is the whole reason the feature exists: an assistant rewriting two thousand
words into nineteen hundred is not "minus one hundred", and knowing *who*
produced the minus one hundred does not recover either figure.

Ask for the moment the log was written for:

```
/api/word-stats?at=2026-08-06T18:00:00Z&offset=0
```

which should answer **1061 added, 114 removed** across 10 observations and 8
pages, split by the tool that made each write:

| Tool | Added | Removed |
|---|---|---|
| file | 417 | 26 |
| web | 414 | 48 |
| claude-code | 230 | 40 |

Four of the lines exist to show something that is easy to get wrong:

- **A rewrite that wrote nothing.** On 6 August `scratch/rust/from-a-talk` is
  `added 24, removed 24`. A net figure would call that day empty; it was a
  morning's work.
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
**9 errors and 36 warnings**: an em dash for each of the first, and a word used
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
real false positives, so it gets the escape hatch and the other four do not. Here
it finds nothing, which is what the two filters behind it exist to make possible:
without them it would report `Then` as a misspelling of `Them`.
