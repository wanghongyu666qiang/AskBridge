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

$artifactRoot = Join-Path $root "package"

try {
    Write-Host "[1/4] Build package artifacts"
    & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot $artifactRoot
    if ($LASTEXITCODE -ne 0) { throw "package.ps1 failed with exit code $LASTEXITCODE." }

    Write-Host "[2/4] Verify expected artifact set"
    $portableRoots = @(Get-ChildItem -LiteralPath $artifactRoot -Directory -Filter "AskBridge-*")
    $zipFiles = @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "AskBridge-*-windows-x64.zip")
    $setupFiles = @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "AskBridge-*-Setup.exe")
    $hashFiles = @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "AskBridge-*-SHA256SUMS.txt")
    if ($portableRoots.Count -ne 1 -or $zipFiles.Count -ne 1 -or $setupFiles.Count -ne 1 -or $hashFiles.Count -ne 1) {
        throw "Package output must contain exactly one portable directory, ZIP, Setup EXE, and SHA256SUMS file."
    }
    foreach ($file in @(
        "askbridge.exe",
        "README.md",
        "PRIVACY.md",
        "TROUBLESHOOTING.md",
        "Install-AskBridge.ps1",
        "Uninstall-AskBridge.ps1",
        "package.json"
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $portableRoots[0].FullName $file) -PathType Leaf)) {
            throw "Portable package is missing $file."
        }
    }

    Write-Host "[3/4] Verify hashes and package metadata"
    foreach ($line in Get-Content -LiteralPath $hashFiles[0].FullName -Encoding ASCII) {
        if ($line -notmatch '^([0-9A-F]{64})  (.+)$') { throw "Malformed SHA256SUMS line: $line" }
        $expectedHash = $Matches[1]
        $leaf = $Matches[2]
        $matches = @(Get-ChildItem -LiteralPath $artifactRoot -Recurse -File | Where-Object Name -EQ $leaf)
        if ($matches.Count -ne 1) { throw "Hash target '$leaf' is missing or ambiguous." }
        $actualHash = (Get-FileHash -LiteralPath $matches[0].FullName -Algorithm SHA256).Hash
        if ($actualHash -ne $expectedHash) { throw "Hash verification failed for '$leaf'." }
    }
    $metadata = Get-Content -LiteralPath (Join-Path $portableRoots[0].FullName "package.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$metadata.product -ne "AskBridge" -or
        [string]$metadata.architecture -ne "windows-x64" -or
        $metadata.auto_submit -ne $false -or
        $metadata.chrome_bundled -ne $false) {
        throw "Package metadata does not preserve the expected 1.0 safety flags."
    }

    Write-Host "[4/4] Verify package does not bundle external runtimes"
    $unexpectedPayload = @(Get-ChildItem -LiteralPath $portableRoots[0].FullName -Recurse -File | Where-Object {
        $_.Name -match 'chrome|rust|cargo' -or $_.Extension -in @(".dll", ".msi")
    })
    if ($unexpectedPayload.Count -gt 0) {
        throw "Package unexpectedly bundled external runtime files: $($unexpectedPayload.FullName -join '; ')"
    }
    Write-Host "Package artifacts, hashes, metadata, and external-runtime exclusion acceptance passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
