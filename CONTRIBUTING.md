# Working on Rhizolog

The [README](README.md) is for somebody using Rhizolog; this is for somebody
changing it. Three other files carry the rest:

- [`AGENTS.md`](AGENTS.md) holds the rules a coding agent follows, and nearly
  all of them apply to people too: the house style, how commit subjects are
  scoped, and the traps this repository has already fallen into.
- [`knowledge-base/`](knowledge-base/index.md) is why the thing is built the way
  it is, kept as a wiki because that is the obvious thing to do here. A design
  decision is recorded there in the same commit as the code.
- [`TODO.md`](TODO.md) is what is known and not done, each entry with the reason
  it is not done.

## Running it from a checkout

You need [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) and
[pnpm](https://pnpm.io/). Build the dashboard once, then run the server against
the example wiki:

```powershell
cd frontend; pnpm install; pnpm build
cd ../backend; $env:RHIZOLOG_ROOT = "../example-wiki"; cargo run
```

On macOS or Linux the last line is `RHIZOLOG_ROOT=../example-wiki cargo run`.
The first compile takes a while: SQLite is built from source, and Swagger UI is
unpacked at build time.

The dashboard is optional here. Skip `pnpm build` and the API works exactly the
same; the server just says there is no frontend to serve.

**Looking at `example-wiki/` writes nothing, and changing it breaks the docs.**
It is a fixture: its [`index.md`](example-wiki/index.md) states exactly what the
dashboard should report about it, and the knowledge base quotes those numbers.
Starting a timer, editing a page, reordering a contents list, and splitting or
merging a page all write to it. Try writes against a copy, or leave
`RHIZOLOG_ROOT` unset, which uses `backend/wiki/`: gitignored, and created on
first run. `git status example-wiki` says whether anything slipped through.

## Layout

This is one git repository. Scaffolding tools like to create nested ones. If a
generator leaves a `.git` inside `backend/` or `frontend/`, delete it, or the
root repository will treat that directory as opaque and stop tracking what is
inside it.

| Path | |
|---|---|
| `backend/` | The Rust library and the headless server (crate `rhizolog`) |
| `desktop/` | The Tauri app (crate and binary `rhizolog-desktop`) |
| `frontend/` | The dashboard: Vite, SolidJS, Tailwind, daisyUI |
| `site/` | The static product site at rhizolog.com: Astro, Tailwind |
| `example-wiki/` | A small wiki, a week of time, a word log and a rules file, to run against |
| `knowledge-base/` | Why the thing is built the way it is |
| `packaging/` | The scripts a release runs: the version check, the notices, the archives and the smoke test |
| `.github/workflows/` | CI on every push, and the release build, both run on the GitHub mirror |

`backend/` and `desktop/` are one cargo workspace, so there is a single
`Cargo.lock` and a single `target/`, both at the root.

## Backend

From `backend/`:

```
cargo run        # start the server
cargo test
cargo fmt
cargo clippy
```

One optional feature: `--features embed-assets` compiles `frontend/dist` into
the executable, so it can be run anywhere without a `dist/` beside it. It is
what the README's `cargo install` uses. It needs `pnpm build` to have happened
first, and it compiles six more tests, the ones covering the dashboard served
out of the binary:

```
cargo build --features embed-assets
cargo test --features embed-assets
```

A directory that exists still wins, so this changes nothing when you are working
in a checkout.

Cargo unifies features across a workspace, and the desktop crate enables that
one, so `cargo test --workspace` builds with it too and needs `pnpm build`
first. `cargo test -p rhizolog` is the server as it actually ships.

Stop the server before `cargo build` on Windows: a running `rhizolog.exe` is
locked, and the build fails with "Access is denied" rather than anything
informative.

## Desktop app

From `desktop/`:

```
cargo run                 # a window onto a server it starts itself
cargo build --release     # target/release/rhizolog-desktop.exe
```

It turns `embed-assets` on, so `pnpm build` has to have run before it will
build at all. The icons in `desktop/icons/` are placeholders.

It remembers which wiki it opened in `rhizolog.settings.json` beside the
executable, which in a checkout is `target/debug/`. Delete that file to get the
first-run folder picker back.

**Run `cargo build` before launching it to check a change.** `cargo check` and
`cargo clippy` link nothing, so launching after one of them runs the previous
binary, which looks exactly like the change having no effect.

The app does not reach into the stores. It starts a real server and talks to it
over HTTP, as a browser pointed at a remote Rhizolog would, and `AGENTS.md` names
the types that rules out. See [the desktop app](knowledge-base/desktop-app.md).

## Frontend

From `frontend/`:

```
pnpm dev         # dev server with HMR, proxying /api to the backend
pnpm build       # production build, which the backend serves
pnpm test
pnpm typecheck
```

`pnpm dev` expects a backend already running on port 3000 and proxies `/api`,
`/api-docs`, and `/swagger-ui` to it.

## Site

From `site/`:

```
pnpm dev         # dev server on :4321
pnpm build       # static output to site/dist
pnpm typecheck   # astro check
```

This one is independent of everything above: the backend does not serve it and
does not know it exists. `pnpm dev` daemonises: the command returns and the
server keeps running, so `pnpm exec astro dev status` is how you find out
whether one is up, and `pnpm exec astro dev stop` ends it. `pnpm typecheck`
reads types that `dev` and `build` generate, so on a fresh checkout run one of
those first.

Astro collects telemetry unless told not to. `pnpm exec astro telemetry disable`
turns it off for your machine; anywhere else, CI included, set
`ASTRO_TELEMETRY_DISABLED=1`.

## API types

The frontend's API types are generated from the OpenAPI document rather than
written by hand, so a backend change that breaks a caller becomes a type error
instead of a runtime surprise. After changing the API:

```
cd backend; cargo run --example dump-openapi
cd ../frontend; pnpm gen:api
```

No server needs to be running: the example writes the spec straight from the
compiled routes.

That exists because downloading it is a trap on Windows, and the obvious way is
the one that does not work. `curl` in PowerShell 5.1 is an alias for
`Invoke-WebRequest`, which decodes a body as Latin-1 when its `Content-Type`
carries no charset, and `application/json` from here carries none. Every
non-ASCII character in the spec comes back mangled, each of its bytes re-encoded
as two. The file stays valid JSON, stays one line, and the diff still reads like
an ordinary regeneration, so nothing catches it. `>` and `Out-File` are no
better; they re-encode too, and add a BOM.

If you do fetch it over HTTP, download bytes and write them verbatim:

```powershell
$data = (New-Object System.Net.WebClient).DownloadData("http://127.0.0.1:3000/api-docs/openapi.json")
[System.IO.File]::WriteAllBytes("$PWD\frontend\openapi.json", $data)
```

Worth checking after a refresh either way: the file should have no BOM, and it
should contain no `Ã` and no `â€`, which are what mangled UTF-8 looks like once
it has been read back as Latin-1.

## Windows

Development happens on Windows, in PowerShell, and three things about
Windows PowerShell 5.1 have each cost a real bug or a wrong diagnosis here:

- It has no `&&`; use `;` to chain.
- Do not round-trip a source file through `Get-Content` and `Set-Content`: 5.1
  reads as ANSI and writes UTF-8 with a BOM, which mangles every non-ASCII
  character in the file and adds a byte-order mark that has, in this project,
  already hidden a page's frontmatter once.
- Write a commit message to a file and use `git commit -F`. 5.1 strips double
  quotes from an argument on its way to a native command, so `git commit -m`
  silently loses every `"` in the message.

## Before you commit

`cargo fmt`, `cargo clippy` and `cargo test -p rhizolog` clean; `pnpm test` and
`pnpm typecheck` in `frontend/` if you touched it, and `pnpm typecheck` in
`site/` if you touched that. CI runs all of it and more on every push to the
GitHub mirror, but running it here first is quicker than waiting to find out.
Scope the subject by the area it changes (`backend:`, `ui:`, `site:`, `kb:`,
`repo:`), and write no em dashes anywhere: `AGENTS.md` says why, and which three
things are exempt.

## Cutting a release

Releases are built by GitHub Actions on the
[mirror](https://github.com/grindshell/rhizolog), and published by hand.
[Releases](knowledge-base/releases.md) says why it works this way.

1. **Set the version** in `[workspace.package]` in the root `Cargo.toml`, the
   only place it is written. Then `cargo check`, so `Cargo.lock` follows, and
   regenerate the API document and types, which carry the version: `cargo run
   --example dump-openapi` from `backend/`, then `pnpm gen:api` from
   `frontend/`. CI fails if you forget.
2. **Date the changelog.** `CHANGELOG.md` needs a section headed
   `## <version> - <yyyy-mm-dd>`. What is under the heading becomes the release
   notes, and the release workflow refuses a tag without it.
3. **Commit, push `master` to both remotes**, and wait for CI to pass on GitHub.
4. **Tag it and push the tag to both**:

   ```
   git tag v<version>
   git push origin v<version>
   git push github v<version>
   ```

5. **Publish the draft.** The Release workflow builds, packages and runs
   everything, then leaves a draft on the releases page. Download an archive or
   two, run them, and press Publish.

To try the process without spending a version, run the Release workflow by hand
from the Actions tab. It builds and checks everything and attaches the archives
to the run.

The same archives can be made locally once the programs are built in release,
which needs [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)
(`cargo install cargo-about --locked --features cli`):

```
pwsh packaging/package.ps1 -Binary server
pwsh packaging/package.ps1 -Binary desktop
pwsh packaging/smoke.ps1
```

They land in `dist/` at the root, which is gitignored.
