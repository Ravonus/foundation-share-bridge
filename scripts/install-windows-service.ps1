Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDir ".."))
$payloadRoot = [System.IO.Path]::GetFullPath((Join-Path $scriptDir ".."))
$serviceName = "Foundation Share Bridge"
$taskName = "FoundationShareBridge"
$runtimeDir = Join-Path $env:LOCALAPPDATA "FoundationShareBridge"
$runtimeBinDir = Join-Path $runtimeDir "bin"
$runtimeScriptDir = Join-Path $runtimeDir "scripts"
$runtimeLogDir = Join-Path $runtimeDir "logs"
$runtimeDataDir = Join-Path $runtimeDir "data\kubo"
$runtimeStateFile = Join-Path $runtimeDir "bridge-state.json"
$runScript = Join-Path $runtimeScriptDir "run-bridge-stack.ps1"
$deepLinkScript = Join-Path $runtimeScriptDir "handle-deep-link.ps1"
$currentUserId = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$protocolKey = "HKCU:\Software\Classes\foundationsharebridge"
$protocolCommandKey = Join-Path $protocolKey "shell\open\command"
$protocolDefaultIconKey = Join-Path $protocolKey "DefaultIcon"

function Resolve-SourceRoot {
  $bundledBinary = Join-Path $payloadRoot "bin\foundation-share-bridge.exe"
  $bundledCompose = Join-Path $payloadRoot "docker-compose.yml"

  if ((Test-Path $bundledBinary) -and (Test-Path $bundledCompose)) {
    return $payloadRoot
  }

  if (Test-Path (Join-Path $repoRoot "Cargo.toml")) {
    return $repoRoot
  }

  throw "Unable to find a payload bundle or repo checkout for $serviceName."
}

function Build-OrResolveBinary([string]$sourceRoot) {
  $bundledBinary = Join-Path $sourceRoot "bin\foundation-share-bridge.exe"
  if (Test-Path $bundledBinary) {
    return $bundledBinary
  }

  $cargo = Get-Command cargo -ErrorAction SilentlyContinue
  if (-not $cargo) {
    throw "cargo is required when installing from source."
  }

  Push-Location $sourceRoot
  try {
    & $cargo.Source build --release
  }
  finally {
    Pop-Location
  }

  return (Join-Path $sourceRoot "target\release\foundation-share-bridge.exe")
}

function Copy-SeedData([string]$sourceRoot) {
  $seedDir = Join-Path $sourceRoot "seed"
  $seedState = Join-Path $seedDir "bridge-state.json"
  $seedKubo = Join-Path $seedDir "data\kubo"

  if ((Test-Path $seedState) -and -not (Test-Path $runtimeStateFile)) {
    Copy-Item $seedState $runtimeStateFile
  }

  if ((Test-Path $seedKubo) -and -not (Test-Path (Join-Path $runtimeDataDir "config"))) {
    New-Item -ItemType Directory -Force -Path $runtimeDataDir | Out-Null
    Copy-Item (Join-Path $seedKubo "*") $runtimeDataDir -Recurse -Force
  }
}

function Warn-MissingContainerRuntime {
  if (Get-Command docker -ErrorAction SilentlyContinue) {
    return
  }

  $dockerDesktop = Join-Path $env:ProgramFiles "Docker\Docker\Docker Desktop.exe"
  if (Test-Path $dockerDesktop) {
    return
  }

  Write-Warning "Docker Desktop was not found. The background task can still be installed, but the bundled Kubo node will not come online until Docker Desktop is installed."
}

$sourceRoot = Resolve-SourceRoot
$binarySource = Build-OrResolveBinary -sourceRoot $sourceRoot
$composeSource = Join-Path $sourceRoot "docker-compose.yml"
$runScriptSource = Join-Path $sourceRoot "scripts\run-bridge-stack.ps1"
$deepLinkScriptSource = Join-Path $sourceRoot "scripts\handle-deep-link.ps1"

if (-not (Test-Path $binarySource)) {
  throw "Bridge binary was not found at $binarySource"
}

if (-not (Test-Path $composeSource) -or -not (Test-Path $runScriptSource) -or -not (Test-Path $deepLinkScriptSource)) {
  throw "Installer assets were not found under $sourceRoot"
}

New-Item -ItemType Directory -Force -Path $runtimeBinDir, $runtimeScriptDir, $runtimeLogDir, $runtimeDataDir | Out-Null
Copy-Item $binarySource (Join-Path $runtimeBinDir "foundation-share-bridge.exe") -Force
Copy-Item $runScriptSource $runScript -Force
Copy-Item $deepLinkScriptSource $deepLinkScript -Force
Copy-Item $composeSource (Join-Path $runtimeDir "docker-compose.yml") -Force

Copy-SeedData -sourceRoot $sourceRoot
Warn-MissingContainerRuntime

$action = New-ScheduledTaskAction -Execute "powershell.exe" -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$runScript`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $currentUserId
$principal = New-ScheduledTaskPrincipal -UserId $currentUserId -LogonType Interactive -RunLevel Limited

Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Description "Keeps the Foundation Share Bridge and bundled Kubo node running in the background." -Force | Out-Null
Start-ScheduledTask -TaskName $taskName

$binaryRuntimePath = Join-Path $runtimeBinDir "foundation-share-bridge.exe"
$protocolCommand = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$deepLinkScript`" `"%1`""

New-Item -Path $protocolKey -Force | Out-Null
New-ItemProperty -Path $protocolKey -Name "(default)" -Value "URL:Foundation Share Bridge Pairing" -PropertyType String -Force | Out-Null
New-ItemProperty -Path $protocolKey -Name "URL Protocol" -Value "" -PropertyType String -Force | Out-Null
New-Item -Path $protocolDefaultIconKey -Force | Out-Null
New-ItemProperty -Path $protocolDefaultIconKey -Name "(default)" -Value $binaryRuntimePath -PropertyType String -Force | Out-Null
New-Item -Path $protocolCommandKey -Force | Out-Null
New-ItemProperty -Path $protocolCommandKey -Name "(default)" -Value $protocolCommand -PropertyType String -Force | Out-Null

Write-Host "Installed and started $serviceName"
Write-Host ""
Write-Host "Scheduled task:"
Write-Host "  $taskName"
Write-Host "Runtime:"
Write-Host "  $runtimeDir"
Write-Host "Logs:"
Write-Host "  $runtimeLogDir"
Write-Host "Link handler:"
Write-Host "  foundationsharebridge://"
