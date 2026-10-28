# Verifies the toolchain needed to build TopBar (Tauri 2 on Windows).
# Usage: .\scripts\check-env.ps1   (exit code 1 if anything required is missing)

$ok = $true

function Report($name, $value, $required = $true) {
    if ($value) {
        Write-Host ("[ OK ] {0,-22} {1}" -f $name, $value) -ForegroundColor Green
    } elseif ($required) {
        Write-Host ("[MISS] {0,-22} required" -f $name) -ForegroundColor Red
        $script:ok = $false
    } else {
        Write-Host ("[ -- ] {0,-22} optional, not found" -f $name) -ForegroundColor Yellow
    }
}

function TryRun([scriptblock]$cmd) {
    try { (& $cmd 2>$null | Select-Object -First 1) } catch { $null }
}

Report 'rustc'  (TryRun { rustc --version })
Report 'cargo'  (TryRun { cargo --version })
Report 'rustup toolchain' (TryRun { rustup show active-toolchain })
Report 'node'   (TryRun { node --version })
Report 'pnpm'   (TryRun { pnpm --version })

# MSVC C++ toolset (Build Tools or full VS with the VC.Tools component)
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$msvc = $null
if (Test-Path $vswhere) {
    $msvc = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName
}
Report 'MSVC C++ build tools' $msvc

$sdkDir = "${env:ProgramFiles(x86)}\Windows Kits\10\Include"
$sdk = if (Test-Path $sdkDir) { (Get-ChildItem $sdkDir | Sort-Object Name -Descending | Select-Object -First 1).Name }
Report 'Windows SDK' $sdk

$wv2 = $null
foreach ($k in @(
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
    'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')) {
    try { $wv2 = (Get-ItemProperty $k -ErrorAction Stop).pv; break } catch {}
}
Report 'WebView2 Runtime' $wv2

if (-not $ok) {
    Write-Host "`nMissing requirements. See the Environment section in PLAN.md for install commands." -ForegroundColor Red
    exit 1
}
Write-Host "`nEnvironment ready." -ForegroundColor Green
