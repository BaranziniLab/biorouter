# Builds the stand-in sidecar for port-release-regression.ps1.
#
# ⚠ Run this with Windows PowerShell 5.1 (`powershell.exe`), not pwsh 7, which
# dropped Add-Type's -OutputAssembly / -OutputType.
[CmdletBinding()]
param(
    [string]$Source = (Join-Path $PSScriptRoot 'port-release-sleeper.cs'),
    [Parameter(Mandatory = $true)][string]$Output
)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition (Get-Content -LiteralPath $Source -Raw) -OutputAssembly $Output -OutputType ConsoleApplication
if (-not (Test-Path -LiteralPath $Output)) { throw "Add-Type produced no executable at $Output" }
Write-Output "built stand-in sidecar: $Output"
