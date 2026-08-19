# Accounts

Rhizolog was designed single-user, and [Product vision](product-vision.md) said
so in as many words: *no accounts, permissions, or multi-tenancy*.
[`config.rs`](../backend/src/config.rs) said something stronger — loopback was
the **whole** security boundary, and the reason an API that writes files
anywhere under the wiki root could be left unauthenticated.

Accounts exist so a Rhizolog can be served over a network. This page records
what that costs, what it deliberately does not change, and where the boundaries
actually are.

See [API design](api-design.md) for the endpoint surface and
[Architecture](architecture.md) for the storage model underneath.

## Nothing changes until an account exists

**A wiki with no accounts is open.** No login page, nothing refused, every
request treated as the single user the product was built around. That is not a
mode or a setting — it is the absence of one, and it is byte-for-byte the
behaviour Rhizolog had before this page was written.

Creating the first account turns authentication on for that wiki. From then on
every `/api` request has to say who it is.

The alternative was a `RHIZOLOG_AUTH` setting, and it is worse for a specific
reason: it can disagree with reality. A wiki with five accounts and the setting
off is an instance whose operator believes it is protected and it is not, and
nothing in the system is in a position to notice. Deriving the answer from the
account directory means there is nothing to disagree with — the question "does
this instance require a sign-in" has exactly one place to look, and it is the
same place the accounts are.

It also means the desktop app, the README's quick start, and every existing test
keep working untouched, and that **serving on a network is a deliberate act**
rather than something that happens by default.

The count is read from the directory on every request rather than cached.
That is a `read_dir` over a directory with a handful of entries, and it buys two
things: an account added by hand works immediately, and no stale copy of the
answer can leave an instance open that should not be.

### `server::start` says which state it is in, and warns about the bad one

The answer is a property of the wiki directory, not of any flag, so there is
nothing to read back. Every start logs one line saying whether this wiki
requires authentication.

The warning is the point. Binding off loopback puts the API on a network, and
with no accounts that API reads and writes files for anybody who can reach the
port. It stays a warning rather than a refusal — a deliberately open instance
behind a firewall is a legitimate thing to run — but nobody should arrive at one
by accident, and the two ways to get there (setting an address, and never
creating an account) are far enough apart in time that startup is the only place
they meet.

## An account is a file

`<wiki>/.rhizolog/users/<username>.md`, in the shape this codebase already has
twice:

```markdown
---
display_name: Tim Yuen
role: owner
password: $argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$...
created: 2026-08-19T10:00:00Z
---

Anything worth saying about this account.
```

Files rather than rows, for the reason [pages and time entries](
architecture.md) are files: it is authored data with no other copy, and a
developer should be able to read the list in an editor without a running server.
The username is the filename and appears nowhere inside it, exactly as a page's
slug is its path — one name in one place, so a rename cannot leave the two
disagreeing.

The alternative was a durable table in `index.db`, alongside `pins` and
`api_usage`. It was rejected because "deleting the index is always safe, it
rebuilds from the wiki" is a promise made in three places — this knowledge base,
`AGENTS.md`, and the gitignore — and accounts in it would quietly make that
false in the worst way: you would find out when nobody could sign in.

### There is no index over accounts, deliberately

Pages and time entries are mirrored into SQLite because they are searched,
filtered, sorted and counted in ways a directory scan cannot answer. Accounts
are none of those. There are a handful, they are listed whole, and the only hot
question is how many there are.

Keeping them out buys the property that matters more: **an account written by
hand works immediately**, with nothing to reindex and no server to restart. It
also means no derived copy of a credential can drift out of step with the file
holding it.

### These files are secrets, and a wiki is usually a git repository

`password` is an Argon2id PHC string. It is not reversible and it is designed to
be stored, but it is still a hash of a real password and it does not belong in a
commit. So **`.rhizolog/users/` is gitignored by name**, beside `index.db` and
`server.json`.

That is why the gitignore names things inside `.rhizolog/` rather than the
directory: `times/` is authored data that *should* be committed. The directory
now holds four kinds of thing, and they want four different treatments:

| | |
|---|---|
| `index.db` | **derived** — rebuilt from the wiki; deleting it costs one scan |
| `times/` | **authored** — the only copy; back it up, commit it |
| `users/` | **authored, and secret** — the only copy; back it up, do *not* commit it |
| `server.json` | **volatile** — where a running server is; meaningless once it stops |

### A username is a filename, and is validated like one

Lowercase ASCII letters, digits, `-` and `_`, starting with a letter or a digit,
at most 39 characters. That character set makes most of [the slug rules](
architecture.md#slug-validation-is-security-critical) unreachable by
construction — no dots, no separators, no `:`, nothing to trim — and the one
that survives is Windows device names, so `con` and `lpt1` are refused by the
same list `Slug` uses.

**Uppercase is refused rather than folded.** Windows filenames are
case-insensitive, so `Tim` and `tim` would be one account here and two on Linux;
a wiki authored on one machine has to mean the same thing on the other. Folding
on input would also make the name in the file and the name somebody typed
different strings, which is precisely the disagreement the "username is the
filename" rule exists to prevent.

## One session, two transports

Signing in mints 256 bits from the OS random source and hands it back twice: as
an `HttpOnly` cookie, and in the response body.

The dashboard uses the cookie and never touches the body's copy. An agent uses
the body's copy in an `Authorization: Bearer` header and ignores the cookie.
They name the same session and the same row, so there is one lifetime, one
expiry, and one way to revoke.

This matters more here than it would elsewhere. The whole premise is that
[the API is the only interface](api-design.md) and that it is as usable by an
agent as by a browser — the same premise that forbids the desktop app a
privileged path. A cookie-only design would make authentication the exact point
at which those two stop being the same API, which is the divergence
[the desktop app page](desktop-app.md) exists to prevent arriving from the other
direction.

Long-lived named API tokens, with their own list-and-revoke lifecycle, are the
obvious next thing and are not built. A session token already works for a
script; what it does not do is survive a password change or carry a label saying
what it is for.

### Sessions are rows, and they are stored as a hash

The session table is in the **durable** half of the schema, and the reason is
not that sessions are precious. Losing them signs everybody out, which is
survivable and is exactly what deleting `index.db` should do. It is that a
[schema version bump](architecture.md#there-are-no-migrations) is an ordinary
consequence of changing how *pages* are indexed, and that has nothing to do with
who is signed in.

What is stored is the **SHA-256 of the token**, never the token. The index is a
file on a disk, and a row that could be lifted out and replayed as a credential
is a row worth not writing.

SHA-256 and not Argon2, which is worth saying out loud given what sits next to
it: a password is short, human-chosen and guessable, so verifying one is
deliberately slow. A session token is 256 random bits with no preimage to guess,
so a slow hash buys nothing — and it would be paid on every request rather than
once per sign-in.

Sessions last 30 days and slide, so an account in daily use is never signed out.
The expiry is pushed out at most once a day, which keeps that from costing a
database write per request; the only cost is that a session may expire up to a
day earlier than a strictly sliding one would.

### `SameSite=Lax` is standing in for a CSRF token

There is no CSRF token anywhere in this codebase, and the cookie's `SameSite`
attribute is why none is needed: a browser will not attach it to a cross-site
`POST`, `PUT`, `PATCH` or `DELETE`, only to a top-level `GET` navigation. Every
state-changing endpoint here is one of the former.

That is a load-bearing attribute rather than a default worth copying. Weaken it
and the gap has to be filled with something else.

`HttpOnly` is the other one, and it is specific to this product: a wiki renders
markdown somebody else may have written, and the session cookie is the one value
on that page that has to survive the renderer being wrong.

`Secure` is **off by default and has to be**, because the server speaks HTTP and
a browser discards a `Secure` cookie that arrives over one — presenting as a
sign-in that returns `200` and leaves you signed out. `RHIZOLOG_SECURE_COOKIES=1`
turns it on for a deployment behind a TLS proxy. It is a separate switch rather
than something inferred from `X-Forwarded-Proto`, because inferring it means
trusting a header anybody who can reach the port can send.

## What a failed sign-in is allowed to say

Nothing that distinguishes a username nobody has from a password that is wrong.
Both are `invalid_credentials`, with no `details` and the same message.

Saying which is a list of the accounts on the instance, one guess at a time —
and so is *timing*, which is the half that is easy to forget. A login for a
missing account would otherwise return in microseconds where a real one costs an
Argon2 run, so the missing case spends one too. The answer is still no; it just
costs the same either way.

The one exception is an account with **no password set**, which says so plainly.
It is only reachable for a file somebody wrote by hand and did not finish, no
password can ever be right for it, and an operator staring at "incorrect
password" has no way to work out why.

## Who may do what

Two roles, and deliberately only two. This is not a permission system: who may
read a given page is decided by that page, and the only instance-wide question
is whether somebody may administer accounts.

| | Owner | Member |
|---|---|---|
| Read and write pages, times, pins | yes | yes |
| List and read accounts | yes | yes |
| Change own name, password, profile | yes | yes |
| Create and delete accounts | yes | no |
| Change a role | yes | no |

Listing accounts is open to any signed-in account because naming somebody in a
page's `readers:` means knowing they exist — it is a prerequisite for using the
feature, not a privilege. What the listing never carries is a password hash, and
the surest way to keep one off the wire is for the type that goes on the wire to
have nowhere to put it.

**An owner is not a superuser over content.** An owner cannot read a private page
they do not own. That is a deliberate line, and it is honest about what it is:
anybody who can read the wiki *directory* can read every page in it whatever its
visibility says, and can replace a password hash with one they know. Accounts are
a boundary between **network callers**, never a boundary against whoever holds
the disk.

### The first account, and the two lockouts

`POST /api/users` succeeds without authentication **only while the wiki has no
accounts**. That is safe rather than merely convenient, and the argument is
worth keeping: a wiki with no accounts is already fully readable and writable by
anybody who can reach it, so claiming the first account grants nothing that was
not already on offer. The door closes behind it.

The first account is an **owner** whatever the request asked for, and the last
owner can be neither deleted nor demoted. Both guard the same outcome: a wiki
that requires authentication with nobody able to add an account to it,
recoverable only by editing files on the server's disk — which is the one thing
somebody administering a remote instance cannot do.

### Changing a password signs that account out everywhere

Including the session that made the change; the response says how many ended.

Anything less is not a password change. A token handed out before it would go on
working for its full thirty days, which is the difference between changing a
password and revoking access — and the case this exists for is a password
somebody thinks has leaked. Signing back in is the cost, and it is the right way
round.

Deleting an account does the same, and additionally **leaves that account's
pages alone**. A page owned by a deleted account keeps saying so, which is
recoverable in two ways; deleting somebody's pages along with their account is
recoverable in none.

## `/api/health` is reachable without an account, and says less

[`endpoint::live`](../backend/src/endpoint.rs) confirms a published
`server.json` by asking `/api/health` and comparing the `wiki_root` it gets
back. That happens *before* anybody could have signed in — it is how a second
copy of the desktop app discovers that a wiki is already open — so gating the
endpoint would break the single-instance check on every wiki with accounts.

What it *says* is gated instead. To an anonymous caller on a wiki that requires
authentication, the counts are omitted: how many pages there are, how many time
entries, whether a timer is running now. Those are facts about the wiki's
contents and none of them are needed to answer "is a server alive here, and is
it serving this directory".

`authentication_required` is added and is reported to anybody, because a client
cannot decide whether to show a sign-in prompt without being told, and the answer
is one bit that is obvious from the response to any other request anyway.

`wiki_root` survives the trim, and that is a filesystem path disclosed to
anybody who can reach the port. It is a real cost, accepted because the handshake
above is built on it and a path is the least interesting thing behind this door.
It is in [`TODO.md`](../TODO.md).

## The gate is one middleware, and its allow-list is five entries

Authentication is resolved once per request, in a middleware innermost of the
three the router carries, and the answer is put in the request's extensions.
Handlers read it with a `Viewer` extractor, which is a lookup rather than a
second resolution.

Being innermost has two consequences, both wanted: every handler can rely on the
viewer being present, and a request refused for having no account is still
counted as the API traffic it was.

It gates `/api` only. **The dashboard itself is served to anybody**, because the
login page is part of the dashboard and a login page behind a login is not a way
in.

Five routes are reachable without an account, and the list has a test whose job
is to notice a sixth: `GET /api/health`, `GET /api/auth/session`,
`POST /api/auth/login`, `POST /api/auth/logout` (signing out when you were not
signed in should be a no-op rather than an error about not being signed in), and
`POST /api/users` (the bootstrap, enforced by the handler rather than the list).

`GET /api/auth/session` is the interesting one. A `401` would be an answer, but
it is one a client cannot tell apart from a session that has just expired —
which is exactly what it is asking about. So it always returns `200` and
describes the situation: whether this wiki wants a sign-in, whether this request
has one, and who it is.

### The `Viewer` extractor fails closed

If a handler is ever mounted outside the gate, the extractor logs an error and
returns a `500` rather than defaulting to anything. The alternative is a route
that silently treats every caller as the single user, which is the one failure
mode of this design that would not announce itself.

## What is not built

- **Page visibility.** `public` / `internal` / `restricted` / `private` in
  frontmatter, and the index and query work to enforce it everywhere. This page
  covers identity only; visibility is the half that has to reach search, listing,
  the graph, tags, stats and pins, because an access-control model that is not
  applied to search is not an access-control model.
- **Rate limiting on sign-in.** Argon2 is a real natural throttle — roughly
  twenty attempts a second per core, and each one costs the attacker the same as
  it costs the server — but it is not a lockout, and a network instance wants
  one. In [`TODO.md`](../TODO.md).
- **Named API tokens.** See "One session, two transports" above.
- **Password reset.** There is no email and no second factor, so the recovery
  path is an owner setting a new password, or editing the file on the server.
