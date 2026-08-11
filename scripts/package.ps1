[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

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

function Write-And-VerifyHashes {
    param(
        [string[]]$Paths,
        [string]$HashPath
    )

    $hashLines = foreach ($path in $Paths) {
        $hash = Get-FileHash -LiteralPath $path -Algorithm SHA256
        "{0}  {1}" -f $hash.Hash, (Split-Path -Leaf $path)
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
        $actualHash = (Get-FileHash -LiteralPath $matches[0] -Algorithm SHA256).Hash
        if ($actualHash -ne $expectedHash) {
            throw "Hash verification failed for '$leafName'."
        }
    }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if (-not [IO.Path]::IsPathRooted($ArtifactRoot)) {
    throw "ArtifactRoot must be an explicit absolute path."
}
$artifactRoot = [IO.Path]::GetFullPath($ArtifactRoot).TrimEnd('\')
New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null

Push-Location $repoRoot
try {
    Assert-Command "cargo"
    Assert-Command "iexpress.exe"
    Write-Host "[1/6] Resolving package version"
    $metadata = cargo metadata --offline --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE." }
    $package = $metadata.packages | Where-Object { $_.name -eq "askbridge-win" } | Select-Object -First 1
    if ($null -eq $package) { throw "askbridge-win metadata was not found." }
    $version = [string]$package.version
    $packageName = "AskBridge-$version"
    $packageRoot = Join-Path $artifactRoot $packageName
    $zipPath = Join-Path $artifactRoot "$packageName-windows-x64.zip"
    $setupPath = Join-Path $artifactRoot "$packageName-Setup.exe"
    $sedPath = Join-Path $artifactRoot "$packageName-Setup.sed"

    foreach ($target in @($packageRoot, $zipPath, $setupPath, $sedPath)) {
        $resolved = [IO.Path]::GetFullPath($target)
        if (-not $resolved.StartsWith([IO.Path]::GetFullPath($artifactRoot) + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "Resolved package target is outside the artifact directory: $resolved"
        }
    }

    Write-Host "[2/6] Building release binary"
    cargo build --workspace --release --offline
    if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE." }

    Write-Host "[3/6] Preparing deterministic package directory"
    if (Test-Path -LiteralPath $packageRoot) {
        Remove-Item -LiteralPath $packageRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
    $payload = [ordered]@{
        (Join-Path $repoRoot "target\release\askbridge.exe") = "askbridge.exe"
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

    Write-Host "[4/6] Creating portable ZIP"
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    Compress-Archive -Path (Join-Path $packageRoot "*") -DestinationPath $zipPath -CompressionLevel Optimal

    Write-Host "[5/6] Creating user-level self-extracting installer"
    $files = @(Get-ChildItem -LiteralPath $packageRoot -File | Sort-Object Name)
    $strings = [Collections.Generic.List[string]]::new()
    $sourceEntries = [Collections.Generic.List[string]]::new()
    for ($index = 0; $index -lt $files.Count; $index++) {
        $strings.Add(('FILE{0}="{1}"' -f $index, $files[$index].Name))
        $sourceEntries.Add(('%FILE{0}%=' -f $index))
    }
    $sed = @(
        "[Version]",
        "Class=IEXPRESS",
        "SEDVersion=3",
        "[Options]",
        "PackagePurpose=InstallApp",
        "ShowInstallProgramWindow=1",
        "HideExtractAnimation=0",
        "UseLongFileName=1",
        "InsideCompressed=0",
        "CAB_FixedSize=0",
        "CAB_ResvCodeSigning=0",
        "RebootMode=N",
        "InstallPrompt=%InstallPrompt%",
        "DisplayLicense=%DisplayLicense%",
        "FinishMessage=%FinishMessage%",
        "TargetName=%TargetName%",
        "FriendlyName=%FriendlyName%",
        "AppLaunched=%AppLaunched%",
        "PostInstallCmd=<None>",
        "AdminQuietInstCmd=",
        "UserQuietInstCmd=",
        "SourceFiles=SourceFiles",
        "[Strings]",
        "InstallPrompt=",
        "DisplayLicense=",
        "FinishMessage=",
        "TargetName=$setupPath",
        "FriendlyName=AskBridge $version Installer",
        "AppLaunched=powershell.exe -NoProfile -ExecutionPolicy Bypass -File Install-AskBridge.ps1"
    ) + $strings + @(
        "[SourceFiles]",
        "SourceFiles0=$packageRoot\",
        "[SourceFiles0]"
    ) + $sourceEntries
    $sed | Set-Content -LiteralPath $sedPath -Encoding Unicode
    & iexpress.exe /N /Q $sedPath
    $iexpressExitCode = $LASTEXITCODE
    if ($iexpressExitCode -ne 0) {
        throw "IExpress installer creation failed with exit code $iexpressExitCode."
    }
    Wait-StableFile -Path $setupPath
    Remove-Item -LiteralPath $sedPath -Force

    Write-Host "[6/6] Writing artifact hashes"
    $hashPath = Join-Path $artifactRoot "$packageName-SHA256SUMS.txt"
    Write-And-VerifyHashes -Paths @($zipPath, $setupPath, (Join-Path $packageRoot "askbridge.exe")) -HashPath $hashPath
    Write-Host "Portable package: $zipPath"
    Write-Host "Installer package: $setupPath"
    Write-Host "Hashes: $hashPath"
}
finally {
    Pop-Location
}
