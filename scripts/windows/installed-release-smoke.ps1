[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Setup,
    [Parameter(Mandatory = $true)][string]$ZipAppDirectory,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$AssetEvidence,
    [Parameter(Mandatory = $true)][string]$Report
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'This installer test requires a disposable Windows GitHub Actions runner.'
}
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw 'Expected a stable release version' }
$Setup = (Resolve-Path -LiteralPath $Setup).Path
$ZipAppDirectory = (Resolve-Path -LiteralPath $ZipAppDirectory).Path
$Report = [IO.Path]::GetFullPath($Report)
$root = Join-Path $env:RUNNER_TEMP ('br-installed-' + [Guid]::NewGuid().ToString('N'))
$local = Join-Path $root 'local'
$installed = Join-Path $local "biorouter_app/app-$Version"
$owned = @{}
$changedEnvironment = @('LOCALAPPDATA', 'APPDATA', 'SQUIRREL_TEMP', 'BIOROUTER_PATH_ROOT',
    'BIOROUTER_DISABLE_KEYRING', 'BIOROUTER_DEV_PROFILE_ROOT', 'BIOROUTER_DEV_PROFILE_NAME',
    'BIOROUTER_SHARED_DAEMON', 'BIOROUTER_EXTERNAL_BACKEND', 'BIOROUTER_EXTERNAL_BACKEND_URL',
    'BIOROUTER_PORT', 'BIOROUTER_SERVER__SECRET_KEY', 'ENABLE_PLAYWRIGHT', 'PLAYWRIGHT_CDP_PORT')
$originalEnvironment = @{}
foreach ($name in $changedEnvironment) {
    $originalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$result = [ordered]@{ version = $Version; passed = $false; assets = @(); installed = $installed;
    files = @(); excludedZipMetadata = @(); cli = $false; daemon = $false; desktop = $false; cleanup = $false }

function Update-OwnedProcesses {
    $processes = @(Get-CimInstance Win32_Process)
    do {
        $added = $false
        foreach ($item in $processes) {
            $key = [string]$item.ProcessId
            if ($owned.ContainsKey($key)) { continue }
            $parent = $owned[[string]$item.ParentProcessId]
            $inRoot = $item.ExecutablePath -and $item.ExecutablePath.StartsWith(
                $root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
            if ($inRoot -or ($parent -and $item.CreationDate -ge $parent.CreationDate)) {
                $owned[$key] = $item
                $added = $true
            }
        }
    } while ($added)
}

function Start-OwnedProcess {
    param([string]$File, [string[]]$Arguments, [string]$Label, [switch]$ShowWindow)
    $options = @{ FilePath = $File; ArgumentList = $Arguments; PassThru = $true;
        RedirectStandardOutput = (Join-Path $root "$Label.stdout.log");
        RedirectStandardError = (Join-Path $root "$Label.stderr.log") }
    if (-not $ShowWindow) { $options.WindowStyle = 'Hidden' }
    $process = Start-Process @options
    $identity = Get-CimInstance Win32_Process -Filter "ProcessId=$($process.Id)"
    if ($identity) { $owned[[string]$process.Id] = $identity }
    return $process
}

function Wait-OwnedExit {
    param([Diagnostics.Process]$Process, [int]$Seconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while (-not $Process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
        Update-OwnedProcesses
        Start-Sleep -Milliseconds 250
        $Process.Refresh()
    }
    if (-not $Process.HasExited) { throw "Process $($Process.Id) did not exit within $Seconds seconds" }
    $Process.WaitForExit()
    if ($Process.ExitCode -ne 0) { throw "Process $($Process.Id) exited with $($Process.ExitCode)" }
}

function Get-FreePort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    return $port
}

function Stop-OwnedProcess {
    param($Identity)
    $current = Get-CimInstance Win32_Process -Filter "ProcessId=$($Identity.ProcessId)"
    if ($current -and $current.CreationDate -eq $Identity.CreationDate -and
        $current.ExecutablePath -eq $Identity.ExecutablePath) {
        Stop-Process -Id $Identity.ProcessId -Force
    }
}

try {
    New-Item -ItemType Directory -Path $local, (Join-Path $root 'roaming'), (Join-Path $root 'electron') -Force | Out-Null
    $result.assets = @(Get-Content -LiteralPath $AssetEvidence -Raw | ConvertFrom-Json)
    if ($result.assets.Count -ne 2) { throw 'Expected authenticated ZIP and Setup asset evidence' }
    foreach ($asset in $result.assets) {
        $digest = (Get-FileHash -LiteralPath $asset.path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($asset.digest -cne "sha256:$digest" -or (Get-Item $asset.path).Length -ne $asset.size) {
            throw "Downloaded asset no longer matches GitHub provenance: $($asset.name)"
        }
    }
    $setupEvidence = @($result.assets | Where-Object { $_.name -ceq "Biorouter-Setup-$Version.exe" })
    if ($setupEvidence.Count -ne 1 -or [IO.Path]::GetFullPath($setupEvidence[0].path) -ne $Setup) {
        throw 'Setup path does not match the authenticated release asset'
    }

    $env:LOCALAPPDATA = $local
    $env:APPDATA = Join-Path $root 'roaming'
    # Squirrel's supported extraction override makes Update.exe install beside SquirrelTemp.
    $env:SQUIRREL_TEMP = $local
    $env:BIOROUTER_PATH_ROOT = Join-Path $root 'profile'
    $env:BIOROUTER_DISABLE_KEYRING = 'true'
    foreach ($name in @('BIOROUTER_DEV_PROFILE_ROOT', 'BIOROUTER_DEV_PROFILE_NAME',
        'BIOROUTER_SHARED_DAEMON', 'BIOROUTER_EXTERNAL_BACKEND', 'BIOROUTER_EXTERNAL_BACKEND_URL')) {
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    $defaultInstall = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'biorouter_app'
    if (Test-Path -LiteralPath $defaultInstall) {
        throw 'Refusing to run Setup with an existing installation outside the owned test directory'
    }
    $installer = Start-OwnedProcess -File $Setup -Arguments @('--silent') -Label 'setup'
    Wait-OwnedExit -Process $installer -Seconds 180
    if (-not (Test-Path -LiteralPath (Join-Path $local 'biorouter_app/Update.exe'))) {
        throw 'Setup did not create the Squirrel installation in isolated LOCALAPPDATA'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $installed 'Biorouter.exe'))) {
        throw "Setup did not install app-$Version"
    }
    $packages = @(Get-ChildItem -LiteralPath (Join-Path $local 'biorouter_app/packages') -File -Filter '*-full.nupkg')
    if ($packages.Count -ne 1) { throw 'Expected the single full NuGet package installed by Setup' }
    $packageEntries = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $packageArchive = [IO.Compression.ZipFile]::OpenRead($packages[0].FullName)
    try {
        foreach ($entry in $packageArchive.Entries) {
            $packageEntries.Add($entry.FullName.Replace('\', '/')) | Out-Null
        }
    } finally {
        $packageArchive.Dispose()
    }
    $result.installedPackageSha256 = (Get-FileHash -LiteralPath $packages[0].FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $files = @(Get-ChildItem -LiteralPath $ZipAppDirectory -File -Recurse)
    if ($files.Count -eq 0) { throw 'ZIP reference tree is empty' }
    foreach ($file in $files) {
        $relative = [IO.Path]::GetRelativePath($ZipAppDirectory, $file.FullName)
        # electron-winstaller's NuSpec omits these top-level metadata files, not runtime payloads.
        if ($relative -cin @('LICENSES.chromium.html', 'version') -and
            -not $packageEntries.Contains("lib/net45/$relative")) {
            $result.excludedZipMetadata += @{ path = $relative; reason = 'Not included by the NuSpec template';
                sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
            continue
        }
        $destination = Join-Path $installed $relative
        if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
            throw "Installer omitted ZIP file: $relative"
        }
        $zipHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        $installedHash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
        if ($zipHash -cne $installedHash) { throw "Installed bytes differ from uploaded ZIP: $relative" }
        $result.files += @{ path = $relative; sha256 = $installedHash.ToLowerInvariant() }
    }
    $cli = Join-Path $installed 'resources/bin/biorouter.exe'
    $daemon = Join-Path $installed 'resources/bin/biorouterd.exe'
    $helper = Join-Path $installed 'resources/computer-use'
    foreach ($file in @($cli, $daemon, (Join-Path $helper 'manifest.json'))) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Installed payload missing: $file" }
    }
    python scripts/computer-use-runtime.py verify win32-x64 --directory $helper
    if ($LASTEXITCODE -ne 0) { throw 'Installed native helper integrity check failed' }
    python scripts/test-computer-use-protocol.py $helper
    if ($LASTEXITCODE -ne 0) { throw 'Installed native helper protocol check failed' }
    foreach ($binary in @($cli, $daemon)) {
        $output = & $binary --version
        if ($LASTEXITCODE -ne 0 -or $output -notmatch "\b$([regex]::Escape($Version))\b") {
            throw "Installed binary did not report $Version"
        }
    }
    & $cli term --help | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Installed CLI terminal command failed' }
    $result.cli = $true

    $port = Get-FreePort
    $env:BIOROUTER_PORT = [string]$port
    $env:BIOROUTER_SERVER__SECRET_KEY = [Guid]::NewGuid().ToString('N')
    $daemonProcess = Start-OwnedProcess -File $daemon -Arguments @('agent') -Label 'daemon'
    $ready = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $deadline -and -not $daemonProcess.HasExited) {
        Update-OwnedProcesses
        try {
            $response = Invoke-WebRequest "http://127.0.0.1:$port/status" -TimeoutSec 2 `
                -Headers @{ 'X-Secret-Key' = $env:BIOROUTER_SERVER__SECRET_KEY }
            $listeners = @(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)
            if ($response.StatusCode -eq 200 -and $listeners.OwningProcess -contains $daemonProcess.Id) {
                $ready = $true
                break
            }
        } catch {}
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw 'Installed daemon never served /status from its own listening process' }
    $result.daemon = $true
    Stop-OwnedProcess -Identity $owned[[string]$daemonProcess.Id]
    $daemonProcess.WaitForExit(10000) | Out-Null
    if (-not $daemonProcess.HasExited) { throw 'Installed standalone daemon did not stop' }

    $env:ENABLE_PLAYWRIGHT = 'true'
    $env:PLAYWRIGHT_CDP_PORT = [string](Get-FreePort)
    [Environment]::SetEnvironmentVariable('BIOROUTER_PORT', $null, 'Process')
    $desktop = Start-OwnedProcess -File (Join-Path $installed 'Biorouter.exe') `
        -Arguments @("--user-data-dir=`"$(Join-Path $root 'electron')`"") -Label 'desktop' -ShowWindow
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline -and -not $desktop.HasExited) {
        Update-OwnedProcesses
        $desktop.Refresh()
        $children = @($owned.Values | Where-Object { $_.ExecutablePath -eq $daemon -and
            $_.ProcessId -ne $daemonProcess.Id -and (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue) })
        $childPids = @($children | ForEach-Object { $_.ProcessId })
        $listeners = @()
        if ($childPids.Count -gt 0) {
            $listeners = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
                Where-Object { $_.OwningProcess -in $childPids })
        }
        try {
            $pages = @(Invoke-RestMethod "http://127.0.0.1:$env:PLAYWRIGHT_CDP_PORT/json/list" -TimeoutSec 2)
            $renderer = @($pages | Where-Object { $_.type -eq 'page' -and $_.url -like 'file:*' })
            if ($desktop.MainWindowHandle -ne 0 -and $listeners.Count -gt 0 -and $renderer.Count -gt 0) {
                $ready = $true
                $result.desktopDaemonPids = $childPids
                break
            }
        } catch {}
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw 'Installed desktop did not create its real window, packaged renderer and backend' }
    Start-Sleep -Seconds 5
    $desktop.Refresh()
    if ($desktop.HasExited) { throw 'Installed desktop exited after startup' }
    $result.desktop = $true
    Update-OwnedProcesses
    foreach ($identity in @($owned.Values | Sort-Object CreationDate -Descending)) {
        Stop-OwnedProcess -Identity $identity
    }
    Start-Sleep -Seconds 2
    & (Join-Path $PSScriptRoot 'normal-release-smoke.ps1') -AppDirectory $installed `
        -ZipAppDirectory $ZipAppDirectory -Version $Version -AssetEvidence $AssetEvidence `
        -Report (Join-Path (Split-Path -Parent $Report) 'normal/windows-normal-smoke.json')
    $result.normalDesktop = $true
    $result.passed = $true
} catch {
    $result.error = $_.Exception.Message
    throw
} finally {
    try {
        Update-OwnedProcesses
        foreach ($identity in @($owned.Values | Sort-Object CreationDate -Descending)) {
            Stop-OwnedProcess -Identity $identity
        }
        Start-Sleep -Seconds 2
        $remaining = @(Get-CimInstance Win32_Process | Where-Object {
            $identity = $owned[[string]$_.ProcessId]
            $identity -and $_.CreationDate -eq $identity.CreationDate -and $_.ExecutablePath -eq $identity.ExecutablePath
        })
        if ($remaining.Count -gt 0) { throw 'Owned installer/runtime processes remained after cleanup' }
        $updater = Join-Path $local 'biorouter_app/Update.exe'
        if (Test-Path -LiteralPath $updater) {
            $uninstaller = Start-OwnedProcess -File $updater -Arguments @('--uninstall', '--silent') -Label 'uninstall'
            Wait-OwnedExit -Process $uninstaller -Seconds 60
        }
        Update-OwnedProcesses
        foreach ($identity in @($owned.Values | Sort-Object CreationDate -Descending)) {
            Stop-OwnedProcess -Identity $identity
        }
        Start-Sleep -Seconds 2
        foreach ($identity in $owned.Values) {
            $current = Get-CimInstance Win32_Process -Filter "ProcessId=$($identity.ProcessId)"
            if ($current -and $current.CreationDate -eq $identity.CreationDate -and
                $current.ExecutablePath -eq $identity.ExecutablePath) {
                throw 'An owned process remained after the installer cleanup'
            }
        }
        $result.cleanup = $true
    } catch {
        $result.passed = $false
        $result.cleanupError = $_.Exception.Message
        throw
    } finally {
        foreach ($name in $changedEnvironment) {
            [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name], 'Process')
        }
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Report) | Out-Null
        foreach ($log in @(Get-ChildItem -LiteralPath $root -Filter '*.log' -File -ErrorAction SilentlyContinue)) {
            Copy-Item -LiteralPath $log.FullName -Destination (Join-Path (Split-Path -Parent $Report) $log.Name)
        }
        $setupLog = Join-Path $local 'SquirrelTemp/SquirrelSetup.log'
        if (Test-Path -LiteralPath $setupLog) {
            Copy-Item -LiteralPath $setupLog -Destination (Join-Path (Split-Path -Parent $Report) 'SquirrelSetup.log')
        }
        try {
            if ($result.cleanup -and (Test-Path -LiteralPath $root)) {
                Remove-Item -LiteralPath $root -Recurse -Force
            }
        } catch {
            $result.passed = $false
            $result.cleanup = $false
            $result.cleanupError = $_.Exception.Message
            throw
        } finally {
            $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Report -Encoding utf8
        }
    }
}
Write-Host "Setup.exe installed and started v$Version; $($result.files.Count) files match the uploaded ZIP"
