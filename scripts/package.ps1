[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot,
    [string]$UpdateSigningKeyFile
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Force UTF-8 on the error stream so localized PowerShell error diagnostics do
# not reach callers as raw OEM/GBK bytes (the xtask CLI test asserts on the
# ASCII marker of a thrown error but must be able to decode the rest too).
try {
    [Console]::OutputEncoding = [Text.Encoding]::UTF8
} catch {
    # Redirected or restricted hosts can refuse this; proceed without it.
}

function Assert-Command {
    param([string]$Name)

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name was not found."
    }
}

function Wait-StableFile {
    param(
        [string]$Path,
        [int]$TimeoutSeconds = 60,
        [int]$StableSamples = 8,
        [int]$IntervalMilliseconds = 250
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $stableCount = 0
    $previousSignature = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            try {
                $stream = [IO.File]::Open(
                    $Path,
                    [IO.FileMode]::Open,
                    [IO.FileAccess]::Read,
                    [IO.FileShare]::None
                )
                $stream.Dispose()
                $item = Get-Item -LiteralPath $Path
                if ($item.Length -gt 0) {
                    $signature = "{0}|{1}" -f $item.Length, $item.LastWriteTimeUtc.Ticks
                    if ($signature -eq $previousSignature) {
                        $stableCount++
                    }
                    else {
                        $stableCount = 1
                        $previousSignature = $signature
                    }
                    if ($stableCount -ge $StableSamples) {
                        return
                    }
                }
            }
            catch [IO.IOException] {
                $stableCount = 0
            }
        }
        Start-Sleep -Milliseconds $IntervalMilliseconds
    }
    throw "File did not become stable within $TimeoutSeconds seconds: $Path"
}

function Get-Sha256Hex {
    param([string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $hasher.ComputeHash($stream)
        return (($bytes | ForEach-Object { $_.ToString("X2") }) -join "")
    }
    finally {
        $hasher.Dispose()
        $stream.Dispose()
    }
}

function Write-And-VerifyHashes {
    param(
        [string[]]$Paths,
        [string]$HashPath
    )

    $hashLines = foreach ($path in $Paths) {
        $hash = Get-Sha256Hex -Path $path
        "{0}  {1}" -f $hash, (Split-Path -Leaf $path)
    }
    $hashLines | Set-Content -LiteralPath $HashPath -Encoding ASCII

    foreach ($line in Get-Content -LiteralPath $HashPath -Encoding ASCII) {
        if ($line -notmatch '^([0-9A-F]{64})  (.+)$') {
            throw "Malformed SHA256SUMS line: $line"
        }
        $expectedHash = $Matches[1]
        $leafName = $Matches[2]
        $matches = @($Paths | Where-Object { (Split-Path -Leaf $_) -eq $leafName })
        if ($matches.Count -ne 1) {
            throw "Hash target '$leafName' is missing or ambiguous."
        }
        $actualHash = Get-Sha256Hex -Path $matches[0]
        if ($actualHash -ne $expectedHash) {
            throw "Hash verification failed for '$leafName'."
        }
    }
}

function Add-SetupPayload {
    param(
        [string]$StubPath,
        [string]$SetupPath,
        [IO.FileInfo[]]$PayloadFiles
    )

    Copy-Item -LiteralPath $StubPath -Destination $SetupPath -Force
    $stream = [IO.File]::Open($SetupPath, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $manifestLines = [Collections.Generic.List[string]]::new()
        foreach ($file in $PayloadFiles) {
            $offset = $stream.Position
            $bytes = [IO.File]::ReadAllBytes($file.FullName)
            $stream.Write($bytes, 0, $bytes.Length)
            $manifestLines.Add(("{0}`t{1}`t{2}" -f $file.Name, $offset, $bytes.Length))
        }
        $manifest = [Text.Encoding]::UTF8.GetBytes(($manifestLines -join "`n"))
        $stream.Write($manifest, 0, $manifest.Length)
        $magic = [Text.Encoding]::ASCII.GetBytes("ASKBRIDGESETUP10")
        $stream.Write($magic, 0, $magic.Length)
        $lengthBytes = [BitConverter]::GetBytes([uint64]$manifest.Length)
        $stream.Write($lengthBytes, 0, $lengthBytes.Length)
    }
    finally {
        $stream.Dispose()
    }
}

$repoRoot = [IO.Path]::GetFullPath((Resolve-Path (Join-Path $PSScriptRoot "..")).Path).TrimEnd('\')
if (-not [IO.Path]::IsPathRooted($ArtifactRoot)) {
    throw "ArtifactRoot must be an explicit absolute path."
}
$artifactRoot = [IO.Path]::GetFullPath($ArtifactRoot).TrimEnd('\')
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target")).TrimEnd('\')
if ($artifactRoot.Equals($repoRoot, [StringComparison]::OrdinalIgnoreCase) -or
    $artifactRoot.Equals($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "ArtifactRoot must be a dedicated package directory, not the repository or target root."
}
if (Test-Path -LiteralPath $artifactRoot) {
    $existingItems = @(Get-ChildItem -LiteralPath $artifactRoot -Force)
    if ($existingItems.Count -gt 0) {
        throw "ArtifactRoot already exists and is not empty; refusing to mix package outputs with existing files."
    }
}
else {
    New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null
}

Push-Location $repoRoot
try {
    Assert-Command "cargo"
    Assert-Command "iexpress.exe"
    Write-Host "[1/7] Resolving package version"
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE." }
    $package = $metadata.packages | Where-Object { $_.name -eq "askbridge-win" } | Select-Object -First 1
    if ($null -eq $package) { throw "askbridge-win metadata was not found." }
    $version = [string]$package.version
    $packageName = "AskBridge-$version"
    $packageRoot = Join-Path $artifactRoot $packageName
    $zipPath = Join-Path $artifactRoot "$packageName-windows-x64.zip"
    $setupPath = Join-Path $artifactRoot "$packageName-Setup.exe"

    foreach ($target in @($packageRoot, $zipPath, $setupPath)) {
        $resolved = [IO.Path]::GetFullPath($target)
        if (-not $resolved.StartsWith([IO.Path]::GetFullPath($artifactRoot) + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "Resolved package target is outside the artifact directory: $resolved"
        }
    }

    Write-Host "[2/7] Building release binary"
    cargo build --workspace --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE." }
    cargo build --package askbridge-win --bin askbridge-setup --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Setup build failed with exit code $LASTEXITCODE." }

    Write-Host "[3/7] Preparing deterministic package directory"
    if (Test-Path -LiteralPath $packageRoot) {
        Remove-Item -LiteralPath $packageRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
    $payload = [ordered]@{
        (Join-Path $repoRoot "target\release\askbridge.exe") = "askbridge.exe"
        (Join-Path $repoRoot "target\release\WebView2Loader.dll") = "WebView2Loader.dll"
        (Join-Path $repoRoot "README.md") = "README.md"
        (Join-Path $repoRoot "docs\PRIVACY.md") = "PRIVACY.md"
        (Join-Path $repoRoot "docs\TROUBLESHOOTING.md") = "TROUBLESHOOTING.md"
        (Join-Path $repoRoot "scripts\Install-AskBridge.ps1") = "Install-AskBridge.ps1"
        (Join-Path $repoRoot "scripts\Uninstall-AskBridge.ps1") = "Uninstall-AskBridge.ps1"
    }
    foreach ($entry in $payload.GetEnumerator()) {
        if (-not (Test-Path -LiteralPath $entry.Key -PathType Leaf)) {
            throw "Package input is missing: $($entry.Key)"
        }
        Copy-Item -LiteralPath $entry.Key -Destination (Join-Path $packageRoot $entry.Value)
    }
    [ordered]@{
        product = "AskBridge"
        version = $version
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot "package.json") -Encoding UTF8

    Write-Host "[4/7] Creating portable ZIP"
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    Compress-Archive -Path (Join-Path $packageRoot "*") -DestinationPath $zipPath -CompressionLevel Optimal

    Write-Host "[5/7] Creating user-level self-extracting installer"
    $setupStub = Join-Path $repoRoot "target\release\askbridge-setup.exe"
    if (-not (Test-Path -LiteralPath $setupStub -PathType Leaf)) {
        throw "Setup stub build did not produce $setupStub"
    }
    $files = @(Get-ChildItem -LiteralPath $packageRoot -File | Sort-Object Name)
    Add-SetupPayload -StubPath $setupStub -SetupPath $setupPath -PayloadFiles $files
    Wait-StableFile -Path $setupPath

    Write-Host "[6/7] Writing artifact hashes"
    $hashPath = Join-Path $artifactRoot "$packageName-SHA256SUMS.txt"
    Write-And-VerifyHashes -Paths @($zipPath, $setupPath, (Join-Path $packageRoot "askbridge.exe")) -HashPath $hashPath

    Write-Host "[7/7] Signing artifact hashes"
    $sigPath = Join-Path $artifactRoot "$packageName-SHA256SUMS.txt.sig"
    $hasKeyFile = -not [string]::IsNullOrWhiteSpace($UpdateSigningKeyFile)
    if ($hasKeyFile -and -not (Test-Path -LiteralPath $UpdateSigningKeyFile -PathType Leaf)) {
        throw "UpdateSigningKeyFile does not exist: $UpdateSigningKeyFile"
    }
    $hasKeyEnv = -not [string]::IsNullOrWhiteSpace([string]$env:ASKBRIDGE_UPDATE_SIGNING_KEY)
    if (-not $hasKeyFile -and -not $hasKeyEnv) {
        throw "An update signing key is required: pass -UpdateSigningKeyFile or set ASKBRIDGE_UPDATE_SIGNING_KEY. The updater refuses unsigned releases."
    }
    $signArguments = @("xtask", "sign-sha256sums", "--input", $hashPath, "--output", $sigPath)
    if ($hasKeyFile) {
        $signArguments += @("--key-file", $UpdateSigningKeyFile)
    }
    & cargo @signArguments
    if ($LASTEXITCODE -ne 0) { throw "cargo xtask sign-sha256sums failed with exit code $LASTEXITCODE." }

    Write-Host "Portable package: $zipPath"
    Write-Host "Installer package: $setupPath"
    Write-Host "Hashes: $hashPath"
    Write-Host "Signature: $sigPath"
}
finally {
    Pop-Location
}
