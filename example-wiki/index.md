---
tags:
  - meta
---

# Example wiki

Six pages, arranged to show what Rhizowiki does with them. Run the server
against this directory and the dashboard reports two orphans and one wanted
page — all three on purpose.

Nothing here is special. It is markdown in a directory; delete the whole thing
and point `RHIZOWIKI_ROOT` at your own notes.

## Start here

- [[notes/rust/async]] — nested slugs, and a link to a page nobody has written
- [[notes/rhizome]] — where the name comes from
- [[notes/deleuze]] — and where *that* comes from

## The orphans

`scratch/inbox` exists too, but nothing links to it. That is what makes it an
orphan, and why the dashboard counts it: in a wiki that branches, the pages you
cannot reach are the ones you forget you wrote.

This page is the other orphan, which is worth knowing before you go looking for
the bug. An entry point has nothing above it to link to it, so the front page of
a wiki is almost always orphaned. The statistic is still doing its job — it just
cannot tell the difference between a page nobody reaches and a page nobody needs
to reach *from inside*.
