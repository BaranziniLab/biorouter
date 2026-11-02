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
using System.Collections.Generic;
using System.IO;
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
    private static extern bool SetHandleInformation(IntPtr handle, uint mask, uint flags);
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

    private static System.Diagnostics.Process PinProcess(ProcessInformation native) {
        try {
            var child = System.Diagnostics.Process.GetProcessById((int)native.dwProcessId);
            try {
                // Keep the PID reserved until the managed observer owns a handle.
                _ = child.Handle;
                return child;
            } catch {
                child.Dispose();
                throw;
            }
        } finally {
            CloseHandle(native.hThread);
            CloseHandle(native.hProcess);
        }
    }

    public static System.Diagnostics.Process StartDetachedFixture(
        string executable, string directory, IDictionary<string, string> environment) {
        using (var input = new FileStream(Path.Combine(directory, "stdin.txt"), FileMode.Create, FileAccess.ReadWrite, FileShare.ReadWrite))
        using (var output = new FileStream(Path.Combine(directory, "stdout.txt"), FileMode.Create, FileAccess.Write, FileShare.ReadWrite))
        using (var error = new FileStream(Path.Combine(directory, "stderr.txt"), FileMode.Create, FileAccess.Write, FileShare.ReadWrite)) {
            var startup = new StartupInfo {
                cb = (uint)Marshal.SizeOf(typeof(StartupInfo)),
                dwFlags = 0x00000100,
                hStdInput = input.SafeFileHandle.DangerousGetHandle(),
                hStdOutput = output.SafeFileHandle.DangerousGetHandle(),
                hStdError = error.SafeFileHandle.DangerousGetHandle(),
            };
            foreach (var handle in new[] { startup.hStdInput, startup.hStdOutput, startup.hStdError }) {
                if (!SetHandleInformation(handle, 1, 1)) {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }
            }
            var entries = new List<string>();
            foreach (var pair in new SortedDictionary<string, string>(environment, StringComparer.OrdinalIgnoreCase)) {
                entries.Add(pair.Key + "=" + pair.Value);
            }
            var block = Marshal.StringToHGlobalUni(string.Join("\0", entries) + "\0\0");
            try {
                var arguments = new System.Text.StringBuilder(
                    "\"" + executable + "\" --exact windows_hostname_fixture --ignored --nocapture --test-threads=1");
                ProcessInformation process;
                // DETACHED_PROCESS avoids the hidden console that CREATE_NO_WINDOW
                // still allocated on the runner; stdio is inherited from files.
                if (!CreateProcessW(executable, arguments, IntPtr.Zero, IntPtr.Zero, true,
                                    0x00000008 | 0x00000400, block, null, ref startup, out process)) {
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                }
                return PinProcess(process);
            } finally {
                Marshal.FreeHGlobal(block);
            }
        }
    }

    public static System.Diagnostics.Process StartVisibleControl(string executable) {
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
        return PinProcess(process);
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
        $process = [CopilotConsoleProbe]::StartVisibleControl($TestBinary)
        $childId = $process.Id
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
    $start.Environment['BIOROUTER_HOSTNAME_FIXTURE'] = $directory
    $start.Environment['BIOROUTER_HOSTNAME_FIXTURE_ROLE'] = $Role
    $start.Environment['BIOROUTER_PATH_ROOT'] = Join-Path $directory 'profile'
    $start.Environment['BIOROUTER_DISABLE_KEYRING'] = 'true'
    $start.Environment['COMPUTERNAME'] = 'NOT-THE-BACKEND-HOST'
    $timeline = [ordered]@{ launch_requested_at = [DateTime]::UtcNow.ToString('o') }
    $process = [CopilotConsoleProbe]::StartDetachedFixture($TestBinary, $directory, $start.Environment)
    $childId = $process.Id
    $source = 'biorouter-hostname-' + [Guid]::NewGuid().ToString('N')
    $registered = $false
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        while (!(Test-Path -LiteralPath (Join-Path $directory 'ready'))) {
            if ($process.HasExited) { throw "Fixture exited before ready: $(Get-Content -Raw -LiteralPath (Join-Path $directory 'stderr.txt'))" }
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Fixture did not become ready' }
            Start-Sleep -Milliseconds 25
        }
        $timeline.fixture_ready_at = [DateTime]::UtcNow.ToString('o')
        $console = Get-Content -Raw -LiteralPath (Join-Path $directory 'console-state.json') | ConvertFrom-Json
        if ($console.initial_process_count -ne 0) { throw 'DETACHED_PROCESS fixture unexpectedly started with a console' }

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
        $timeline.observation_started_at = [DateTime]::UtcNow.ToString('o')
        [IO.File]::WriteAllText((Join-Path $directory 'go'), 'observe')
        Start-Sleep -Milliseconds 500
        # HOSTNAME.EXE can exit between screenshots. The process trace is the
        # spawn evidence; the held calibration above proves console visibility.
        Save-Screenshot (Join-Path $directory 'desktop-during-polling.png')
        if (!$process.WaitForExit(30000)) { throw 'Fixture exceeded the bounded polling window' }
        $timeline.process_exited_at = [DateTime]::UtcNow.ToString('o')
        Start-Sleep -Milliseconds 500
        $events = @(Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue | ForEach-Object {
            $created = $_.SourceEventArgs.NewEvent
            [ordered]@{
                name = [string]$created.ProcessName
                pid = [uint32]$created.ProcessID
                parent_pid = [uint32]$created.ParentProcessID
                observed_at = $_.TimeGenerated.ToUniversalTime().ToString('o')
                event_created_at = [DateTime]::FromFileTimeUtc([int64]$created.TIME_CREATED).ToString('o')
            }
        })
        $timeline.events_collected_at = [DateTime]::UtcNow.ToString('o')
        $resultPath = Join-Path $directory 'result.json'
        if (Test-Path -LiteralPath $resultPath) {
            $timeline.fixture_result_written_at = (Get-Item -LiteralPath $resultPath).LastWriteTimeUtc.ToString('o')
        }
        [ordered]@{ role = $Role; pid = $childId; console = $console; timeline = $timeline; process_trace = $events } |
            ConvertTo-Json -Depth 8 |
            Set-Content -LiteralPath (Join-Path $directory 'observation.json') -Encoding utf8
        if ($process.ExitCode -ne 0) { throw "Fixture failed with exit $($process.ExitCode); see $directory" }
        $result = Get-Content -Raw -LiteralPath $resultPath | ConvertFrom-Json
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
            creation_flags = 'DETACHED_PROCESS | CREATE_UNICODE_ENVIRONMENT'
            timeline = $timeline
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
