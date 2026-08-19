# Page visibility

[Accounts](accounts.md) answered "who is this request". This is the other half:
what they are allowed to see. Together they are what makes serving a Rhizolog
over a network mean something more than sharing the whole wiki.

## The ladder

Four rungs, each strictly narrower than the one above it, in a page's
frontmatter:

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
| `internal` | Any account on this wiki — **and what an unmarked page means** |
| `restricted` | The `readers` list, plus the owner |
| `private` | The owner alone |

Plus one rule that cuts across all four: **your own pages are always yours.** A
page you own is readable by you whatever its visibility says, so marking
something private does not hide it from yourself.

Ordering them is not decoration. It makes "can this account read this page" one
comparison rather than a policy engine, and it is what lets the whole thing be
[one SQL predicate](#one-predicate-pasted-everywhere).

### `internal` is the default, and that is the load-bearing choice

Every page in an existing wiki has no `visibility:` line. Whatever an unmarked
page means is therefore what the entire wiki means on the day somebody creates
the first account, and the two obvious answers are both wrong:

- **`public`** would protect nothing. Turning authentication on would leave the
  wiki exactly as exposed as it was until every page had been marked by hand,
  which is the wrong way round for a switch whose whole purpose is protection.
- **`private`** would make every page vanish for everyone but its owner — and
  unmarked pages have no owner, so it would make them vanish for *everybody*.
  Five minutes after switching authentication on you would have an empty wiki
  and a lot of bulk editing ahead of you.

`internal` is the answer that changes nothing for the people already using the
wiki and nothing for strangers, which is what a default should do.

### An unrecognised word means `private`, not the default

`visibility: privte` is not a parse error and does not fall back to `internal`.
It reads as `private`.

Somebody who typed that was trying to *restrict* a page. Falling back to the
default would do the opposite of what they asked, silently, on the page they
were most careful about. Failing closed costs them a page that is briefly too
hidden — visible to its owner, who is the person who can fix it.

It is deliberately not a hard error either. A page whose frontmatter will not
parse is reported as malformed and drops out of every listing, taking its title,
tags and `created` with it; discovering a typo that way is worse than the typo.

## `public` needs the instance to agree

Marking a page `public` does nothing on its own. An anonymous caller only ever
reaches it when the instance sets `RHIZOLOG_ANONYMOUS_READ`; without that,
`public` behaves as `internal`.

**Publishing to the open internet takes two deliberate acts**, in two different
places, by two different means — a line in a file and a variable in a
deployment. Neither is much use without the other, and neither is a thing you
do by accident. The specific accident that shape prevents is the common one:
somebody marks a page `public` on their laptop meaning "fine for the team", and
six months later the instance goes on a network.

It is a configuration switch rather than something derived from the wiki, which
is the opposite of [how authentication itself is decided](accounts.md). The
difference is what kind of question each one is. "Does this wiki have accounts"
is a fact about a directory and can be looked up. "Should strangers be able to
read this instance" is a statement of deployment intent, and nothing on disk
knows it.

Anonymous access is **reads only, and only of `public` pages**. There is no
configuration in Rhizolog that lets an unauthenticated caller write anything.
The times and pins endpoints are excluded too: they are the operator's own
working state — what they were doing and when — and no page being public says
anything about wanting that published.

## An owner is not a superuser over content

An owner can create and delete accounts. An owner **cannot** read a private page
they do not own.

That line is drawn deliberately, and it is worth being honest about what it is
and is not. It is not a security boundary against the person running the
server: anybody who can read the wiki *directory* can read every page in it
whatever its frontmatter says, and can put a password hash they know into
`.rhizolog/users/`. Rhizolog is a boundary between **network callers**, never a
boundary against whoever holds the disk.

Given that, "owners can read everything" would buy nothing — anyone who wants it
already has it by other means — and would cost the only thing `private` is for,
which is being able to keep a scratch page out of colleagues' search results
without thinking about who administers the instance this month.

## One predicate, pasted everywhere

Visibility enforced in the page-reading handler and nowhere else is not
visibility. The same private page leaks through:

- the **listing**, as a row;
- **search**, as a slug, a title, a tag, *and an excerpt of its body with the
  match highlighted*;
- a **backlink**, as a slug and a title;
- the **tag histogram**, as a tag nobody else uses and a count;
- **`most_linked`**, by name;
- the **graph**, as a node;
- and **`total`** in any of the above, as a number that says something exists.

Seven shapes, each with its own fix. So the rule lives in
[`index/audience.rs`](../backend/src/index/audience.rs) as one SQL fragment,
and every query that can return a page pastes it in.

That is deliberately the boring, repetitive answer. The two alternatives each
fail in a way that matters:

- **A view over `pages`** cannot be parameterised by who is asking without a
  session variable SQLite does not have.
- **Filtering in Rust after the query** breaks `count(*)`, `limit`/`offset` and
  every aggregate — which is how a paginated listing ends up reporting twenty
  results and running out after three, and how a "total" becomes a statement
  about pages the caller cannot see.

The touched queries use **named parameters**. The ones they replaced bound
`?1`..`?5`, and adding two more positional parameters to each would have meant
renumbering by hand at every call site — the kind of edit that compiles and
returns the wrong rows.

### A link to a page you cannot read disappears

Not "loses its title". The distinction is the single easiest thing to get
backwards here, and getting it backwards is worse than not filtering at all.

A link whose target is a page nobody has written is a **wanted page**, and those
are most of the point of the graph — a branch someone gestured at. If a link to
a *private* page merely lost its title it would become indistinguishable from
one, so the graph would draw it, name it by its slug, and advertise it as
something worth writing. A private page's slug is usually its title.

So the filter is not "the target is visible". It is "no page the caller cannot
read sits at the target", which leaves genuinely unwritten pages alone.

### What none of this can promise

**A slug written in a page body is readable by anyone who can read that body.**
If `notes/plans` says `See [[secret/acquisition]]` and you can read
`notes/plans`, you can see that string. No filtering in the index changes that;
the markdown is the markdown.

What visibility protects is the private page's **contents, title and
existence** — everything behind the slug. `a_slug_written_in_a_readable_body_is_readable_and_that_is_accepted`
in `backend/tests/visibility.rs` states that as a test, so nobody later mistakes
it for a bug.

### Everything is computed within the subgraph you can see

One consequence looks like a defect and is not. A page linked only from a page
you cannot read is an **orphan to you** and not to its owner. Two accounts can
see different degrees for the same node, different tag counts, and different
totals.

Both answers are correct; they are answers to different questions. The
alternative — reporting a page as linked without being able to say from where —
is how you learn that something you cannot see points at it.

## 404, never 403

Asking for a page you may not read gets the same response as asking for a page
that is not there: same status, same code, same message, same shape.

A `403` would confirm that something exists at a slug somebody guessed, and for
a private page the slug is usually the title. `secret/acquisition` returning
"forbidden" rather than "not found" is most of what was being protected.

The same applies to every write. `PUT`, `PATCH`, `DELETE` and `POST /api/move`
all check first and answer `404`, because a write that failed differently would
be the same oracle with extra steps — and a `PUT` that succeeded would overwrite
a page its author could not see going.

`GET /api/links/{slug}` reports `exists: false` for the same reason, which is
also the honest answer: it is the answer a genuinely absent page gives, and the
inbound links it lists are only ever from pages the caller can already read.

`PUT /api/pins/{slug}` is the one that is easy to miss. It already answered
`404` for a page that is not there — so left alone it would answer `200` for a
page that is there and unreadable, which is the oracle in one request. It reads
the page and checks before pinning.

## Two spellings of one rule, and the test that keeps them honest

Visibility is decided twice.

`index/audience.rs` decides it in **SQL**, over rows, for the listing and the
graph. `api/pages.rs` decides it in **Rust**, over a file that has just been
read, for a single page.

Duplication like that is exactly what drifts, and it is worth the risk because
the two answer different questions. A single implementation would mean either
reading every page off disk to list them, or serving a page under whatever
visibility it had at the last reindex — and the second is a real bug, because
the file is the truth and the index is a cache of it.

`the_single_page_read_and_the_listing_agree_about_every_page` is the guard: six
pages, one on each rung, asked for through both paths by two different accounts,
asserting the two never disagree.

## Owners get filled in, and cannot be removed by accident

A `restricted` or `private` page with no owner is readable by nobody at all —
technically correct, useless, and fixable only by editing the file on the
server, which is the one thing somebody administering a remote instance cannot
do.

So writing such a page fills the owner in with the account doing the writing.
`PUT` does it too, which matters more than it sounds: `PUT` replaces every
field, so an editor that saved a page without sending the owner would hand
every page it touched to whoever last pressed save.

The one case that is refused rather than filled in is an **explicit**
`owner: null` alongside a narrowing visibility. Somebody who sent that asked for
exactly this, and quietly writing their own name instead would be ignoring what
they said — so it comes back as `ownerless_page`, a `400` naming the slug and
the visibility.

## What is not built

- **Per-directory or per-tag defaults.** Every page carries its own line. A wiki
  where `private/**` is private by convention has to say so on each page, which
  is fine for a handful and tedious for a branch. The natural shape is a
  `.rhizolog/visibility.toml` of prefix rules, and the reason to wait is that
  it introduces a second place a page's visibility is decided — see the section
  above on how much trouble the *first* second place already is.
- **Groups.** `readers:` is a list of accounts. On a wiki with three people that
  is the same thing as a group and considerably clearer.
- **Write permissions distinct from read.** Anybody who can read a page can edit
  it. Splitting the two is a real feature and a different one; today the model
  is that visibility decides who is in the room.
- **Visibility for pins and the time log themselves.** Both are wiki-wide state
  shared by everyone who can sign in, and there is no way to mark an entry as
  one person's. What they *do* carry is the page join: a pin to, or an entry
  against, a page you cannot read comes back with **no title**, exactly as one
  whose page has been deleted does. The slug stays, because it is the pin's or
  the entry's own content and the list is shared — the same limit as a slug
  written in a page body, above. The statistics rank such a page under its slug
  rather than dropping it, because the time really was spent.
