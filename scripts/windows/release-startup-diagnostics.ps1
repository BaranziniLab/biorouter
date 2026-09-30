function Protect-ReleaseDiagnosticText {
    param([AllowEmptyString()][string]$Text)
    $safe = [regex]::Replace([string]$Text,
        '(?im)^.*(?:authorization|token|credential|\bsk-[A-Za-z0-9_-]+|\bgh[pousr]_[A-Za-z0-9_]+|\bgithub_pat_[A-Za-z0-9_]+|\bBearer\s+|x-secret-key|x-user-action|api[_ -]?key|api_secret|user_action_key|secret[_ -]?key|password|passphrase|\benv(?:ironment)?\b|\bargv\b|\barguments\b|\bargs\b|\bparams\b|\bparameters\b|\bcommand.?line\b|\bheaders\b).*$',
        '[redacted sensitive diagnostic line]')
    return [regex]::Replace($safe, '(?i)\b[0-9a-f]{32,128}\b', '[redacted long hex value]')
}

function Copy-RedactedReleaseLog {
    param([string]$Source, [string]$Destination)
    if (Test-Path -LiteralPath $Source -PathType Leaf) {
        Protect-ReleaseDiagnosticText -Text ([string](Get-Content -LiteralPath $Source -Raw)) |
            Set-Content -LiteralPath $Destination -Encoding utf8
    }
}

function Get-ReleaseStartupSnapshot {
    param([Collections.IDictionary]$Owned, [Diagnostics.Process]$Desktop,
        [string]$DaemonPath, [int]$CDPPort)
    $snapshot = [ordered]@{ processes = @(); daemonChildren = @(); listeners = @();
        fileTargets = @(); cdpTargetCount = 0; visibleOwnedWindows = $null }
    $currentProcesses = @(Get-CimInstance Win32_Process)
    foreach ($identity in $Owned.Values) {
        $live = @($currentProcesses | Where-Object { $_.ProcessId -eq $identity.ProcessId -and
            $_.CreationDate -eq $identity.CreationDate -and $_.ExecutablePath -eq $identity.ExecutablePath })
        $item = @{ pid = $identity.ProcessId; parentPid = $identity.ParentProcessId;
            executable = $identity.ExecutablePath; created = $identity.CreationDate; alive = $live.Count -eq 1 }
        $snapshot.processes += $item
        if ($identity.ExecutablePath -eq $DaemonPath) { $snapshot.daemonChildren += $item }
    }
    if ($Desktop) {
        $Desktop.Refresh()
        $snapshot.desktop = @{ pid = $Desktop.Id; exited = $Desktop.HasExited; mainWindowHandle = '0' }
        if ($Desktop.HasExited) { $snapshot.desktop.exitCode = $Desktop.ExitCode }
        else { $snapshot.desktop.mainWindowHandle = [string]$Desktop.MainWindowHandle }
    }
    try {
        Add-Type -AssemblyName UIAutomationClient
        Add-Type -AssemblyName UIAutomationTypes
        $windows = [Windows.Automation.AutomationElement]::RootElement.FindAll(
            [Windows.Automation.TreeScope]::Children, [Windows.Automation.Condition]::TrueCondition)
        $snapshot.visibleOwnedWindows = @($windows | Where-Object {
            $Owned.Contains([string]$_.Current.ProcessId) -and -not $_.Current.IsOffscreen }).Count
    } catch { $snapshot.windowObservationError = 'Owned window enumeration was unavailable' }
    $ownedPids = @($Owned.Values | ForEach-Object { $_.ProcessId })
    $snapshot.listeners = @(Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
        Where-Object { $_.OwningProcess -in $ownedPids -or ($CDPPort -gt 0 -and $_.LocalPort -eq $CDPPort) } |
        ForEach-Object { @{ address = $_.LocalAddress; port = $_.LocalPort; ownerPid = $_.OwningProcess } })
    if ($CDPPort -gt 0) {
        try {
            $targets = @(Invoke-RestMethod "http://127.0.0.1:$CDPPort/json/list" -TimeoutSec 2)
            $snapshot.cdpTargetCount = $targets.Count
            $snapshot.fileTargets = @($targets | Where-Object { $_.type -eq 'page' -and $_.url -like 'file:*' } |
                ForEach-Object { @{ id = $_.id; type = $_.type;
                    url = ([Uri]$_.url).GetLeftPart([UriPartial]::Path) } })
        } catch { $snapshot.cdpObservationError = 'CDP target listing was unavailable' }
    }
    return $snapshot
}
