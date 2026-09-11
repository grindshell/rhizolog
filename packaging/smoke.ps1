<#
.SYNOPSIS
    Runs what was packaged: every server archive in dist/, from an empty
    directory, against a one-page wiki.

.DESCRIPTION
    Checks each archive carries LICENSE, THIRD-PARTY-NOTICES.txt and
    README.txt. Then, for each server, unpacks it somewhere with no checkout
    beside it, starts it, and checks it reports the release's version, serves
    the dashboard it was built with, serves Swagger UI, and puts the version in
    its OpenAPI document too.

    That is the one thing no `cargo test` can say: rust-embed reads from disk
    in debug builds, so a test passing there says nothing about what a shipped
    program serves. The desktop app's archive is checked for its files only;
    CI has no display to open a window on. See knowledge-base/releases.md.
#>
[CmdletBinding()]
param([string] $Path)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path $PSScriptRoot -Parent
$Path = if ($Path) { [IO.Path]::GetFullPath($Path, (Get-Location).Path) } else { Join-Path $root 'dist' }
$version = & (Join-Path $PSScriptRoot 'version.ps1')

$archives = @(Get-ChildItem $Path -File | Where-Object { $_.Name -match '\.(zip|tar\.gz)$' })
if ($archives.Count -eq 0) { throw "no archives in $Path" }

foreach ($archive in $archives) {
    $work = Join-Path ([IO.Path]::GetTempPath()) ('rhizolog-smoke-' + [guid]::NewGuid().ToString('n'))
    New-Item -ItemType Directory $work | Out-Null
    if ($archive.Name.EndsWith('.zip')) {
        Expand-Archive $archive.FullName $work
    } else {
        tar -xzf $archive.FullName -C $work
        if ($LASTEXITCODE -ne 0) { throw "could not unpack $($archive.Name)" }
    }
    $unpacked = @(Get-ChildItem $work -Directory)
    if ($unpacked.Count -ne 1) { throw "$($archive.Name) should hold exactly one directory" }
    $directory = $unpacked[0].FullName
    foreach ($required in 'LICENSE', 'THIRD-PARTY-NOTICES.txt', 'README.txt') {
        if (-not (Test-Path (Join-Path $directory $required))) { throw "$($archive.Name) has no $required" }
    }

    $server = Join-Path $directory ('rhizolog' + $(if ($IsWindows) { '.exe' } else { '' }))
    if (-not (Test-Path $server)) {
        "$($archive.Name): files present; not a server, so not started"
        continue
    }

    $wiki = Join-Path $work 'wiki'
    New-Item -ItemType Directory $wiki | Out-Null
    [IO.File]::WriteAllText((Join-Path $wiki 'hello.md'), "# Hello`n`nA page with a [[wanted]] link.`n")
    $port = 38000 + (Get-Random -Maximum 1000)
    $base = "http://127.0.0.1:$port"
    $env:RHIZOLOG_ROOT = $wiki
    $env:RHIZOLOG_ADDR = "127.0.0.1:$port"
    $log = Join-Path $work 'server.log'
    $process = Start-Process $server -WorkingDirectory $work -PassThru `
        -RedirectStandardError $log -RedirectStandardOutput (Join-Path $work 'server.out')
    try {
        $ready = Join-Path $wiki '.rhizolog/server.json'
        $deadline = (Get-Date).AddSeconds(60)
        while (-not (Test-Path $ready)) {
            if ($process.HasExited) { throw "the server exited with $($process.ExitCode) before it was ready" }
            if ((Get-Date) -gt $deadline) { throw 'the server was not ready within a minute' }
            Start-Sleep -Milliseconds 250
        }

        $health = Invoke-RestMethod "$base/api/health"
        if ($health.version -ne $version) { throw "/api/health says $($health.version), not $version" }
        if ($health.pages -ne 1) { throw "/api/health counts $($health.pages) pages, not the one written" }

        $dashboard = Invoke-WebRequest "$base/" -SkipHttpErrorCheck
        $type = @($dashboard.Headers['Content-Type']) -join ';'
        if ($dashboard.StatusCode -ne 200 -or $type -notmatch 'text/html') {
            throw "/ answered $($dashboard.StatusCode) $type, not the dashboard"
        }

        $swagger = Invoke-WebRequest "$base/swagger-ui/" -SkipHttpErrorCheck
        if ($swagger.StatusCode -ne 200) { throw "/swagger-ui/ answered $($swagger.StatusCode)" }

        $document = Invoke-RestMethod "$base/api-docs/openapi.json"
        if ($document.info.version -ne $version) { throw "the OpenAPI document says $($document.info.version), not $version" }

        "$($archive.Name): $version, the dashboard, Swagger UI and the OpenAPI document, all served"
    } catch {
        Write-Host '--- the server said:'
        if (Test-Path $log) { Get-Content $log | Write-Host }
        throw
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
        Remove-Item Env:RHIZOLOG_ROOT, Env:RHIZOLOG_ADDR -ErrorAction SilentlyContinue
    }
}
