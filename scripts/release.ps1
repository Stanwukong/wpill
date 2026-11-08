# Builds Cay's installer, fills the release notes with its SHA-256 and creates a DRAFT pre-release on GitHub.
# It never publishes: read the draft on GitHub and publish it yourself.
#
# Usage: .\scripts\release.ps1 -Version 1.0.0 [-WhatsNew notes\whats-new.md] [-SkipBuild]
#   -Version    the version in package.json, src-tauri/tauri.conf.json and src-tauri/Cargo.toml (no "v")
#   -WhatsNew   a Markdown file with the "What's new" section; without it the draft gets a placeholder to fill in
#   -SkipBuild  reuse the installer already in src-tauri/target/release/bundle/nsis
# Needs a clean working tree, HEAD pushed to origin, and `gh auth login`.

param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$WhatsNew,
    [switch]$SkipBuild
)

# Not 'Stop': in Windows PowerShell 5.1 that turns a native command's stderr into a terminating error.
# Every native call below checks $LASTEXITCODE instead.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Fail($message) {
    Write-Host "release: $message" -ForegroundColor Red
    exit 1
}

if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { Fail "'$Version' isn't a version like 1.0.0" }
$tag = "v$Version"

# 1. Clean tree, HEAD on origin.
if (git status --porcelain) { Fail 'the working tree has uncommitted changes; commit or stash them first' }
$head = (git rev-parse HEAD).Trim()
git fetch --quiet origin
if ($LASTEXITCODE -ne 0) { Fail 'git fetch origin failed' }
if (-not (git branch -r --contains $head)) { Fail "HEAD ($head) isn't on origin yet; push it first" }

# 2. The tag and the release must not exist yet.
git rev-parse -q --verify "refs/tags/$tag" *> $null
if ($LASTEXITCODE -eq 0) { Fail "tag $tag already exists locally" }
if (git ls-remote --tags origin "refs/tags/$tag") { Fail "tag $tag already exists on origin" }
gh release view $tag *> $null
if ($LASTEXITCODE -eq 0) { Fail "a release (or draft) named $tag already exists" }

# 3. One version everywhere.
$versions = [ordered]@{
    'package.json'              = (Get-Content -Raw package.json | ConvertFrom-Json).version
    'src-tauri/tauri.conf.json' = (Get-Content -Raw src-tauri\tauri.conf.json | ConvertFrom-Json).version
    'src-tauri/Cargo.toml'      = ([regex]::Match((Get-Content -Raw src-tauri\Cargo.toml), '(?m)^version\s*=\s*"([^"]+)"')).Groups[1].Value
}
$wrong = @($versions.GetEnumerator() | Where-Object { $_.Value -ne $Version })
if ($wrong.Count -gt 0) {
    $list = ($versions.GetEnumerator() | ForEach-Object { "$($_.Key) = $($_.Value)" }) -join ', '
    Fail "the manifests don't all say $Version ($list)"
}

# 4. Build.
if (-not $SkipBuild) {
    Write-Host 'release: pnpm tauri build (about 11 minutes cold)...'
    pnpm tauri build
    if ($LASTEXITCODE -ne 0) { Fail 'pnpm tauri build failed' }
}

$nsis = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$installer = Join-Path $nsis "Cay_${Version}_x64-setup.exe"
if (-not (Test-Path $installer)) { Fail "no installer at $installer" }

# 5. Checksum and notes.
$sha = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
$utf8 = New-Object System.Text.UTF8Encoding($false)
$template = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'release-notes.md'), $utf8)

if ($WhatsNew) {
    if (-not (Test-Path $WhatsNew)) { Fail "no file at $WhatsNew" }
    $news = [IO.File]::ReadAllText((Resolve-Path $WhatsNew), $utf8).Trim()
} else {
    $news = "## What's new`n`nTODO: write what's new before publishing."
    Write-Host "release: no -WhatsNew file; the draft has a TODO to fill in" -ForegroundColor Yellow
}

$notes = $template.Replace('{{WHATS_NEW}}', $news).Replace('{{VERSION}}', $Version).Replace('{{SHA256}}', $sha)
$notesPath = Join-Path $nsis "Cay_${Version}_release-notes.md"
[IO.File]::WriteAllText($notesPath, $notes, $utf8)

Write-Host "release: installer $installer"
Write-Host "release: SHA-256   $sha"
Write-Host "release: notes     $notesPath"

# 6. Draft pre-release. The tag is created on HEAD when you publish it.
$url = gh release create $tag $installer --prerelease --draft --target $head --title "Cay $Version" --notes-file $notesPath
if ($LASTEXITCODE -ne 0) { Fail 'gh release create failed' }
Write-Host "release: draft created, read it and publish it yourself:" -ForegroundColor Green
Write-Host $url
