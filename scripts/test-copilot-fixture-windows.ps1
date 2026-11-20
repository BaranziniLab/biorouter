# Drive Biorouter Copilot's WinForms fixture app through Copilot's own tools (AD-21), on a
# Windows desktop session that nobody is using: CI's hosted runner, or a test machine.
#
#   powershell -NoProfile -File scripts/test-copilot-fixture-windows.ps1 PAYLOAD_DIR
#
# PAYLOAD_DIR is the built engine payload (target/computer-use/win32-x64). The fixture is
# compiled to its own executable first, because Copilot treats a PowerShell host as a terminal
# and would refuse to type into it. Windows PowerShell 5.1 compiles it (its Add-Type can write
# an executable; PowerShell 7's cannot), which is why this script runs under powershell.exe.
param([Parameter(Mandatory = $true)][string]$Payload)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

Add-Type -Namespace Probe -Name Session -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern bool ProcessIdToSessionId(uint pid, out uint session);
'@
$session = 0
[void][Probe.Session]::ProcessIdToSessionId([uint32]$PID, [ref]$session)
if ($session -eq 0) {
  throw 'Windows session 0 has no interactive desktop; the fixture cannot be shown or driven here.'
}

$out = Join-Path (Get-Location) 'target\copilot-fixture'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$exe = Join-Path $out 'BiorouterCopilotFixture.exe'
Remove-Item -Force -ErrorAction SilentlyContinue $exe
Add-Type -OutputType WindowsApplication -OutputAssembly $exe `
  -ReferencedAssemblies System.Windows.Forms, System.Drawing `
  -TypeDefinition (Get-Content -Raw 'scripts\copilot-fixture-winforms.cs')

$env:BIOROUTER_COMPUTER_USE_DIR = (Resolve-Path $Payload).Path
$env:BIOROUTER_COPILOT_FIXTURE = (ConvertTo-Json -Compress @($exe))
# cargo writes its progress and warnings to stderr; under 'Stop' a redirected stderr line would
# end the script, so the exit code alone decides.
$ErrorActionPreference = 'Continue'
cargo test --locked -p biorouter-mcp --test computer_use_fixture -- --ignored --nocapture --test-threads=1
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
