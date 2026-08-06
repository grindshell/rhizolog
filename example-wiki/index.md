---
tags:
  - meta
---

# Example wiki

Nine pages and a week of tracked time, arranged to show what Rhizolog does with
them. Run the server against this directory and the dashboard reports two
orphans and one wanted page — all three on purpose.

Nothing here is special. It is markdown in a directory; delete the whole thing
and point `RHIZOLOG_ROOT` at your own notes.

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
