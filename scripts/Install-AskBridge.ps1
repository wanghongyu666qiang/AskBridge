[CmdletBinding()]
param(
    [string]$InstallRoot,
    [switch]$StartOnLogin,
    [switch]$CreateStartMenuShortcut
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Resolve-SafeInstallRoot {
    param([string]$RequestedPath, [string]$PackageRoot)

    if ([string]::IsNullOrWhiteSpace($RequestedPath)) {
        $RequestedPath = $env:ASKBRIDGE_INSTALL_ROOT
    }
    if ([string]::IsNullOrWhiteSpace($RequestedPath)) {
        $RequestedPath = Read-Host "请输入 AskBridge 安装目录的绝对路径（不会默认写入 C 盘）"
    }
    if ([string]::IsNullOrWhiteSpace($RequestedPath) -or -not [IO.Path]::IsPathRooted($RequestedPath)) {
        throw "InstallRoot must be a non-empty absolute path."
    }
    $resolved = [IO.Path]::GetFullPath($RequestedPath).TrimEnd('\')
    $root = [IO.Path]::GetPathRoot($resolved).TrimEnd('\')
    if ($resolved.Equals($root, [StringComparison]::OrdinalIgnoreCase)) {
        throw "A drive root cannot be used as the AskBridge install directory."
    }
    $sourceRoot = [IO.Path]::GetFullPath($PackageRoot).TrimEnd('\')
    if ($resolved.Equals($sourceRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "InstallRoot must be a dedicated AskBridge install directory, not the package, repository, or target root."
    }
    $repositoryRoot = Find-NearestRepositoryRoot $sourceRoot
    if ($null -ne $repositoryRoot) {
        $repositoryTargetRoot = [IO.Path]::GetFullPath((Join-Path $repositoryRoot "target")).TrimEnd('\')
        if ($resolved.Equals($repositoryRoot, [StringComparison]::OrdinalIgnoreCase) -or
            $resolved.Equals($repositoryTargetRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "InstallRoot must be a dedicated AskBridge install directory, not the package, repository, or target root."
        }
    }
    return $resolved
}

function Find-NearestRepositoryRoot {
    param([string]$StartDirectory)

    $current = [IO.Path]::GetFullPath($StartDirectory).TrimEnd('\')
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath (Join-Path $current ".git")) {
            return $current
        }
        $parent = Split-Path -Parent $current
        if ([string]::IsNullOrWhiteSpace($parent) -or $parent.Equals($current, [StringComparison]::OrdinalIgnoreCase)) {
            return $null
        }
        $current = [IO.Path]::GetFullPath($parent).TrimEnd('\')
    }
    return $null
}

function Get-RunningInstalledProcess {
    param([string]$ExecutablePath)

    return Get-Process -Name askbridge -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -and $_.Path.Equals($ExecutablePath, [StringComparison]::OrdinalIgnoreCase) }
}

function Install-FileAtomically {
    param([string]$Source, [string]$Destination)

    $temporary = "$Destination.update-$PID"
    Copy-Item -LiteralPath $Source -Destination $temporary -Force
    try {
        Move-Item -LiteralPath $temporary -Destination $Destination -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

function Assert-PackageMetadataShape {
    param([psobject]$Metadata)

    $expectedProperties = @("architecture", "auto_submit", "chrome_bundled", "product", "version")
    $actualProperties = @($Metadata.PSObject.Properties.Name | Sort-Object)
    $missingProperties = @($expectedProperties | Where-Object { $_ -notin $actualProperties })
    $unexpectedProperties = @($actualProperties | Where-Object { $_ -notin $expectedProperties })
    if ($missingProperties.Count -gt 0 -or $unexpectedProperties.Count -gt 0) {
        throw "package.json does not match the expected AskBridge package field set. missing=$($missingProperties -join ',') unexpected=$($unexpectedProperties -join ',')"
    }
}

function Assert-BooleanFalseProperty {
    param([psobject]$Metadata, [string]$Name)

    $value = $Metadata.PSObject.Properties[$Name].Value
    if ($value -isnot [bool]) {
        throw "package.json property '$Name' must be the JSON boolean false."
    }
    if ($value -ne $false) {
        throw "package.json property '$Name' must be false."
    }
}

function Assert-StringProperty {
    param([psobject]$Metadata, [string]$Name)

    $value = $Metadata.PSObject.Properties[$Name].Value
    if ($value -isnot [string]) {
        throw "package.json property '$Name' must be a JSON string."
    }
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "package.json property '$Name' must be non-empty."
    }
}

function Enable-StartOnLoginInConfig {
    param([string]$InstallDirectory)

    $dataDirectory = Join-Path $InstallDirectory "data"
    $configPath = Join-Path $dataDirectory "config.json"
    New-Item -ItemType Directory -Path $dataDirectory -Force | Out-Null
    if (Test-Path -LiteralPath $configPath -PathType Leaf) {
        $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($null -eq $config.general) {
            $config | Add-Member -MemberType NoteProperty -Name general -Value ([PSCustomObject]@{})
        }
        if ($null -eq $config.general.PSObject.Properties["start_on_login"]) {
            $config.general | Add-Member -MemberType NoteProperty -Name start_on_login -Value $true
        }
        else {
            $config.general.start_on_login = $true
        }
        if ($null -ne $config.general.PSObject.Properties["auto_submit"]) {
            $config.general.auto_submit = $false
        }
    }
    else {
        $config = [ordered]@{
            schema_version = 3
            general = [ordered]@{
                start_on_login = $true
                auto_submit = $false
            }
        }
    }

    $temporary = "$configPath.install-$PID"
    $json = $config | ConvertTo-Json -Depth 16
    [IO.File]::WriteAllText($temporary, $json, (New-Object Text.UTF8Encoding($false)))
    try {
        if (Test-Path -LiteralPath $configPath -PathType Leaf) {
            [IO.File]::Replace($temporary, $configPath, $null, $true)
        }
        else {
            Move-Item -LiteralPath $temporary -Destination $configPath
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

$packageRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSCommandPath)).TrimEnd('\')
$packageMetadataPath = Join-Path $packageRoot "package.json"
if (-not (Test-Path -LiteralPath $packageMetadataPath -PathType Leaf)) {
    throw "package.json is missing from the AskBridge installer package."
}
$packageMetadata = Get-Content -LiteralPath $packageMetadataPath -Raw -Encoding UTF8 | ConvertFrom-Json
Assert-PackageMetadataShape $packageMetadata
Assert-StringProperty $packageMetadata "product"
Assert-StringProperty $packageMetadata "version"
Assert-StringProperty $packageMetadata "architecture"
if ($packageMetadata.product -ne "AskBridge" -or
    $packageMetadata.architecture -ne "windows-x64") {
    throw "package.json does not describe a safe AskBridge windows-x64 package."
}
Assert-BooleanFalseProperty $packageMetadata "auto_submit"
Assert-BooleanFalseProperty $packageMetadata "chrome_bundled"
$requiredFiles = @(
    "askbridge.exe",
    "README.md",
    "PRIVACY.md",
    "TROUBLESHOOTING.md",
    "Uninstall-AskBridge.ps1",
    "package.json"
)
foreach ($file in $requiredFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $file) -PathType Leaf)) {
        throw "Installer payload is incomplete: $file is missing."
    }
}

$resolvedInstallRoot = Resolve-SafeInstallRoot $InstallRoot $packageRoot
$targetExecutable = Join-Path $resolvedInstallRoot "askbridge.exe"
$running = @(Get-RunningInstalledProcess $targetExecutable)
if ($running.Count -gt 0) {
    throw "Close the installed AskBridge process before installing or upgrading."
}

New-Item -ItemType Directory -Path $resolvedInstallRoot -Force | Out-Null
foreach ($file in $requiredFiles) {
    Install-FileAtomically -Source (Join-Path $packageRoot $file) -Destination (Join-Path $resolvedInstallRoot $file)
}

$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
if ($StartOnLogin) {
    Enable-StartOnLoginInConfig $resolvedInstallRoot
    New-Item -Path $runKey -Force | Out-Null
    New-ItemProperty -Path $runKey -Name "AskBridge" -Value ('"' + $targetExecutable + '"') -PropertyType String -Force | Out-Null
}

$shortcutPath = $null
if ($CreateStartMenuShortcut) {
    $shortcutPath = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\AskBridge.lnk"
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $targetExecutable
    $shortcut.WorkingDirectory = $resolvedInstallRoot
    $shortcut.Description = "AskBridge"
    $shortcut.Save()
}

$installManifest = [ordered]@{
    product = "AskBridge"
    version = [string]$packageMetadata.version
    install_root = $resolvedInstallRoot
    installed_at = [DateTimeOffset]::Now.ToString("o")
    files = $requiredFiles
    data_directory = (Join-Path $resolvedInstallRoot "data")
    start_menu_shortcut = $shortcutPath
}
$manifestPath = Join-Path $resolvedInstallRoot "install-manifest.json"
$installManifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding UTF8

Write-Host "AskBridge $($packageMetadata.version) installed or upgraded at $resolvedInstallRoot"
Write-Host "User data is stored at $(Join-Path $resolvedInstallRoot 'data') and is preserved by upgrades."
