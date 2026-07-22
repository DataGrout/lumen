<#
.SYNOPSIS
  One-command Lumen launcher for Windows: start the daemon (if needed), verify
  the proxy is actually listening, then launch a client (Claude Desktop / Cursor)
  wired to the proxy with the Lumen CA trusted.

.DESCRIPTION
  Windows users otherwise have to (1) run lumen-core.exe in a terminal, (2) hope
  it bound its port, (3) hand-find Claude's install path, and (4) set two env
  vars before launching. This script does all four and - crucially - tells you
  plainly if the proxy did NOT come up, instead of the failure looking exactly
  like "Lumen is off" (client gets connection-refused).

  It is intentionally standalone (no build step) so a tester can drop it next to
  the daemon .exe and run it.

.PARAMETER Exe
  Path to the lumen-core daemon .exe. Defaults to a `lumen-core*.exe` sitting
  next to this script.

.PARAMETER Client
  Which client to launch: "Claude" (default) or "Cursor". Use "None" to only
  start/verify the daemon.

.EXAMPLE
  .\lumen.ps1
  .\lumen.ps1 -Client Cursor
  .\lumen.ps1 -Exe C:\Tools\lumen-core.exe -ProxyPort 9090
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [ValidateSet('Claude', 'Cursor', 'None')]
    [string]$Client = 'Claude',
    [int]$ProxyPort = 9090,
    [int]$ApiPort = 9091
)

$ErrorActionPreference = 'Stop'

function Test-PortListening([int]$Port) {
    try {
        $null = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction Stop
        return $true
    } catch {
        # Fallback for environments without the NetTCPIP module.
        try { return (Test-NetConnection -ComputerName 127.0.0.1 -Port $Port -WarningAction SilentlyContinue).TcpTestSucceeded }
        catch { return $false }
    }
}

function Test-DaemonHealthy([int]$Port) {
    try {
        $null = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/health" -TimeoutSec 2
        return $true
    } catch { return $false }
}

# -- 1. Start the daemon if it isn't already up ------------------------------
if (Test-DaemonHealthy $ApiPort) {
    Write-Host "Lumen daemon already running (API :$ApiPort)." -ForegroundColor Green
} else {
    if (-not $Exe) {
        $Exe = Get-ChildItem -Path $PSScriptRoot -Filter 'lumen-core*.exe' -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $Exe -or -not (Test-Path $Exe)) {
        Write-Error "Could not find the lumen-core .exe. Pass -Exe <path>, or place lumen-core*.exe next to this script."
        exit 1
    }

    Write-Host "Starting Lumen daemon: $Exe" -ForegroundColor Cyan
    Start-Process -FilePath $Exe `
        -ArgumentList @("--proxy-port", $ProxyPort, "--api-port", $ApiPort) `
        -WindowStyle Hidden | Out-Null
}

# -- 2. Verify the PROXY port is actually listening --------------------------
# This is the check that turns a silent "looks like Lumen is off" failure into a
# clear diagnosis. On Windows a bind can fail even when nothing is using the
# port (Hyper-V / WSL2 reserve port ranges -> WSAEACCES).
$deadline = (Get-Date).AddSeconds(12)
while (-not (Test-PortListening $ProxyPort) -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 400
}

if (-not (Test-PortListening $ProxyPort)) {
    Write-Host ""
    Write-Error @"
Lumen proxy is NOT listening on 127.0.0.1:$ProxyPort.

Clients pointed at this port will get 'connection refused', which looks exactly
like Lumen being off. On Windows the usual cause is a reserved/excluded port
range or another process on the port. Check:

    netsh interface ipv4 show excludedportrange protocol=tcp
    netstat -ano | findstr :$ProxyPort

If $ProxyPort falls inside an excluded range, start Lumen on a different port:

    .\lumen.ps1 -ProxyPort 9490
"@
    exit 1
}
Write-Host "Proxy listening on 127.0.0.1:$ProxyPort." -ForegroundColor Green

if ($Client -eq 'None') { exit 0 }

# -- 3. Resolve the client path (both known install locations) ---------------
$proxyUrl = "http://127.0.0.1:$ProxyPort"
$caPath = Join-Path $env:USERPROFILE ".lumen\ca.pem"
if (-not (Test-Path $caPath)) {
    Write-Warning "CA not found at $caPath - the daemon should create it on first run. TLS interception may fail until it exists."
}

if ($Client -eq 'Claude') {
    # Standalone installer OR Microsoft Store / winget build (version+hash vary).
    $exePaths = @(
        (Join-Path $env:LOCALAPPDATA 'AnthropicClaude\Claude.exe'),
        'C:\Program Files\WindowsApps\Claude_*\app\Claude.exe'
    )
    $label = 'Claude Desktop'
} else {
    $exePaths = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\cursor\Cursor.exe'),
        'C:\Program Files\WindowsApps\*Cursor*\Cursor.exe'
    )
    $label = 'Cursor'
}

$clientExe = Get-ChildItem -Path $exePaths -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName

if (-not $clientExe) {
    Write-Error "$label not found in the known install locations. Install it, or launch it yourself with:`n  `$env:HTTPS_PROXY='$proxyUrl'; `$env:NODE_EXTRA_CA_CERTS='$caPath'; & <path-to-exe>"
    exit 1
}

# -- 4. Launch the client through the proxy ----------------------------------
Write-Host "Launching $label through Lumen ($clientExe)" -ForegroundColor Cyan
$env:HTTPS_PROXY = $proxyUrl
$env:NODE_EXTRA_CA_CERTS = $caPath
& $clientExe
