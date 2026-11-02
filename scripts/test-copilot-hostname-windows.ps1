param(
    [Parameter(Mandatory = $true)][string]$TestBinary,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$TestBinary = (Resolve-Path -LiteralPath $TestBinary).Path
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null
$RunDirectory = Join-Path $OutputDirectory ('run-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($RunDirectory) | Out-Null

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class CopilotConsoleProbe {
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
    private struct StartupInfo {
        public uint cb;
        public string lpReserved, lpDesktop, lpTitle;
        public uint dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public ushort wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInformation {
        public IntPtr hProcess, hThread;
        public uint dwProcessId, dwThreadId;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern bool CreateProcessW(
        string application, System.Text.StringBuilder commandLine,
        IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles,
        uint creationFlags, IntPtr environment, string directory,
        ref StartupInfo startup, out ProcessInformation process);
    [DllImport("kernel32.dll")]
    private static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError=true)]
    private static extern bool AttachConsole(uint processId);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool FreeConsole();
    [DllImport("kernel32.dll")]
    public static extern IntPtr GetConsoleWindow();
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);

    public static int AttachConsoleError(uint processId) {
        return AttachConsole(processId) ? 0 : Marshal.GetLastWin32Error();
    }

    public static uint StartVisibleControl(string executable) {
        var startup = new StartupInfo {
            cb = (uint)Marshal.SizeOf(typeof(StartupInfo)),
            lpTitle = "Biorouter hostname console regression positive control",
            dwFlags = 1,
            wShowWindow = 5,
        };
        var arguments = new System.Text.StringBuilder(
            "\"" + executable + "\" --exact windows_visible_console_fixture --ignored --nocapture --test-threads=1");
        ProcessInformation process;
        if (!CreateProcessW(executable, arguments, IntPtr.Zero, IntPtr.Zero, false,
                            0x00000010, IntPtr.Zero, null, ref startup, out process)) {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
        return process.dwProcessId;
    }
}
'@

function Save-Screenshot([string]$Path) {
    $screen = [Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = [Drawing.Bitmap]::new($screen.Width, $screen.Height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($screen.X, $screen.Y, 0, 0, $bitmap.Size)
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

function Measure-VisibleControl {
    $directory = Join-Path $RunDirectory 'visible-control'
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $prior = [Environment]::GetEnvironmentVariable('BIOROUTER_HOSTNAME_VISIBLE_CONTROL')
    $process = $null
    $attached = $false
    try {
        [CopilotConsoleProbe]::FreeConsole() | Out-Null
        [Environment]::SetEnvironmentVariable('BIOROUTER_HOSTNAME_VISIBLE_CONTROL', $directory)
        $childId = [CopilotConsoleProbe]::StartVisibleControl($TestBinary)
        $process = [Diagnostics.Process]::GetProcessById($childId)
        $null = $process.Handle
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while (!(Test-Path -LiteralPath (Join-Path $directory 'ready'))) {
            if ($process.HasExited) { throw 'Visible-console control exited before its handshake' }
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Visible-console control did not become ready' }
            Start-Sleep -Milliseconds 25
        }
        $attachError = [CopilotConsoleProbe]::AttachConsoleError([uint32]$childId)
        if ($attachError -ne 0) {
            throw "Cannot attach to explicit CREATE_NEW_CONSOLE control (error $attachError)"
        }
        $attached = $true
        $window = [CopilotConsoleProbe]::GetConsoleWindow()
        if ($window -eq [IntPtr]::Zero -or ![CopilotConsoleProbe]::IsWindowVisible($window)) {
            throw 'Positive control has no visible native console; the observer cannot certify visibility'
        }
        Save-Screenshot (Join-Path $directory 'visible-console-control.png')
        $verdict = [ordered]@{
            pid = $childId
            creation_flags = 'CREATE_NEW_CONSOLE'
            console_window_present = $true
            console_window_visible = $true
        }
        $verdict | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $directory 'evidence.json') -Encoding utf8
    } finally {
        if ($attached) { [CopilotConsoleProbe]::FreeConsole() | Out-Null }
        [IO.File]::WriteAllText((Join-Path $directory 'release'), 'observed')
        if ($process) {
            if (!$process.WaitForExit(5000)) { $process.Kill($true); $process.WaitForExit() }
            $exitCode = $process.ExitCode
            $process.Dispose()
        }
        [Environment]::SetEnvironmentVariable('BIOROUTER_HOSTNAME_VISIBLE_CONTROL', $prior)
    }
    if ([CopilotConsoleProbe]::GetConsoleWindow() -ne [IntPtr]::Zero) {
        throw 'Observer did not return to a consoleless state after calibration'
    }
    if ($exitCode -ne 0) { throw "Visible-console control failed with exit $exitCode" }
    return $verdict
}

function Measure-Polling([string]$Role) {
    $directory = Join-Path $RunDirectory $Role
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $start = [Diagnostics.ProcessStartInfo]::new($TestBinary)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in @('--exact', 'windows_hostname_fixture', '--ignored', '--nocapture', '--test-threads=1')) {
        $start.ArgumentList.Add($argument)
    }
    $start.Environment['BIOROUTER_HOSTNAME_FIXTURE'] = $directory
    $start.Environment['BIOROUTER_HOSTNAME_FIXTURE_ROLE'] = $Role
    $start.Environment['BIOROUTER_PATH_ROOT'] = Join-Path $directory 'profile'
    $start.Environment['BIOROUTER_DISABLE_KEYRING'] = 'true'
    $start.Environment['COMPUTERNAME'] = 'NOT-THE-BACKEND-HOST'
    $process = [Diagnostics.Process]::Start($start)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $source = 'biorouter-hostname-' + [Guid]::NewGuid().ToString('N')
    $registered = $false
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while (!(Test-Path -LiteralPath (Join-Path $directory 'ready'))) {
            if ($process.HasExited) { throw "Fixture exited before ready: $($stderr.GetAwaiter().GetResult())" }
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Fixture did not become ready' }
            Start-Sleep -Milliseconds 25
        }

        # The fixture explicitly detaches before its handshake. Verify that
        # console inheritance cannot mask the dependency spawn under test.
        [CopilotConsoleProbe]::FreeConsole() | Out-Null
        $attachError = [CopilotConsoleProbe]::AttachConsoleError([uint32]$process.Id)
        if ($attachError -eq 0) {
            [CopilotConsoleProbe]::FreeConsole() | Out-Null
            throw 'Fixture unexpectedly owns a console; the baseline would be meaningless'
        }
        if ($attachError -ne 6) { throw "Cannot establish consoleless fixture (AttachConsole error $attachError)" }

        $query = 'SELECT * FROM Win32_ProcessStartTrace WHERE ParentProcessID = ' + $process.Id
        Register-CimIndicationEvent -Namespace root/cimv2 -Query $query -SourceIdentifier $source | Out-Null
        $registered = $true
        Start-Sleep -Milliseconds 500
        [IO.File]::WriteAllText((Join-Path $directory 'go'), 'observe')
        Start-Sleep -Milliseconds 500
        # HOSTNAME.EXE can exit between screenshots. The process trace is the
        # spawn evidence; the held calibration above proves console visibility.
        Save-Screenshot (Join-Path $directory 'desktop-during-polling.png')
        if (!$process.WaitForExit(30000)) { throw 'Fixture exceeded the bounded polling window' }
        Start-Sleep -Milliseconds 500
        $events = @(Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue | ForEach-Object {
            $created = $_.SourceEventArgs.NewEvent
            [ordered]@{
                name = [string]$created.ProcessName
                pid = [uint32]$created.ProcessID
                parent_pid = [uint32]$created.ParentProcessID
                observed_at = $_.TimeGenerated.ToUniversalTime().ToString('o')
            }
        })
        [IO.File]::WriteAllText((Join-Path $directory 'stdout.txt'), $stdout.GetAwaiter().GetResult())
        [IO.File]::WriteAllText((Join-Path $directory 'stderr.txt'), $stderr.GetAwaiter().GetResult())
        if ($process.ExitCode -ne 0) { throw "Fixture failed with exit $($process.ExitCode); see $directory" }
        $result = Get-Content -Raw -LiteralPath (Join-Path $directory 'result.json') | ConvertFrom-Json
        $expected = [Net.Dns]::GetHostName() + ' (windows)'
        if (![string]::Equals($result.target, $expected, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Production scope target '$($result.target)' did not match native host '$expected'"
        }
        $hostnameEvents = @($events | Where-Object { $_.name -ieq 'hostname.exe' })
        $expectedSpawns = if ($Role -eq 'baseline') { 6 } else { 0 }
        if ($hostnameEvents.Count -ne $expectedSpawns) {
            throw "$Role expected $expectedSpawns hostname.exe children, observed $($hostnameEvents.Count)"
        }
        if ($Role -eq 'fixed' -and $events.Count -ne 0) {
            throw "Production status/bind_task unexpectedly created child processes: $($events | ConvertTo-Json -Compress)"
        }
        $verdict = [ordered]@{
            role = $Role
            consoleless_parent = $true
            fixture = $result
            hostname_processes = $hostnameEvents.Count
            process_trace = $events
        }
        $verdict | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $directory 'evidence.json') -Encoding utf8
        return $verdict
    } finally {
        if ($registered) { Unregister-Event -SourceIdentifier $source -ErrorAction SilentlyContinue }
        Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue | Remove-Event -ErrorAction SilentlyContinue
        if (!$process.HasExited) { $process.Kill($true); $process.WaitForExit() }
        $process.Dispose()
    }
}

$visible = Measure-VisibleControl
$baseline = Measure-Polling 'baseline'
$fixed = Measure-Polling 'fixed'
[ordered]@{ visible_console_control = $visible; baseline = $baseline; fixed = $fixed } |
    ConvertTo-Json -Depth 10 |
    Set-Content -LiteralPath (Join-Path $RunDirectory 'summary.json') -Encoding utf8
Write-Host "Evidence: $RunDirectory"
Write-Host 'PASS: native Copilot status/task binding created no child process; the old dependency positive control created six HOSTNAME.EXE processes.'
