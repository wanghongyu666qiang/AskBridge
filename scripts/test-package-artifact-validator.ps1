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

function Write-MinimalPeFixture {
    param([string]$Path)

    $bytes = New-Object byte[] 132
    $bytes[0] = 0x4D
    $bytes[1] = 0x5A
    $bytes[0x3C] = 0x80
    $bytes[0x80] = 0x50
    $bytes[0x81] = 0x45
    $bytes[0x82] = 0x00
    $bytes[0x83] = 0x00
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function New-PackageFixture {
    param(
        [string]$ArtifactRoot,
        [string]$Version = "9.9.9",
        [switch]$IncludeRuntimePayload,
        [switch]$NumberVersion,
        [switch]$IncludeUnexpectedMetadata,
        [switch]$StringSafetyFlags,
        [switch]$IncludeUnexpectedPortableFile,
        [switch]$IncludeHiddenPortableResidue,
        [switch]$CreateZipContentMismatch,
        [switch]$CreateBrokenZip,
        [switch]$CreateCorruptZip,
        [switch]$InvalidSetupHeader
    )

    New-Item -ItemType Directory -Path $ArtifactRoot -Force | Out-Null
    $packageName = "AskBridge-$Version"
    $portableRoot = Join-Path $ArtifactRoot $packageName
    New-Item -ItemType Directory -Path $portableRoot -Force | Out-Null

    ([ordered]@{
        "askbridge.exe" = "fixture-exe"
        "WebView2Loader.dll" = "fixture-loader"
        "README.md" = "readme"
        "PRIVACY.md" = "privacy"
        "TROUBLESHOOTING.md" = "troubleshooting"
        "Install-AskBridge.ps1" = "install"
        "Uninstall-AskBridge.ps1" = "uninstall"
    }).GetEnumerator() | ForEach-Object {
        Set-Content -LiteralPath (Join-Path $portableRoot $_.Key) -Encoding ASCII -Value $_.Value
    }
    $autoSubmit = $false
    $chromeBundled = $false
    if ($StringSafetyFlags) {
        $autoSubmit = "false"
        $chromeBundled = "false"
    }
    $metadataVersion = $Version
    if ($NumberVersion) {
        $metadataVersion = 999
    }
    $metadata = [ordered]@{
        product = "AskBridge"
        version = $metadataVersion
        architecture = "windows-x64"
        auto_submit = $autoSubmit
        chrome_bundled = $chromeBundled
    }
    if ($IncludeUnexpectedMetadata) {
        $metadata.legacy_auto_send = $true
    }
    $metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $portableRoot "package.json") -Encoding UTF8

    if ($IncludeRuntimePayload) {
        Set-Content -LiteralPath (Join-Path $portableRoot "chrome.exe") -Encoding ASCII -Value "unexpected-runtime"
    }
    if ($IncludeUnexpectedPortableFile) {
        Set-Content -LiteralPath (Join-Path $portableRoot "debug.log") -Encoding ASCII -Value "unexpected portable residue"
    }
    if ($IncludeHiddenPortableResidue) {
        $hiddenFile = Join-Path $portableRoot "debug.log"
        Set-Content -LiteralPath $hiddenFile -Encoding ASCII -Value "hidden portable residue"
        (Get-Item -LiteralPath $hiddenFile).Attributes = [IO.FileAttributes]::Hidden
        $hiddenDirectory = Join-Path $portableRoot "cache"
        New-Item -ItemType Directory -Path $hiddenDirectory -Force | Out-Null
        (Get-Item -LiteralPath $hiddenDirectory).Attributes = [IO.FileAttributes]::Hidden
    }

    $zipPath = Join-Path $ArtifactRoot "$packageName-windows-x64.zip"
    $setupPath = Join-Path $ArtifactRoot "$packageName-Setup.exe"
    if ($InvalidSetupHeader) {
        $invalidPe = New-Object byte[] 132
        $invalidPe[0] = 0x4E
        $invalidPe[1] = 0x4F
        [IO.File]::WriteAllBytes($setupPath, $invalidPe)
    }
    else {
        Write-MinimalPeFixture $setupPath
    }
    if ($CreateCorruptZip) {
        [IO.File]::WriteAllBytes($zipPath, [Text.Encoding]::ASCII.GetBytes("not a zip"))
    }
    elseif ($CreateBrokenZip) {
        $zipFixture = Join-Path $ArtifactRoot "zip-fixture"
        New-Item -ItemType Directory -Path $zipFixture -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $zipFixture "askbridge.exe") -Encoding ASCII -Value "fixture-exe"
        Compress-Archive -Path (Join-Path $zipFixture "*") -DestinationPath $zipPath -CompressionLevel Optimal
        Remove-Item -LiteralPath $zipFixture -Recurse -Force
    }
    elseif ($CreateZipContentMismatch) {
        $zipFixture = Join-Path $ArtifactRoot "zip-fixture"
        New-Item -ItemType Directory -Path $zipFixture -Force | Out-Null
        Get-ChildItem -LiteralPath $portableRoot -Force -File | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $zipFixture $_.Name)
        }
        Set-Content -LiteralPath (Join-Path $zipFixture "README.md") -Encoding ASCII -Value "tampered readme"
        Compress-Archive -Path (Join-Path $zipFixture "*") -DestinationPath $zipPath -CompressionLevel Optimal
        Remove-Item -LiteralPath $zipFixture -Recurse -Force
    }
    else {
        Compress-Archive -Path (Join-Path $portableRoot "*") -DestinationPath $zipPath -CompressionLevel Optimal
    }

    $hashPath = Join-Path $ArtifactRoot "$packageName-SHA256SUMS.txt"
    $hashLines = @()
    foreach ($path in @($zipPath, $setupPath, (Join-Path $portableRoot "askbridge.exe"))) {
        $hash = Get-FileHash -LiteralPath $path -Algorithm SHA256
        $hashLines += "{0}  {1}" -f $hash.Hash, (Split-Path -Leaf $path)
    }
    $hashLines | Set-Content -LiteralPath $hashPath -Encoding ASCII

    return $ArtifactRoot
}

function New-SourceFixture {
    param([string]$SourceRoot)

    New-Item -ItemType Directory -Path (Join-Path $SourceRoot "docs") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $SourceRoot "scripts") -Force | Out-Null
    $sourceFixturePayload = [ordered]@{
        "README.md" = "readme"
        "docs\PRIVACY.md" = "privacy"
        "docs\TROUBLESHOOTING.md" = "troubleshooting"
        "scripts\Install-AskBridge.ps1" = "install"
        "scripts\Uninstall-AskBridge.ps1" = "uninstall"
    }
    $sourceFixturePayload.GetEnumerator() | ForEach-Object {
        Set-Content -LiteralPath (Join-Path $SourceRoot $_.Key) -Encoding ASCII -Value $_.Value
    }
    return $SourceRoot
}

function Invoke-PackageValidator {
    param(
        [string]$Artifact = $validRoot,
        [string]$Version = "9.9.9",
        [string]$ReleaseExe = $validExePath,
        [string]$Source = $sourceRoot
    )

    & $validator `
        -ArtifactRoot $Artifact `
        -ExpectedVersion $Version `
        -ExpectedReleaseExePath $ReleaseExe `
        -ExpectedSourceRoot $Source
}

try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    $validator = Join-Path $repoRoot "scripts\validate-package-artifacts.ps1"
    $sourceRoot = New-SourceFixture (Join-Path $root "source")

    Write-Host "[1/9] Accept a valid minimal artifact fixture"
    $validRoot = New-PackageFixture (Join-Path $root "valid")
    $validExePath = Join-Path $validRoot "AskBridge-9.9.9\askbridge.exe"
    Invoke-PackageValidator

    Write-Host "[2/9] Reject missing final package evidence paths"
    Invoke-ExpectedFailure {
        & $validator `
            -ArtifactRoot $validRoot `
            -ExpectedReleaseExePath $validExePath `
            -ExpectedSourceRoot $sourceRoot
    } "ExpectedVersion is required for final package artifact validation."
    Invoke-ExpectedFailure {
        & $validator `
            -ArtifactRoot $validRoot `
            -ExpectedVersion "9.9.9" `
            -ExpectedSourceRoot $sourceRoot
    } "ExpectedReleaseExePath is required for final package artifact validation."
    Invoke-ExpectedFailure {
        & $validator `
            -ArtifactRoot $validRoot `
            -ExpectedVersion "9.9.9" `
            -ExpectedReleaseExePath $validExePath
    } "ExpectedSourceRoot is required for final package artifact validation."

    Write-Host "[3/9] Reject relative artifact paths"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact "relative-package"
    } "ArtifactRoot must be an explicit absolute path."

    Write-Host "[4/9] Reject wrong versions"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Version "1.0.0"
    } "Artifact names do not match expected version"

    $extraMetadataRoot = New-PackageFixture (Join-Path $root "extra-metadata") -IncludeUnexpectedMetadata
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $extraMetadataRoot
    } "Package metadata does not match the expected 1.0 field set."
    $stringSafetyFlagsRoot = New-PackageFixture (Join-Path $root "string-safety-flags") -StringSafetyFlags
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $stringSafetyFlagsRoot
    } "Package metadata property 'auto_submit' must be the JSON boolean false."
    $numberVersionRoot = New-PackageFixture (Join-Path $root "number-version") -NumberVersion
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $numberVersionRoot
    } "Package metadata property 'version' must be a JSON string."

    Write-Host "[5/9] Reject hash manifest, ZIP, and release EXE mismatches"
    $hashMismatchRoot = New-PackageFixture (Join-Path $root "hash-mismatch")
    Add-Content -LiteralPath (Join-Path $hashMismatchRoot "AskBridge-9.9.9-Setup.exe") -Encoding ASCII -Value "changed"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $hashMismatchRoot
    } "Hash verification failed for"

    $extraHashRoot = New-PackageFixture (Join-Path $root "extra-hash-target")
    $extraHashPath = Join-Path $extraHashRoot "AskBridge-9.9.9-SHA256SUMS.txt"
    $readmeHash = Get-FileHash -LiteralPath (Join-Path $extraHashRoot "AskBridge-9.9.9\README.md") -Algorithm SHA256
    Add-Content -LiteralPath $extraHashPath -Encoding ASCII -Value ("{0}  README.md" -f $readmeHash.Hash)
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $extraHashRoot
    } "SHA256SUMS includes unexpected target:"

    $zipMismatchRoot = New-PackageFixture (Join-Path $root "zip-mismatch") -CreateBrokenZip
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $zipMismatchRoot
    } "ZIP entry count does not match"
    $corruptZipRoot = New-PackageFixture (Join-Path $root "corrupt-zip") -CreateCorruptZip
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $corruptZipRoot
    } "Portable ZIP does not have the expected file header."
    $invalidSetupRoot = New-PackageFixture (Join-Path $root "invalid-setup-header") -InvalidSetupHeader
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $invalidSetupRoot
    } "Setup EXE does not have the expected PE DOS header."
    $zipContentMismatchRoot = New-PackageFixture (Join-Path $root "zip-content-mismatch") -CreateZipContentMismatch
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $zipContentMismatchRoot
    } "ZIP entry 'README.md' hash does not match the portable directory payload."

    $expectedExeMismatchPath = Join-Path $root "expected-release.exe"
    Set-Content -LiteralPath $expectedExeMismatchPath -Encoding ASCII -Value "different-release"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -ReleaseExe $expectedExeMismatchPath
    } "Packaged askbridge.exe hash does not match the expected Release EXE."

    Write-Host "[6/9] Reject relative release EXE paths"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -ReleaseExe "target\release\askbridge.exe"
    } "ExpectedReleaseExePath must be an explicit absolute path."

    Write-Host "[7/9] Reject bundled external runtimes"
    $runtimeRoot = New-PackageFixture (Join-Path $root "runtime") -IncludeRuntimePayload
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $runtimeRoot
    } "Package unexpectedly bundled external runtime files:"
    $extraPortableFileRoot = New-PackageFixture (Join-Path $root "extra-portable-file") -IncludeUnexpectedPortableFile
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $extraPortableFileRoot
    } "Portable package contains unexpected files:"
    $hiddenPortableResidueRoot = New-PackageFixture (Join-Path $root "hidden-portable-residue") -IncludeHiddenPortableResidue
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $hiddenPortableResidueRoot
    } "Portable package contains unexpected files:"

    Write-Host "[8/9] Reject unexpected top-level artifacts"
    $extraTopLevelFileRoot = New-PackageFixture (Join-Path $root "extra-top-level-file")
    Set-Content -LiteralPath (Join-Path $extraTopLevelFileRoot "AskBridge-9.9.9-Setup.sed") -Encoding ASCII -Value "stale sed"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $extraTopLevelFileRoot
    } "Artifact output contains unexpected top-level items:"

    $extraTopLevelDirectoryRoot = New-PackageFixture (Join-Path $root "extra-top-level-directory")
    New-Item -ItemType Directory -Path (Join-Path $extraTopLevelDirectoryRoot "logs") -Force | Out-Null
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Artifact $extraTopLevelDirectoryRoot
    } "Artifact output contains unexpected top-level items:"

    Write-Host "[9/9] Reject stale source payload"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator -Source "relative-source"
    } "ExpectedSourceRoot must be an explicit absolute path."

    Invoke-PackageValidator
    Set-Content -LiteralPath (Join-Path $sourceRoot "README.md") -Encoding ASCII -Value "updated readme"
    Invoke-ExpectedFailure {
        Invoke-PackageValidator
    } "Packaged README.md hash does not match source README.md."

    Write-Host "Package artifact validator acceptance passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
