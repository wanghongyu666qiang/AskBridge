[CmdletBinding()]
param(
    [string]$AcceptanceRoot,
    [switch]$SelfTestFailureHandling,
    [string]$UpdateSigningKeyFile
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = [IO.Path]::GetFullPath((Resolve-Path (Join-Path $PSScriptRoot "..")).Path).TrimEnd('\')
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target")).TrimEnd('\') + '\'

# Fall back to the conventional gitignored key location next to the repository.
$UpdateSigningKeyFile = [string]$UpdateSigningKeyFile
if ([string]::IsNullOrWhiteSpace($UpdateSigningKeyFile)) {
    $defaultKeyFile = Join-Path $repoRoot ".update-signing-key.json"
    if (Test-Path -LiteralPath $defaultKeyFile -PathType Leaf) {
        $UpdateSigningKeyFile = $defaultKeyFile
    }
}

function Invoke-Step {
    param([string]$Name, [scriptblock]$Command)

    Write-Host $Name
    $global:LASTEXITCODE = 0
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE."
    }
}

function Test-NativeFailureHandling {
    try {
        Invoke-Step "[native-failure-self-test] Verify native command failures stop the local release gate" {
            & powershell -NoProfile -ExecutionPolicy Bypass -Command "exit 17"
        }
    }
    catch {
        if ($_.Exception.Message -like "*exit code 17*") {
            Write-Host "Local release gate native failure handling is active."
            return
        }
        throw
    }
    throw "Native failure handling self-test expected Invoke-Step to reject a native non-zero exit code."
}

if ($SelfTestFailureHandling) {
    Test-NativeFailureHandling
    exit 0
}

if ([string]::IsNullOrWhiteSpace($AcceptanceRoot) -or -not [IO.Path]::IsPathRooted($AcceptanceRoot)) {
    throw "AcceptanceRoot must be an explicit absolute path."
}
$root = [IO.Path]::GetFullPath($AcceptanceRoot).TrimEnd('\')
if (-not $root.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "AcceptanceRoot must be a new child of the repository target directory."
}
if (Test-Path -LiteralPath $root) {
    throw "AcceptanceRoot already exists; refusing to overwrite it."
}

try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null

    Test-NativeFailureHandling

    Invoke-Step "[1/10] Validate PowerShell script syntax" {
        & (Join-Path $repoRoot "scripts\test-powershell-syntax.ps1")
    }

    Invoke-Step "[2/10] Validate package artifact validator success and failure paths" {
        Push-Location $repoRoot
        try {
            cargo test -p xtask package_artifacts --locked --offline
        }
        finally {
            Pop-Location
        }
    }

    Invoke-Step "[3/10] Validate performance report xtask and CLI failure paths" {
        Push-Location $repoRoot
        try {
            cargo run --package xtask --locked --offline -- help
            if ($LASTEXITCODE -ne 0) {
                throw "xtask help check failed with exit code $LASTEXITCODE."
            }
            cargo test -p xtask performance_report --locked --offline
            if ($LASTEXITCODE -ne 0) {
                throw "performance report tests failed with exit code $LASTEXITCODE."
            }
            cargo test -p xtask --test cli --locked --offline
        }
        finally {
            Pop-Location
        }
    }

    Invoke-Step "[4/10] Validate acceptance root guards" {
        & (Join-Path $repoRoot "scripts\test-acceptance-root-guards.ps1") `
            -AcceptanceRoot (Join-Path $root "acceptance-root-guards")
    }

    Invoke-Step "[5/10] Validate package root guards" {
        & (Join-Path $repoRoot "scripts\test-package-root-guards.ps1") `
            -AcceptanceRoot (Join-Path $root "package-root-guards")
    }

    Invoke-Step "[6/10] Validate installer metadata gate" {
        & (Join-Path $repoRoot "scripts\test-install-metadata-validator.ps1") `
            -AcceptanceRoot (Join-Path $root "install-metadata-validator")
    }

    Invoke-Step "[7/10] Build and validate temporary package artifacts" {
        & (Join-Path $repoRoot "scripts\test-package.ps1") `
            -AcceptanceRoot (Join-Path $root "package") `
            -UpdateSigningKeyFile $UpdateSigningKeyFile
    }

    Invoke-Step "[8/10] Run Rust format, Clippy, and tests" {
        & (Join-Path $repoRoot "scripts\test.ps1")
    }

    Invoke-Step "[9/10] Run Debug and Release builds" {
        & (Join-Path $repoRoot "scripts\build.ps1")
    }

    Invoke-Step "[10/10] Check staged diff whitespace" {
        Push-Location $repoRoot
        try {
            git diff --check
        }
        finally {
            Pop-Location
        }
    }

    Write-Host "Local release validation passed. Real browser, hotkey, installer-launch, performance-duration, and final-artifact checks remain separate."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
