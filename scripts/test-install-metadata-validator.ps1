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

function New-InstallerFixture {
    param([string]$PackageRoot)

    New-Item -ItemType Directory -Path $PackageRoot -Force | Out-Null
    $payload = [ordered]@{
        "askbridge.exe" = "fixture-exe"
        "README.md" = "readme"
        "PRIVACY.md" = "privacy"
        "TROUBLESHOOTING.md" = "troubleshooting"
        "Uninstall-AskBridge.ps1" = "uninstall"
    }
    $payload.GetEnumerator() | ForEach-Object {
        Set-Content -LiteralPath (Join-Path $PackageRoot $_.Key) -Encoding ASCII -Value $_.Value
    }
    Copy-Item -LiteralPath (Join-Path $repoRoot "scripts\Install-AskBridge.ps1") -Destination (Join-Path $PackageRoot "Install-AskBridge.ps1")
}

function Set-PackageMetadata {
    param(
        [string]$PackageRoot,
        [string]$Product = "AskBridge",
        [string]$Version = "0.9.0-acceptance",
        [string]$Architecture = "windows-x64",
        [bool]$AutoSubmit = $false,
        [bool]$ChromeBundled = $false
    )

    [ordered]@{
        product = $Product
        version = $Version
        architecture = $Architecture
        auto_submit = $AutoSubmit
        chrome_bundled = $ChromeBundled
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PackageRoot "package.json") -Encoding UTF8
}

try {
    $packageRoot = Join-Path $root "package"
    $installRoot = Join-Path $root "installed"
    New-InstallerFixture $packageRoot
    Set-PackageMetadata $packageRoot

    Write-Host "[1/9] Accept safe package metadata"
    & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    $manifest = Get-Content -LiteralPath (Join-Path $installRoot "install-manifest.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$manifest.version -ne "0.9.0-acceptance") {
        throw "Safe package install did not write the expected manifest version."
    }
    Remove-Item -LiteralPath $installRoot -Recurse -Force

    Write-Host "[2/9] Reject unsafe install roots"
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $repoRoot
    } "InstallRoot must be a dedicated AskBridge install directory, not the package, repository, or target root."
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot (Join-Path $repoRoot "target")
    } "InstallRoot must be a dedicated AskBridge install directory, not the package, repository, or target root."
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $packageRoot
    } "InstallRoot must be a dedicated AskBridge install directory, not the package, repository, or target root."
    if (Test-Path -LiteralPath $installRoot) {
        throw "Unsafe install root checks created the install directory."
    }

    Write-Host "[3/9] Reject auto-submit package metadata"
    Set-PackageMetadata $packageRoot -AutoSubmit $true
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'auto_submit' must be false."

    Write-Host "[4/9] Reject bundled-Chrome package metadata"
    Set-PackageMetadata $packageRoot -ChromeBundled $true
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'chrome_bundled' must be false."

    Write-Host "[5/9] Reject wrong architecture package metadata"
    Set-PackageMetadata $packageRoot -Architecture "windows-arm64"
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json does not describe a safe AskBridge windows-x64 package."

    Write-Host "[6/9] Reject missing version package metadata"
    Set-PackageMetadata $packageRoot -Version ""
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'version' must be non-empty."

    Write-Host "[7/9] Reject non-string package metadata fields"
    [ordered]@{
        product = @("AskBridge")
        version = "0.9.0-acceptance"
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot "package.json") -Encoding UTF8
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'product' must be a JSON string."
    [ordered]@{
        product = "AskBridge"
        version = 900
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot "package.json") -Encoding UTF8
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'version' must be a JSON string."

    Write-Host "[8/9] Reject string-typed safety flags"
    [ordered]@{
        product = "AskBridge"
        version = "0.9.0-acceptance"
        architecture = "windows-x64"
        auto_submit = "false"
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot "package.json") -Encoding UTF8
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'auto_submit' must be the JSON boolean false."
    [ordered]@{
        product = "AskBridge"
        version = "0.9.0-acceptance"
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = "false"
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot "package.json") -Encoding UTF8
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json property 'chrome_bundled' must be the JSON boolean false."

    Write-Host "[9/9] Reject unexpected package metadata fields"
    [ordered]@{
        product = "AskBridge"
        version = "0.9.0-acceptance"
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
        legacy_auto_send = $true
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $packageRoot "package.json") -Encoding UTF8
    Invoke-ExpectedFailure {
        & (Join-Path $packageRoot "Install-AskBridge.ps1") -InstallRoot $installRoot
    } "package.json does not match the expected AskBridge package field set."

    Write-Host "Installer metadata validation passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
