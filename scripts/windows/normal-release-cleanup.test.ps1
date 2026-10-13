$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tokens = $null
$errors = $null
$source = Join-Path $PSScriptRoot 'normal-release-smoke.ps1'
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count -gt 0) { throw 'The normal acceptance helper did not parse' }
$functions = @($ast.FindAll({ param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Stop-NormalOwnedProcess'
}, $true))
if ($functions.Count -ne 1) { throw 'Expected the one real owned-process cleanup implementation' }
Invoke-Expression $functions[0].Extent.Text
$calls = @($ast.FindAll({ param($node)
    $node -is [System.Management.Automation.Language.CommandAst] -and $node.GetCommandName() -eq 'Stop-NormalOwnedProcess'
}, $true))
if ($calls.Count -ne 1) { throw 'Normal teardown must use the tested owned-process helper' }
if ($IsWindows) {
    $self = Get-Process -Id $PID -ErrorAction Stop
    try {
        if ($self.get_Handle() -eq [IntPtr]::Zero) { throw 'The native process getter did not retain a handle' }
        $nativeIdentity = Get-CimInstance Win32_Process -Filter "ProcessId=$PID"
        if ($self.StartTime.ToUniversalTime().ToString('yyyyMMddHHmmssffffff') -cne
            $nativeIdentity.CreationDate.ToUniversalTime().ToString('yyyyMMddHHmmssffffff')) {
            throw 'Native process and CIM creation timestamps differ at microsecond precision'
        }
        Write-Host 'PASS native self-process UTC/CIM precision (read-only handle)'
    } finally { $self.Dispose() }
}

function Get-CimInstance {
    [CmdletBinding()]
    param([string]$ClassName, [string]$Filter)
    if ($ClassName -ne 'Win32_Process' -or $Filter -ne 'ProcessId=7116') { throw 'Unexpected process inventory request' }
    $script:state.inventory++
    if ($script:state.inventory -eq 1) {
        if ($script:state.case.initial -eq 'replacement') { return $script:replacement }
        return $script:identity
    }
    switch ($script:state.current) {
        'owned' { return $script:identity }
        'replacement' { return $script:replacement }
        'path-mismatch' { return $script:pathMismatch }
        'gone' { return $null }
        default { throw 'Unexpected synthetic process state' }
    }
}

function Get-Process {
    [CmdletBinding()]
    param([int]$Id)
    if ($Id -ne 7116) { throw 'Unexpected process acquisition' }
    $script:state.acquisitions++
    if ($script:state.case.lookup -eq 'gone') {
        $script:state.current = $script:state.case.after
        $record = [System.Management.Automation.ErrorRecord]::new(
            [ArgumentException]::new('Synthetic exited process'), 'NoProcessFoundForGivenId',
            [System.Management.Automation.ErrorCategory]::ObjectNotFound, $Id)
        $PSCmdlet.ThrowTerminatingError($record)
    }
    return $script:process
}

function Stop-Process { throw 'Cleanup must never kill through a fresh PID lookup' }

$script:identity = [pscustomobject]@{ ProcessId = 7116;
    CreationDate = [DateTime]::SpecifyKind([DateTime]::Parse('2026-10-13T07:09:00.123456'), [DateTimeKind]::Utc);
    ExecutablePath = 'C:\synthetic\Biorouter.exe' }
$script:replacement = [pscustomobject]@{ ProcessId = 7116;
    CreationDate = $script:identity.CreationDate.AddSeconds(1); ExecutablePath = $script:identity.ExecutablePath }
$script:pathMismatch = [pscustomobject]@{ ProcessId = 7116;
    CreationDate = $script:identity.CreationDate; ExecutablePath = 'C:\unrelated\Other.exe' }
$cases = @(
    @{ name = 'live-owned'; kill = 1 },
    @{ name = 'native-submicrosecond'; kill = 1; ticks = 9 },
    @{ name = 'local-start-time'; kill = 1; local = $true },
    @{ name = 'exit-before-acquire'; lookup = 'gone'; after = 'gone'; dispose = 0 },
    @{ name = 'pid-replaced-before-acquire'; lookup = 'gone'; after = 'replacement'; dispose = 0 },
    @{ name = 'missing-but-owned-live'; lookup = 'gone'; after = 'owned'; error = $true; dispose = 0 },
    @{ name = 'exit-opening-handle'; handleError = 'exited'; after = 'gone' },
    @{ name = 'gone-native-handle'; handleError = 'invalid-pid'; after = 'gone' },
    @{ name = 'handle-error-but-owned-live'; handleError = 'invalid-pid'; after = 'owned'; error = $true },
    @{ name = 'access-denied-even-if-gone'; handleError = 'denied'; after = 'gone'; error = $true },
    @{ name = 'generic-error-even-if-gone'; handleError = 'other'; after = 'gone'; error = $true },
    @{ name = 'exit-after-handle'; after = 'gone' },
    @{ name = 'exited-captured-instance'; exited = $true },
    @{ name = 'pid-replaced-after-handle'; after = 'replacement' },
    @{ name = 'path-mismatch-after-handle'; after = 'path-mismatch' },
    @{ name = 'unrelated-at-initial-check'; initial = 'replacement'; acquisitions = 0; dispose = 0 },
    @{ name = 'captured-start-mismatch'; ticks = 10; error = $true },
    @{ name = 'captured-pid-mismatch'; capturedId = 9999; error = $true },
    @{ name = 'exit-between-check-and-kill'; killError = 'exited'; kill = 1 },
    @{ name = 'kill-access-denied'; killError = 'denied'; kill = 1; error = $true },
    @{ name = 'kill-failure-even-if-gone'; killError = 'other'; kill = 1; error = $true }
)
$passed = 0
foreach ($case in $cases) {
    foreach ($entry in @{ initial = 'owned'; lookup = 'live'; after = 'owned'; handleError = '';
        killError = ''; kill = 0; error = $false; dispose = 1; acquisitions = 1;
        ticks = 0; local = $false; exited = $false; capturedId = 7116 }.GetEnumerator()) {
        if (-not $case.ContainsKey($entry.Key)) { $case[$entry.Key] = $entry.Value }
    }
    $script:state = @{ case = $case; inventory = 0; acquisitions = 0; handles = 0;
        kills = 0; disposals = 0; current = 'owned' }
    $start = $script:identity.CreationDate.AddTicks($case.ticks)
    if ($case.local) { $start = $start.ToLocalTime() }
    $script:process = [pscustomobject]@{ Id = $case.capturedId; StartTime = $start; HasExited = $case.exited }
    $script:process | Add-Member ScriptMethod get_Handle {
        $script:state.handles++
        $script:state.current = $script:state.case.after
        switch ($script:state.case.handleError) {
            'exited' { throw [InvalidOperationException]::new('Synthetic exited process') }
            'invalid-pid' { throw [ComponentModel.Win32Exception]::new(87) }
            'denied' { throw [ComponentModel.Win32Exception]::new(5) }
            'other' { throw [Exception]::new('Synthetic handle failure') }
        }
        return [IntPtr]123
    }
    $script:process | Add-Member ScriptMethod Kill {
        if ($script:state.handles -ne 1) { throw 'Cleanup did not pin the process before kill' }
        $script:state.kills++
        switch ($script:state.case.killError) {
            'exited' {
                $script:state.current = 'gone'
                $this.HasExited = $true
            }
            'denied' { throw [ComponentModel.Win32Exception]::new(5) }
            'other' {
                $script:state.current = 'gone'
                throw [Exception]::new('Synthetic live kill failure')
            }
        }
    }
    $script:process | Add-Member ScriptMethod Dispose { $script:state.disposals++ }
    $failure = $null
    try { Stop-NormalOwnedProcess -Identity $script:identity } catch { $failure = $_ }
    if (($null -ne $failure) -ne $case.error -or $script:state.kills -ne $case.kill -or
        $script:state.acquisitions -ne $case.acquisitions -or $script:state.disposals -ne $case.dispose) {
        throw ("Cleanup case failed: $($case.name); expectedError=$($case.error); " +
            "actualError=$($null -ne $failure); kills=$($script:state.kills); " +
            "acquisitions=$($script:state.acquisitions); disposals=$($script:state.disposals)")
    }
    $passed++
    Write-Host "PASS owned cleanup $($case.name)"
}
if ($passed -ne 21) { throw 'Not all owned-process cleanup assertions executed' }
Write-Host 'Owned-process cleanup: 21/21 passed (synthetic process methods; no processes started or stopped)'
