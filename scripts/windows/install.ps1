# Installs Hece for the current user (no administrator rights):
#   %LOCALAPPDATA%\Programs\Hece   the binaries
#   Start menu shortcut "Hece"     starts hecew.exe (no console window)
#   autostart at login             hece autostart on
#
#   powershell -ExecutionPolicy Bypass -File install.ps1 [-NoAutostart]
param([switch]$NoAutostart)
$ErrorActionPreference = 'Stop'
$source = $PSScriptRoot
$target = Join-Path $env:LOCALAPPDATA 'Programs\Hece'

Get-Process hecew, hece -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force $target | Out-Null
Get-ChildItem $source -File | Where-Object { $_.Extension -in '.exe', '.dll', '.py', '.md', '.ps1' -or $_.Name -eq 'LICENSE' } |
    Copy-Item -Destination $target -Force

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Hece.lnk'))
$link.TargetPath = Join-Path $target 'hecew.exe'
$link.WorkingDirectory = $target
$link.Description = 'Hece voice dictation'
$link.Save()

$hece = Join-Path $target 'hece.exe'
if (-not $NoAutostart) { & $hece autostart on }
$config = & $hece config

$cudaOnPath = ($env:PATH -split ';' | Where-Object { $_ -and (Test-Path (Join-Path $_ 'cublas64_13.dll')) })
if (-not (Test-Path (Join-Path $target 'cublas64_13.dll')) -and -not $cudaOnPath) {
    Write-Warning 'CUDA runtime (cublas64_13.dll) not found: the local engine will not start. Install the CUDA Toolkit 13.x or use a cloud provider.'
}
Write-Output "Installed to $target"
$models = Join-Path $env:LOCALAPPDATA 'hece\models\large-v3-turkish'
Write-Output "Next: prepare the model (uv run `"$target\prepare_model.py`" `"$models`"), set [provider] model_dir = '$models' in $config, then start Hece from the Start menu."
