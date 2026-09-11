# Releases

A release of Rhizolog is a tag, `v` and the version in the root `Cargo.toml`,
pushed to the GitHub mirror, where a workflow builds every archive from it and
leaves a draft release for a person to publish. This page is why it has that
shape. The steps are in [`CONTRIBUTING.md`](../CONTRIBUTING.md), and what each
release contains is in [`CHANGELOG.md`](../CHANGELOG.md).

## Where it runs

Development happens on a self-hosted Forgejo, and CI runs on GitHub Actions
against the public mirror. That is why the mirror exists: GitHub's hosted
runners supply a clean Windows machine, which is the platform Rhizolog is
developed and used on, and the one a self-hosted runner would have had to be.
The cost is that the mirror has to be pushed to, `master` and tags both, before
CI sees anything, and the two remotes are kept in step by hand.

## One version

The version is written once, in `[workspace.package]` in the root
`Cargo.toml`. Both crates inherit it. `desktop/tauri.conf.json` leaves it out,
so Tauri reads it from Cargo, and it still lands in the executable's version
resource, which is what Explorer's Details tab shows: checked, 0.1.0 with the
field gone. It is the number `/api/health`, `.rhizolog/server.json` and the
OpenAPI document report. Before this, three files said `0.1.0` and were bumped
by hand in step.

`packaging/version.ps1` is the one place anything asks for it. It fails when the
crates disagree, fails a tag that is not `v` and that number, and fails a tag
whose `CHANGELOG.md` section has no date, so a release cannot go out without its
notes. `frontend/package.json` still says `0.0.0`, because the dashboard is
never released on its own.

Both crates also say `publish = false`. Rhizolog is released as binaries, never
to crates.io, and that one line is also what tells `cargo-deny` and
`cargo-about` that the AGPL crates they find are the project rather than a
dependency of it.

## What a release carries

| File | What it is |
|---|---|
| `rhizolog-<version>-x86_64-pc-windows-msvc.zip` | The server for Windows, with the dashboard compiled in |
| `rhizolog-<version>-x86_64-unknown-linux-gnu.tar.gz` | The same for Linux, built on Ubuntu 22.04 so it runs on glibc 2.35 and newer |
| `rhizolog-desktop-<version>-x86_64-pc-windows-msvc.zip` | The desktop app, for Windows |
| `SHA256SUMS.txt` | The checksum of each |

Each archive holds one directory named for it, containing the program,
`LICENSE`, `THIRD-PARTY-NOTICES.txt` and a `README.txt` saying how to run it.
Target triples in the names rather than friendlier words, because they are
unambiguous and they are what anybody scripting a download reaches for.

**Archives rather than bare executables**, because the notices have to travel
with the program ([Dependency licences](dependency-licences.md)), and a file
beside a download is one the next person it is copied to never sees. One
directory inside each, so unpacking never scatters files across Downloads. The
Linux one is a tarball because a zip loses the executable bit.

**Windows and Linux, x64.** Windows is where it is developed and used. Linux
costs one runner and catches what Windows cannot: the server building and
passing its tests somewhere else, which nobody had checked until CI did. macOS
and ARM were offered and not taken, because a binary nobody has run is a binary
nobody can support.

## The desktop app ships as a preview

0.1.0 includes the desktop app with all four of the things
[`TODO.md`](../TODO.md) lists before it goes to anyone else still open:
placeholder icons, no code signing, no check for a missing WebView2 runtime, and
browser data in `%LOCALAPPDATA%\dev.rhizolog.app` rather than beside the
executable. Getting it into hands was chosen over getting those done first, and
the release says so wherever a user will look: the archive's `README.txt`, which
also says what to click past SmartScreen, the release notes, and the changelog.
"Preview" comes off when the four are done, and nothing else has to change.

## Built by CI, published by a person

Pushing a tag runs `.github/workflows/release.yml`. It checks the tag, builds
each archive on its own platform, runs every server it packaged, and creates a
**draft** GitHub release with the archives, `SHA256SUMS.txt`, and the
changelog's section as the notes. It never publishes. Publishing is the one
outward act in the process and cannot be taken back once somebody has
downloaded, so it is a person's button, pressed after downloading an archive or
two from the draft and running them.

Running the workflow by hand is a dry run: the same builds and checks, with the
archives attached to the run instead of a release, and no tag or dated changelog
needed. That is how a change to the process gets tried without spending a
version number on it.

The release builds use no cache, unlike CI, so what ships was compiled from
nothing on a clean machine. The steps that do the work are PowerShell scripts in
`packaging/`, which every hosted runner has, rather than YAML, so they run the
same way on a desk as in CI.

## Notices, one file per program

`packaging/notices.ps1` writes `THIRD-PARTY-NOTICES.txt` for one program at a
time, and for the one platform it is packaging, because the lists differ: the
desktop app's crates are about twice the server's, and a Windows program has no
business listing crates only Linux compiles. The file has five parts, and the
script fails rather than write one with a part missing:

- **The Rust crates**, from `cargo-about` with `packaging/about.toml`, whose
  `accepted` list is the review's allowlist. A crate under anything else fails
  the release.
- **The dashboard's npm packages**, each from its own licence file, which is
  where its copyright line is. A package without one fails the release.
- **Swagger UI's licence and NOTICE**, read out of the archive
  `utoipa-swagger-ui`'s build script downloaded, so they belong to the version
  actually embedded.
- **SQLite**, one paragraph, since it is public domain and asks for nothing.
- **For the desktop app, the WebView2 SDK's licence and notice file**, from the
  `Microsoft.Web.WebView2` NuGet package for SDK 1.0.3650.58, the version
  `webview2-com-sys` 0.38.2 carries the loader from. The script refuses to run
  once `Cargo.lock` has a different `webview2-com-sys`, because that SDK version
  was read off the crate's changelog by hand and the next one may bring other
  terms.

Only the file carries them for now. Serving them from the program itself, from
an About view in the dashboard and the desktop app, is the better answer for a
single executable that gets copied around, and is in [`TODO.md`](../TODO.md).

## The smoke test

`packaging/smoke.ps1` unpacks each server archive into an empty directory,
starts it against a one-page wiki, and checks that `/api/health` reports the
release's version and the one page, that `/` serves the dashboard, that
`/swagger-ui/` answers, and that the OpenAPI document carries the version too.
That closes a gap no `cargo test` could: `rust-embed` reads from disk in debug
builds, so a passing test said nothing about what a shipped binary serves. The
desktop archive is checked for its files only, since CI has no display to open a
window on.

## CI on every push

`.github/workflows/ci.yml` runs what [`CONTRIBUTING.md`](../CONTRIBUTING.md)
asks of a commit on Windows, plus four things nobody was running by hand:

- **The server on Linux**, with its tests.
- **The headless server's dependency graph reaching no GUI toolkit**, checked
  with `cargo tree` rather than by building on a machine without one, because
  the property is about the graph and a runner has whatever it happens to have.
- **The committed OpenAPI document and generated types being current**, by
  regenerating both and failing on any difference. This is also why bumping the
  version means regenerating them: the document carries it.
- **The licences**, with `cargo deny check licenses` against `deny.toml`, the
  same list as the notices.

The Windows runner's git converts line endings on checkout by default. Every job
turns that off first, so CI tests the same bytes a checkout here has. Every step
runs in bash, Windows included, because bash stops at the first command that
fails and GitHub's PowerShell wrapper only looks at the last.

## What Linux found the first time

Two bugs, both invisible on Windows, turned up the first time the server's tests
ran on Linux, in WSL, while this process was being built and before CI had run
once:

- **The page walk followed the filesystem's order.** A directory lists its
  entries alphabetically on NTFS and in hash order on ext4, and the startup scan
  writes a word-log baseline for each new page in the order it meets them, so
  the same wiki produced the same lines in a different order on each. The walk
  sorts by file name now, and the order is part of its contract.
- **The watcher took reading for writing.** inotify reports a file or directory
  being opened and closed, which ReadDirectoryChangesW never does, and the
  watcher treated every event as a change. Its own listing of `.rhizolog/times`
  at startup classified as the time log changing and forced a rescan, which
  swept up a page written a moment later and logged it as the scan's baseline
  instead of somebody's writing. Access events are ignored now, except a file
  closed after being written.

Neither was going to be found by anything that runs only on Windows, which is
the whole case for the Linux job.

## Not done

- **Signing.** Unsigned Windows executables get a SmartScreen warning, the
  desktop app's more visibly than the server's, and its `README.txt` says what
  to click.
- **The notices inside the programs**, above.
- **Reproducible builds.** Nothing checks that one tag builds the same bytes
  twice.
- **macOS, ARM, installers and auto-update**, not for now, for the reasons in
  [The desktop app](desktop-app.md).
