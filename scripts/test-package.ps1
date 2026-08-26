[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AcceptanceRoot,
    [string]$UpdateSigningKeyFile
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
# Fall back to the conventional gitignored key location next to the repository.
$UpdateSigningKeyFile = [string]$UpdateSigningKeyFile
if ([string]::IsNullOrWhiteSpace($UpdateSigningKeyFile)) {
    $defaultKeyFile = Join-Path $repoRoot ".update-signing-key.json"
    if (Test-Path -LiteralPath $defaultKeyFile -PathType Leaf) {
        $UpdateSigningKeyFile = $defaultKeyFile
    }
}
if (Test-Path -LiteralPath $root) {
    throw "AcceptanceRoot already exists; refusing to overwrite it."
}

$artifactRoot = Join-Path $root "package"

function Get-AskBridgePackageVersion {
    Push-Location $repoRoot
    try {
        $metadata = cargo metadata --offline --no-deps --format-version 1 | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE." }
        $package = $metadata.packages | Where-Object { $_.name -eq "askbridge-win" } | Select-Object -First 1
        if ($null -eq $package) { throw "askbridge-win metadata was not found." }
        return [string]$package.version
    }
    finally {
        Pop-Location
    }
}

try {
    Write-Host "[1/2] Build package artifacts"
    & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot $artifactRoot -UpdateSigningKeyFile $UpdateSigningKeyFile
    if ($LASTEXITCODE -ne 0) { throw "package.ps1 failed with exit code $LASTEXITCODE." }

    Write-Host "[2/2] Validate package artifacts"
    $expectedVersion = Get-AskBridgePackageVersion
    Push-Location $repoRoot
    try {
        cargo xtask validate-package-artifacts `
            --artifact-root $artifactRoot `
            --expected-version $expectedVersion `
            --expected-release-exe-path (Join-Path $repoRoot "target\release\askbridge.exe") `
            --expected-source-root $repoRoot `
            --require-update-signature
        if ($LASTEXITCODE -ne 0) {
            throw "package artifact validator failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
    Write-Host "Package artifacts, version, hashes, signature, metadata, Release EXE identity, and external-runtime exclusion acceptance passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
