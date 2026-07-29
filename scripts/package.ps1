$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-Cargo {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo was not found. Install a stable Rust toolchain and reopen the terminal."
    }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$artifactRoot = Join-Path $repoRoot "artifacts"
$packageRoot = Join-Path $artifactRoot "AskBridge-0.1.0"

if (-not $packageRoot.StartsWith($artifactRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Resolved package path is outside the artifact directory."
}

Push-Location $repoRoot
try {
    Assert-Cargo
    Write-Host "[1/3] Building release binary"
    cargo build --workspace --release
    if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE." }

    Write-Host "[2/3] Preparing reproducible artifact directory"
    if (Test-Path -LiteralPath $packageRoot) {
        Remove-Item -LiteralPath $packageRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $packageRoot | Out-Null

    Write-Host "[3/3] Copying portable package files"
    Copy-Item -LiteralPath (Join-Path $repoRoot "target\release\askbridge.exe") -Destination $packageRoot
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $packageRoot
    Write-Host "Package created at $packageRoot"
}
finally {
    Pop-Location
}
