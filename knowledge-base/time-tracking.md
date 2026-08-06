# Time tracking

A wiki that tracks how knowledge branches should be able to track where the
hours went while it branched. Rhizolog records **time entries**: a name, a
start, usually an end, optionally a note, and optionally the pages the time was
spent on.

See [Architecture](architecture.md) for the storage model this builds on and
[API design](api-design.md) for the rest of the endpoint surface.

## An entry is a file, like everything else

```
<wiki root>/
  notes/rust/async.md
  .rhizolog/
    index.db                                  # derived; safe to delete
    times/
      2026-08/
        20260806T142530-123456789.md          # NOT derived; the only copy
```

```markdown
---
name: Deep work
start: 2026-08-06T14:25:30Z
end: 2026-08-06T15:40:00Z
pages:
  - notes/rust/async
---

Chased down a lifetime error in the poll loop.
```

The alternative was rows in the durable half of the index, beside
[pins](pins.md). It was rejected, and the reason is the difference between a
pin and a time log. A pin is a shortcut: fifty of them, all rebuildable by hand
in a minute. A year of tracked time is **primary data** — the thing you would
be most upset to lose — and putting it somewhere the architecture actively
encourages you to delete would be indefensible.

Files also buy the same three things they buy for pages, and the reasons have
not changed since [Architecture](architecture.md) argued them: you can `grep`
last March, you can fix a typo in your editor, and `git log` gives you a
history nobody had to build. An agent can append an entry by writing a file.

The cost is a lot of small files. That is what the `YYYY-MM` directory is for,
and it is derived **from the id, not from the entry's `start`** — so a path is a
pure function of an id, and editing an entry's start time never moves its file.

### Why it lives under `.rhizolog/`

A visible `times/` at the wiki root would read better, and it would mean
teaching three separate things about a directory name: the page walker would
have to skip it, `Slug::parse` would have to refuse anything inside it, and the
file watcher would have to classify it. It would also steal a perfectly good
top-level name from anyone who wants pages there.

`.rhizolog/` already has all three properties, for free — the walker skips
dot-directories, slug validation rejects dot-segments, and the watcher ignores
them. `store::INTERNAL_DIR` was documented from the start as "the derived
index, and anything else Rhizolog needs to keep inside the wiki without
treating it as content", which is exactly this.

One consequence has to be said out loud, because the directory's name suggests
the opposite: **`.rhizolog/` is no longer all disposable.** `index.db` is;
`times/` is not. The repository's `.gitignore` ignores the database by name
rather than the directory, and a wiki kept in git should do the same.

## Ids are rigid because they become paths

`20260806T142530-123456789` — the UTC start, compacted, plus nanoseconds.

Two properties are load-bearing. It **sorts chronologically as text**, so the
directory listing is the log in order and so is any `order by id`. And it is
**exactly twenty-five characters of digits, one `T` and one `-`**, which is
what makes it safe.

A [`Slug`](architecture.md) has to accommodate whatever a person wants to call
a page, so its validation is a long list of hazards to exclude — `..`, drive
prefixes, device names, trailing dots. An id is generated and never typed, so
it can be validated by exact shape instead, and a shape that admits only digits
and two separators cannot express any of those hazards. There is nothing to
enumerate and nothing to miss.

`TimeId::mint` takes a `nudge` because a manually entered start is whole
seconds: two entries logged for `09:00:00` would mint the same id and the
second would overwrite the first. The store walks the nanosecond half upward
until the file is free.

**An id survives an edit.** Moving an entry's start time does not remint it and
does not move its file. An id names the entry; it is not a claim about its
contents, and a `PATCH` that quietly handed back a different one would break
every reference a caller was holding.

## Grouping is the name, exactly as written

There is no group object to create, rename, or delete. A group exists because
entries carry its name, and it is gone when the last of them is. `Deep work`
and `deep work` are two groups.

That last part is deliberate and it is the same answer [tags get](api-design.md):
normalising would mean picking a canonical spelling, and picking one means
being wrong about `TODO` versus `todo` for somebody. A single-user tool can
afford to let the user be consistent on their own.

## Several timers, overlapping, on purpose

An entry with no `end` is running. Nothing limits how many run at once and
nothing rejects an overlap.

This is not laziness about a constraint. Attention is not exclusive: pairing
while a build runs while a meeting happens in the background is three true
things about one hour, and a tracker that insists on one would be asking you to
lie to it. The totals add up to more than the wall clock, and that is the
correct answer to the question being asked.

`POST /api/times/{id}/stop` returns `409` (`time_not_running`) rather than a
silent success. A stop that did nothing usually means a second tab got there
first, and a caller that could not tell would display the wrong duration.

## A time link is not a link

An entry names the pages it was spent on, and those names are edges into the
wiki. They live in `time_pages`, well away from `links`, and this is the single
most important decision here.

The reason is arithmetic. A page you actually work on collects one of these
every time you start a timer, so hundreds is normal and thousands is not
absurd. Put them in `links` and every one of them becomes a backlink: the
page's backlink panel turns into a scrolling list of `Deep work`, its referrer
count makes it the most-linked page in the wiki by an order of magnitude, and
`most_linked` in `/api/stats` stops meaning anything. Nothing would be
*wrong*, exactly — it would be swamped, and the signal the link graph exists to
carry, which pages the *writing* points at, would be gone.

So it is a different kind of edge, reported separately, and rendered the way a
hundred of anything should be: one line with a total on it, in the page's
`times` block, with `GET /api/times?page={slug}` for the rest. Tracked time
also does not stop a page being an orphan, which is right — nobody has linked
to it, you have just been working on it.

What time links **share** with links is resolution. `time_pages.target` is a
slug as written, joined against `pages` at read time and never resolved once
and cached, so time can be tracked against a page before it is written and
attaches itself the moment somebody writes it. That is the same property the
[link graph](architecture.md) has and it is worth the consistency.

### Only the frontmatter counts

A wikilink written inside an entry's *note* renders as a link and is not
indexed as a time link. Attaching a page to an entry is a structured act,
because these edges drive the numbers: if merely mentioning a page in a note
added its hours to that page's total, every total would be an accident of
prose.

## Statistics are computed in Rust, not in SQL

`GET /api/time-stats` answers day, week, month and year in one call, plus a
heat map of the hours. The obvious implementation is `group by strftime(...)`.
Two things rule it out.

**An entry can span a bucket boundary.** A session from 23:00 to 01:30 is not
two and a half hours on Tuesday; it is one hour on Tuesday and ninety minutes
on Wednesday, and on an hour-of-day heat map it should light three cells, not
one. Grouping by a formatted start attributes the whole entry to the bucket it
began in — which is what most trackers settle for, and it makes the heat map
lie about exactly the sessions worth looking at. Everything is split across
boundaries instead.

**The windows are local.** Entries are stored in UTC because an instant is an
instant, but "how much did I work today" and "when am I usually working" are
questions about a wall clock. The offset comes in as a parameter — `offset`, in
minutes east of UTC, which is `-new Date().getTimezoneOffset()` — and every
boundary is cut in it.

Both are ordinary loops over a few thousand rows, and both would be
hard-to-review date arithmetic in SQL. `times::stats::build` is a pure function
of `(samples, now, offset)`, which is the only reason its boundary cases are
testable at all.

### The offset is not a timezone, and that is a real limitation

A window straddling a daylight-saving change is bucketed throughout with
today's offset, so one day in the past can come out an hour short or long. The
fix is an IANA zone, which means another dependency and a zone database to keep
current, for a single-user tool where the affected numbers are two Sundays a
year. A deliberate trade, not an oversight.

### The covering window is not simply the year

`stats::covering_window` is the union of all four period windows, and it exists
because the week containing New Year's Day starts in December. Loading "the
year" and computing the week from it would quietly leave half of "this week"
out of the numbers every January.

Weeks start on Monday. A dashboard that starts its week on Sunday and a
calendar that does not would disagree about what "this week" means every
Sunday.

## Endpoint shapes

| Method | Path | Notes |
|---|---|---|
| `GET` | `/api/times` | Newest first; `name`, `page`, `running`, `from`, `to`, `sort`, `order` |
| `POST` | `/api/times` | Starts a timer, or logs a finished entry |
| `GET` | `/api/times/{id}` | The entry and its note; `?render=true` for HTML |
| `PATCH` | `/api/times/{id}` | `end: null` clears the end and sets it running |
| `DELETE` | `/api/times/{id}` | |
| `POST` | `/api/times/{id}/stop` | `409` (`time_not_running`) if it had stopped |
| `GET` | `/api/time-groups` | Groups with totals, most time first |
| `GET` | `/api/time-stats` | Day, week, month, year, and the heat map |

**One endpoint creates both kinds of entry.** `POST /api/times` with just a
`name` starts a timer now; the same call with a `start` and an `end` records
time that is already over. There is no `/api/times/start`, because there is no
separate thing: a running entry is one whose `end` has not been written yet,
and an absent field says that more honestly than a mode flag would.

`stop` is the exception and it earns its place by being the one operation whose
entire content is "now". Expressing it as a `PATCH` would mean every client
reading its own clock and sending a timestamp, and a client whose clock is
wrong writing it down.

**These routes are not wildcards.** The page routes capture `{*slug}` because a
slug contains `/`; an id cannot, which is the only reason
`/api/times/{id}/stop` can exist at all — `matchit` requires a catch-all to be
the final segment, and that restriction is why moving a page had to become
[`/api/move`](api-design.md).

`GET /api/times` is the one listing in Rhizolog that defaults to **descending**.
A log is read from the end.

A listing carries `has_note` rather than the notes themselves, for the same
reason a page listing carries no bodies: a year of notes on the wire every time
someone opens the screen would make the cheapest call the most expensive one.

## In the dashboard

Three surfaces, described in [The dashboard](dashboard.md):

- **The top bar** shows the longest-running timer's clock, not just a count. A
  count tells you something is running; a clock tells you whether it should be.
  It sits before the pins menu because a pin left in place is harmless and a
  timer left running overnight is not.
- **`/times`** is the log: start a timer, log an hour you forgot, read back what
  you did. Filters live in the URL, so "everything I did on the async notes" is
  a link.
- **A page** gets a Track time button and, when there is any, a summary of the
  time spent on it.

The displayed duration of a running timer is recomputed locally from its
`start` rather than re-fetched every second — the same arithmetic the server
does, so the only value the two can disagree about is one caused by a clock
that is genuinely wrong. The *set* of running timers is polled, because a
second tab, an agent, or a hand-edited file can all change it.

## Deliberately out of scope

- **Full-text search over notes.** They are not in `pages_fts`, so
  `GET /api/search` does not find them. Notes are usually a sentence, and a
  second FTS table earns its place only once they are not.
- **Rounding, rates, billing, invoices.** This is a developer's tool for
  knowing where the hours went, not a timesheet.
- **Idle detection and reminders.** They need something watching the desktop,
  which a wiki backend is not.
- **A browsable URL per entry.** An id is a machine's handle; unlike a slug it
  is not something anyone would link to, so entries are read and edited in the
  log itself.
