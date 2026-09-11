# Changelog

What changed in each release of Rhizolog, newest first. The version is the one
in the root `Cargo.toml`, each release is the tag `v` and that version, and its
builds are on the [releases page](https://github.com/grindshell/rhizolog/releases).

A section is headed `## <version> - <date>` once it is released. The release
workflow refuses a tag whose section has no date, and uses what is under the
heading as the release's notes. [`CONTRIBUTING.md`](CONTRIBUTING.md) has the
steps.

## 0.1.0 - 2026-09-11

The first release. Everything in it is new, so this is what there is rather than
what changed.

- **A wiki over a directory of markdown files.** The files are the source of
  truth; the search index is derived from them, rebuilds on startup, and a file
  watcher picks up edits made outside while the server runs. Wikilinks and
  ordinary markdown links are both tracked, and a link to a page nobody has
  written is a wanted page rather than an error.
- **An HTTP API first**, with an OpenAPI document generated from the routes and
  Swagger UI to browse it, and a dashboard that is its first client: search,
  an editor, tags, pins, meta-stats and a drawn link graph.
- **Single user by default.** A wiki with no accounts is open on loopback.
  Creating the first account turns authentication on, and pages can then be
  public, internal, restricted or private.
- **Time tracking**: timers and entries as files, grouped by name, attached to
  pages, with a statistics section and a heat map of the week.
- **Idea Inbox**: one-field capture, suggestions computed locally with the
  arithmetic shown, lifecycle receipts, and promotion into a page.
- **Long-form writing**: pages assembled into a manuscript with a manifest, a
  word log that counts words added and removed rather than their difference,
  pacing against a target and a date, splitting and merging chapters, and prose
  rules you write yourself.

### Downloads

- **The server**, for Windows x64 and Linux x64, with the dashboard compiled
  in. The Linux build runs on glibc 2.35 or newer: Ubuntu 22.04, Debian 12 and
  their contemporaries.
- **The desktop app**, for Windows x64, **as a preview.** It works, and four
  things that should be true of a download are not yet: the icons are
  placeholders, it is not signed so SmartScreen warns about it, it opens a blank
  window rather than an explanation when the WebView2 runtime is missing, and it
  keeps its browser data in `%LOCALAPPDATA%\dev.rhizolog.app` rather than
  beside itself.

Each archive holds the program, `LICENSE`, `THIRD-PARTY-NOTICES.txt` and a
`README.txt` saying how to run it.

### Known limitations

- No TLS of its own, no rate limiting on sign-in and no audit log, so serving a
  wiki to other people wants a reverse proxy in front.
- Developed and tested on Windows. The Linux server passes the same tests in CI
  and has not been run by hand.
- No page history, link rewriting on move, attachments or transclusion, on
  purpose; see the README.
