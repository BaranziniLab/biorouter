# Does a packaged Windows daemon still hold its port after it dies, because a
# child process inherited the listening socket?
#
# ⚠ The obvious version of this test PROVES NOTHING, and it was written first:
# start the daemon, stop it, see whether the port is free. That passes on a
# BROKEN build, because the only child present is conhost.exe, which Windows
# spawns for the console — not a child created through the daemon's OWN spawn
# path with bInheritHandles TRUE. A listening socket can only be leaked to a
# child the daemon itself spawned, so a run with no such child cannot fail.
#
# So this points the daemon at a stand-in sidecar (BIOROUTER_LLAMACPP_BIN) that
# ignores its arguments and sleeps, asks the daemon to ensure that sidecar — the
# daemon spawns it while the HTTP listener is open — then stops ONLY the daemon,
# by exact pid, and deliberately leaves the child alive. That is precisely the
# condition under which an inherited handle keeps the port bound.
#
# Every outcome that means "this run did not exercise the bug" EXITS NON-ZERO.
# In CI nobody reads the output, so a run that silently stops spawning the child
# would go green forever on a build with the defect back, occupying the slot
# where a real test would sit.
#
# Exit codes: 0 the port was released; 1 it was still held, or the run could not
# exercise the bug (which is a failure of this test, not a pass of the build).
#
# Measured when it was written (2026-09-22): the v1.91.0 daemon holds the port
# past 15 s, with the listener attributed to the dead daemon's pid and an empty
# process name; the v1.91.1 daemon frees it in 0 s. Two trials each, order
# reversed, fresh ports, the arms differing only in the binary.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Daemon,
    [Parameter(Mandatory = $true)][string]$Sleeper,
    [int]$Port = 0,
    [string]$Label = 'port release'
)

$ErrorActionPreference = 'Continue'
$secret = 'porttest'
$held = $null
$root = $null
$daemonProc = $null
$spawned = @()

function Test-PortBindable {
    param([int]$P)
    try {
        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $P)
        $listener.Start(); $listener.Stop(); return $true
    } catch { return $false }
}

function Fail {
    param([string]$Message)
    Write-Output "  FAIL: $Message"
    $script:failed = $true
}

function Invoke-PortReleaseTest {
    Write-Output "---- $Label ----"
    try {
        if (-not (Test-Path -LiteralPath $Daemon)) { Fail "no daemon at $Daemon"; return }
        if (-not (Test-Path -LiteralPath $Sleeper)) { Fail "no stand-in sidecar at $Sleeper"; return }

        # A port OUTSIDE the ephemeral range, so the kernel never hands this number
        # to anything else while the test runs.
        if ($Port -le 0) {
            foreach ($candidate in 17000..17099) {
                if (Test-PortBindable -P $candidate) { $Port = $candidate; break }
            }
        }
        if ($Port -le 0 -or -not (Test-PortBindable -P $Port)) { Fail "no free port in 17000-17099"; return }
        Write-Output "  port       : $Port"

        $root = Join-Path $env:TEMP ("br-portrel-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
        New-Item -ItemType Directory -Path $root -Force | Out-Null

        $env:BIOROUTER_PATH_ROOT = $root
        $env:BIOROUTER_DISABLE_KEYRING = 'true'
        $env:BIOROUTER_PORT = "$Port"
        $env:BIOROUTER_SERVER__SECRET_KEY = $secret
        $env:BIOROUTER_LLAMACPP_BIN = $Sleeper

        $daemonProc = Start-Process -FilePath $Daemon -ArgumentList 'agent' -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput "$root\out.txt" -RedirectStandardError "$root\err.txt"
        Write-Output "  daemon pid : $($daemonProc.Id)"

        $headers = @{ 'X-Secret-Key' = $secret; 'X-User-Action' = '1' }
        $up = $false
        for ($i = 0; $i -lt 60; $i++) {
            Start-Sleep -Milliseconds 500
            try {
                $probe = Invoke-WebRequest "http://127.0.0.1:$Port/status" -Headers $headers -TimeoutSec 3 -UseBasicParsing
                if ($probe.StatusCode -eq 200) { $up = $true; break }
            } catch {}
        }
        Write-Output "  listening  : $up"
        if (-not $up) { Fail "the daemon never answered /status, so nothing was exercised"; return }

        # The model NAME comes from the daemon's own catalog. A hard-coded name that
        # leaves the catalog turns into a 422 — a run that spawns nothing and would
        # otherwise pass.
        $model = $null
        try {
            $status = Invoke-RestMethod "http://127.0.0.1:$Port/llamacpp/status" -Headers $headers -TimeoutSec 30
            $model = @($status.catalog | ForEach-Object { $_.name })[0]
        } catch { Write-Output "  /llamacpp/status: $($_.Exception.Message)" }
        if ([string]::IsNullOrWhiteSpace($model)) { Fail "the daemon's llamacpp catalog named no model to ensure"; return }
        Write-Output "  model      : $model"

        try {
            Invoke-RestMethod "http://127.0.0.1:$Port/llamacpp/ensure" -Method Post -Headers $headers `
                -ContentType 'application/json' -Body (@{ model = $model } | ConvertTo-Json -Compress) -TimeoutSec 60 | Out-Null
            Write-Output "  ensure     : accepted"
        } catch { Write-Output "  ensure     : $($_.Exception.Message)" }

        for ($i = 0; $i -lt 20; $i++) {
            Start-Sleep -Milliseconds 500
            $spawned = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($daemonProc.Id)" |
                Where-Object { $_.Name -ne 'conhost.exe' })
            if ($spawned.Count) { break }
        }
        if (-not $spawned.Count) {
            Fail "the daemon spawned no child, so nothing could inherit the socket and this run cannot fail on a broken build"
            return
        }
        Write-Output "  children   : $(($spawned | ForEach-Object { "$($_.Name):$($_.ProcessId)" }) -join ', ')"

        Write-Output "  stopping ONLY the daemon (pid $($daemonProc.Id)); its child is left alive on purpose"
        try { Stop-Process -Id $daemonProc.Id -Force -ErrorAction Stop } catch {}
        $daemonProc.WaitForExit(30000) | Out-Null
        if (-not $daemonProc.HasExited) { Fail "the daemon did not exit, so the port test would be meaningless"; return }

        $alive = @($spawned | Where-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue })
        if (-not $alive.Count) {
            Fail "every spawned child died with the daemon, so no child was left to hold the socket"
            return
        }
        Write-Output "  still alive: $(($alive | ForEach-Object { "$($_.Name):$($_.ProcessId)" }) -join ', ')"

        $watch = [Diagnostics.Stopwatch]::StartNew()
        $freeAt = $null
        for ($i = 0; $i -lt 60; $i++) {
            if (Test-PortBindable -P $Port) { $freeAt = $watch.Elapsed.TotalSeconds; break }
            Start-Sleep -Milliseconds 250
        }
        $watch.Stop()

        if ($null -ne $freeAt) {
            Write-Output "  RESULT     : port rebindable after $([math]::Round($freeAt, 2))s"
        } else {
            $held = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
                Select-Object -ExpandProperty OwningProcess -Unique)
            foreach ($owner in $held) {
                $name = (Get-Process -Id $owner -ErrorAction SilentlyContinue).ProcessName
                Write-Output "    held by pid $owner ($name)"
            }
            Fail "PORT STILL HELD 15s after the daemon died: a child inherited its listening socket"
        }
        } finally {
        foreach ($child in $spawned) {
            if (Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue) {
                try { Stop-Process -Id $child.ProcessId -Force -ErrorAction Stop } catch {}
            }
        }
        if ($null -ne $daemonProc -and -not $daemonProc.HasExited) {
            try { Stop-Process -Id $daemonProc.Id -Force -ErrorAction Stop } catch {}
        }
        if ($null -ne $root) { Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue }
    }
}

# ⚠ The body is a function, and this decision is OUTSIDE it, because `return` at
# a script's top level ends the script: written inline, every `Fail`-then-return
# path would have exited 0 — a test that cannot fail, which is the exact shape
# this test exists to catch.
$script:failed = $false
Invoke-PortReleaseTest
if ($script:failed) { exit 1 }
Write-Output "  PASS: the packaged daemon released its port with a spawned child still alive"
exit 0
