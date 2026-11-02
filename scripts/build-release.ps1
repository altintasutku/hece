# Builds the Windows installer: target\dist\Hece_<version>_x64-setup.exe and SHA256SUMS.txt.
#
#   powershell -File scripts/build-release.ps1
#
# Used locally and by .github/workflows/release.yml. Needs the CUDA Toolkit 13.x, and cmake, rust
# and bun from mise.toml. Steps:
#   1. hece, hecew and the CUDA engine (scripts/build-engine.ps1, +crt-static).
#   2. onnxruntime.dll (Silero VAD) from Microsoft's release, pinned by sha256.
#   3. The settings app and the NSIS installer (Tauri), which ships 1 and 2 next to hece-settings.exe.
# The CUDA runtime DLLs and Whisper models are not bundled; Hece downloads them on demand.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$ortVersion = '1.31.1'
$ortSha256 = '5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c'
$triple = 'x86_64-pc-windows-msvc'

& powershell -NoProfile -File scripts\build-engine.ps1
if ($LASTEXITCODE -ne 0) { throw 'engine build failed' }

# Tauri ships externalBin files named <name>-<target triple>.exe as <name>.exe.
$bins = Join-Path $root 'apps\settings\src-tauri\binaries'
New-Item -ItemType Directory -Force $bins | Out-Null
foreach ($name in 'hece', 'hecew', 'hece-engine') {
    Copy-Item "target\release\$name.exe" (Join-Path $bins "$name-$triple.exe") -Force
}

$ortDir = Join-Path $root 'target\onnxruntime'
$ortZip = Join-Path $ortDir "onnxruntime-win-x64-$ortVersion.zip"
New-Item -ItemType Directory -Force $ortDir | Out-Null
if (-not (Test-Path $ortZip)) {
    Invoke-WebRequest -UseBasicParsing -OutFile $ortZip `
        "https://github.com/microsoft/onnxruntime/releases/download/v$ortVersion/onnxruntime-win-x64-$ortVersion.zip"
}
$actual = (Get-FileHash -Algorithm SHA256 $ortZip).Hash.ToLowerInvariant()
if ($actual -ne $ortSha256) { Remove-Item $ortZip; throw "onnxruntime checksum mismatch: $actual" }
Expand-Archive -Force $ortZip $ortDir
$ortLib = Join-Path $ortDir "onnxruntime-win-x64-$ortVersion"
Copy-Item (Join-Path $ortLib 'lib\onnxruntime.dll') $bins -Force
Copy-Item (Join-Path $ortLib 'LICENSE') (Join-Path $bins 'onnxruntime-LICENSE.txt') -Force
Copy-Item (Join-Path $ortLib 'ThirdPartyNotices.txt') (Join-Path $bins 'onnxruntime-ThirdPartyNotices.txt') -Force

Push-Location apps\settings
try {
    & mise x -- bun install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'bun install failed' }
    & mise x -- bunx tauri build --bundles nsis --config src-tauri\tauri.release.conf.json
    if ($LASTEXITCODE -ne 0) { throw 'installer build failed' }
} finally {
    Pop-Location
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$dist = Join-Path $root 'target\dist'
Remove-Item -Recurse -Force $dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $dist | Out-Null
$setup = Get-ChildItem 'target\release\bundle\nsis' -Filter "*_${version}_x64-setup.exe" | Select-Object -First 1
if (-not $setup) { throw 'installer not found' }
$out = Join-Path $dist "Hece_${version}_x64-setup.exe"
Copy-Item $setup.FullName $out
$hash = (Get-FileHash -Algorithm SHA256 $out).Hash.ToLowerInvariant()
Set-Content -Encoding ascii (Join-Path $dist 'SHA256SUMS.txt') "$hash  $(Split-Path -Leaf $out)"
Write-Output $out
