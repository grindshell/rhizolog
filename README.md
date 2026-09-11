# Rhizolog

A wiki over a directory of markdown files, with an HTTP API that is meant to be
used by you, and by whatever agents you point at it.

"Rhizome" plus "log". A rhizome is a root system with no trunk: any point
connects to any other and there is no privileged centre. That is the bet this
project makes about notes: that knowledge branches off chaotically, and that
filing it as though it were a tree loses the connections worth keeping.

What makes it different from the wikis you already know:

- **Markdown files on disk are the source of truth.** Not a database with an
  export button. Edit them in your editor, move them with `mv`, keep them in
  git. The search index is derived and can be deleted at any time; it rebuilds
  on startup, and a file watcher picks up outside edits while the server runs.
- **API first.** A full HTTP API with an OpenAPI document that is generated from
  the routes, so it cannot drift from them. Errors are uniform and carry stable
  machine-readable codes. The dashboard is the API's first client, not its
  privileged one.
- **Single user by default.** A wiki with no accounts is open: it binds to
  loopback, asks nobody to sign in, and refuses nothing. Creating an account is
  what turns authentication on, which is how you serve one over a network. From
  then on a page can be public, internal, restricted to named readers, or
  private. This is a developer tool for managing a knowledge base, not a public
  wiki engine: a handful of named accounts, not registration and moderation.
- **It tracks time, too.** Timers you can start and stop, entries you can type
  in after the fact, and a note on any of them. Attach an entry to the pages it
  was spent on and the dashboard will tell you where the hours went. Entries
  are files as well, so `git log` gives you a history of your time nobody had
  to build.
- **It keeps unfinished thoughts.** Idea Inbox takes a thought in one text field
  and one action, notices which ones keep coming back, and shows the arithmetic
  behind every word it says about them. When one is ready it becomes an ordinary
  page. No LLM, no embeddings, no network request, and nothing is ever connected
  without you saying so.
- **It assembles manuscripts.** A page can name the pages it is made of, in
  order, and compile into one document with a map back to every part. It counts
  words added and words removed rather than their difference, and by which tool,
  because a day an assistant rewrote two thousand words into nineteen hundred is
  not "minus one hundred". And it holds prose to rules you wrote down in one
  file, quoting the text every finding fired on.

## Where it is today

It works, and it is early. **There is no download yet**: you build it from
source, which is a handful of commands and one long compile, below. It is
developed and tested on Windows. The server should build anywhere Rust does, and
nobody has tried it on macOS or Linux yet; the desktop app is Windows only.

Some things are missing on purpose: page history and diffs (a folder of notes is
very likely a git repository already, and git does that better), rewriting links
when a page moves, file attachments, and transclusion. Others are missing
because nobody has built them yet, and those matter if you serve a wiki to other
people: there is no TLS of its own, no rate limiting on sign-in, and no audit
log. [`TODO.md`](TODO.md) is the whole list, each entry with the reason it is not
done.

## Install

You need three things:

- [Rust](https://rustup.rs/). On Windows, rustup offers to install the Visual
  Studio C++ build tools as well. Say yes: they are also what compiles the
  SQLite that Rhizolog carries inside it.
- [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/), to build the
  dashboard. Neither is needed to run Rhizolog once it is built.
- [git](https://git-scm.com/), to fetch the source.

Then:

```powershell
git clone https://github.com/grindshell/rhizolog
cd rhizolog/frontend
pnpm install
pnpm build
cd ..
cargo install --path backend --features embed-assets --locked
```

The same commands work in a macOS or Linux shell. The last one takes a few
minutes the first time.

What that leaves behind is one program, `rhizolog`, in Cargo's `bin` directory,
which rustup put on your `PATH`; open a new terminal if the command is not found.
The dashboard is compiled into it, so it runs from anywhere with nothing beside
it. Nothing else is installed: no service, no background process, nothing that
starts with your computer. `cargo uninstall rhizolog` removes it again and
leaves your notes alone.

To update, from the `rhizolog` directory:

```powershell
git pull
cd frontend; pnpm install; pnpm build; cd ..
cargo install --path backend --features embed-assets --locked
```

## Run it on your notes

Point it at a folder of markdown files and start it:

```powershell
$env:RHIZOLOG_ROOT = "C:\Users\you\notes"
rhizolog
```

On macOS or Linux, `RHIZOLOG_ROOT=~/notes rhizolog`.

It prints the address it is listening on, normally `http://127.0.0.1:3000`, and
the dashboard is there in any browser. If something else already has port 3000
it takes any free port instead, so read the line rather than assuming. `Ctrl+C`
stops it.

Nothing needs importing. Every `.md` file under the folder is a page, and its
path is its name: `notes/rust/async.md` is `notes/rust/async`. The folder can be
empty, or not exist yet, in which case it is created. **Always set
`RHIZOLOG_ROOT`**: without it the wiki is a folder called `wiki` in whichever
directory you happened to start from, which is rarely where you meant.

### Try the example wiki first

The checkout carries a small wiki arranged to show the features off. From the
`rhizolog` directory:

```powershell
$env:RHIZOLOG_ROOT = "example-wiki"
rhizolog
```

It is seventeen pages: nested slugs, wikilinks, a page that is linked but not
written, two orphans, and the same directory name in two places, which is what
makes the two path filters differ. Eight of them are a short book, so that a
contents page, an assembled document and a manifest with a gap, a repeat, a cut
scene and a bad entry in it are things you can click on rather than read about.
Every one of those eight says what it is for and what stage of drafting it is
at, which is what the Manuscript panel's cards and its stage summary are drawn
from. It also carries a week of tracked time (eighteen entries, two overlapping
timers, a session that runs past midnight, hours logged against the page nobody
has written), a week of writing in the word log, and a rules file with one of
each kind of prose rule in it. Read [its index](example-wiki/index.md) first; it
explains what the dashboard will say about it and why, including why Today is
empty.

Reading it changes nothing on disk: the word log already agrees with every page,
so the startup scan finds nothing to record. Start a timer or edit a page while
`RHIZOLOG_ROOT` points there and it does write, and the numbers that index states
stop being true. `git status example-wiki` shows what it wrote.

## The desktop app

On Windows, the same server can live in a window of its own, for when you would
rather not keep a terminal open. Build it from the same checkout, after
`pnpm build`:

```powershell
cargo build --release -p rhizolog-desktop
```

That makes `target\release\rhizolog-desktop.exe`, and it is portable: copy it
wherever you like, run it from there, and delete it to uninstall. It needs
Microsoft's WebView2 runtime. Windows 11 has it, and most Windows 10 machines
have it from Windows Update; if yours does not, the Evergreen runtime is a free
download from Microsoft. The app does not yet check for it and say so.

**It asks which wiki to open**, the first time and any time the one it remembers
has gone. There is no default, deliberately: the API creates directories it is
pointed at, so a guess would mean an empty wiki materialising somewhere you would
never look for it. **File → Open Wiki…** changes it, which restarts the app.

The answer is remembered in `rhizolog.settings.json` **beside the executable**,
so a copied folder takes its wiki with it. If that directory cannot be written
to, it falls back to the usual per-user config directory. `RHIZOLOG_ROOT`
overrides all of that and is not remembered; it is how to point the app at a
scratch wiki for an afternoon.

**File → Settings…** picks the port. Leave the box empty for the usual
behaviour: 3000 when it is free, any free port when it is not. A port you type
is a requirement rather than a preference: if something else has it, Rhizolog
says so and offers to forget the setting rather than start somewhere you were
not expecting. Only the port, deliberately. The app is for a wiki on this
machine, so it binds loopback and does not offer to change the host; putting a
wiki on the network is a decision for a shell and a firewall, not a settings
field.

The same window names the **log folder** and opens it. A window has no console
to print to, so that file is the app's only account of itself and the first
thing worth attaching to a bug report.

**One window per wiki.** Opening the app again on a wiki it is already serving
tells you where that window is and offers to open a different wiki instead. Two
windows on two *different* wikis is fine.

It is not a second program with its own powers. It starts a real Rhizolog on
loopback and points a webview at it, so the window talks to the same HTTP API
anything else would, and an agent can work against the app while you have it
open without knowing it is an app.

## What it does to your folder

**Your markdown files stay yours.** Rhizolog reads every `.md` file under the
folder and leaves everything else alone, and it changes a page only when you ask
it to, through the dashboard or the API. Saving a page rewrites its frontmatter
in a tidy standard form, so hand-written YAML comes back reformatted after its
first save through Rhizolog; the text under it is saved exactly as you wrote it.
Keeping the folder in git is the easiest way to see what any tool did to it, this
one included.

Everything else it keeps is in one directory, `.rhizolog/`, at the top of the
folder:

| | What it is | What to do with it |
|---|---|---|
| `index.db` | The search index and link graph, built from your pages | Nothing. Delete it whenever you like; it rebuilds on the next start |
| `words/` | The word log: what was written, when, and by which tool | Back it up. There is no other copy |
| `times/` | The time log | Back it up. There is no other copy |
| `ideas/` | Idea Inbox's captures, threads and decisions | Back it up. There is no other copy |
| `prose.toml` | Your prose rules, if you write any | It is yours; absent is the ordinary case |
| `users/` | Accounts, if you create any, each with a password hash | Back it up, and never commit it |
| `server.json` | Where the running server is listening | Nothing. It goes when the server stops |

The first start on a folder writes a line for every page to the word log: the
baseline later edits are counted from. So a folder that was already a git
repository shows `.rhizolog/` as new. If yours is one, commit `words/`,
`times/`, `ideas/` and `prose.toml` with your notes, and ignore the rest:

```gitignore
.rhizolog/index.db
.rhizolog/index.db-*
.rhizolog/server.json
.rhizolog/users/
```

Check that nothing else in your ignore rules catches the word log. Its files end
in `.log`, which a lot of global gitignores throw away; `!.rhizolog/words/*.log`
brings them back, and `git status --untracked-files=all` says whether it worked.

## Keeping it private

**A wiki with no accounts is open.** Nobody signs in and nothing is refused,
which is right for a wiki on your own machine, and is how Rhizolog starts. That
is safe because it listens on `127.0.0.1`, which only this computer can reach.

Two things change that, and both are deliberate:

- **Setting `RHIZOLOG_ADDR` to anything but loopback puts it on the network.**
  Without an account, anybody who can reach the port can then read and write
  every page, because the API writes files. The server warns about it at
  startup; believe the warning.
- **Creating an account turns authentication on.** From then on every request
  has to say who it is, and a page can be public, internal, restricted or
  private. That is how you serve a wiki to other people; see
  [Accounts](#accounts).

Nothing it does leaves your machine by itself: no telemetry, no update check, no
model, no request to anybody's server. Idea Inbox's suggestions are arithmetic
over your own captures, done where they are.

## Reporting a bug

On [GitHub](https://github.com/grindshell/rhizolog/issues). Worth including:

- the version, which `/api/health` and `.rhizolog/server.json` both report;
- whether it was the server or the desktop app, and on which operating system;
- what the server printed, or for the desktop app its log folder, which
  **File → Settings…** names and opens;
- what you did, what you expected, and what happened instead.

---

The rest of this file is the manual: how pages, links, time, ideas and
manuscripts work, and every setting. Until the site has documentation of its
own, this is it.

## Pages

A page is a markdown file with optional YAML frontmatter:

```markdown
---
title: Async in Rust
tags:
  - rust
  - async
---

# Async in Rust

Futures are lazy. See [[notes/rust/pinning]].
```

Everything in the frontmatter is optional, including the whole block. Without a
`title` the page takes one from its first heading, and failing that from its
slug. It keeps following that heading as you edit it.

A page's **slug** is its path under the wiki root without the `.md`:
`notes/rust/async.md` is `notes/rust/async`. Slugs are validated against the
stricter of the Windows and POSIX rules on every platform, so a wiki written on
one stays valid on the other.

Links come in two spellings and both are tracked:

- `[[notes/rust/pinning]]`: a wikilink, always absolute from the wiki root.
- `[pinning](pinning.md)`: an ordinary markdown link, resolved relative to the
  page it appears in. The `.md` is optional.

A link to a page that does not exist is not an error. It is a **wanted page**,
it shows up on the dashboard, and it starts working the moment somebody writes
it, with no reindex. Links inside code fences are not links, because they are pulled
out of the parsed document rather than scanned for.

## The graph

`/graph` draws the pages and the links between them, and `GET /api/graph`
returns the same thing as nodes and edges. Wanted pages are drawn too, as dashed
rings, because they are branches the wiki has reached for and leaving them out
would make it look tidier than it is.

Narrow it with `?root=` for one page's neighbourhood (a walk of `?depth=` hops,
following links in both directions), or with the same `?prefix=` and `?tag=`
filters the page listing takes. Every page links to its own neighbourhood from
its Graph button.

The layout is deterministic: node positions come from a hash of the slug rather
than a random seed, so the same wiki draws the same picture every time and a
shape that changed means the wiki changed.

## Time

A time entry has a name, a start, usually an end, and optionally a markdown
note and a list of pages it was spent on. Entries are grouped by name. There
is nothing to create or delete; a group exists because entries carry its name.

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

They live in `<root>/.rhizolog/times/<YYYY-MM>/`, beside the derived index but
**not** derived: that directory is the only copy, so ignore
`.rhizolog/index.db` in git rather than the whole directory. They are not
pages, so they will not appear in a listing or in search.

Start one from the top bar, from any page, or with
`POST /api/times {"name": "Deep work"}`. Several can run at once and they are
allowed to overlap, because attention is not exclusive and a tracker that
insisted otherwise would be asking you to lie to it.

The dashboard's time section splits day, week, month and year, ranks the
activities and the pages the hours went to, and draws a heat map of every hour
of the week. A session that ran past midnight is split across both days and
lights every hour it touched, rather than being filed under the hour it started
in.

Names and notes are searchable with `GET /api/times?q=`, or the box on the Time
screen. It is a filter rather than a mode: it narrows the log alongside the
group, page and date filters instead of replacing them, and it leaves the log in
order, so "what did I write about the poll loop last week" is one request.
Entries are deliberately absent from `/api/search`, which is about pages.

Time attached to a page shows on that page as one line with a total on it, not
as backlinks. That is deliberate: a page you actually work on collects an entry
every time you start a timer, and folding hundreds of them into the link graph
would bury the links.

## Idea Inbox

Most of what a knowledge base is made of arrives before you have decided
anything, and a page is a decision. The inbox is where the rest goes.

A capture is one piece of text and nothing else. No title, no slug, no tag, no
folder:

```powershell
Invoke-RestMethod -Uri http://127.0.0.1:3000/api/captures -Method Post -ContentType application/json -Body '{"text":"Dungeon quests should require finding particular seeds."}'
```

Captures live in `<root>/.rhizolog/ideas/`, with the threads you name and every
decision you take about them. Like the time log it is authored data with no other
copy, and unlike the accounts it holds no secrets, so commit it with the wiki if
the wiki is in git. Captures are not pages: they stay out of search, the link
graph, tags, orphans and wanted pages until you promote one.

Ask what a capture might belong with and you get up to three suggestions, each
with the words the two share and what each word was worth:

```
GET /api/captures/{id}/candidates
```

That is `tfidf/v1`: term frequency over your own captures and nobody else's,
computed locally, with no model and no network. It is advisory and it stays
advisory. **Nothing is ever connected for you**, however alike two thoughts look.

Connect a few and the thread gets a lifecycle state and a momentum score, worked
out when you read it rather than stored, so the same files answer differently
tomorrow. `GET /api/ideas/{id}/receipt` shows the whole sum: every component,
the window each was measured against, and every capture and decision that was
counted. An idea whose captures you deleted gets no state and no score at all,
and says so, because there is nothing left to derive one from.

Opening the inbox may offer **one** rediscovery card: a dormant thread with more
than one capture, chosen from the day's date, so refreshing hands back the same
card rather than dealing another. Say you are still interested or dismiss it for
thirty days, and rediscovery stays away until the page is loaded again. One gap
is left, and it is written down rather than papered over: answering changes which
ideas are eligible, so a reload after a dismissal does deal a different card.
There are no notifications, no streaks, and nothing is written down about having
shown it.

When a thread is ready, promote it. That is three steps on purpose:
`GET /api/ideas/{id}/draft` assembles the markdown from every capture it holds,
you create an ordinary page with the ordinary page API, and
`PUT /api/ideas/{id}/promotion` records what it became. The dashboard does all
three from one form. The captures stay exactly where they were.

## Long-form writing

A wiki page is something you have decided. A manuscript is something you hand
over, on a date, at a length, in a format. Three fields in a page's frontmatter
turn one into the other, and they do nothing at all on a wiki that never writes
them:

```yaml
---
title: The Long Way Round
target: 90000
due: 2027-03-01
contents:
  - book/one/opening
  - book/one/the-ferry
  - book/two
---
```

**A page contributes its body, then each page in its `contents:` list, in order,
recursively.** That is the whole rule. Holding a list is what makes a page a
contents page; there is no flag and no depth parameter, and a link written in
prose is never structure. Structure lives in frontmatter so that reflowing a
paragraph cannot reorder a book, and so that "insert a chapter after the ferry"
is an unambiguous edit an assistant can make without touching a word of prose.

`GET /api/compile?root=book` assembles it. Headings shift down by depth, so an H1
in a chapter becomes an H3 under a book with parts, and the whole document is
rendered after assembly rather than page by page. `?format=` takes `markdown`
(the default), `html` or `json`.

It comes back with a **manifest**: every section in order with its slug, depth,
word count and byte range in the output. A chapter nobody has written yet holds
its position and is reported as `wanted`, which is the difference between a gap
and an omission; a mistyped slug is `invalid` and costs its position rather than
the page it was written on. Compile is a context loader before it is an export:
handing an assistant chapter nine in the light of chapter two is the thing the
manifest makes possible, because a finding over the whole book maps back to the
page and offset that produced it.

Each entry also says which `contents:` list named it and where in that list, which
is what lets the dashboard's Manuscript panel move a chapter without anybody
opening a YAML list in a textarea. A move is a `PATCH` of that list, and a gap or
a typo in it survives one: those are things somebody wrote.

Every page has a `words` count, prose rather than bytes: code fences, inline
code, frontmatter and raw HTML blocks are all excluded, so a page that is mostly
a code sample is large and nearly wordless. `?sort=words` orders by it and the
listing carries a total over the whole filtered set.

### Where the words went

Every write is observed: **words added and words removed**, never their
difference. An assistant rewriting two thousand words into nineteen hundred is
not "minus one hundred", and knowing who produced the minus one hundred does not
recover either figure. `GET /api/word-stats` answers the series by day, by tool
and by page, and the dashboard draws it beside the hours: added above the line,
removed below it.

Each write also records an **actor**: `web` for the dashboard, `file` for an edit
your own editor made, `scan` for the startup reconciliation, and whatever a
caller sends in `X-Rhizolog-Actor` for everything else. That is a claim rather
than a proof, which is fine, because the question it answers is bookkeeping about
your own tools rather than security. On a wiki with accounts the account comes
from the session and no header can touch it.

The log is files, at `<root>/.rhizolog/words/<YYYY-MM>.log`, one line a write.
Deleting `index.db` and restarting reproduces the whole series and adds nothing
to the log, which is the property that made it a file rather than a table.

### Pacing

`GET /api/pace?root=book` divides one by the other. A `target` says how long the
work should end up, a `due` says when, the compiled total says where it is now,
and the word log says what the last fortnight actually came to:

```
1,394 to go · 56 days left · 24.9 a day to make it
Last 14 days · +606 (726 added, 120 removed) on 4 days · 43.3 a day
At that rate, Sep 7, 2026 (33 days)
```

Two rates, the same unit, side by side, and **nothing that has an opinion about
which is bigger.** No streak, no badge, no colour that changes when a number
crosses a line. Every figure comes back with the values it was divided from, so
the arithmetic can be checked from the response rather than believed.

This is the one place a **net** is the right number, and the reason is worth
stating: a target is a length rather than an amount of effort, so cutting two
hundred words moves you away from it exactly as surely as writing two hundred
moves you toward it. Both halves come back beside it, in that order, and the log
itself still stores no difference anywhere.

It also reports what was written in the same fortnight on pages the document does
**not** carry: a cut scene, a chapter under an excluded part, a page deleted
since. Those words are real and they are in the chart, and they are in no rate
here, because the rate has to be in the same currency as the remainder or
dividing one by the other means nothing.

Half of it is read off the word log, so like `/api/word-stats` it is refused to a
caller who has not signed in, even under `RHIZOLOG_ANONYMOUS_READ`. The dashboard
draws it as a strip under the manuscript's progress bar.

### Cutting a chapter in two, and putting one back

`POST /api/split` takes a page, a byte offset and a slug for the second half. It
writes both files and puts the new one into every `contents:` list that named the
first, immediately after it. `POST /api/merge` is the inverse: one page's body
goes to the end of another, its file is removed, and every list that named it
loses the entry. Both answer with which lists were rewritten and what each one
says now.

Doing it by hand is four things that have to agree, and getting the last one
wrong is a book with a chapter missing and nothing to tell you.

**Neither writes or unwrites a word**, and the log says so: a split records two
markers and a merge records one, all of them zero added and zero removed, so
moving a boundary never shows up as a day's work on the chart. That is the same
reading a signed net would have given, which is the thing the word log exists to
refuse.

The second half inherits the tags, the stage, the due date and who can read the
page. It inherits no synopsis and no target: a synopsis is a claim about what a
chapter does and the half cut off one is not that chapter, and copying a target
would double what the book is aiming at.

**Neither will touch a page that assembles others.** A merge moves text to where
you said; a split has to work out where the second half goes, and on a page with
chapters under it that is after every one of them, which is a document quietly
restructuring itself.

The editor grows a Split and merge block under the findings strip, on any page
that exists. It cuts at the cursor and shows the line the new page would start
with, and both controls wait until there is nothing unsaved, because they act on
the file rather than on what is in the box.

### Prose rules you wrote down

`.rhizolog/prose.toml` holds rules; `prose/v1` runs them. It is voice defence
rather than a grammar checker, because drafting alone with an assistant the
failure is drift and you cannot see it happening: you read the prose as it
arrives.

```toml
[[rule]]
id       = "no-em-dash"
kind     = "forbid"
severity = "error"
literals = ["\u2014"]
```

Five kinds: `forbid` for a literal, `phrase` for a sequence of words, `echo` for
a word used twice close together, `uniformity` for a run of sentences that are
all the same length, and `consistent` for one name spelled two ways. Code blocks
and inline code are excluded from all of them, always, so a page documenting a
syntax is never flagged for containing it.

**Every finding quotes the text it fired on and carries the arithmetic behind
it**, and there is no dismissal store on purpose: a finding is your own rule
firing on your own text, so if it fires where it should not, the rule is wrong
and the fix is one edit to one file. `GET /api/prose/rules` returns the rules
with their defaults filled in and a digest that findings quote, so an agent
handed a finding can reproduce it without ever reading the file. Nothing here
calls a model or touches the network.

The editor grows a findings strip under the textarea, and clicking a finding
selects it in the text.

## Configuration

All optional, all environment variables.

| Variable | Default | What it is |
|---|---|---|
| `RHIZOLOG_ROOT` | `./wiki` | The wiki directory. Created if missing. |
| `RHIZOLOG_DB` | `<root>/.rhizolog/index.db` | The derived index. Safe to delete. |
| `RHIZOLOG_ADDR` | `127.0.0.1:3000`, or any free port | Where to listen. |
| `RHIZOLOG_ASSETS` | `../frontend/dist` | A built dashboard to serve instead of the one compiled in. That default only exists in a checkout; without it, the installed program serves its own copy. |
| `RHIZOLOG_LOG` | `rhizolog=info,tower_http=info` | `tracing` filter. |
| `RHIZOLOG_SECURE_COOKIES` | off | Mark the session cookie `Secure`. Set it behind a TLS proxy. |
| `RHIZOLOG_ANONYMOUS_READ` | off | Serve `public` pages to callers who have not signed in. |

A relative path is relative to the directory the server was started from. What
lives under `.rhizolog/` is in [What it does to your folder](#what-it-does-to-your-folder).

Think before changing `RHIZOLOG_ADDR`. **A wiki with no accounts is open**, and
the API writes files, so binding anything but loopback means anybody who can
reach the port can read and write every page. The server says so at startup, in
a warning it is worth not ignoring.

## Accounts

A fresh wiki has none, and behaves exactly as it always did: no login page,
nothing refused, every request treated as the one user. That is the intended
state for a wiki on your own machine, and for the desktop app.

**Creating the first account is what turns authentication on.** Go to
`/accounts` in the dashboard and fill in the form, or:

```powershell
$body = '{"username":"tim","password":"correct horse battery staple"}'
Invoke-RestMethod -Uri http://127.0.0.1:3000/api/users -Method Post -ContentType application/json -Body $body
```

Anybody may create that first account, because until it exists the wiki is
already fully readable and writable by anybody who can reach it. Every account
after it needs an owner, and the first one is always an owner.

From then on every `/api` request has to say who it is. Signing in gives you
both a cookie, which the dashboard uses, and a token for everything else:

```powershell
$login = Invoke-RestMethod -Uri http://127.0.0.1:3000/api/auth/login -Method Post -ContentType application/json -Body $body
Invoke-RestMethod -Uri http://127.0.0.1:3000/api/pages -Headers @{ Authorization = "Bearer $($login.token)" }
```

Accounts are files under `.rhizolog/users/`, one per account, holding an Argon2
hash of the password. Back that directory up, because there is no other copy,
and keep it out of git.

Serving over a network in earnest wants TLS in front and
`RHIZOLOG_SECURE_COOKIES=1` with it; without that, passwords cross the network
in the clear. There is no rate limiting on sign-in yet; see [`TODO.md`](TODO.md).

## Who can read which page

Once a wiki has accounts, a page's frontmatter decides who it is for:

```markdown
---
title: Project Roadrunner
visibility: restricted
owner: tim
readers: [alice, bob]
---
```

| | Who |
|---|---|
| `public` | Anyone, including callers who have not signed in |
| `internal` | Any account on this wiki, and **what a page with no `visibility:` means** |
| `restricted` | The `readers` list, plus the owner |
| `private` | The owner alone |

A page you own is readable by you whatever it says, so marking one private does
not hide it from yourself. A word that is not one of the four reads as `private`:
somebody who typed `privte` was trying to restrict a page, and the safe way to
get that wrong is to hide too much.

A page you may not read answers `404`, the same as one that is not there, and
not just when you ask for it directly. It is absent from the listing, from search,
from the tag counts, from the graph, from every total, and from the titles a pin
or a time entry resolves.

**`public` needs the instance to agree.** Marking a page `public` does nothing
until it is served with `RHIZOLOG_ANONYMOUS_READ=1`, which lets callers who have
not signed in read the `public` pages and nothing else. Publishing to the open
internet therefore takes two deliberate acts, in two places: a line in a file and
a variable in a deployment. Anonymous callers can never write, and never reach
the pins or the time log.

The editor offers all of this as a dropdown, and the page view marks anything
that is not `internal`.

## Finding a running server

By default the server takes port 3000 if it can and **any free port if it
cannot**, so a second copy still starts, and so does one on a machine where
something else got there first. Setting `RHIZOLOG_ADDR` turns that off: an address you asked
for by name is used or the server refuses to start, because you have probably
written that port down somewhere else too.

Which means the port is not always knowable in advance, so a running server
writes it down:

```json
{
  "url": "http://127.0.0.1:3000",
  "wiki_root": "C:\\Users\\tim\\wiki",
  "pid": 24601,
  "version": "0.1.0",
  "started": "2026-08-06T14:25:30Z"
}
```

It appears at `<root>/.rhizolog/server.json` only once the server is **ready**:
listening, with its index reconciled. So finding one means you can use it
immediately. A clean shutdown removes it.

For a script or an agent, the order to try is `RHIZOLOG_ADDR`, then
`server.json` beside the wiki, then `http://127.0.0.1:3000`. Treat the file as a
hint rather than proof: a server killed hard leaves it behind, and process ids
get reused, so confirm with `GET /api/health` and check the `wiki_root` it
reports is the wiki you meant. That is one request and it cannot be fooled by a
stale file.

The API itself is browsable at `/swagger-ui` on the same address, and the
OpenAPI document it is drawn from is at `/api-docs/openapi.json`.

## Why it is built this way

[`knowledge-base/`](knowledge-base/index.md) is the long answer, kept as a wiki
because that is the obvious thing to do here. Start with
[the architecture](knowledge-base/architecture.md) for the storage model, or
[the API design](knowledge-base/api-design.md) for what "friendly to agents"
was taken to mean concretely.

## Working on it

Running it from a checkout, the tests, the layout of the repository and the
house rules are in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Licence

Copyright (C) 2026 Tim Yuen. Rhizolog is free software under the
[GNU Affero General Public License](LICENSE), version 3 or later.

The Affero clause is why that one rather than the ordinary GPL. Rhizolog is a
server, so the usual way to use somebody else's copy is over a network, which
the GPL says nothing about, because it is not distribution. Section 13 does: a
modified Rhizolog that other people are allowed to talk to over a network has to
offer them its source as well. Running your own copy, changing it, and never
letting anyone else near it triggers none of that.

### An additional permission for the WebView2 loader

The desktop app is linked with Microsoft's WebView2 loader, which comes as a
compiled library with no source. So that anybody may pass on a desktop app built
from Rhizolog, changed or not, this is granted under section 7 of the licence:

> **Additional permission under GNU AGPL version 3 section 7**
>
> If you modify Rhizolog, or any covered work, by linking or combining it with
> the Microsoft Edge WebView2 loader (`WebView2Loader.dll` or
> `WebView2LoaderStatic.lib`, as distributed in Microsoft's WebView2 SDK, or a
> modified version of either), containing parts covered by the terms of
> Microsoft's licence for that SDK, the licensors of Rhizolog grant you
> additional permission to convey the resulting work. Corresponding Source for
> a non-source form of such a combination need not include the source code of
> the loader.

It names the loader and nothing else; the rest of Rhizolog is under the AGPL as
written. Section 7 lets anybody passing on a copy remove the permission, so a
version without the desktop app can drop it.

Everything else Rhizolog is built from is under licences that combine with the
AGPL as they stand; see
[Dependency licences](knowledge-base/dependency-licences.md).
