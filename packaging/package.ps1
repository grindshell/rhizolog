<#
.SYNOPSIS
    Packs one of Rhizolog's release builds into the archive a release carries.

.DESCRIPTION
    The program, LICENSE, THIRD-PARTY-NOTICES.txt and a README.txt, in one
    directory named for the archive: a zip on Windows, a tar.gz on Linux, in
    dist/ at the root of the checkout. Prints the archive's path.

    It builds nothing. Run it after the release build, as the release workflow
    does:

        cargo build --release --locked -p rhizolog --features embed-assets
        cargo build --release --locked -p rhizolog-desktop

    It needs cargo-about, for the notices. See knowledge-base/releases.md.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [ValidateSet('server', 'desktop')] [string] $Binary,
    [string] $OutDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
$OutDir = if ($OutDir) { [IO.Path]::GetFullPath($OutDir, (Get-Location).Path) } else { Join-Path $root 'dist' }
$version = & (Join-Path $PSScriptRoot 'version.ps1')

if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') { throw 'only x64 is released' }
$target = if ($IsWindows) { 'x86_64-pc-windows-msvc' }
    elseif ($IsLinux) { 'x86_64-unknown-linux-gnu' }
    else { throw 'Rhizolog is released for Windows and Linux' }
if ($Binary -eq 'desktop' -and -not $IsWindows) { throw 'the desktop app is released for Windows only' }

$program = @{ server = 'rhizolog'; desktop = 'rhizolog-desktop' }[$Binary]
$file = $program + $(if ($IsWindows) { '.exe' } else { '' })
$built = Join-Path $root "target/release/$file"
if (-not (Test-Path $built)) { throw "there is no $built; build it in release first" }

$name = "$program-$version-$target"
$stage = Join-Path $OutDir "stage/$name"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item $built $stage
Copy-Item (Join-Path $root 'LICENSE') $stage
& (Join-Path $PSScriptRoot 'notices.ps1') -Binary $Binary -OutFile (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')
$readme = [IO.File]::ReadAllText((Join-Path $PSScriptRoot "$Binary-README.txt")).Replace('{version}', $version)
[IO.File]::WriteAllText((Join-Path $stage 'README.txt'), $readme, [Text.UTF8Encoding]::new($false))

if ($IsWindows) {
    $archive = Join-Path $OutDir "$name.zip"
    if (Test-Path $archive) { Remove-Item $archive }
    Compress-Archive -Path $stage -DestinationPath $archive
} else {
    # tar rather than a zip, because it keeps the executable bit.
    $archive = Join-Path $OutDir "$name.tar.gz"
    tar -czf $archive -C (Split-Path $stage) $name
    if ($LASTEXITCODE -ne 0) { throw 'tar failed' }
}
$archive
