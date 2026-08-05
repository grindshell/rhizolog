---
tags:
  - rust
  - async
---

# Async in Rust

Futures are lazy. Nothing runs until something polls them, which is why an
async function that is never awaited does not merely finish late — it never
starts at all.

Two things follow from that, and they are the whole subject:

- A future has to be able to hold a borrow across an await point, so it becomes
  a self-referential struct. That is what [[notes/rust/pinning]] is about.
- Something has to do the polling. [[notes/rust/streams]] is not written yet,
  which is why it shows up as a *wanted page* — the link is not broken, it is a
  note about what to write next.

You can also link by path: [pinning](pinning.md) resolves the same way, relative
to this page's own directory.

This page has no `title` in its frontmatter. Its title comes from the heading
above, and follows it — edit the heading and the title changes with it.

## Not a link

A wikilink inside code stays inside the code, because links are pulled from the
parsed document rather than scanned for:

```markdown
[[notes/rust/streams]]
```

`notes/rust/streams` appears twice on this page and the graph records one link:
the mention above. This one is code, and a regex over the source would have had
to work that out for itself.

[[notes/rust/pinning]] is linked twice too, once as a wikilink and once as a
path, and those *are* two edges — different kinds of link to the same page. The
dashboard shows them as one row, because a reader wants the page, not the
bookkeeping.

The [Rust async book](https://rust-lang.github.io/async-book/) is the long
version.
