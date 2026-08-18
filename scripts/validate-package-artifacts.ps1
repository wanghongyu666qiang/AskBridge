[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ArtifactRoot,
    [string]$ExpectedVersion,
    [string]$ExpectedReleaseExePath,
    [string]$ExpectedSourceRoot,
    [int64]$MaxReleaseExeBytes = 15MB,
    [int64]$MaxSetupBytes = 25MB,
    [int64]$MaxStaticResourceBytes = 2MB
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ExpectedVersion)) {
    throw "ExpectedVersion is required for final package artifact validation."
}
if ([string]::IsNullOrWhiteSpace($ExpectedReleaseExePath)) {
    throw "ExpectedReleaseExePath is required for final package artifact validation."
}
if ([string]::IsNullOrWhiteSpace($ExpectedSourceRoot)) {
    throw "ExpectedSourceRoot is required for final package artifact validation."
}

function Resolve-RequiredDirectory {
    param([string]$Path, [string]$Label)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathRooted($Path)) {
        throw "$Label must be an explicit absolute path."
    }
    $resolved = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        throw "$Label does not exist: $resolved"
    }
    return $resolved
}

function Resolve-RequiredFile {
    param([string]$Path, [string]$Label)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathRooted($Path)) {
        throw "$Label must be an explicit absolute path."
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "$Label does not exist: $resolved"
    }
    return $resolved
}

function Assert-SingleFile {
    param([IO.FileInfo[]]$Files, [string]$Label)

    if ($Files.Count -ne 1) {
        throw "Artifact output must contain exactly one $Label; found $($Files.Count)."
    }
    return $Files[0]
}

function Assert-SingleDirectory {
    param([IO.DirectoryInfo[]]$Directories, [string]$Label)

    if ($Directories.Count -ne 1) {
        throw "Artifact output must contain exactly one $Label; found $($Directories.Count)."
    }
    return $Directories[0]
}

function Get-ZipEntryNames {
    param([string]$Path)

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        return @($archive.Entries | ForEach-Object { $_.FullName.Replace('/', '\') })
    }
    finally {
        $archive.Dispose()
    }
}

function Get-ZipEntryHash {
    param([string]$ZipPath, [string]$EntryName)

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($ZipPath)
    try {
        $entry = $archive.Entries | Where-Object { $_.FullName.Replace('/', '\') -eq $EntryName } | Select-Object -First 1
        if ($null -eq $entry) {
            throw "ZIP entry is missing during hash verification: $EntryName"
        }
        $sha256 = [Security.Cryptography.SHA256]::Create()
        $stream = $entry.Open()
        try {
            return [BitConverter]::ToString($sha256.ComputeHash($stream)).Replace("-", "")
        }
        finally {
            $stream.Dispose()
            $sha256.Dispose()
        }
    }
    finally {
        $archive.Dispose()
    }
}

function Assert-FileStartsWithBytes {
    param([string]$Path, [byte[]]$ExpectedPrefix, [string]$Label)

    $stream = [IO.File]::OpenRead($Path)
    try {
        if ($stream.Length -lt $ExpectedPrefix.Length) {
            throw "$Label is too small to be a valid artifact."
        }
        $buffer = New-Object byte[] $ExpectedPrefix.Length
        $bytesRead = $stream.Read($buffer, 0, $buffer.Length)
        if ($bytesRead -ne $ExpectedPrefix.Length) {
            throw "$Label is too small to be a valid artifact."
        }
        for ($index = 0; $index -lt $ExpectedPrefix.Length; $index++) {
            if ($buffer[$index] -ne $ExpectedPrefix[$index]) {
                throw "$Label does not have the expected file header."
            }
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-PortableExecutableHeader {
    param([string]$Path, [string]$Label)

    $stream = [IO.File]::OpenRead($Path)
    try {
        if ($stream.Length -lt 0x40) {
            throw "$Label is too small to be a valid PE artifact."
        }
        $reader = New-Object IO.BinaryReader $stream
        try {
            if ($reader.ReadByte() -ne 0x4D -or $reader.ReadByte() -ne 0x5A) {
                throw "$Label does not have the expected PE DOS header."
            }
            $stream.Seek(0x3C, [IO.SeekOrigin]::Begin) | Out-Null
            $peOffset = $reader.ReadInt32()
            if ($peOffset -lt 0x40 -or $peOffset -gt ($stream.Length - 4)) {
                throw "$Label has an invalid PE header offset."
            }
            $stream.Seek($peOffset, [IO.SeekOrigin]::Begin) | Out-Null
            if ($reader.ReadByte() -ne 0x50 -or
                $reader.ReadByte() -ne 0x45 -or
                $reader.ReadByte() -ne 0x00 -or
                $reader.ReadByte() -ne 0x00) {
                throw "$Label does not have the expected PE signature."
            }
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-PackageMetadataShape {
    param([psobject]$Metadata)

    $expectedProperties = @("architecture", "auto_submit", "chrome_bundled", "product", "version")
    $actualProperties = @($Metadata.PSObject.Properties.Name | Sort-Object)
    $missingProperties = @($expectedProperties | Where-Object { $_ -notin $actualProperties })
    $unexpectedProperties = @($actualProperties | Where-Object { $_ -notin $expectedProperties })
    if ($missingProperties.Count -gt 0 -or $unexpectedProperties.Count -gt 0) {
        throw "Package metadata does not match the expected 1.0 field set. missing=$($missingProperties -join ',') unexpected=$($unexpectedProperties -join ',')"
    }
}

function Assert-BooleanFalseProperty {
    param([psobject]$Metadata, [string]$Name)

    $value = $Metadata.PSObject.Properties[$Name].Value
    if ($value -isnot [bool]) {
        throw "Package metadata property '$Name' must be the JSON boolean false."
    }
    if ($value -ne $false) {
        throw "Package metadata property '$Name' must be false."
    }
}

function Assert-StringProperty {
    param([psobject]$Metadata, [string]$Name)

    $value = $Metadata.PSObject.Properties[$Name].Value
    if ($value -isnot [string]) {
        throw "Package metadata property '$Name' must be a JSON string."
    }
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "Package metadata property '$Name' must be non-empty."
    }
}

$artifactRoot = Resolve-RequiredDirectory $ArtifactRoot "ArtifactRoot"

Write-Host "[1/5] Verify expected artifact set"
$portableRoot = Assert-SingleDirectory @(Get-ChildItem -LiteralPath $artifactRoot -Directory -Filter "AskBridge-*") "portable directory"
$zipFile = Assert-SingleFile @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "AskBridge-*-windows-x64.zip") "portable ZIP"
$setupFile = Assert-SingleFile @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "AskBridge-*-Setup.exe") "Setup EXE"
$hashFile = Assert-SingleFile @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "AskBridge-*-SHA256SUMS.txt") "SHA256SUMS file"

$allowedTopLevelNames = @(
    $portableRoot.Name,
    $zipFile.Name,
    $setupFile.Name,
    $hashFile.Name
)
$unexpectedTopLevelItems = @(Get-ChildItem -LiteralPath $artifactRoot -Force | Where-Object {
    $_.Name -notin $allowedTopLevelNames
})
if ($unexpectedTopLevelItems.Count -gt 0) {
    throw "Artifact output contains unexpected top-level items: $($unexpectedTopLevelItems.Name -join '; ')"
}

$expectedName = "AskBridge-$ExpectedVersion"
if ($portableRoot.Name -ne $expectedName -or
    $zipFile.Name -ne "$expectedName-windows-x64.zip" -or
    $setupFile.Name -ne "$expectedName-Setup.exe" -or
    $hashFile.Name -ne "$expectedName-SHA256SUMS.txt") {
    throw "Artifact names do not match expected version $ExpectedVersion."
}
Assert-FileStartsWithBytes $zipFile.FullName ([byte[]](0x50, 0x4B)) "Portable ZIP"
Assert-PortableExecutableHeader $setupFile.FullName "Setup EXE"

$requiredPayload = @(
    "askbridge.exe",
    "WebView2Loader.dll",
    "README.md",
    "PRIVACY.md",
    "TROUBLESHOOTING.md",
    "Install-AskBridge.ps1",
    "Uninstall-AskBridge.ps1",
    "package.json"
)
foreach ($file in $requiredPayload) {
    if (-not (Test-Path -LiteralPath (Join-Path $portableRoot.FullName $file) -PathType Leaf)) {
        throw "Portable package is missing $file."
    }
}
$portableFileNames = @((Get-ChildItem -LiteralPath $portableRoot.FullName -Force -File | ForEach-Object Name) | Sort-Object)
$unexpectedRuntimePayload = @(Get-ChildItem -LiteralPath $portableRoot.FullName -Force -Recurse -File | Where-Object {
    $_.Name -match 'chrome|rust|cargo' -or
    ($_.Extension -in @(".dll", ".msi") -and $_.Name -ne "WebView2Loader.dll")
})
if ($unexpectedRuntimePayload.Count -gt 0) {
    throw "Package unexpectedly bundled external runtime files: $($unexpectedRuntimePayload.FullName -join '; ')"
}
foreach ($file in $portableFileNames) {
    if ($file -notin $requiredPayload) {
        throw "Portable package contains unexpected files: $file"
    }
}
$unexpectedPortableDirectories = @(Get-ChildItem -LiteralPath $portableRoot.FullName -Force -Directory)
if ($unexpectedPortableDirectories.Count -gt 0) {
    throw "Portable package must be flat; found directories: $($unexpectedPortableDirectories.Name -join '; ')"
}
$zipEntryNames = @(Get-ZipEntryNames $zipFile.FullName | Sort-Object)
if ($zipEntryNames.Count -ne $portableFileNames.Count) {
    throw "ZIP entry count does not match portable directory file count."
}
for ($index = 0; $index -lt $portableFileNames.Count; $index++) {
    if ($zipEntryNames[$index] -ne $portableFileNames[$index]) {
        throw "ZIP entries do not match the portable directory payload."
    }
}
foreach ($file in $portableFileNames) {
    $portableHash = (Get-FileHash -LiteralPath (Join-Path $portableRoot.FullName $file) -Algorithm SHA256).Hash
    $zipHash = Get-ZipEntryHash $zipFile.FullName $file
    if ($zipHash -ne $portableHash) {
        throw "ZIP entry '$file' hash does not match the portable directory payload."
    }
}

Write-Host "[2/5] Verify hashes"
$allFiles = @(Get-ChildItem -LiteralPath $artifactRoot -Recurse -File)
$hashTargets = @{}
foreach ($line in Get-Content -LiteralPath $hashFile.FullName -Encoding ASCII) {
    if ($line -notmatch '^([0-9A-F]{64})  (.+)$') {
        throw "Malformed SHA256SUMS line: $line"
    }
    $expectedHash = $Matches[1]
    $leaf = $Matches[2]
    if ($hashTargets.ContainsKey($leaf)) {
        throw "Duplicate hash target in SHA256SUMS: $leaf"
    }
    $matches = @($allFiles | Where-Object Name -EQ $leaf)
    if ($matches.Count -ne 1) {
        throw "Hash target '$leaf' is missing or ambiguous."
    }
    $actualHash = (Get-FileHash -LiteralPath $matches[0].FullName -Algorithm SHA256).Hash
    if ($actualHash -ne $expectedHash) {
        throw "Hash verification failed for '$leaf'."
    }
    $hashTargets[$leaf] = $true
}
foreach ($expectedLeaf in @($zipFile.Name, $setupFile.Name, "askbridge.exe")) {
    if (-not $hashTargets.ContainsKey($expectedLeaf)) {
        throw "SHA256SUMS does not include $expectedLeaf."
    }
}
$expectedHashTargets = @($zipFile.Name, $setupFile.Name, "askbridge.exe")
foreach ($leaf in $hashTargets.Keys) {
    if ($leaf -notin $expectedHashTargets) {
        throw "SHA256SUMS includes unexpected target: $leaf"
    }
}

Write-Host "[3/5] Verify package metadata"
$metadata = Get-Content -LiteralPath (Join-Path $portableRoot.FullName "package.json") -Raw -Encoding UTF8 | ConvertFrom-Json
Assert-PackageMetadataShape $metadata
Assert-StringProperty $metadata "product"
Assert-StringProperty $metadata "version"
Assert-StringProperty $metadata "architecture"
if ($metadata.product -ne "AskBridge" -or
    $metadata.architecture -ne "windows-x64") {
    throw "Package metadata does not preserve the expected 1.0 safety flags."
}
Assert-BooleanFalseProperty $metadata "auto_submit"
Assert-BooleanFalseProperty $metadata "chrome_bundled"
if ($metadata.version -ne $ExpectedVersion) {
    throw "Package metadata version is $($metadata.version), expected $ExpectedVersion."
}

Write-Host "[4/5] Verify package does not bundle external runtimes"

Write-Host "[5/5] Verify size bounds and Release EXE identity"
$releaseExe = Get-Item -LiteralPath (Join-Path $portableRoot.FullName "askbridge.exe")
if ($releaseExe.Length -gt $MaxReleaseExeBytes) {
    throw "Release EXE is $($releaseExe.Length) bytes, expected at most $MaxReleaseExeBytes."
}
$expectedReleaseExe = Resolve-RequiredFile $ExpectedReleaseExePath "ExpectedReleaseExePath"
$expectedHash = (Get-FileHash -LiteralPath $expectedReleaseExe -Algorithm SHA256).Hash
$packagedHash = (Get-FileHash -LiteralPath $releaseExe.FullName -Algorithm SHA256).Hash
if ($packagedHash -ne $expectedHash) {
    throw "Packaged askbridge.exe hash does not match the expected Release EXE. package=$packagedHash expected=$expectedHash"
}
$expectedSourceRoot = Resolve-RequiredDirectory $ExpectedSourceRoot "ExpectedSourceRoot"
$sourcePayload = [ordered]@{
    "README.md" = "README.md"
    "docs\PRIVACY.md" = "PRIVACY.md"
    "docs\TROUBLESHOOTING.md" = "TROUBLESHOOTING.md"
    "scripts\Install-AskBridge.ps1" = "Install-AskBridge.ps1"
    "scripts\Uninstall-AskBridge.ps1" = "Uninstall-AskBridge.ps1"
}
foreach ($entry in $sourcePayload.GetEnumerator()) {
    $sourcePath = Join-Path $expectedSourceRoot $entry.Key
    $packagedPath = Join-Path $portableRoot.FullName $entry.Value
    $sourceFile = Resolve-RequiredFile $sourcePath "source payload $($entry.Key)"
    $sourceHash = (Get-FileHash -LiteralPath $sourceFile -Algorithm SHA256).Hash
    $packagedHash = (Get-FileHash -LiteralPath $packagedPath -Algorithm SHA256).Hash
    if ($packagedHash -ne $sourceHash) {
        throw "Packaged $($entry.Value) hash does not match source $($entry.Key)."
    }
}
if ($setupFile.Length -gt $MaxSetupBytes) {
    throw "Setup EXE is $($setupFile.Length) bytes, expected at most $MaxSetupBytes."
}
$staticResources = @(Get-ChildItem -LiteralPath $portableRoot.FullName -Force -File | Where-Object {
    $_.Name -notin @(
        "askbridge.exe",
        "WebView2Loader.dll",
        "README.md",
        "PRIVACY.md",
        "TROUBLESHOOTING.md",
        "Install-AskBridge.ps1",
        "Uninstall-AskBridge.ps1",
        "package.json"
    )
})
$staticResourceBytes = 0
foreach ($resource in $staticResources) {
    $staticResourceBytes += $resource.Length
}
if ([int64]$staticResourceBytes -gt $MaxStaticResourceBytes) {
    throw "Static resources are $staticResourceBytes bytes, expected at most $MaxStaticResourceBytes."
}

Write-Host "Package artifact validation passed."
