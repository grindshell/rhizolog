# Rhizolog Knowledge Base

This knowledge base tracks the design and implementation of Rhizolog itself.
Every page should be reachable from this index.

## Pages

- [Product vision](product-vision.md) — what Rhizolog is and who it's for
- [Tech stack](tech-stack.md) — chosen technologies and why they were picked
- [MVP plan](mvp-plan.md) — scope, milestones, dependencies, and risks
- [Architecture](architecture.md) — storage model, page format, link graph, module layout
- [API design](api-design.md) — endpoint surface and what makes it agent-friendly
- [The dashboard](dashboard.md) — the admin UI: screens, the editor, and where HTML may be injected
- [Drawing the link graph](link-graph.md) — the graph endpoint, and the
  deterministic layout that draws it
- [Pins](pins.md) — pages kept within reach, and why they are server state
- [Accounts](accounts.md) — signing in, and why a wiki with no accounts is still
  the open single-user one it always was
- [Page visibility](visibility.md) — public, internal, restricted, private, and
  the one SQL predicate that enforces all four
- [Time tracking](time-tracking.md) — timers, the time log on disk, and why a
  time link is not a link
- [Idea Inbox](idea-inbox.md): low-friction capture, explainable recurrence,
  lifecycle receipts, and promotion into wiki pages
- [The desktop app](desktop-app.md) — packaging as a portable Tauri app, and
  keeping the server the only interface
- [The product site](product-site.md) — rhizolog.com: what the landing page is
  for before there is anything to download, and why the demo reuses the
  dashboard's own components
