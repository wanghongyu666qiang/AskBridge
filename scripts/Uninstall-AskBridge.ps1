[CmdletBinding()]
param(
    [string]$InstallRoot = (Split-Path -Parent $PSCommandPath),
    [switch]$RemoveData,
    [switch]$PreserveData
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($RemoveData -and $PreserveData) {
    throw "RemoveData and PreserveData cannot be used together."
}
if (-not [IO.Path]::IsPathRooted($InstallRoot)) {
    throw "InstallRoot must be an absolute path."
}
$resolvedInstallRoot = [IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
$driveRoot = [IO.Path]::GetPathRoot($resolvedInstallRoot).TrimEnd('\')
if ($resolvedInstallRoot.Equals($driveRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to uninstall from a drive root."
}

$manifestPath = Join-Path $resolvedInstallRoot "install-manifest.json"
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "install-manifest.json is missing; refusing an unscoped uninstall."
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if (-not ([string]$manifest.product).Equals("AskBridge", [StringComparison]::Ordinal)) {
    throw "The install manifest does not identify AskBridge."
}
if (-not ([IO.Path]::GetFullPath([string]$manifest.install_root).TrimEnd('\')).Equals($resolvedInstallRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "The install manifest belongs to a different directory."
}

$targetExecutable = Join-Path $resolvedInstallRoot "askbridge.exe"
$running = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.Equals($targetExecutable, [StringComparison]::OrdinalIgnoreCase) })
if ($running.Count -gt 0) {
    throw "Close the installed AskBridge process before uninstalling."
}

$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Remove-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
if ($null -ne $manifest.start_menu_shortcut -and -not [string]::IsNullOrWhiteSpace([string]$manifest.start_menu_shortcut)) {
    Remove-Item -LiteralPath ([string]$manifest.start_menu_shortcut) -Force -ErrorAction SilentlyContinue
}

$deleteData = $RemoveData
if (-not $RemoveData -and -not $PreserveData) {
    Write-Host "AskBridge data may contain settings and the dedicated Chrome login state."
    $answer = Read-Host "Delete the data directory too? Type DELETE to confirm"
    $deleteData = $answer -ceq "DELETE"
}

foreach ($file in @($manifest.files)) {
    $candidate = [IO.Path]::GetFullPath((Join-Path $resolvedInstallRoot ([string]$file)))
    if (-not $candidate.StartsWith($resolvedInstallRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "The install manifest contains an out-of-scope file path."
    }
    Remove-Item -LiteralPath $candidate -Force -ErrorAction SilentlyContinue
}
Remove-Item -LiteralPath $manifestPath -Force -ErrorAction SilentlyContinue

$dataRoot = [IO.Path]::GetFullPath((Join-Path $resolvedInstallRoot "data"))
if ($deleteData) {
    if (-not $dataRoot.StartsWith($resolvedInstallRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Resolved data path is outside the install directory."
    }
    Remove-Item -LiteralPath $dataRoot -Recurse -Force -ErrorAction SilentlyContinue
}

$remaining = @(Get-ChildItem -LiteralPath $resolvedInstallRoot -Force -ErrorAction SilentlyContinue)
if ($remaining.Count -eq 0) {
    Remove-Item -LiteralPath $resolvedInstallRoot -Force -ErrorAction SilentlyContinue
}

if ($deleteData) {
    Write-Host "AskBridge, its startup entry, and its user data were removed."
} else {
    Write-Host "AskBridge and its startup entry were removed. User data was preserved at $dataRoot"
}
