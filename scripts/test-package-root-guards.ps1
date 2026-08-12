[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AcceptanceRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Resolve-Path (Join-Path $PSScriptRoot "..")).Path).TrimEnd('\')
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target")).TrimEnd('\') + '\'
if (-not [IO.Path]::IsPathRooted($AcceptanceRoot)) {
    throw "AcceptanceRoot must be an explicit absolute path."
}
$root = [IO.Path]::GetFullPath($AcceptanceRoot).TrimEnd('\')
if (-not $root.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "AcceptanceRoot must be a new child of the repository target directory."
}
if (Test-Path -LiteralPath $root) {
    throw "AcceptanceRoot already exists; refusing to overwrite it."
}

function Invoke-ExpectedFailure {
    param([scriptblock]$Command, [string]$ExpectedMessagePrefix)

    try {
        & $Command
    }
    catch {
        if ($_.Exception.Message.StartsWith($ExpectedMessagePrefix, [StringComparison]::Ordinal)) {
            return
        }
        throw
    }
    throw "Expected command to fail with '$ExpectedMessagePrefix'."
}

try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null

    Write-Host "[1/3] Reject relative artifact roots"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot "relative-package-root"
    } "ArtifactRoot must be an explicit absolute path."

    Write-Host "[2/3] Reject repository and target roots"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot $repoRoot
    } "ArtifactRoot must be a dedicated package directory, not the repository or target root."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot (Join-Path $repoRoot "target")
    } "ArtifactRoot must be a dedicated package directory, not the repository or target root."

    Write-Host "[3/3] Reject non-empty artifact roots before building"
    $nonEmptyRoot = Join-Path $root "non-empty-package-root"
    New-Item -ItemType Directory -Path $nonEmptyRoot -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $nonEmptyRoot "stale.txt") -Encoding ASCII -Value "stale"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot $nonEmptyRoot
    } "ArtifactRoot already exists and is not empty; refusing to mix package outputs with existing files."

    Write-Host "Package root guard validation passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
