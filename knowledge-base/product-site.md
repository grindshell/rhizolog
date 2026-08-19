# The product site

`site/` is the static site at rhizolog.com. It is not served by the backend and
the backend does not know it exists, which is the one structural thing worth
saying about it: `frontend/` is a client of the API and ships inside the binary,
while this is a separate artefact with its own build and its own deployment.

## What the page is for

The site goes up **before** there is anything to download. The desktop app is
not signed, there are no released binaries, and [`TODO.md`](../TODO.md) has a
list of things that have to happen before a stranger can be handed a copy. So
the landing page's job is not conversion to a download; it is to explain what
Rhizolog is well enough that somebody clones it.

That fixes the calls to action, and they are deliberately a pair rather than a
button:

- **Source on GitHub** is primary. The GitHub repository is a mirror of the
  Forgejo instance development actually happens on, and it exists so that CI can
  build releases and so that people have somewhere to file bugs.
- **Read the docs** sits beside it.
- **"Signed builds coming soon"** is a sentence, not a greyed-out button. A
  download control that does not download is worse than an honest line.

When there is a release, the download section drops into the hero without the
page changing shape. The Windows portable executable and the headless server
binaries are two different offers with two different stories, so they get two
entries rather than one button that guesses at the reader's platform, and the
SmartScreen warning on an unsigned executable gets said out loud rather than
discovered.

## Why it reads the way it does

The copy is drawn from [`README.md`](../README.md) and from this knowledge base
rather than written fresh. That is not laziness about marketing: the project's
existing prose gives a reason for every claim it makes, and a landing page that
suddenly started asserting things without reasons would read as a different
product than the one the reader is about to clone.

Two consequences:

- **Every feature claim carries its because.** "Several timers can run at once,
  and they are allowed to overlap, because attention is not exclusive and a
  tracker that insisted otherwise would be asking you to lie to it" sells the
  product. "Flexible time tracking" does not.
- **There is a "What it doesn't do" section**, split into what is deliberately
  absent and what is thin and known. It says there is no TLS of its own, no rate
  limiting on sign-in, and no audit log. Against a landscape of pages that admit
  nothing this reads as confidence, and the audience for a developer tool checks
  anyway.

**No em dashes.** They are the single most recognisable tell of machine-written
prose, and a landing page is read by people deciding whether a project is
serious.

This started as a rule for `site/` alone, on the reasoning that the em dash was
the existing house voice everywhere else and marketing copy was the only place
it would be read as a tell. That reasoning did not survive contact with the
question of who else reads this repository: the README is the first thing a
stranger sees, the knowledge base is the argument for taking the design
seriously, and a doc comment is published into the OpenAPI document that agents
read as the manual. None of those are less exposed than the landing page.

So the rule is project-wide, and it lives in [`AGENTS.md`](../AGENTS.md) under
House style along with its three exceptions, all of which are specimens rather
than prose: the encoding warnings that use a literal em dash to show what
mojibake does to it, `example-wiki/` because it is a fixture, and generated
files, which get fixed upstream and regenerated.

## The mark

The accent is the mint green of the branching-node glyph in `desktop/icons/`,
which is six nodes and five edges with no trunk. That is a rhizome, which is
where the project's name comes from and the only thing in the repository that
was already saying so visually.

`frontend/public/favicon.svg` used to be an unrelated purple zigzag, evidently
downloaded, carrying a stack of Figma blur filters and no relationship to
anything else here. It is now the same branching mark, redrawn as plain geometry
so the dashboard tab, the app icon and the site agree.

**Amber belongs to wanted pages** and to nothing else on the site, so that a
reader who has met one dashed ring recognises the next one.

## The two figures are placeholders

The link graph and the hours heat map are hand-authored, and both components say
so at the top. They are meant to become **fixtures captured at build time** from
a real server run against `example-wiki/`, for the same reason the OpenAPI
document is generated from the routes rather than written: a figure that has
quietly stopped being true is worse than no figure, because a reader cannot tell
which ones still hold.

Two things the capture has to get right:

- The graph's node positions come from a hash of the slug, not from a layout
  algorithm and not from taste. The page claims determinism, so the picture on
  the page has to be the one the endpoint produces.
- The heat map needs `?at=2026-08-06T18:00:00Z&offset=0`. The committed entries
  are pinned to 30 July to 6 August 2026, so a capture without it returns an
  empty week and the figure renders blank. See
  [`example-wiki/index.md`](../example-wiki/index.md), which explains why Today
  is empty and always will be.

## The demo is one static wiki

Not a page per feature, and not screenshots. The build captures `example-wiki/`
through the API and renders a browsable copy: pages, search, the graph, tags and
the time log.

It **reuses the dashboard's own SolidJS components**, prerendered, rather than
reimplementing them. That costs a real build step, since the components have to
be rendered and frozen rather than just fetched as JSON, and it buys the
property worth having: the demo cannot show a UI that the download does not
have, and a
dashboard change appears in the demo on the next build without anybody redrawing
anything.

This is what settled the toolchain. The landing page alone needs no framework at
all. The docs section wants markdown, and the demo wants Solid components
prerendered to static HTML, and Astro is the one option where both are a
documented path rather than something to invent.

## Deployment

The site is a directory of static files, deployed to the same box as
grindshell.com and its siblings, behind Caddy, with the scripts in
`server-configs/static`. `./deploy.sh ../../../rhizolog/dist rhizolog` unpacks
a build into a timestamped release and swaps a symlink, so a release is atomic
and a rollback is repointing it.

The Caddy block for `rhizolog.com` predates this site: it was written for an
earlier version of Rhizolog that served a single-page app, and it had to change
in three ways for a statically generated one.

- **`try_files {path} /index.html` had to go.** That is the SPA fallback, and on
  a prerendered tree it answers every unknown path with the landing page and a
  `200`. The site would never 404, and a crawler would find the same page at
  every address nobody wrote. The replacement is
  `try_files {path} {path}/index.html {path}.html`, which is what the
  grindshell.com block already does.
- **The cache rule matched nothing.** It hard-cached `/assets/*`, which is
  Vite's output directory. Astro's content-hashed assets live in `/_astro/`, so
  in practice the stylesheet carried no `Cache-Control` at all, and the
  `no-cache` on `/index.html` never fired because browsers ask for `/`.
- **Errors had nowhere to go.** `src/pages/404.astro` builds to `/404.html` and
  nothing serves it without a `handle_errors` block rewriting to it. The SPA
  fallback had made a 404 page unreachable by construction, which is why there
  was not one.

The 404 page is the wanted-page idea, spent where it costs nothing: a dashed
amber ring, and the observation that inside a wiki this would not be an error at
all, because a link to a page nobody has written is a wanted page rather than a
failure. On a website it is just a 404, and the page says so.

## Deliberately not decided yet

- **Where Docs and API point.** Both are linked from the nav and neither exists.
  They are written as real links so the day they land is a routing change rather
  than a redesign.
- **Publishing this knowledge base.** It is the best writing in the project and
  it would make the site considerably more interesting than a landing page
  alone. It is also written for its author, and a couple of pages argue with
  earlier versions of themselves. Publishing it is a deliberate act, not a
  default.
