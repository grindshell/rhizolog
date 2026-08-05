---
tags:
  - meta
---

# Example wiki

Nine pages, arranged to show what Rhizowiki does with them. Run the server
against this directory and the dashboard reports two orphans and one wanted
page — all three on purpose.

Nothing here is special. It is markdown in a directory; delete the whole thing
and point `RHIZOWIKI_ROOT` at your own notes.

## Start here

- [[notes/rust/async]] — nested slugs, and a link to a page nobody has written
- [[notes/rhizome]] — where the name comes from
- [[notes/deleuze]] — and where *that* comes from

## Two ways to read a slug

Open [[notes/rust/async]] and there are two ways to follow the `rust` in its
slug. Three of the pages here exist to show that they are not the same way.

The breadcrumb walks the tree. `rust` there means `notes/rust`, and asks for
what is at or under it: [[notes/rust]] itself, plus `async` and `pinning`. It
does not return [[notes/rustlings]], which only starts with the same characters.

The badge beside the tags walks nothing. `/rust` there means *any* directory
called `rust`, and this wiki has two — so it also returns
[[scratch/rust/from-a-talk]], which the breadcrumb cannot reach from here at
all.

Both are correct, and they are separate controls because they answer different
questions. The second one is what [[notes/rhizome]] argues for, arriving as a
filter rather than as a metaphor.

## The orphans

`scratch/inbox` exists too, but nothing links to it. That is what makes it an
orphan, and why the dashboard counts it: in a wiki that branches, the pages you
cannot reach are the ones you forget you wrote.

This page is the other orphan, which is worth knowing before you go looking for
the bug. An entry point has nothing above it to link to it, so the front page of
a wiki is almost always orphaned. The statistic is still doing its job — it just
cannot tell the difference between a page nobody reaches and a page nobody needs
to reach *from inside*.
