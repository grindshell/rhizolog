# Product vision

Rhizolog is a wiki backend and server in the same vein as MediaWiki and
TiddlyWiki. The name combines "rhizome" and "log": knowledge branches off
chaotically, and Rhizolog is built to track that.

## How it differs from MediaWiki and TiddlyWiki

- **Developer tool.** Rhizolog is for developers managing knowledge bases,
  not for hosting public community wikis.
- **Rich, agent-friendly API.** The HTTP API is a primary interface, designed
  for access by AI agents as well as humans, with OpenAPI definitions.
- **Single-user by default.** A wiki with no accounts is open: no sign-in,
  nothing refused, every request treated as the one user. See below.

## Single-user is the default, not the only shape

This used to read "no accounts, permissions, or multi-tenancy", and loopback was
the entire security boundary. Serving a wiki over a network is what changed it.

What has *not* changed is the default. A wiki with no accounts behaves exactly
as it always did, and creating the first account is what turns authentication
on — so the local case pays nothing, and putting a Rhizolog on a network stays a
deliberate act. It is still not a public community wiki engine: the model is a
handful of named accounts who can be given access to particular pages, not
registration, moderation or anonymous editing.

[Accounts](accounts.md) has the reasoning, the costs, and where the boundary
actually is — which is between network callers, never against whoever holds the
disk. [Page visibility](visibility.md) is the other half: which of those named
accounts a given page is for, and the one rule that has to be applied to every
query rather than to the page read alone.

## Interface

An admin dashboard-style UI (reflecting the single-user focus) that supports:

- searching wiki pages
- authoring wiki pages
- viewing the wiki's meta-stats: links between pages, tags, API usage, etc.

## The licence is AGPL-3.0-or-later, because this is a server

Until it had a `LICENSE` file the project was all-rights-reserved by default,
which meant nobody handed a copy had permission to run it — the state
[`TODO.md`](../TODO.md) called the only thing that stopped a release outright.

The ordinary GPL would be the wrong copyleft here. It is triggered by
*distribution*, and the natural way to use somebody else's Rhizolog is to talk
to it over a network, which is not distribution — so a hosted fork could take
the improvements and give nothing back without ever breaching it. Section 13 of
the AGPL is exactly that gap closed, and this project is the shape the clause
was written for: a thing whose primary interface is
[an HTTP API](api-design.md) and whose whole design premise is that the API is
the *only* interface, local app included. If the desktop shell is not allowed to
be a privileged client, a network user should not be a second-class one either.

`-or-later` rather than `-only` is the FSF's own recommendation and what their
boilerplate says, and it avoids a relicensing exercise if there is ever an
AGPLv4.

It costs nothing for the single user the product is designed around. Running
your own copy, modifying it, and never letting anyone else near it triggers
none of the licence at all.

Two consequences that are not done yet, both in [`TODO.md`](../TODO.md): the
dependency trees have never been checked for anything the AGPL cannot be
combined with, and the per-file notices the licence's own appendix asks for do
not exist. The `license` field is set once in `[workspace.package]` and
inherited, so the two crates cannot drift apart on the answer.
