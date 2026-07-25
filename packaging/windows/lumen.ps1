<#
.SYNOPSIS
  One-command Lumen launcher for Windows: start the daemon (if needed), verify
  the proxy is actually listening, trust Lumen's CA where a client needs it, then
  launch a client (Claude Desktop / Claude Code / Cursor) wired to the proxy.

.DESCRIPTION
  Windows users otherwise have to (1) run lumen-core.exe in a terminal, (2) hope
  it bound its port, (3) hand-find Claude's install path, and (4) set env vars +
  trust a CA before launching. This script does all of that and - crucially -
  tells you plainly if the proxy did NOT come up, instead of the failure looking
  exactly like "Lumen is off" (client gets connection-refused).

  Claude Desktop on Windows is now shipped from claude.ai/download as an MSIX
  (packaged) app under the ACL-locked WindowsApps folder. Such apps cannot be
  launched by spawning their .exe with a custom environment, and their Electron/
  Chromium network stack validates TLS against the WINDOWS CERT STORE (not
  NODE_EXTRA_CA_CERTS). So for a packaged Claude Desktop this script:
    * imports Lumen's CA into CurrentUser\Root  (so Chromium trusts interception)
    * PERSISTS the proxy env vars at User scope (packaged apps read env from the
      registry-built block, not from the launching process)
    * launches via Invoke-CommandInDesktopPackage (the one method that reliably
      hands the packaged process the persisted environment)
  Undo the CA trust + persisted env with:  run.bat -Cleanup

.PARAMETER Exe
  Path to the lumen-core daemon .exe. Defaults to a `lumen-core*.exe` sitting
  next to this script.

.PARAMETER Client
  Which client to launch: Auto (default), Claude, Code, Cursor, or None.

.PARAMETER NoTrustCA
  Do NOT import Lumen's CA into the Windows trust store. GUI/Chromium clients
  (Claude Desktop, Cursor) will then fail TLS interception until the CA is
  trusted by some other means.

.PARAMETER Cleanup
  Remove Lumen's CA from CurrentUser\Root and clear the persisted proxy env vars
  (HTTPS_PROXY / HTTP_PROXY / NODE_EXTRA_CA_CERTS at User scope), then exit.

.EXAMPLE
  .\lumen.ps1
  .\lumen.ps1 -Client Cursor
  .\lumen.ps1 -Client Claude          # works for MSIX (packaged) Claude Desktop
  .\lumen.ps1 -Cleanup                # revert CA trust + persisted proxy env
#>
[CmdletBinding()]
param(
    [string]$Exe,
    [ValidateSet('Auto', 'Claude', 'Code', 'Cursor', 'None')]
    [string]$Client = 'Auto',
    [int]$ProxyPort = 9090,
    [int]$ApiPort = 9091,
    [switch]$NoTrustCA,
    [switch]$Cleanup,
    [switch]$Stop
)

$ErrorActionPreference = 'Stop'
$caPath = Join-Path $env:USERPROFILE ".lumen\ca.pem"

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

# Trust Lumen's interception CA so Chromium/WinHTTP-based clients (Claude Desktop,
# Cursor) can validate the man-in-the-middle certificate. CurrentUser\Root needs
# no admin. Idempotent. This is a security-weighted step - announced, not silent.
function Import-LumenCA([string]$CaFile) {
    if (-not (Test-Path $CaFile)) {
        Write-Warning "CA not found at $CaFile - cannot establish trust. The daemon creates it on first run."
        return
    }
    $ca = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $CaFile
    $already = Get-ChildItem Cert:\CurrentUser\Root -ErrorAction SilentlyContinue | Where-Object { $_.Thumbprint -eq $ca.Thumbprint }
    if ($already) {
        Write-Host "Lumen CA already trusted (CurrentUser\Root)." -ForegroundColor DarkGray
        return
    }
    Write-Host "Trusting Lumen's interception CA in CurrentUser\Root:" -ForegroundColor Yellow
    Write-Host "  $($ca.Subject)" -ForegroundColor Yellow
    Write-Host "  This lets Chromium-based clients validate Lumen's TLS interception." -ForegroundColor DarkGray
    Write-Host "  SECURITY: while trusted, anything holding $env:USERPROFILE\.lumen\ca-key.pem" -ForegroundColor DarkGray
    Write-Host "  can intercept ANY TLS for your user account - not just Claude. Undo: run.bat -Cleanup" -ForegroundColor DarkGray
    $store = New-Object System.Security.Cryptography.X509Certificates.X509Store('Root', 'CurrentUser')
    $store.Open('ReadWrite'); $store.Add($ca); $store.Close()
    Write-Host "  CA imported." -ForegroundColor Green
}

function Remove-LumenTrust([string]$CaFile) {
    if (Test-Path $CaFile) {
        $ca = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2 $CaFile
        $store = New-Object System.Security.Cryptography.X509Certificates.X509Store('Root', 'CurrentUser')
        $store.Open('ReadWrite')
        $match = $store.Certificates | Where-Object { $_.Thumbprint -eq $ca.Thumbprint }
        if ($match) { $store.Remove($match[0]); Write-Host "Removed Lumen CA from CurrentUser\Root." -ForegroundColor Green }
        else { Write-Host "Lumen CA was not present in CurrentUser\Root." -ForegroundColor DarkGray }
        $store.Close()
    } else {
        Write-Host "No CA file at $CaFile; skipping cert removal." -ForegroundColor DarkGray
    }
    foreach ($v in 'HTTPS_PROXY', 'HTTP_PROXY', 'NODE_EXTRA_CA_CERTS') {
        if ([Environment]::GetEnvironmentVariable($v, 'User')) {
            [Environment]::SetEnvironmentVariable($v, $null, 'User')
            Write-Host "Cleared persisted User env: $v" -ForegroundColor Green
        }
    }
}

# Detect a packaged (MSIX/AppX) Claude Desktop and return the handles needed to
# launch it. Derived dynamically so this works across versions/machines - do NOT
# hardcode the package full name (it embeds a version + publisher hash).
function Get-ClaudeMsix {
    $pkg = Get-AppxPackage -Name '*Claude*' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $pkg) { return $null }
    try { $app = ($pkg | Get-AppxPackageManifest).Package.Applications.Application | Select-Object -First 1 }
    catch { return $null }
    if (-not $app) { return $null }
    return [pscustomobject]@{
        Pfn   = $pkg.PackageFamilyName                        # e.g. Claude_pzs8sxrjxfjjc
        AppId = $app.Id                                       # e.g. Claude
        Exe   = Join-Path $pkg.InstallLocation $app.Executable # ...\app\Claude.exe
        Aumid = "$($pkg.PackageFamilyName)!$($app.Id)"
    }
}

# -- 0. Cleanup shortcut -----------------------------------------------------
if ($Cleanup) {
    Write-Host "Reverting Lumen client wiring..." -ForegroundColor Cyan
    Remove-LumenTrust -CaFile $caPath
    Write-Host "Cleanup done." -ForegroundColor Cyan
    exit 0
}

# -- 0b. Stop shortcut -------------------------------------------------------
# The daemon runs in the background and keeps tracking after a client closes;
# this asks it to shut down cleanly, so you don't have to end it in Task Manager.
if ($Stop) {
    try {
        Invoke-RestMethod -Method Post "http://127.0.0.1:$ApiPort/shutdown" -TimeoutSec 3 | Out-Null
        Write-Host "Sent shutdown to the Lumen daemon (API :$ApiPort)." -ForegroundColor Green
    } catch {
        Write-Host "No running Lumen daemon on API :$ApiPort (it may already be stopped)." -ForegroundColor DarkGray
    }
    exit 0
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

# -- 3. Resolve the client ---------------------------------------------------
$proxyUrl = "http://127.0.0.1:$ProxyPort"
if (-not (Test-Path $caPath)) {
    Write-Warning "CA not found at $caPath - the daemon should create it on first run. TLS interception may fail until it exists."
}

$relayUrl = "$proxyUrl/anthropic"
$labels = @{ Claude = 'Claude Desktop'; Code = 'Claude Code'; Cursor = 'Cursor' }

# Return a launchable exe path for a STANDALONE (non-packaged) client, or $null.
# Packaged Claude Desktop is handled separately via Get-ClaudeMsix.
function Resolve-ClientExe([string]$name) {
    switch ($name) {
        'Claude' {
            # Standalone (Squirrel) install: stub in %LOCALAPPDATA%\AnthropicClaude
            # or the real binary under app-<version>\.
            $paths = @(
                (Join-Path $env:LOCALAPPDATA 'AnthropicClaude\Claude.exe'),
                (Join-Path $env:LOCALAPPDATA 'AnthropicClaude\app-*\Claude.exe'),
                (Join-Path $env:LOCALAPPDATA 'Programs\Claude\Claude.exe'),
                (Join-Path $env:ProgramFiles 'Claude\Claude.exe')
            )
        }
        'Code' {
            # Claude Code = the CLI (claude.exe) from claude.ai/install.ps1.
            $paths = @(
                (Join-Path $env:USERPROFILE '.local\bin\claude.exe'),
                (Join-Path $env:APPDATA 'npm\claude.cmd')
            )
        }
        'Cursor' {
            $paths = @( (Join-Path $env:LOCALAPPDATA 'Programs\cursor\Cursor.exe') )
        }
    }
    $exe = Get-ChildItem -Path $paths -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
    # Claude Code is often reachable only via the `claude` shim on PATH.
    if (-not $exe -and $name -eq 'Code') {
        $cmd = Get-Command claude -ErrorAction SilentlyContinue
        if ($cmd) { $exe = $cmd.Source }
    }
    return $exe
}

$clientExe  = $null
$claudeMsix = $null

if ($Client -eq 'Auto') {
    foreach ($c in @('Claude', 'Code', 'Cursor')) {
        $clientExe = Resolve-ClientExe $c
        if ($clientExe) {
            $Client = $c
            Write-Host "Auto-detected $($labels[$c]) (standalone). (Override with -Client Claude|Code|Cursor.)" -ForegroundColor Cyan
            break
        }
    }
    if (-not $clientExe) {
        # No standalone exe; a packaged Claude Desktop is now the common case.
        $claudeMsix = Get-ClaudeMsix
        if ($claudeMsix) {
            $Client = 'Claude'
            Write-Host "Auto-detected Claude Desktop (packaged/MSIX). (Override with -Client Code|Cursor.)" -ForegroundColor Cyan
        }
    }
    if (-not $clientExe -and -not $claudeMsix) {
        Write-Host "No launchable client found (Claude Desktop, Claude Code, or Cursor)." -ForegroundColor Red
        Write-Host "Install Claude Code (claude.ai/install.ps1), Claude Desktop (claude.ai/download), or Cursor; or pass -Client explicitly." -ForegroundColor Red
        exit 1
    }
} else {
    if ($Client -eq 'Claude') {
        $clientExe = Resolve-ClientExe 'Claude'
        if (-not $clientExe) { $claudeMsix = Get-ClaudeMsix }
        if (-not $clientExe -and -not $claudeMsix) {
            Write-Host "Claude Desktop not found (neither standalone nor packaged/MSIX)." -ForegroundColor Red
            if (Resolve-ClientExe 'Code') {
                Write-Host "Claude Code (the CLI) IS installed - a different product. To monitor it:  run.bat -Client Code" -ForegroundColor Yellow
            }
            exit 1
        }
    } else {
        $clientExe = Resolve-ClientExe $Client
        if (-not $clientExe) {
            Write-Host "$($labels[$Client]) not found." -ForegroundColor Red
            Write-Host "Or launch it yourself with:  `$env:HTTPS_PROXY='$proxyUrl'; `$env:NODE_EXTRA_CA_CERTS='$caPath'; & <path-to-exe>" -ForegroundColor DarkGray
            exit 1
        }
    }
}

$label = $labels[$Client]

# -- 4. Launch the client through Lumen --------------------------------------
if ($Client -eq 'Code') {
    # Claude Code talks to the Anthropic API - point it at Lumen's relay
    # (no CA trust / TLS interception needed, and it never touches claude.ai).
    Write-Host "Launching $label through Lumen (relay: $relayUrl)" -ForegroundColor Cyan
    $env:ANTHROPIC_BASE_URL = $relayUrl
    Write-Host "  ANTHROPIC_BASE_URL=$relayUrl" -ForegroundColor DarkGray
    & $clientExe
}
elseif ($claudeMsix) {
    # Packaged (MSIX) Claude Desktop - Electron/Chromium under WindowsApps.
    #   1) Chromium validates TLS against the Windows cert store, so the CA must
    #      be trusted there (NODE_EXTRA_CA_CERTS only covers Node-side TLS).
    #   2) The exe can't be spawned with a custom env; packaged apps read their
    #      environment from the registry-built block, so the proxy vars must be
    #      PERSISTED at User scope (transient $env is NOT inherited).
    #   3) Invoke-CommandInDesktopPackage reliably hands the packaged process the
    #      persisted env; the shell:AppsFolder/AUMID path is flaky (explorer
    #      caches its env block and may launch with a stale one).
    if (-not $NoTrustCA) { Import-LumenCA $caPath }

    Write-Host "Persisting proxy env at User scope so the packaged app inherits it..." -ForegroundColor Cyan
    [Environment]::SetEnvironmentVariable('HTTPS_PROXY',         $proxyUrl, 'User')
    [Environment]::SetEnvironmentVariable('HTTP_PROXY',          $proxyUrl, 'User')
    [Environment]::SetEnvironmentVariable('NODE_EXTRA_CA_CERTS', $caPath,   'User')
    Write-Host "  HTTPS_PROXY / HTTP_PROXY = $proxyUrl" -ForegroundColor DarkGray
    Write-Host "  NODE_EXTRA_CA_CERTS      = $caPath" -ForegroundColor DarkGray
    Write-Host "  NOTE: these persist for your user (affecting other env-respecting apps)" -ForegroundColor DarkGray
    Write-Host "        until you run:  run.bat -Cleanup" -ForegroundColor DarkGray

    Write-Host "Launching $label ($($claudeMsix.Aumid)) via package activation..." -ForegroundColor Cyan
    if (Get-Command Invoke-CommandInDesktopPackage -ErrorAction SilentlyContinue) {
        # Kill any running (un-injected) instance first so it re-reads the env.
        Get-Process -Name 'claude' -ErrorAction SilentlyContinue |
            Where-Object { $_.Path -like '*WindowsApps*Claude*' } | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
        Invoke-CommandInDesktopPackage -PackageFamilyName $claudeMsix.Pfn -AppId $claudeMsix.AppId -Command $claudeMsix.Exe
    } else {
        Write-Warning "Invoke-CommandInDesktopPackage unavailable; falling back to AUMID launch (env injection may be unreliable)."
        Start-Process "shell:AppsFolder\$($claudeMsix.Aumid)"
    }
    Write-Host "Launched. Watch usage at http://127.0.0.1:$ApiPort/dashboard" -ForegroundColor Green
    Write-Host "If Claude Desktop shows an SSL error, the CA trust didn't apply - re-run, or fully quit Claude and retry." -ForegroundColor DarkGray
}
else {
    # Standalone GUI client (Squirrel Claude Desktop / Cursor). Chromium-based,
    # so trust the CA in the Windows store too; env is inherited as a child proc.
    if (-not $NoTrustCA) { Import-LumenCA $caPath }
    Write-Host "Launching $label through Lumen ($clientExe)" -ForegroundColor Cyan
    $env:HTTPS_PROXY = $proxyUrl
    $env:NODE_EXTRA_CA_CERTS = $caPath
    & $clientExe
}
