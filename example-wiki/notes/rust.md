---
title: Rust
tags:
  - rust
---

# Rust

A page that names a directory. `notes/rust.md` sits beside the `notes/rust/`
directory and nothing collides: the slug `notes/rust` is this file, and the
pages underneath have slugs of their own.

It matters for one filter. Following `rust` in the breadcrumb of
[[notes/rust/async]] asks for everything *at or under* `notes/rust`, and this
page comes back along with its children — a page sitting where a directory sits
is that directory's index, and leaving it out of its own listing would be a
surprise.

What is under here:

- [[notes/rust/async]] and [[notes/rust/pinning]].

What is not, despite appearances:

- [[notes/rustlings]], which only starts with the same characters.
- [[scratch/rust/from-a-talk]], which is in the other `rust` directory.

This page is also not in a `rust` directory — it *is* the `rust` one. Its own
name is not a directory it sits in, so the flat filter answers for it under
`notes` and not under `rust`.
