# Dependency licences

Rhizolog is AGPL-3.0-or-later, for the reasons in
[Product vision](product-vision.md). The AGPL is strong copyleft, so a
dependency under terms it cannot be combined with would be a real problem rather
than a paperwork one. This page is the check, done by hand on 11 September 2026
before the repository was published in full for the first time, and what it
found.

**Nothing in either tree is incompatible, and the source can be published as it
is.** Three things are owed or open, and none of them is about the source:

1. A binary release owes its dependencies' notices, and nothing collects them.
2. The desktop app statically links a Microsoft binary that has no source, and
   now has a section 7 permission that names it.
3. The product site served its fonts without their licence text, which
   `site/public/OFL.txt` now carries.

## How it was checked

Nothing had to be installed, which is what made it a job of minutes:

- **Rust.** `cargo metadata --format-version 1 --locked` lists every package in
  the lock file with the `license` its manifest declares, and its `resolve`
  graph says how each one is reached. It was run twice: across every platform,
  which is the conservative answer and needed a `cargo fetch` first for crates
  only other platforms use, and with `--filter-platform x86_64-pc-windows-msvc`,
  which is what a Windows build compiles. Each package was classed by how a
  workspace member reaches it: through normal dependencies, meaning linked into
  a binary (proc-macros count, being ordinary dependencies that run at compile
  time), through a build dependency, or only through a dev-dependency.
- **npm.** `pnpm licenses list --prod --json` and `--dev --json`, in
  `frontend/` and in `site/` separately, since they have separate lock files.
- **The repository itself.** Nothing is vendored. The only binary or asset files
  in git are the two favicons and the generated placeholder icons in
  `desktop/icons/`, all of them this project's own, and the lock files name
  dependencies without containing any of their code.

`cargo-deny` is the tool for CI once there is CI. It does the same check against
an allowlist, and fails the build when a new dependency brings a licence nobody
has looked at, which is the failure a check done once by hand cannot catch.

## The Rust tree

549 crates across every platform, 351 of them in a Windows build. Every one
declares a licence, and every expression is permissive or offers a permissive
choice, apart from five crates under MPL-2.0.

| Expression | Crates |
|---|---|
| `MIT OR Apache-2.0`, in either order | 325 |
| `MIT` | 143 |
| Other choices that include MIT or Apache-2.0 | 39 |
| `Unicode-3.0` | 18 |
| `Apache-2.0`, `BSD-3-Clause`, `ISC`, `Zlib`, `BSD-2-Clause`, `CC0-1.0` or `Apache-2.0 WITH LLVM-exception` alone | 14 |
| `MPL-2.0` | 5 |
| Two permissive licences at once | 5 |

**The five conjunctions** are `brotli` and `matchit` (BSD-3-Clause and MIT),
`dpi` (Apache-2.0 and MIT), `unicode-ident` (MIT or Apache-2.0, and Unicode-3.0)
and `finl_unicode` (MIT or Apache-2.0, and Unicode-DFS-2016). "And" means both
apply at once, usually to different parts of the crate, and every licence named
is permissive and compatible with the GPL family.

**LGPL-2.1-or-later appears only as a third choice**, beside MIT and Apache-2.0,
in two crates. Take MIT.

**The five MPL-2.0 crates** are `cssparser`, `cssparser-macros`, `dtoa-short`,
`selectors` and `option-ext`. The MPL is copyleft by file, and it combines with
the GPL family through its "secondary licenses" clause unless a file opts out
with the notice in its Exhibit B. None of these does. That phrase is in their
`LICENSE` files, because the MPL's own text carries Exhibit B as a template, and
in none of their source files, which is where an opt-out has to be written. So
they combine with AGPL code, and the one obligation they add is that modified
versions of those files stay under the MPL. All five belong to the desktop app,
through Tauri. The headless server's graph, `cargo tree -p rhizolog -e
normal,build`, contains none of them.

### Compiled in, but not as a crate

- **SQLite**, compiled from the C source `libsqlite3-sys` bundles. Public
  domain.
- **Swagger UI**, whose built assets `utoipa-swagger-ui` downloads and unpacks
  at build time and embeds in the server. Apache-2.0.
- **The dashboard**, when `embed-assets` is on, which it is in the desktop app
  and in the README's `cargo install`. What it contains is the npm section
  below.
- **Microsoft's WebView2 loader**, in the desktop app only, which has a section
  of its own below.

## The npm trees

**The dashboard ships five packages, all MIT:** Solid, its router, and what
those two depend on. Everything else under `frontend/` is a build tool: 185
packages under MIT, ISC, Apache-2.0, the BSD licences, BlueOak-1.0.0, CC0-1.0,
MIT-0, Python-2.0, CC-BY-4.0 and MPL-2.0 (`lightningcss`). None of them reaches
the bundle except through the CSS Tailwind and daisyUI generate, which is MIT.

**The site ships no JavaScript at all.** Its dependencies are build tools, and
two things of theirs end up in `site/dist`: the CSS Tailwind generates, which is
MIT, and the IBM Plex fonts, which are OFL-1.1. The only licence in its tree that
is not permissive is `@img/sharp-win32-x64`, the native half of the image
library Astro uses, whose bundled libvips is LGPL-3.0-or-later. It runs at build
time if at all, and nothing of it reaches the output.

## What is owed

### A binary owes its dependencies' notices

Publishing the source asks nothing of the dependencies, because the repository
holds none of their code. A compiled copy does hold it, and MIT, the BSD licences
and Apache-2.0 all ask for their copyright notice and licence text to travel with
copies in binary form; Apache-2.0 asks for any NOTICE file as well. The server
contains a few hundred crates, Swagger UI and the dashboard's five packages, and
the desktop app more than that. Nothing gathers their notices today.

`cargo-about` generates the Rust half from the same metadata used here, and the
npm half is small enough for `pnpm licenses list`. It belongs to the release
process, which does not exist yet either; both are in [`TODO.md`](../TODO.md).

### The desktop app links a binary that has no source

On the MSVC target `webview2-com-sys` declares the loader's functions with
`link(name = "WebView2LoaderStatic", kind = "static")`, so
`WebView2LoaderStatic.lib` is linked into `rhizolog-desktop.exe`; the crate
carries it prebuilt for x64, x86 and arm64. It comes from Microsoft's WebView2
SDK. The crate itself is MIT and carries no licence file for those binaries, so
the SDK's own terms were not read here; they are in the `Microsoft.Web.WebView2`
NuGet package.

The AGPL makes whoever conveys object code offer the corresponding source for
all of it, except System Libraries, which section 1 defines narrowly. Whether a
loader that comes with an SDK rather than with the operating system is one is
arguable. It constrains nobody but a third party, since the copyright holder can
distribute their own code linked with anything they like. Somebody
redistributing a modified desktop app is who it could catch, and the usual
answer is an additional permission under section 7 that names the loader. The
headless server does not link the loader, and every Tauri app on Windows is in
the same position.

**The permission was granted on 11 September 2026**, and its text is in the
README's Licence section. It follows the shape of the FSF's own template for
linking with a library the GPL cannot absorb, without that template's optional
sentence requiring the library's source, since there is none to include. It
names both forms of the loader, the static library the MSVC target links and the
DLL other targets link, and nothing else, so it cannot be read as a licence to
combine Rhizolog with anything a future dependency happens to bring. Section 7
lets anybody passing on a copy remove it. The `license` field in the manifests
stays `AGPL-3.0-or-later`: an additional permission only loosens the licence,
and SPDX has no identifier for this one.

### The fonts' licence text was not served with them

IBM Plex Sans and IBM Plex Mono are under the SIL Open Font License 1.1, which
asks every copy to include IBM's copyright and the licence, either as a file
alongside or in the font's own metadata, and serving a webfont is distributing
it. The name tables of the `.woff` files `@fontsource` builds carry the
copyright (name ID 0) and the licence's URL (name ID 14), and not the licence
text (name ID 13), which subsetting dropped. Only the `.woff` files were read;
the `.woff2` ones are Brotli-compressed and come from the same subsetter.

`site/public/OFL.txt` fixes it, added on 11 September 2026: the two
`@fontsource` packages' `LICENSE` files, byte for byte, one after the other,
since each carries its own family's copyright lines above the same licence. It
is served at `/OFL.txt`, a stand-alone text file beside the fonts, which is the
first of the three places the OFL allows. It concerns the site alone: the
dashboard uses none of these fonts. A font family added to the site later needs
its own licence added to the same file.

## Doing it again

```
cargo fetch --locked
cargo metadata --format-version 1 --locked
cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
cd frontend; pnpm licenses list --prod --json; pnpm licenses list --dev --json
cd ../site; pnpm licenses list --prod --json; pnpm licenses list --dev --json
```

The `license` field of each package in the first output is the answer for it,
and an expression with `AND` or parentheses in it wants reading rather than
matching. For any MPL-2.0 crate, search its source files, not its `LICENSE`, for
"Incompatible With Secondary Licenses". The desktop app is where anything new is
most likely to arrive, because Tauri brings most of the tree.
