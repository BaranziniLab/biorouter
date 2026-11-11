#Requires -Version 5.0
# Windows launcher for the exported Biorouter app "prevent-ucsf-risk-calculator".
$ErrorActionPreference = "Stop"
$AppId = "prevent-ucsf-risk-calculator"
$Dir   = Split-Path -Parent $MyInvocation.MyCommand.Definition
# Biorouter uses an XDG-style config dir; honour XDG_CONFIG_HOME, else ~/.config.
if ($env:XDG_CONFIG_HOME) { $Cfg = Join-Path $env:XDG_CONFIG_HOME "biorouter" }
else { $Cfg = Join-Path $HOME ".config\biorouter" }
$Store = Join-Path $Cfg "agent_drafter\$AppId"

function Find-Biorouterd {
  if ($env:BIOROUTERD_BIN -and (Test-Path $env:BIOROUTERD_BIN)) { return $env:BIOROUTERD_BIN }
  $bundled = Join-Path $Dir "payload\bin\biorouterd.exe"
  if (Test-Path $bundled) { return $bundled }               # fat export: self-contained
  $cmd = Get-Command biorouterd -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  foreach ($p in @(
      (Join-Path $env:LOCALAPPDATA "Programs\Biorouter\resources\bin\biorouterd.exe"),
      "C:\Program Files\Biorouter\resources\bin\biorouterd.exe")) {
    if ($p -and (Test-Path $p)) { return $p }
  }
  throw "biorouterd not found. Install Biorouter, or set BIOROUTERD_BIN."
}

# Copy the runtime files into the store so the daemon can resolve the app.
function Install-App {
  if (-not (Test-Path (Join-Path $Dir "manifest.json"))) {
    throw "manifest.json missing from this folder - re-export the app."
  }
  New-Item -ItemType Directory -Force -Path $Store | Out-Null
  Copy-Item (Join-Path $Dir "manifest.json") (Join-Path $Store "manifest.json") -Force
  if (Test-Path (Join-Path $Dir "index.html")) {
    Copy-Item (Join-Path $Dir "index.html") (Join-Path $Store "index.html") -Force
  }
  foreach ($sub in @("src", "dist", "assets")) {
    if (Test-Path (Join-Path $Dir $sub)) {
      $dst = Join-Path $Store $sub
      if (Test-Path $dst) { Remove-Item $dst -Recurse -Force }
      Copy-Item (Join-Path $Dir $sub) $dst -Recurse -Force
    }
  }
}

# First-run payload install (full-mode exports). Mirrors bash install_payload:
# import knowledge bases + skills once, guarded by a marker, consent required
# (set BIOROUTER_EXPORT_YES=1 to skip the prompt).
function Install-Payload {
  $payload = Join-Path $Dir "payload"
  if (-not (Test-Path $payload)) { return }                 # launcher-mode export
  $marker = Join-Path $Cfg ".apps-payload-installed-$AppId"
  if (Test-Path $marker) { return }                         # already installed

  $kbs = @(); $skills = @()
  $kdir = Join-Path $payload "knowledge"
  if (Test-Path $kdir) { $kbs = @(Get-ChildItem -Path $kdir -Filter *.brkb -ErrorAction SilentlyContinue) }
  $sdir = Join-Path $payload "skills"
  if (Test-Path $sdir) { $skills = @(Get-ChildItem -Path $sdir -Directory -ErrorAction SilentlyContinue) }
  if ($kbs.Count -eq 0 -and $skills.Count -eq 0) { return }

  Write-Host ""
  Write-Host "This app ships a payload to install into your Biorouter (see payload/export.json):"
  foreach ($f in $kbs)    { Write-Host "  - knowledge base: $($f.Name)" }
  foreach ($d in $skills) { Write-Host "  - skill: $($d.Name)" }

  if ($env:BIOROUTER_EXPORT_YES -ne "1") {
    $reply = Read-Host "Install this payload now? [y/N]"
    if ($reply -notmatch '^(y|Y|yes|YES)$') {
      Write-Host "Skipping payload install. Re-run to install it later."
      return
    }
  }

  New-Item -ItemType Directory -Force -Path (Join-Path $Cfg "skills") | Out-Null
  # No `biorouter knowledge import` CLI subcommand exists yet (2026-07 tree), so
  # stage each .brkb and point the user at the Knowledge panel importer. Prefer
  # the CLI if a future version grows the command.
  foreach ($f in $kbs) {
    $bio = Get-Command biorouter -ErrorAction SilentlyContinue
    $canImport = $false
    if ($bio) { & $bio.Source knowledge import --help *> $null; $canImport = ($LASTEXITCODE -eq 0) }
    if ($canImport) {
      & $bio.Source knowledge import $f.FullName
      Write-Host "  - imported $($f.Name)"
    } else {
      $imp = Join-Path $Cfg "knowledge-imports"
      New-Item -ItemType Directory -Force -Path $imp | Out-Null
      Copy-Item $f.FullName $imp -Force
      Write-Host "  - staged $($f.Name) in $imp (import it from Biorouter's Knowledge panel)"
    }
  }
  foreach ($d in $skills) {
    $dst = Join-Path (Join-Path $Cfg "skills") $d.Name
    if (Test-Path $dst) { Remove-Item $dst -Recurse -Force }
    Copy-Item $d.FullName $dst -Recurse -Force
    Write-Host "  - installed skill $($d.Name)"
  }

  New-Item -ItemType Directory -Force -Path $Cfg | Out-Null
  New-Item -ItemType File -Force -Path $marker | Out-Null
  Write-Host "Payload installed."
}

function Test-Port([int]$Port) {
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/status" -TimeoutSec 1 -UseBasicParsing
    return ($r.StatusCode -eq 200)
  } catch { return $false }
}

# A daemon serves an app's page only to a browser holding that app's access
# cookie, and the one-time link that sets it is handed only to a caller holding
# the daemon's secret. So this launcher uses a daemon it started (its port and
# secret are kept in $StateFile, under your profile) or one you name with
# BIOROUTERD_PORT and BIOROUTER_SERVER__SECRET_KEY, and no other.
$StateFile = Join-Path $Cfg "app-launcher\daemon"

function New-Secret {
  $bytes = New-Object byte[] 32
  $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
  try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
  return (($bytes | ForEach-Object { $_.ToString("x2") }) -join "")
}

function Test-Secret([int]$Port, [string]$Secret) {
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/apps" -Headers @{ "X-Secret-Key" = $Secret } -TimeoutSec 2 -UseBasicParsing
    return ($r.StatusCode -eq 200)
  } catch { return $false }
}

function Save-Daemon([int]$Port, [string]$Secret) {
  try {
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $StateFile) | Out-Null
    Set-Content -Path $StateFile -Value "$Port $Secret" -NoNewline
  } catch { Write-Host "  (could not remember this daemon; the next run starts another)" }
}

# Use a daemon whose secret is known, else start one headlessly with a new secret.
function Start-Daemon {
  if ($env:BIOROUTER_SERVER__SECRET_KEY -and $env:BIOROUTERD_PORT -and
      (Test-Secret ([int]$env:BIOROUTERD_PORT) $env:BIOROUTER_SERVER__SECRET_KEY)) {
    Write-Host "Using the Biorouter daemon on :$env:BIOROUTERD_PORT"
    return @([int]$env:BIOROUTERD_PORT, [string]$env:BIOROUTER_SERVER__SECRET_KEY)
  }
  if (Test-Path $StateFile) {
    try {
      $saved = ((Get-Content $StateFile -Raw) -as [string]).Trim() -split "\s+"
      if ($saved.Count -eq 2 -and (Test-Secret ([int]$saved[0]) $saved[1])) {
        Write-Host "Using the Biorouter daemon this launcher started on :$($saved[0])"
        return @([int]$saved[0], [string]$saved[1])
      }
    } catch { }  # an unreadable record: start a daemon of our own
  }
  $bin = Find-Biorouterd
  $preferred = if ($env:BIOROUTERD_PORT) { [int]$env:BIOROUTERD_PORT } else { 3000 }
  foreach ($p in @($preferred, 3001, 3002, 3003, 3004, 3005)) {
    # A daemon already here is not one whose secret this launcher knows.
    if (Test-Port $p) { continue }
    # A new secret for every attempt, so one asked of whatever held the port
    # never becomes the secret of a daemon started elsewhere.
    $secret = New-Secret
    Write-Host "Starting $bin on :$p ..."
    $env:BIOROUTER_PORT = "$p"
    $env:BIOROUTER_SERVER__SECRET_KEY = $secret
    Start-Process -FilePath $bin -ArgumentList "agent" -WindowStyle Hidden | Out-Null
    # Only the daemon needs it: the browser started below inherits this
    # process's environment, so the secret must not stay in it.
    Remove-Item Env:BIOROUTER_SERVER__SECRET_KEY -ErrorAction SilentlyContinue
    for ($i = 0; $i -lt 40; $i++) {
      if (Test-Secret $p $secret) { Save-Daemon $p $secret; return @($p, $secret) }
      Start-Sleep -Milliseconds 500
    }
  }
  throw "could not start biorouterd."
}

Install-App
Install-Payload
$Daemon = Start-Daemon
$Port = [int]$Daemon[0]
$Secret = [string]$Daemon[1]
$Base = "http://127.0.0.1:$Port"
# The app's one-time launch link: opening it sets the app's access cookie in the
# browser and lands on the app. It works once and for a few minutes.
$LaunchPath = $null
try {
  $LaunchPath = [string](Invoke-RestMethod -Method Post -Uri "$Base/apps/$AppId/launch" -Headers @{ "X-Secret-Key" = $Secret } -TimeoutSec 5).path
} catch { $LaunchPath = $null }
if (-not $LaunchPath -or $LaunchPath -notmatch '^/apps/[A-Za-z0-9_-]+/\?t=[0-9a-f]{64}$') {
  throw "the daemon on :$Port does not serve '$AppId'."
}
Write-Host "Opening $AppId at $Base/apps/$AppId/"
# The link opens the app for whoever uses it first, so it never goes on a
# command line: it is written into a page under your profile, which only you can
# read, and the page sends the browser on to it. Pages older than ten minutes
# hold expired links and are removed here.
$Link = "$Base$LaunchPath"
$Pages = Join-Path $Cfg "app-launcher\open"
New-Item -ItemType Directory -Force -Path $Pages | Out-Null
Get-ChildItem -Path $Pages -Filter "launch-*.html" -File -ErrorAction SilentlyContinue |
  Where-Object { $_.LastWriteTime -lt (Get-Date).AddMinutes(-10) } |
  Remove-Item -Force -ErrorAction SilentlyContinue
$Page = Join-Path $Pages ("launch-" + (New-Secret).Substring(0, 32) + ".html")
$Html = '<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer>' +
  '<meta http-equiv="refresh" content="0;url=' + $Link + '"><title>Opening Biorouter app</title>' +
  '<p>Opening the app. If nothing happens, <a href="' + $Link + '">open it here</a>. The address works once.</p>'
try {
  Set-Content -LiteralPath $Page -Value $Html -Encoding UTF8
  Invoke-Item -LiteralPath $Page
  # This window is yours alone, so the link can be shown here too.
  Write-Host "If the app does not appear, open this address instead (it works once): $Link"
} catch {
  Write-Host "Open this address in your browser (it works once): $Link"
}
