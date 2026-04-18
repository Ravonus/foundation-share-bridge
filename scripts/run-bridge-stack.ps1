Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$runtimeDir = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$logDir = Join-Path $runtimeDir "logs"
$binPath = Join-Path $runtimeDir "bin\foundation-share-bridge.exe"
$composeFile = Join-Path $runtimeDir "docker-compose.yml"
$dockerDesktop = Join-Path $env:ProgramFiles "Docker\Docker\Docker Desktop.exe"

New-Item -ItemType Directory -Force -Path $logDir, (Join-Path $runtimeDir "data\kubo") | Out-Null

function Wait-ForDocker {
  for ($attempt = 0; $attempt -lt 60; $attempt++) {
    try {
      & docker info *> $null
      if ($LASTEXITCODE -eq 0) {
        return
      }
    }
    catch {
    }
    Start-Sleep -Seconds 2
  }

  throw "Docker Desktop is installed but did not become ready in time."
}

function Ensure-ContainerRuntime {
  if (Get-Command docker -ErrorAction SilentlyContinue) {
    try {
      & docker info *> $null
      if ($LASTEXITCODE -eq 0) {
        return
      }
    }
    catch {
    }
  }
  else {
    throw "docker was not found on PATH. Install Docker Desktop first."
  }

  if (Test-Path $dockerDesktop) {
    Start-Process -FilePath $dockerDesktop | Out-Null
    Wait-ForDocker
    return
  }

  throw "Docker Desktop was not found. Install it first."
}

function Wait-ForKuboApi {
  for ($attempt = 0; $attempt -lt 60; $attempt++) {
    try {
      Invoke-WebRequest -UseBasicParsing -Method Post -Uri "http://127.0.0.1:5001/api/v0/version" | Out-Null
      return
    }
    catch {
      Start-Sleep -Seconds 2
    }
  }

  throw "The bundled Kubo API did not come online at http://127.0.0.1:5001."
}

Ensure-ContainerRuntime
& docker compose -f $composeFile up -d kubo | Out-Null
Wait-ForKuboApi

if (-not (Test-Path $binPath)) {
  throw "Bridge binary was not installed at $binPath"
}

$env:IPFS_API_URL = "http://127.0.0.1:5001"
$env:BRIDGE_STATE_FILE = Join-Path $runtimeDir "bridge-state.json"

if (-not $env:SELF_REPAIR_INTERVAL_SECONDS) {
  $env:SELF_REPAIR_INTERVAL_SECONDS = "900"
}

& $binPath 1>> (Join-Path $logDir "bridge.out.log") 2>> (Join-Path $logDir "bridge.err.log")
