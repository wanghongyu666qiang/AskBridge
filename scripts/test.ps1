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
    Write-Host "[1/3] Checking formatting"
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw "Formatting check failed with exit code $LASTEXITCODE." }

    Write-Host "[2/3] Running Clippy"
    cargo clippy --workspace --all-targets --offline -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "Clippy failed with exit code $LASTEXITCODE." }

    Write-Host "[3/3] Running tests"
    cargo test --workspace --offline
    if ($LASTEXITCODE -ne 0) { throw "Tests failed with exit code $LASTEXITCODE." }
}
finally {
    Pop-Location
}
