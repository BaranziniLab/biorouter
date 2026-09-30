[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$AppDirectory,
    [Parameter(Mandatory = $true)][string]$ZipAppDirectory,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$AssetEvidence,
    [Parameter(Mandatory = $true)][string]$Report
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'release-startup-diagnostics.ps1')
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Normal startup acceptance requires a disposable Windows GitHub Actions runner.'
}
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw 'Expected a stable release version' }
if ([Version]$Version -lt [Version]'1.92.0') {
    throw 'Normal startup acceptance requires v1.92.0 or later with explicit private user-data-dir support.'
}
$AppDirectory = (Resolve-Path -LiteralPath $AppDirectory).Path
$ZipAppDirectory = (Resolve-Path -LiteralPath $ZipAppDirectory).Path
$Report = [IO.Path]::GetFullPath($Report)
$root = Join-Path $env:RUNNER_TEMP ('br-normal-' + [Guid]::NewGuid().ToString('N'))
$normalUserDataDirectory = Join-Path $root 'electron'
$desktopPath = Join-Path $AppDirectory 'Biorouter.exe'
$daemonPath = Join-Path $AppDirectory 'resources/bin/biorouterd.exe'
$owned = @{}
$desktop = $null
$cdpPort = 0
$environmentNames = @('ENABLE_PLAYWRIGHT', 'PLAYWRIGHT_CDP_PORT', 'BIOROUTER_DEV_PROFILE_ROOT',
    'BIOROUTER_DEV_PROFILE_NAME', 'BIOROUTER_DEV_AUTO_CONFIRM_SHARE', 'BIOROUTER_SHARED_DAEMON',
    'BIOROUTER_EXTERNAL_BACKEND', 'BIOROUTER_EXTERNAL_BACKEND_URL', 'BIOROUTER_PORT',
    'BIOROUTER_SERVER__SECRET_KEY', 'BIOROUTER_PATH_ROOT', 'BIOROUTER_DISABLE_KEYRING',
    'LOCALAPPDATA', 'APPDATA', 'HOME', 'USERPROFILE')
$originalEnvironment = @{}
foreach ($name in $environmentNames) {
    $originalEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$result = [ordered]@{ version = $Version; passed = $false; cleanup = $false;
    mode = 'normal Windows app-owned TCP backend; generated in-memory authentication and user-action proof';
    persistentProfileRuntimeSupported = $false; files = @(); samples = @(); assets = @();
    secondWindow = $false }

function Update-OwnedProcesses {
    $processes = @(Get-CimInstance Win32_Process)
    do {
        $added = $false
        foreach ($item in $processes) {
            $key = [string]$item.ProcessId
            if ($owned.ContainsKey($key)) { continue }
            $parent = $owned[[string]$item.ParentProcessId]
            if ($parent -and $item.CreationDate -ge $parent.CreationDate -and
                @($processes | Where-Object { $_.ProcessId -eq $parent.ProcessId -and
                    $_.CreationDate -eq $parent.CreationDate -and $_.ExecutablePath -eq $parent.ExecutablePath }).Count -eq 1) {
                $owned[$key] = $item
                $added = $true
            }
        }
    } while ($added)
}

function Invoke-CDP {
    param([string]$Socket, [string]$Method, [hashtable]$Parameters)
    $websocket = [Net.WebSockets.ClientWebSocket]::new()
    $timeout = [Threading.CancellationTokenSource]::new(10000)
    try {
        $websocket.ConnectAsync([Uri]$Socket, $timeout.Token).GetAwaiter().GetResult()
        $request = @{ id = 1; method = $Method; params = $Parameters } | ConvertTo-Json -Depth 10 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($request)
        $websocket.SendAsync([ArraySegment[byte]]::new($bytes),
            [Net.WebSockets.WebSocketMessageType]::Text, $true, $timeout.Token).GetAwaiter().GetResult()
        $buffer = [byte[]]::new(65536)
        while ($true) {
            $message = [IO.MemoryStream]::new()
            try {
                do {
                    $received = $websocket.ReceiveAsync([ArraySegment[byte]]::new($buffer), $timeout.Token).GetAwaiter().GetResult()
                    if ($received.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) {
                        throw 'Renderer inspector closed before answering'
                    }
                    $message.Write($buffer, 0, $received.Count)
                } while (-not $received.EndOfMessage)
                $response = [Text.Encoding]::UTF8.GetString($message.ToArray()) | ConvertFrom-Json
            } finally { $message.Dispose() }
            if ($response.PSObject.Properties.Name -contains 'id' -and $response.id -eq 1) {
                if ($response.PSObject.Properties.Name -contains 'error') { throw 'Renderer inspector rejected the request' }
                return $response.result
            }
        }
    } finally {
        $timeout.Dispose()
        $websocket.Dispose()
    }
}

function Assert-NoNativeSecretPrompt {
    $windows = [Windows.Automation.AutomationElement]::RootElement.FindAll(
        [Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition)
    foreach ($window in $windows) {
        if (-not $owned.ContainsKey([string]$window.Current.ProcessId)) { continue }
        $elements = $window.FindAll([Windows.Automation.TreeScope]::Descendants,
            [Windows.Automation.Condition]::TrueCondition)
        $names = @($window.Current.Name)
        foreach ($element in $elements) { $names += $element.Current.Name }
        if (($names -join ' ') -match '(?i)temporary\s+password|daemon\s+approval|approval\s+secret|enter.{0,40}background service.{0,30}(password|secret)') {
            throw 'Normal startup displayed a native temporary-password or daemon approval conversation'
        }
    }
}

$expression = @'
(async () => {
  const api = window.electron;
  if (!api || !document.body || document.readyState !== 'complete') return {ready:false};
  const text = document.body.innerText;
  if (/temporary\s+password|daemon\s+approval|approval\s+secret|enter.{0,40}background service.{0,30}(password|secret)/i.test(text))
    throw new Error('Normal startup displayed a temporary-password or daemon approval conversation');
  const host = await api.getBiorouterdHostPort();
  if (!host) return {ready:false};
  const url = new URL(host);
  if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1') throw new Error('Backend is not owned loopback TCP');
  const secret = await api.getSecretKey();
  const proof = await api.getUserActionKey();
  if (!/^[0-9a-f]{64}$/i.test(secret) || !/^[0-9a-f]{64}$/i.test(proof) || secret === proof)
    throw new Error('Normal startup did not generate separate authentication and user-action proof');
  const request = (path, headers) => fetch(host + path, {headers, signal:AbortSignal.timeout(4000)});
  const status = await request('/status', {'X-Secret-Key':secret});
  const info = await request('/system_info', {'X-Secret-Key':secret});
  const unauthorized = await request('/system_info', {});
  const noProof = await request('/crew/connections', {'X-Secret-Key':secret});
  const authorized = await request('/crew/connections', {'X-Secret-Key':secret, 'X-User-Action':proof});
  const system = info.ok ? await info.json() : {};
  return {ready: text.trim().length > 50 && status.status === 200 && info.status === 200 &&
      [401,403].includes(unauthorized.status) && noProof.status === 403 && authorized.status === 200,
    port:Number(url.port), renderer:location.protocol, textLength:text.trim().length,
    status:status.status, authenticated:info.status, unauthenticated:unauthorized.status,
    missingProof:noProof.status, generatedProof:authorized.status, appVersion:system.app_version,
    generatedSecret:true, generatedUserActionProof:true, secretPrompt:false};
})()
'@

try {
    New-Item -ItemType Directory -Force -Path $normalUserDataDirectory, (Join-Path $root 'local'), (Join-Path $root 'roaming'),
        (Join-Path $root 'home'), (Split-Path -Parent $Report) | Out-Null
    $result.assets = @(Get-Content -LiteralPath $AssetEvidence -Raw | ConvertFrom-Json)
    if ($result.assets.Count -ne 2) { throw 'Expected authenticated ZIP and Setup asset evidence' }
    foreach ($asset in $result.assets) {
        $digest = (Get-FileHash -LiteralPath $asset.path -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($asset.digest -cne "sha256:$digest" -or (Get-Item -LiteralPath $asset.path).Length -ne $asset.size) {
            throw 'Release asset no longer matches authenticated GitHub metadata'
        }
    }
    foreach ($relative in @('Biorouter.exe', 'resources/bin/biorouter.exe', 'resources/bin/biorouterd.exe',
        'resources/computer-use/manifest.json')) {
        $actual = Join-Path $AppDirectory $relative
        $expected = Join-Path $ZipAppDirectory $relative
        $hash = (Get-FileHash -LiteralPath $actual -Algorithm SHA256).Hash
        if ($hash -cne (Get-FileHash -LiteralPath $expected -Algorithm SHA256).Hash) {
            throw "Normal startup payload differs from authenticated ZIP: $relative"
        }
        $result.files += @{ path = $relative; sha256 = $hash.ToLowerInvariant() }
    }
    foreach ($binary in @((Join-Path $AppDirectory 'resources/bin/biorouter.exe'), $daemonPath)) {
        $reported = & $binary --version
        if ($LASTEXITCODE -ne 0 -or $reported -notmatch "\b$([regex]::Escape($Version))\b") {
            throw 'Normal startup executable version differs from the release'
        }
    }
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
    $env:BIOROUTER_PATH_ROOT = Join-Path $root 'profile'
    $env:BIOROUTER_DISABLE_KEYRING = 'true'
    $env:LOCALAPPDATA = Join-Path $root 'local'
    $env:APPDATA = Join-Path $root 'roaming'
    $env:HOME = Join-Path $root 'home'
    $env:USERPROFILE = $env:HOME
    $portListener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $portListener.Start()
    $cdpPort = $portListener.LocalEndpoint.Port
    $portListener.Stop()
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $desktop = Start-Process -FilePath $desktopPath -PassThru -ArgumentList @(
        "--user-data-dir=`"$normalUserDataDirectory`"", "--remote-debugging-port=$cdpPort", '--remote-debugging-address=127.0.0.1') `
        -RedirectStandardOutput (Join-Path $root 'normal.stdout.log') `
        -RedirectStandardError (Join-Path $root 'normal.stderr.log')
    $identity = Get-CimInstance Win32_Process -Filter "ProcessId=$($desktop.Id)"
    if (-not $identity -or $identity.ExecutablePath -ne $desktopPath) { throw 'Normal desktop process identity did not match payload' }
    $owned[[string]$desktop.Id] = $identity
    $result.desktopPid = $desktop.Id
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $ready = $false
    $secondWindowRequested = $false
    $firstPageId = $null
    while ([DateTime]::UtcNow -lt $deadline -and -not $desktop.HasExited) {
        Update-OwnedProcesses
        Assert-NoNativeSecretPrompt
        $desktop.Refresh()
        $pages = @()
        try { $pages = @(Invoke-RestMethod "http://127.0.0.1:$cdpPort/json/list" -TimeoutSec 2) } catch {}
        foreach ($page in @($pages | Where-Object { $_.type -eq 'page' -and $_.url -like 'file:*' })) {
            $evaluation = Invoke-CDP -Socket $page.webSocketDebuggerUrl -Method 'Runtime.evaluate' `
                -Parameters @{ expression = $expression; awaitPromise = $true; returnByValue = $true }
            if ($evaluation.PSObject.Properties.Name -contains 'exceptionDetails') {
                throw 'Normal renderer rejected the authentication or no-secret-prompt acceptance assertions'
            }
            $sample = $evaluation.result.value
            if (-not $sample.ready) { continue }
            if ($sample.appVersion -ne $Version) { throw 'Live packaged backend version differs from the release' }
            $children = @($owned.Values | Where-Object { $_.ExecutablePath -eq $daemonPath -and
                (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue) })
            $childPids = @($children | ForEach-Object { $_.ProcessId })
            $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $sample.port -ErrorAction SilentlyContinue |
                Where-Object { $_.OwningProcess -in $childPids })
            $visibleWindows = @([Windows.Automation.AutomationElement]::RootElement.FindAll(
                [Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition) |
                Where-Object { $_.Current.ProcessId -eq $desktop.Id -and -not $_.Current.IsOffscreen })
            if ($desktop.MainWindowHandle -eq 0 -or $visibleWindows.Count -eq 0 -or
                $children.Count -ne 1 -or $listeners.Count -eq 0) { continue }
            $result.visibleWindows = $visibleWindows.Count
            if ($result.samples.Count -gt 0 -and $sample.port -ne $result.samples[0].port) {
                throw 'Normal windows did not share the same app-owned backend'
            }
            $result.samples += $sample
            $result.daemonPid = $children[0].ProcessId
            if (-not $secondWindowRequested) {
                $opened = Invoke-CDP -Socket $page.webSocketDebuggerUrl -Method 'Runtime.evaluate' `
                    -Parameters @{ expression = 'window.electron.createChatWindow(); true'; returnByValue = $true }
                if ($opened.PSObject.Properties.Name -contains 'exceptionDetails') { throw 'Normal second window could not be opened' }
                $firstPageId = $page.id
                $secondWindowRequested = $true
            }
            if ($page.id -ne $firstPageId) { $result.secondWindow = $true }
            if ($result.samples.Count -ge 5 -and $result.secondWindow -and $visibleWindows.Count -ge 2) {
                $image = Invoke-CDP -Socket $page.webSocketDebuggerUrl -Method 'Page.captureScreenshot' -Parameters @{}
                [IO.File]::WriteAllBytes((Join-Path (Split-Path -Parent $Report) 'normal-renderer.png'),
                    [Convert]::FromBase64String($image.data))
                $ready = $true
            }
            if ($ready) { break }
        }
        if ($ready) { break }
        Start-Sleep -Seconds 2
    }
    if (-not $ready) { throw 'Normal desktop did not prove visible renderer, generated proof and owned authenticated backend readiness' }
    foreach ($name in @('runtime.json', 'user-action-key.json')) {
        if (Test-Path -LiteralPath (Join-Path $env:BIOROUTER_PATH_ROOT "state/daemon/$name")) {
            throw 'Windows unexpectedly created an unsupported persistent profile daemon runtime'
        }
    }
    $result.passed = $true
} catch {
    $result.error = Protect-ReleaseDiagnosticText -Text $_.Exception.Message
    try {
        Update-OwnedProcesses
        $result.startupDiagnostics = Get-ReleaseStartupSnapshot -Owned $owned -Desktop $desktop `
            -DaemonPath $daemonPath -CDPPort $cdpPort
    } catch { $result.startupDiagnosticError = Protect-ReleaseDiagnosticText -Text $_.Exception.Message }
    throw
} finally {
    try {
        Update-OwnedProcesses
        foreach ($identity in @($owned.Values | Sort-Object CreationDate -Descending)) {
            $current = Get-CimInstance Win32_Process -Filter "ProcessId=$($identity.ProcessId)"
            if ($current -and $current.CreationDate -eq $identity.CreationDate -and
                $current.ExecutablePath -eq $identity.ExecutablePath) { Stop-Process -Id $identity.ProcessId -Force }
        }
        Start-Sleep -Seconds 2
        foreach ($identity in $owned.Values) {
            $current = Get-CimInstance Win32_Process -Filter "ProcessId=$($identity.ProcessId)"
            if ($current -and $current.CreationDate -eq $identity.CreationDate -and
                $current.ExecutablePath -eq $identity.ExecutablePath) { throw 'An owned normal-start process survived cleanup' }
        }
        $result.cleanup = $true
    } catch {
        $result.passed = $false
        $result.cleanupError = $_.Exception.Message
        throw
    } finally {
        foreach ($name in $environmentNames) {
            [Environment]::SetEnvironmentVariable($name, $originalEnvironment[$name], 'Process')
        }
        foreach ($log in @(Get-ChildItem -LiteralPath $root -Filter '*.log' -File -Recurse -ErrorAction SilentlyContinue)) {
            if ($log.DirectoryName -eq $root -or $log.DirectoryName -eq (Join-Path $normalUserDataDirectory 'logs')) {
                Copy-RedactedReleaseLog -Source $log.FullName -Destination (Join-Path (Split-Path -Parent $Report) $log.Name)
            }
        }
        try {
            if ($result.cleanup -and (Test-Path -LiteralPath $root)) { Remove-Item -LiteralPath $root -Recurse -Force }
        } finally { $result | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $Report -Encoding utf8 }
    }
}
Write-Host "Normal Windows startup verified v$Version without a daemon approval conversation"
