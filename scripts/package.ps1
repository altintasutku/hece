# Builds a Windows release zip: hece.exe, hecew.exe, hece-engine.exe and the install scripts.
#
#   powershell -File scripts/package.ps1 [-NoCuda]
#
# The CUDA runtime DLLs the engine needs (cudart, cuBLAS; ~600 MB, allowed by the CUDA EULA's
# redistribution list) are included, so the target machine needs only the NVIDIA driver.
# -NoCuda leaves them out; the target then needs the CUDA Toolkit runtime on PATH. The Whisper
# model is never bundled; scripts/prepare_model.py prepares it.
param([switch]$NoCuda)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

& powershell -NoProfile -File scripts\build-engine.ps1
if ($LASTEXITCODE -ne 0) { throw 'build failed' }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "hece-$version-windows-x64"
$stage = Join-Path $root "target\dist\$name"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($exe in 'hece.exe', 'hecew.exe', 'hece-engine.exe') {
    Copy-Item "target\release\$exe" $stage
}
Copy-Item scripts\windows\install.ps1, scripts\windows\uninstall.ps1, scripts\prepare_model.py, scripts\prepare_model.py.lock $stage
Copy-Item README.md, LICENSE $stage

if (-not $NoCuda) {
    $cuda = Get-ChildItem 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA' -Directory |
        Sort-Object Name -Descending | Select-Object -First 1
    foreach ($dll in 'cudart64_13.dll', 'cublas64_13.dll', 'cublasLt64_13.dll') {
        Copy-Item (Join-Path $cuda.FullName "bin\x64\$dll") $stage
    }
}

$zip = Join-Path $root "target\dist\$name.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
# The folder itself, so unpacking never mixes the release with other files.
Compress-Archive -Path $stage -DestinationPath $zip
Write-Output $zip
