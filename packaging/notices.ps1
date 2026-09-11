<#
.SYNOPSIS
    Writes THIRD-PARTY-NOTICES.txt for one of Rhizolog's two programs.

.DESCRIPTION
    Every licence the program's dependencies carry, and what carries it: the
    Rust crates it is compiled from, through cargo-about; the dashboard's npm
    packages; Swagger UI; SQLite; and for the desktop app, Microsoft's WebView2
    loader.

    Run it after the release build of the program, because Swagger UI's
    licence is read from the archive that build downloaded. It fails rather
    than write a notices file with something missing from it. See
    knowledge-base/releases.md.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [ValidateSet('server', 'desktop')] [string] $Binary,
    [Parameter(Mandatory)] [string] $OutFile
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = Split-Path $PSScriptRoot -Parent
$OutFile = [IO.Path]::GetFullPath($OutFile, (Get-Location).Path)
$version = & (Join-Path $PSScriptRoot 'version.ps1')
$text = [Text.StringBuilder]::new()

function Add-Text([string] $block = '') {
    [void] $text.Append(($block -replace "`r`n?", "`n").TrimEnd()).Append("`n")
}

function Add-Heading([string] $title) {
    Add-Text
    Add-Text ('=' * 80)
    Add-Text $title
    Add-Text ('=' * 80)
    Add-Text
}

function Read-ZipEntry([string] $path, [string] $pattern) {
    $archive = [IO.Compression.ZipFile]::OpenRead($path)
    try {
        $entry = $archive.Entries | Where-Object { $_.FullName -match $pattern } | Select-Object -First 1
        if (-not $entry) { return $null }
        $reader = [IO.StreamReader]::new($entry.Open())
        try { $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally {
        $archive.Dispose()
    }
}

$what = @{ server = 'the server'; desktop = 'the desktop app' }[$Binary]
$loader = if ($Binary -eq 'desktop') { ",`nand Microsoft's WebView2 loader" } else { '' }
Add-Text "Rhizolog $version, ${what}: third-party notices"
Add-Text
Add-Text @"
Rhizolog is Copyright (C) 2026 Tim Yuen. It is free software under the GNU
Affero General Public License, version 3 or later, which is LICENSE beside this
file. Its source is https://github.com/grindshell/rhizolog/tree/v$version.

It is built from other people's work as well, under the licences below: the
Rust crates it is compiled from, the JavaScript packages in its dashboard,
Swagger UI and SQLite$loader.
"@

# The Rust crates. The desktop app is a different crate graph from the server,
# which is why there is one notices file per program rather than one per
# release.
Add-Heading 'Rust crates'
Add-Text @"
Each licence is followed by the crates under it. A crate that offers a choice
of licences is listed under the one taken. The Rust standard library, compiled
into every Rust program, is under MIT or Apache-2.0 too; copyrights in it are
retained by its contributors.
"@
Add-Text
$manifest = Join-Path $root (@{ server = 'backend/Cargo.toml'; desktop = 'desktop/Cargo.toml' }[$Binary])
$features = if ($Binary -eq 'server') { @('--features', 'embed-assets') } else { @() }
# The platform this is packaging, so a Windows program's notices do not list
# crates only Linux compiles, or the other way round.
$target = if ($IsWindows) { 'x86_64-pc-windows-msvc' } else { 'x86_64-unknown-linux-gnu' }
$crates = Join-Path ([IO.Path]::GetTempPath()) "rhizolog-crates-$Binary.txt"
cargo about generate --locked --fail `
    --config (Join-Path $PSScriptRoot 'about.toml') `
    --manifest-path $manifest @features `
    --target $target `
    --output-file $crates `
    (Join-Path $PSScriptRoot 'about.hbs')
if ($LASTEXITCODE -ne 0) { throw 'cargo about could not account for every crate; its output above says which' }
Add-Text ([IO.File]::ReadAllText($crates))

# The dashboard, compiled into both programs. Each package's own licence file,
# because that is where its copyright line is.
Add-Heading 'The dashboard'
Add-Text 'JavaScript packages bundled into the dashboard this program serves.'
Add-Text
Push-Location (Join-Path $root 'frontend')
try { $json = (pnpm licenses list --prod --json) -join "`n" } finally { Pop-Location }
if ($LASTEXITCODE -ne 0) { throw 'pnpm licenses list failed; run pnpm install in frontend/ first' }
$packages = @(($json | ConvertFrom-Json -AsHashtable).Values | ForEach-Object { $_ } | Sort-Object { $_.name })
if ($packages.Count -eq 0) { throw 'pnpm listed no dashboard packages; run pnpm install in frontend/ first' }
foreach ($package in $packages) {
    $directory = @($package.paths)[0]
    $file = Get-ChildItem -LiteralPath $directory -File |
        Where-Object Name -Match '^(LICEN[CS]E|COPYING)([.-].*)?$' |
        Select-Object -First 1
    if (-not $file) { throw "$($package.name) has no licence file in $directory; read what it is under before shipping it" }
    Add-Text "$($package.name) $(@($package.versions) -join ', '), $($package.license)"
    Add-Text
    Add-Text ([IO.File]::ReadAllText($file.FullName))
    Add-Text
}

# Swagger UI's assets are downloaded and embedded by utoipa-swagger-ui's build
# script, so its licence and NOTICE are read out of the very archive that was
# embedded rather than written down here.
Add-Heading 'Swagger UI'
$zip = Get-ChildItem (Join-Path $root 'target/release/build') -Directory -Filter 'utoipa-swagger-ui-*' -ErrorAction SilentlyContinue |
    ForEach-Object { Get-ChildItem (Join-Path $_.FullName 'out') -File -Filter '*.zip' -ErrorAction SilentlyContinue } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $zip) { throw "no Swagger UI archive under target/release/build; build the $Binary in release first" }
$licence = Read-ZipEntry $zip.FullName '^swagger-ui-[^/]+/LICENSE$'
$notice = Read-ZipEntry $zip.FullName '^swagger-ui-[^/]+/NOTICE$'
if (-not $licence -or -not $notice) { throw "$($zip.Name) has no LICENSE or no NOTICE" }
Add-Text "Swagger UI $($zip.BaseName.TrimStart('v')), served at /swagger-ui, is under Apache-2.0. Its NOTICE, then its licence:"
Add-Text
Add-Text $notice
Add-Text
Add-Text $licence

Add-Heading 'SQLite'
Add-Text @"
SQLite is compiled in through the libsqlite3-sys crate. Its authors have
dedicated it to the public domain (https://sqlite.org/copyright.html), so it
asks for no notice; it is named here because it is here.
"@

if ($Binary -eq 'desktop') {
    # The SDK version was read by hand from webview2-com-sys's changelog, which
    # says 0.38.2 moved to it. Another crate version may carry another loader
    # under other terms, so this refuses to guess.
    $crate = '0.38.2'
    $sdk = '1.0.3650.58'
    $lock = [IO.File]::ReadAllText((Join-Path $root 'Cargo.lock'))
    $locked = [regex]::Match($lock, '(?m)^name = "webview2-com-sys"\r?\nversion = "(?<version>[^"]+)"')
    if (-not $locked.Success) {
        throw 'Cargo.lock has no webview2-com-sys, so the desktop app no longer links the loader and this section should go'
    }
    if ($locked.Groups['version'].Value -ne $crate) {
        throw ("Cargo.lock has webview2-com-sys $($locked.Groups['version'].Value), and the SDK version in " +
            "packaging/notices.ps1 was read from $crate's changelog. Read the new one's, update both, and check " +
            'the licence still says what knowledge-base/dependency-licences.md says it does.')
    }
    $package = Join-Path ([IO.Path]::GetTempPath()) "microsoft.web.webview2.$sdk.nupkg"
    if (-not (Test-Path $package)) {
        Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/microsoft.web.webview2/$sdk/microsoft.web.webview2.$sdk.nupkg" -OutFile $package
    }
    $licence = Read-ZipEntry $package '^LICENSE\.txt$'
    $notice = Read-ZipEntry $package '^NOTICE\.txt$'
    if (-not $licence -or -not $notice) { throw "the Microsoft.Web.WebView2 $sdk package has no LICENSE.txt or no NOTICE.txt" }

    Add-Heading 'Microsoft Edge WebView2 loader'
    Add-Text @"
WebView2LoaderStatic.lib from the Microsoft.Web.WebView2 SDK $sdk, linked in
through the webview2-com-sys crate. Rhizolog's licence carries an additional
permission for it, which README.txt beside this file states. The SDK's licence,
then the notice file it ships with:
"@
    Add-Text
    Add-Text $licence
    Add-Text
    Add-Text $notice
}

[IO.File]::WriteAllText($OutFile, $text.ToString(), [Text.UTF8Encoding]::new($false))
