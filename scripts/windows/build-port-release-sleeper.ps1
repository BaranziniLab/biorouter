# Builds the stand-in sidecar for port-release-regression.ps1.
#
# ⚠ Run this with Windows PowerShell 5.1 (`powershell.exe`), not pwsh 7, which
# dropped Add-Type's -OutputAssembly / -OutputType.
[CmdletBinding()]
param(
    [string]$Source,
    [Parameter(Mandatory = $true)][string]$Output
)

$ErrorActionPreference = 'Stop'

# ⚠ Resolved HERE, not as a param default: under `powershell.exe -File`,
# $PSScriptRoot is empty while the param block is evaluated, and Join-Path then
# dies on an empty path (measured on windows-latest, 2026-09-22).
if ([string]::IsNullOrWhiteSpace($Source)) {
    $here = Split-Path -Parent $MyInvocation.MyCommand.Path
    $Source = Join-Path $here 'port-release-sleeper.cs'
}
if (-not (Test-Path -LiteralPath $Source)) { throw "no sidecar source at $Source" }
Add-Type -TypeDefinition (Get-Content -LiteralPath $Source -Raw) -OutputAssembly $Output -OutputType ConsoleApplication
if (-not (Test-Path -LiteralPath $Output)) { throw "Add-Type produced no executable at $Output" }
Write-Output "built stand-in sidecar: $Output"
