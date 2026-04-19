Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$serviceName = "Foundation Share Bridge"
$taskName = "FoundationShareBridge"
$runtimeDir = Join-Path $env:LOCALAPPDATA "FoundationShareBridge"
$composeFile = Join-Path $runtimeDir "docker-compose.yml"
$protocolKey = "HKCU:\Software\Classes\foundationsharebridge"
$purgeData = $false

foreach ($arg in $args) {
  if ($arg -eq "--purge-data") {
    $purgeData = $true
  }
}

Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
Remove-Item $protocolKey -Recurse -Force -ErrorAction SilentlyContinue

if ((Get-Command docker -ErrorAction SilentlyContinue) -and (Test-Path $composeFile)) {
  & docker compose -f $composeFile down | Out-Null
}

if ($purgeData -and (Test-Path $runtimeDir)) {
  Remove-Item $runtimeDir -Recurse -Force
}

if ($purgeData) {
  $dataMessage = "Runtime data deleted."
}
else {
  $dataMessage = "Pass --purge-data if you also want to delete the watched-pin state and bundled Kubo repo from the runtime directory."
}

Write-Host "Removed $serviceName"
Write-Host ""
Write-Host "Scheduled task:"
Write-Host "  $taskName"
Write-Host "Runtime data:"
Write-Host "  $runtimeDir"
Write-Host "Link handler:"
Write-Host "  foundationsharebridge://"
Write-Host ""
Write-Host $dataMessage
