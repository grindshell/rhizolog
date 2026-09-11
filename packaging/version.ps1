<#
.SYNOPSIS
    The release version, and the checks a tag has to pass to become one.

.DESCRIPTION
    Prints the version from the root Cargo.toml, which both crates inherit.

    With -Tag, fails unless the tag is `v` and that version, and unless
    CHANGELOG.md has a dated section for it: a release cannot go out without
    its notes. With -NotesOut, writes that section to a file as the release
    notes; a dry run, which has no dated section yet, gets a line saying so.

    The release workflow runs it, and so can anybody. See
    knowledge-base/releases.md.
#>
[CmdletBinding()]
param(
    [string] $Tag,
    [string] $NotesOut
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent

$metadata = cargo metadata --no-deps --format-version 1 --locked --manifest-path (Join-Path $root 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$versions = @((($metadata -join "`n") | ConvertFrom-Json).packages.version | Sort-Object -Unique)
if ($versions.Count -ne 1) {
    throw "the crates disagree about the version ($($versions -join ', ')); both should say version.workspace = true"
}
$version = $versions[0]

$changelog = [IO.File]::ReadAllText((Join-Path $root 'CHANGELOG.md'))
$pattern = '(?ms)^## ' + [regex]::Escape($version) +
    ' - (?<date>\d{4}-\d{2}-\d{2})[ \t]*\r?\n(?<notes>.*?)(?=^## |\z)'
$section = [regex]::Match($changelog, $pattern)

if ($Tag) {
    if ($Tag -ne "v$version") {
        throw "the tag is $Tag and Cargo.toml says $version; this release would be tagged v$version"
    }
    if (-not $section.Success) {
        throw "CHANGELOG.md has no section headed '## $version - <date>'; date it before tagging"
    }
}

if ($NotesOut) {
    $notes = if ($section.Success) {
        $section.Groups['notes'].Value.Trim()
    } else {
        "A dry run of $version. CHANGELOG.md has no dated section for it yet."
    }
    $notes += "`n`nEvery archive holds LICENSE and THIRD-PARTY-NOTICES.txt beside the program, " +
        "and SHA256SUMS.txt has the checksum of each archive. The source is the tag v$version.`n"
    $path = [IO.Path]::GetFullPath($NotesOut, (Get-Location).Path)
    [IO.File]::WriteAllText($path, $notes, [Text.UTF8Encoding]::new($false))
}

$version
