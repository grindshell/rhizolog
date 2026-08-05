---
title: Pinning
tags:
  - rust
  - async
---

# Why `Pin` exists

A future that holds a reference into itself cannot be moved, because moving it
would leave that reference pointing at the old address. `Pin` is the promise not
to move it.

This page *does* have a `title` in its frontmatter, so its title is "Pinning"
even though the heading says something else. Compare [[notes/rust/async]], which
has no stored title and takes one from its heading.

Both are ordinary states for a page to be in, and the API says which one you are
looking at — see `title_derived`.
