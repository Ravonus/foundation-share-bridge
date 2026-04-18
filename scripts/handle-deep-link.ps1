Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string]$DeepLink
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$runtimeDir = [System.IO.Path]::GetFullPath((Join-Path $scriptDir ".."))
$binaryPath = Join-Path $runtimeDir "bin\foundation-share-bridge.exe"
$bridgeUrl = "http://127.0.0.1:43128"

if (-not (Test-Path $binaryPath)) {
  throw "Bridge binary was not found at $binaryPath"
}

try {
  Start-ScheduledTask -TaskName "FoundationShareBridge" -ErrorAction SilentlyContinue | Out-Null
}
catch {
}

& $binaryPath handle-url $DeepLink
$exitCode = $LASTEXITCODE

if ($exitCode -eq 0) {
  Start-Process "$bridgeUrl/?linked=1" | Out-Null
  exit 0
}

Start-Process "$bridgeUrl/" | Out-Null
exit $exitCode
