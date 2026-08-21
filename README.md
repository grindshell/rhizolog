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

## Quick start

You need [Rust](https://rustup.rs/) (edition 2024) and, for the dashboard,
[pnpm](https://pnpm.io/).

Build the dashboard once, then run the server against the example wiki:

```powershell
cd frontend; pnpm install; pnpm build
cd ../backend; $env:RHIZOLOG_ROOT = "../example-wiki"; cargo run
```

On macOS or Linux the last line is `RHIZOLOG_ROOT=../example-wiki cargo run`.

Then open:

| | |
|---|---|
| http://127.0.0.1:3000 | the dashboard |
| http://127.0.0.1:3000/swagger-ui | the API, browsable |
| http://127.0.0.1:3000/api-docs/openapi.json | the OpenAPI document |

The first compile takes a while: SQLite is built from source, and Swagger UI is
unpacked at build time.

`example-wiki/` is nine pages arranged to show the features off: nested slugs,
wikilinks, a page that is linked but not written, two orphans, and the same
directory name in two places, which is what makes the two path filters differ.
It also carries a week of tracked time: eighteen entries, two overlapping
timers, a session that runs past midnight, and hours logged against the page
nobody has written. Read [its index](example-wiki/index.md) first; it explains
what the dashboard will say about it and why, including why Today is empty.

To use your own notes instead, point `RHIZOLOG_ROOT` at any directory of
markdown files. Nothing needs importing.

The dashboard is optional. Skip `pnpm build` and the API works exactly the same;
the server just says there is no frontend to serve.

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
than one capture, chosen from the day's date so refreshing does not deal another.
Say you are still interested or dismiss it for thirty days. There are no
notifications, no streaks, and nothing is written down about having shown it.

When a thread is ready, promote it. That is three steps on purpose:
`GET /api/ideas/{id}/draft` assembles the markdown from every capture it holds,
you create an ordinary page with the ordinary page API, and
`PUT /api/ideas/{id}/promotion` records what it became. The dashboard does all
three from one form. The captures stay exactly where they were.

## Configuration

All optional, all environment variables.

| Variable | Default | What it is |
|---|---|---|
| `RHIZOLOG_ROOT` | `./wiki` | The wiki directory. Created if missing. |
| `RHIZOLOG_DB` | `<root>/.rhizolog/index.db` | The derived index. Safe to delete. |
| none | `<root>/.rhizolog/times/` | The time log. **Not** derived; back it up. |
| none | `<root>/.rhizolog/ideas/` | Captures, threads and decisions. **Not** derived; back it up. |
| none | `<root>/.rhizolog/users/` | Accounts. **Not** derived, and secret; back it up, don't commit it. |
| none | `<root>/.rhizolog/server.json` | Where the running server is. Gone when it stops. |
| `RHIZOLOG_ADDR` | `127.0.0.1:3000`, or any free port | Where to listen. |
| `RHIZOLOG_ASSETS` | `../frontend/dist` | The built dashboard. Missing is fine. |
| `RHIZOLOG_LOG` | `rhizolog=info,tower_http=info` | `tracing` filter. |
| `RHIZOLOG_SECURE_COOKIES` | off | Mark the session cookie `Secure`. Set it behind a TLS proxy. |
| `RHIZOLOG_ANONYMOUS_READ` | off | Serve `public` pages to callers who have not signed in. |

Defaults are relative to the working directory, which is assumed to be
`backend/`.

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
and keep it out of git, which the repository's `.gitignore` already does.

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

### Finding a running server

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

## Development

This is one git repository. Scaffolding tools like to create nested ones. If a
generator leaves a `.git` inside `backend/` or `frontend/`, delete it, or the
root repository will treat that directory as opaque and stop tracking what is
inside it.

| Path | |
|---|---|
| `backend/` | The Rust library and the headless server (crate `rhizolog`) |
| `desktop/` | The Tauri app (crate and binary `rhizolog-desktop`) |
| `frontend/` | The dashboard: Vite, SolidJS, Tailwind, daisyUI |
| `site/` | The static product site at rhizolog.com: Astro, Tailwind |
| `example-wiki/` | A small wiki, and a week of time, to run against |
| `knowledge-base/` | Why the thing is built the way it is |

`backend/` and `desktop/` are one cargo workspace, so there is a single
`Cargo.lock` and a single `target/`, both at the root.

Backend, from `backend/`:

```
cargo run        # start the server
cargo test       # 688 tests
cargo fmt
cargo clippy
```

One optional feature: `--features embed-assets` compiles `frontend/dist` into
the executable, so it can be run anywhere without a `dist/` beside it. It needs
`pnpm build` to have happened first, and it adds six more tests:

```
cargo build --features embed-assets
cargo test --features embed-assets    # 694 tests
```

A directory that exists still wins, so this changes nothing when you are working
in a checkout.

Cargo unifies features across a workspace, and the desktop crate enables that
one, so `cargo test --workspace` builds with it too and needs `pnpm build`
first. `cargo test -p rhizolog` is the server as it actually ships.

Desktop app, from `desktop/`:

```
cargo run                 # a window onto a server it starts itself
cargo build --release     # target/release/rhizolog-desktop.exe
```

It turns `embed-assets` on, so `pnpm build` has to have run before it will
build at all. The icons in `desktop/icons/` are placeholders.

### The desktop app is the server, in a window

It starts a real Rhizolog on loopback in its own process and points a webview at
it, so the dashboard in the window is talking to the same HTTP API anything else
would, and `.rhizolog/server.json` says where, exactly as it does for the
headless server. An agent can work against the app while you have it open,
without knowing it is an app.

That is the whole reason it is built this way: nothing the app can do is
something a browser pointed at a remote Rhizolog cannot.

**It asks which wiki to open**, the first time and any time the one it
remembers has gone. There is no default, deliberately: the API creates
directories it is pointed at, so a guess would mean an empty wiki materialising
somewhere you would never look for it. **File → Open Wiki…** changes it, which
restarts the app.

The answer is remembered in `rhizolog.settings.json` **beside the executable**,
so a copied folder takes its wiki with it. If that directory cannot be written
to, it falls back to the usual per-user config directory.

`RHIZOLOG_ROOT` overrides all of that and is not remembered. It is how to point
the app at a scratch wiki for an afternoon. The environment always wins; a
remembered choice never overrides something you typed.

**File → Settings…** picks the port. Leave the box empty for the usual
behaviour: 3000 when it is free, any free port when it is not. A port you type
is a requirement rather than a preference, the same as `RHIZOLOG_ADDR`: if
something else has it, Rhizolog says so and offers to forget the setting rather
than start somewhere you were not expecting. Saving restarts the app on the new
port; it reopens the same wiki. `RHIZOLOG_ADDR` overrides it, and the window
says so instead of leaving a box that does nothing.

Only the port, deliberately. The app is for a wiki on this machine, so it binds
loopback and does not offer to change the host. A box that accepts `0.0.0.0`
would put a wiki on the network by typing, which is a decision for a shell and a
firewall rather than a settings field. Serving one to other people is what
`RHIZOLOG_ADDR` and an account are for.

The same window names the **log folder** and opens it. A window has no console
to print to, so that file is the app's only account of itself and the first
thing worth attaching to a bug report.

**One window per wiki.** Opening the app again on a wiki it is already serving
tells you where that window is and offers to open a different wiki instead.
Two of them on one wiki would mean two writers on one index and a published
address that is only true for one. Two windows on two *different* wikis is fine
and works.

Frontend, from `frontend/`:

```
pnpm dev         # dev server with HMR, proxying /api to the backend
pnpm build       # production build, which the backend serves
pnpm test        # 125 tests
pnpm typecheck
```

`pnpm dev` expects a backend already running on port 3000 and proxies `/api`,
`/api-docs`, and `/swagger-ui` to it.

Site, from `site/`:

```
pnpm dev         # dev server on :4321
pnpm build       # static output to site/dist
pnpm typecheck   # astro check
```

This one is independent of everything above: the backend does not serve it and
does not know it exists. Note that `pnpm dev` daemonises: the command returns
and the server keeps running, so `pnpm exec astro dev status` is how you find
out whether one is up, and `pnpm exec astro dev stop` ends it.

The frontend's API types are generated from the OpenAPI document rather than
written by hand, so a backend change that breaks a caller becomes a type error
instead of a runtime surprise. After changing the API:

```
cd backend; cargo run --example dump-openapi
cd ../frontend; pnpm gen:api
```

No server needs to be running: the example writes the spec straight from the
compiled routes.

That exists because downloading it is a trap on Windows, and the obvious way is
the one that does not work. `curl` in PowerShell 5.1 is an alias for
`Invoke-WebRequest`, which decodes a body as Latin-1 when its `Content-Type`
carries no charset, and `application/json` from here carries none. Every
non-ASCII character in the spec comes back mangled, each of its bytes re-encoded
as two. The file stays valid JSON, stays one line, and the diff still reads like
an ordinary regeneration, so nothing catches it. `>` and `Out-File` are no
better; they re-encode too, and add a BOM.

If you do fetch it over HTTP, download bytes and write them verbatim:

```powershell
$data = (New-Object System.Net.WebClient).DownloadData("http://127.0.0.1:3000/api-docs/openapi.json")
[System.IO.File]::WriteAllBytes("$PWD\frontend\openapi.json", $data)
```

Worth checking after a refresh either way: the file should have no BOM, and it
should contain no `Ã` and no `â€`, which are what mangled UTF-8 looks like once
it has been read back as Latin-1.

Windows PowerShell 5.1 has no `&&`; use `;` to chain. And do not round-trip a
source file through `Get-Content` and `Set-Content`: 5.1 reads as ANSI and
writes UTF-8 with a BOM, which mangles every non-ASCII character in the file and
adds a byte-order mark that has, in this project, already hidden a page's
frontmatter once.

## Why it is built this way

[`knowledge-base/`](knowledge-base/index.md) is the long answer, kept as a wiki
because that is the obvious thing to do here. Start with
[the architecture](knowledge-base/architecture.md) for the storage model, or
[the API design](knowledge-base/api-design.md) for what "friendly to agents"
was taken to mean concretely.

## Status

The MVP is complete: pages, search, tags, the link graph, meta-stats, live
pickup of outside edits, and a dashboard you can write in. Since then: pinned
pages, time tracking end to end (timers, manual entries, notes, groups, search
over the log, and the statistics section), the drawn graph, the desktop app
described above, accounts with per-page visibility, and Idea Inbox end to end
(capture, local candidates, lifecycle receipts, rediscovery and promotion into a
page).

What is still thin about serving one over a network is the operational half:
there is no TLS of its own, no rate limiting on sign-in, and no audit log.
Put it behind a reverse proxy.

The desktop app runs and is not yet a download. Real icons, a check for a
missing WebView2 runtime, and code signing are what stand between the two.
[`TODO.md`](TODO.md) has that list and the rest of what is known and not done,
each entry with the reason it is not done.

Not implemented, on purpose: page history and diffs, link rewriting on move,
file attachments, and transclusion.

## Licence

Copyright (C) 2026 Tim Yuen. Rhizolog is free software under the
[GNU Affero General Public License](LICENSE), version 3 or later.

The Affero clause is why that one rather than the ordinary GPL. Rhizolog is a
server, so the usual way to use somebody else's copy is over a network, which
the GPL says nothing about, because it is not distribution. Section 13 does: a
modified Rhizolog that other people are allowed to talk to over a network has to
offer them its source as well. Running your own copy, changing it, and never
letting anyone else near it triggers none of that.
