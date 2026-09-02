$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-Cargo {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo was not found. Install a stable Rust toolchain and reopen the terminal."
    }
}

Push-Location (Resolve-Path (Join-Path $PSScriptRoot ".."))
try {
    Assert-Cargo
    Write-Host "[1/2] Building debug workspace"
    cargo build --workspace --locked --offline
    if ($LASTEXITCODE -ne 0) { throw "Debug build failed with exit code $LASTEXITCODE." }

    Write-Host "[2/2] Building release workspace"
    cargo build --workspace --release --locked --offline
    if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE." }
}
finally {
    Pop-Location
}
