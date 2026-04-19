Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$installer = Join-Path $scriptDir "uninstall-windows-service.ps1"

if (-not (Test-Path $installer)) {
  throw "Windows uninstaller not found at $installer"
}

& $installer @args
