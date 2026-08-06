# Product vision

Rhizolog is a wiki backend and server in the same vein as MediaWiki and
TiddlyWiki. The name combines "rhizome" and "log": knowledge branches off
chaotically, and Rhizolog is built to track that.

## How it differs from MediaWiki and TiddlyWiki

- **Developer tool.** Rhizolog is for developers managing knowledge bases,
  not for hosting public community wikis.
- **Rich, agent-friendly API.** The HTTP API is a primary interface, designed
  for access by AI agents as well as humans, with OpenAPI definitions.
- **Single-user.** No accounts, permissions, or multi-tenancy.

## Interface

An admin dashboard-style UI (reflecting the single-user focus) that supports:

- searching wiki pages
- authoring wiki pages
- viewing the wiki's meta-stats: links between pages, tags, API usage, etc.
