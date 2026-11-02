# Removes Hece: autostart, Start menu shortcut and binaries. Config, history and logs stay unless
# -Purge is given (they are under %APPDATA%\hece and %LOCALAPPDATA%\hece). API keys stay in the
# Windows Credential Manager until removed with `hece key delete <provider>`.
#
#   powershell -ExecutionPolicy Bypass -File uninstall.ps1 [-Purge]
param([switch]$Purge)
$ErrorActionPreference = 'Stop'
$target = Join-Path $env:LOCALAPPDATA 'Programs\Hece'
$hece = Join-Path $target 'hece.exe'

if (Test-Path $hece) { & $hece autostart off }
Get-Process hecew, hece, hece-engine -ErrorAction SilentlyContinue | Stop-Process -Force
Remove-Item -Force (Join-Path ([Environment]::GetFolderPath('Programs')) 'Hece.lnk') -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $target -ErrorAction SilentlyContinue
if ($Purge) {
    Remove-Item -Recurse -Force (Join-Path $env:APPDATA 'hece'), (Join-Path $env:LOCALAPPDATA 'hece') -ErrorAction SilentlyContinue
}
Write-Output 'Hece removed.'
