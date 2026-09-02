[CmdletBinding()]
param(
    [string]$InstallRoot = (Split-Path -Parent $PSCommandPath),
    [switch]$RemoveData,
    [switch]$PreserveData
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($RemoveData -and $PreserveData) {
    throw "RemoveData and PreserveData cannot be used together."
}
if (-not [IO.Path]::IsPathRooted($InstallRoot)) {
    throw "InstallRoot must be an absolute path."
}
$resolvedInstallRoot = [IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
$driveRoot = [IO.Path]::GetPathRoot($resolvedInstallRoot).TrimEnd('\')
if ($resolvedInstallRoot.Equals($driveRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to uninstall from a drive root."
}

function Get-ManifestPropertyValue {
    param([psobject]$Manifest, [string]$Name)

    return $Manifest.PSObject.Properties[$Name].Value
}

function Assert-InstallManifestShape {
    param([psobject]$Manifest)

    $expectedProperties = @("data_directory", "desktop_shortcut", "files", "install_root", "installed_at", "product", "start_menu_shortcut", "version")
    $actualProperties = @($Manifest.PSObject.Properties.Name | Sort-Object)
    $missingProperties = @($expectedProperties | Where-Object { $_ -notin $actualProperties })
    $unexpectedProperties = @($actualProperties | Where-Object { $_ -notin $expectedProperties })
    if ($missingProperties.Count -gt 0 -or $unexpectedProperties.Count -gt 0) {
        throw "The install manifest does not match the expected AskBridge field set. missing=$($missingProperties -join ',') unexpected=$($unexpectedProperties -join ',')"
    }
}

function Assert-ManifestStringProperty {
    param([psobject]$Manifest, [string]$Name)

    $value = Get-ManifestPropertyValue $Manifest $Name
    if ($value -isnot [string]) {
        throw "The install manifest property '$Name' must be a JSON string."
    }
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "The install manifest property '$Name' must be non-empty."
    }
    return [string]$value
}

function Assert-ManifestNullableStringProperty {
    param([psobject]$Manifest, [string]$Name)

    $value = Get-ManifestPropertyValue $Manifest $Name
    if ($null -eq $value) {
        return $null
    }
    if ($value -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$value)) {
        throw "The install manifest property '$Name' must be null or a non-empty JSON string."
    }
    return [string]$value
}

function Assert-ManifestTimestampProperty {
    param([psobject]$Manifest, [string]$Name)

    $value = Get-ManifestPropertyValue $Manifest $Name
    if ($value -is [string]) {
        $timestamp = [string]$value
    }
    elseif ($value -is [DateTime]) {
        # PowerShell 7 automatically converts ISO JSON strings to DateTime.
        # Windows PowerShell keeps the same JSON value as a string.
        $timestamp = $value.ToString("o", [Globalization.CultureInfo]::InvariantCulture)
    }
    elseif ($value -is [DateTimeOffset]) {
        $timestamp = $value.ToString("o", [Globalization.CultureInfo]::InvariantCulture)
    }
    else {
        throw "The install manifest property '$Name' must be a JSON timestamp string."
    }
    if ([string]::IsNullOrWhiteSpace($timestamp)) {
        throw "The install manifest property '$Name' must be non-empty."
    }
    return $timestamp
}

function Remove-ShortcutOwnedByInstall {
    param(
        [string]$ShortcutPath,
        [string]$ExecutablePath
    )

    if (-not (Test-Path -LiteralPath $ShortcutPath -PathType Leaf)) {
        return
    }
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($ShortcutPath)
    if ([string]::IsNullOrWhiteSpace([string]$shortcut.TargetPath)) {
        Write-Warning "The recorded AskBridge shortcut has no target and was preserved: $ShortcutPath"
        return
    }
    $actualTarget = [IO.Path]::GetFullPath([string]$shortcut.TargetPath)
    $expectedTarget = [IO.Path]::GetFullPath($ExecutablePath)
    if (-not $actualTarget.Equals($expectedTarget, [StringComparison]::OrdinalIgnoreCase)) {
        Write-Warning "The recorded AskBridge shortcut now targets another program and was preserved: $ShortcutPath"
        return
    }
    Remove-Item -LiteralPath $ShortcutPath -Force
}

function Remove-StartOnLoginOwnedByInstall {
    param([string]$ExecutablePath)

    $runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
    $entry = Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
    if ($null -eq $entry) {
        return
    }
    $actualValue = [string]$entry.AskBridge
    $expectedValue = '"' + [IO.Path]::GetFullPath($ExecutablePath) + '"'
    if ($actualValue.Equals($expectedValue, [StringComparison]::OrdinalIgnoreCase)) {
        Remove-ItemProperty -Path $runKey -Name "AskBridge" -Force
    }
}

function Assert-ManifestFileList {
    param([psobject]$Manifest)

    $value = Get-ManifestPropertyValue $Manifest "files"
    if ($null -eq $value -or $value -is [string]) {
        throw "The install manifest file list must be a JSON array of expected files."
    }
    $files = @($value)
    $expectedFiles = @(
        "askbridge.exe",
        "WebView2Loader.dll",
        "README.md",
        "PRIVACY.md",
        "TROUBLESHOOTING.md",
        "Uninstall-AskBridge.ps1",
        "package.json"
    )
    if ($files.Count -ne $expectedFiles.Count) {
        throw "The install manifest file list does not match the expected AskBridge payload."
    }
    foreach ($file in $files) {
        if ($file -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$file)) {
            throw "The install manifest file list must contain only non-empty JSON strings."
        }
    }
    foreach ($expectedFile in $expectedFiles) {
        $matches = @($files | Where-Object { ([string]$_).Equals($expectedFile, [StringComparison]::OrdinalIgnoreCase) })
        if ($matches.Count -ne 1) {
            throw "The install manifest file list does not match the expected AskBridge payload."
        }
    }
    foreach ($file in $files) {
        $matches = @($expectedFiles | Where-Object { $_.Equals([string]$file, [StringComparison]::OrdinalIgnoreCase) })
        if ($matches.Count -ne 1) {
            throw "The install manifest file list does not match the expected AskBridge payload."
        }
    }
    return @($files | ForEach-Object { [string]$_ })
}

$manifestPath = Join-Path $resolvedInstallRoot "install-manifest.json"
if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "install-manifest.json is missing; refusing an unscoped uninstall."
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
Assert-InstallManifestShape $manifest
$manifestProduct = Assert-ManifestStringProperty $manifest "product"
if (-not $manifestProduct.Equals("AskBridge", [StringComparison]::Ordinal)) {
    throw "The install manifest does not identify AskBridge."
}
$manifestVersion = Assert-ManifestStringProperty $manifest "version"
$manifestInstalledAt = Assert-ManifestTimestampProperty $manifest "installed_at"
$manifestInstallRoot = Assert-ManifestStringProperty $manifest "install_root"
$manifestDataRoot = Assert-ManifestStringProperty $manifest "data_directory"
$manifestDesktopShortcut = Assert-ManifestNullableStringProperty $manifest "desktop_shortcut"
$manifestStartMenuShortcut = Assert-ManifestNullableStringProperty $manifest "start_menu_shortcut"
$manifestFiles = Assert-ManifestFileList $manifest
if (-not ([IO.Path]::GetFullPath($manifestInstallRoot).TrimEnd('\')).Equals($resolvedInstallRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "The install manifest belongs to a different directory."
}
if (-not ([IO.Path]::GetFullPath($manifestDataRoot).TrimEnd('\')).Equals([IO.Path]::GetFullPath((Join-Path $resolvedInstallRoot "data")).TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw "The install manifest data directory does not belong to this installation."
}
[DateTimeOffset]::Parse($manifestInstalledAt) | Out-Null
$manifestVersion | Out-Null

$targetExecutable = Join-Path $resolvedInstallRoot "askbridge.exe"
$running = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.Equals($targetExecutable, [StringComparison]::OrdinalIgnoreCase) })
if ($running.Count -gt 0) {
    throw "Close the installed AskBridge process before uninstalling."
}

$filesToRemove = @()
foreach ($file in $manifestFiles) {
    $candidate = [IO.Path]::GetFullPath((Join-Path $resolvedInstallRoot ([string]$file)))
    if (-not $candidate.StartsWith($resolvedInstallRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "The install manifest contains an out-of-scope file path."
    }
    $filesToRemove += $candidate
}
$startMenuShortcutToRemove = $null
if ($null -ne $manifestStartMenuShortcut) {
    $startMenuShortcutToRemove = [IO.Path]::GetFullPath($manifestStartMenuShortcut)
    $programsRoot = [IO.Path]::GetFullPath((Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs")).TrimEnd('\')
    if ([string]$env:ASKBRIDGE_INSTALLER_TEST_MODE -eq "1" -and -not [string]::IsNullOrWhiteSpace([string]$env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT)) {
        $programsRoot = [IO.Path]::GetFullPath((Join-Path $env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT "StartMenuPrograms")).TrimEnd('\')
    }
    $expectedShortcut = [IO.Path]::GetFullPath((Join-Path $programsRoot "AskBridge.lnk"))
    if (-not $startMenuShortcutToRemove.Equals($expectedShortcut, [StringComparison]::OrdinalIgnoreCase)) {
        throw "The install manifest contains an out-of-scope start menu shortcut path."
    }
}
$desktopShortcutToRemove = $null
if ($null -ne $manifestDesktopShortcut) {
    $desktopShortcutToRemove = [IO.Path]::GetFullPath($manifestDesktopShortcut)
    $desktopRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    if ([string]$env:ASKBRIDGE_INSTALLER_TEST_MODE -eq "1" -and -not [string]::IsNullOrWhiteSpace([string]$env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT)) {
        $desktopRoot = Join-Path $env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT "Desktop"
    }
    $expectedShortcut = [IO.Path]::GetFullPath((Join-Path $desktopRoot "AskBridge.lnk"))
    if (-not $desktopShortcutToRemove.Equals($expectedShortcut, [StringComparison]::OrdinalIgnoreCase)) {
        throw "The install manifest contains an out-of-scope desktop shortcut path."
    }
}

Remove-StartOnLoginOwnedByInstall $targetExecutable
if ($null -ne $startMenuShortcutToRemove) {
    Remove-ShortcutOwnedByInstall $startMenuShortcutToRemove $targetExecutable
}
if ($null -ne $desktopShortcutToRemove) {
    Remove-ShortcutOwnedByInstall $desktopShortcutToRemove $targetExecutable
}

$deleteData = $RemoveData
if (-not $RemoveData -and -not $PreserveData) {
    Write-Host "AskBridge data may contain settings and the dedicated Chrome login state."
    $answer = Read-Host "Delete the data directory too? Type DELETE to confirm"
    $deleteData = $answer -ceq "DELETE"
}

foreach ($candidate in $filesToRemove) {
    Remove-Item -LiteralPath $candidate -Force -ErrorAction SilentlyContinue
}
Remove-Item -LiteralPath $manifestPath -Force -ErrorAction SilentlyContinue

$dataRoot = [IO.Path]::GetFullPath((Join-Path $resolvedInstallRoot "data"))
if ($deleteData) {
    if (-not $dataRoot.StartsWith($resolvedInstallRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Resolved data path is outside the install directory."
    }
    Remove-Item -LiteralPath $dataRoot -Recurse -Force -ErrorAction SilentlyContinue
}

$remaining = @(Get-ChildItem -LiteralPath $resolvedInstallRoot -Force -ErrorAction SilentlyContinue)
if ($remaining.Count -eq 0) {
    Remove-Item -LiteralPath $resolvedInstallRoot -Force -ErrorAction SilentlyContinue
}

if ($deleteData) {
    Write-Host "AskBridge, its startup entry, and its user data were removed."
} else {
    Write-Host "AskBridge and its startup entry were removed. User data was preserved at $dataRoot"
}
