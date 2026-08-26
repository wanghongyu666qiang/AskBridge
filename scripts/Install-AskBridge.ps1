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
    if ($resolved.Equals($sourceRoot, [StringComparison]::OrdinalIgnoreCase) -or
        $resolved.StartsWith($sourceRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
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
    Assert-DedicatedInstallRoot $resolved
    return $resolved
}

function Assert-DedicatedInstallRoot {
    param([string]$ResolvedInstallRoot)

    $hasGit = Test-Path -LiteralPath (Join-Path $ResolvedInstallRoot ".git")
    $hasCargoManifest = Test-Path -LiteralPath (Join-Path $ResolvedInstallRoot "Cargo.toml")
    $hasCratesDirectory = Test-Path -LiteralPath (Join-Path $ResolvedInstallRoot "crates")
    if ($hasGit -or $hasCargoManifest -or $hasCratesDirectory) {
        throw "InstallRoot must be a dedicated AskBridge install directory, not a source repository root."
    }
    $leaf = Split-Path -Leaf $ResolvedInstallRoot
    $parent = Split-Path -Parent $ResolvedInstallRoot
    $parentHasGit = (-not [string]::IsNullOrWhiteSpace($parent)) -and (Test-Path -LiteralPath (Join-Path $parent ".git"))
    $parentHasCargoManifest = (-not [string]::IsNullOrWhiteSpace($parent)) -and (Test-Path -LiteralPath (Join-Path $parent "Cargo.toml"))
    $parentHasCratesDirectory = (-not [string]::IsNullOrWhiteSpace($parent)) -and (Test-Path -LiteralPath (Join-Path $parent "crates"))
    if ($leaf.Equals("target", [StringComparison]::OrdinalIgnoreCase) -and
        -not [string]::IsNullOrWhiteSpace($parent) -and
        ($parentHasGit -or $parentHasCargoManifest -or $parentHasCratesDirectory)) {
        throw "InstallRoot must be a dedicated AskBridge install directory, not a source repository target root."
    }
}

function Find-NearestRepositoryRoot {
    param([string]$StartDirectory)

    $current = [IO.Path]::GetFullPath($StartDirectory).TrimEnd('\')
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath (Join-Path $current ".git")) {
            return $current
        }
        $pathRoot = [IO.Path]::GetPathRoot($current).TrimEnd('\')
        if ($current.Equals($pathRoot, [StringComparison]::OrdinalIgnoreCase)) {
            return $null
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

function Test-UpdateRequested {
    return (-not [string]::IsNullOrWhiteSpace([string]$env:ASKBRIDGE_UPDATE_PARENT_PID)) -or
        ([string]$env:ASKBRIDGE_RESTART_AFTER_INSTALL -eq "1")
}

function Assert-ExistingUpdateInstall {
    param(
        [string]$InstallDirectory,
        [string]$ExecutablePath
    )

    if (-not (Test-Path -LiteralPath $InstallDirectory -PathType Container) -or
        -not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        throw "ASKBRIDGE_INSTALL_ROOT must name an existing AskBridge installation for an update."
    }
    $manifestPath = Join-Path $InstallDirectory "install-manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "ASKBRIDGE_INSTALL_ROOT is missing the AskBridge install manifest."
    }
    try {
        $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    catch {
        throw "ASKBRIDGE_INSTALL_ROOT contains an invalid AskBridge install manifest."
    }
    if ($null -eq $manifest -or [string]$manifest.product -ne "AskBridge") {
        throw "ASKBRIDGE_INSTALL_ROOT does not contain an AskBridge install manifest."
    }
    try {
        $manifestRoot = [IO.Path]::GetFullPath([string]$manifest.install_root).TrimEnd('\')
    }
    catch {
        throw "ASKBRIDGE_INSTALL_ROOT install manifest contains an invalid absolute path."
    }
    if (-not $manifestRoot.Equals($InstallDirectory, [StringComparison]::OrdinalIgnoreCase)) {
        throw "ASKBRIDGE_INSTALL_ROOT does not match the existing AskBridge install manifest."
    }
}

function Get-ValidatedUpdateParentProcess {
    param(
        [string]$TargetExecutable,
        [string]$ParentPidText
    )

    if ([string]::IsNullOrWhiteSpace($ParentPidText) -or $ParentPidText -notmatch '^[1-9][0-9]*$') {
        throw "ASKBRIDGE_UPDATE_PARENT_PID must be a positive decimal process ID."
    }
    [UInt64]$numericPid = 0
    if (-not [UInt64]::TryParse($ParentPidText, [Globalization.NumberStyles]::None, [Globalization.CultureInfo]::InvariantCulture, [ref]$numericPid) -or
        $numericPid -gt [Int32]::MaxValue -or $numericPid -eq [UInt64]$PID) {
        throw "ASKBRIDGE_UPDATE_PARENT_PID must identify another valid process."
    }
    $process = Get-Process -Id ([Int32]$numericPid) -ErrorAction SilentlyContinue
    if ($null -eq $process) {
        # The application may have exited between launching Setup.exe and this check.
        return $null
    }
    try {
        if ($process.HasExited) {
            return $null
        }
        $processPath = [IO.Path]::GetFullPath([string]$process.Path)
    }
    catch {
        throw "ASKBRIDGE_UPDATE_PARENT_PID process path could not be verified."
    }
    if ([string]::IsNullOrWhiteSpace($processPath) -or
        -not $processPath.Equals($TargetExecutable, [StringComparison]::OrdinalIgnoreCase)) {
        throw "ASKBRIDGE_UPDATE_PARENT_PID does not identify the AskBridge process at the requested install root."
    }
    return $process
}

function Wait-ForUpdateParentExit {
    param(
        [string]$TargetExecutable,
        [string]$ParentPidText
    )

    $process = Get-ValidatedUpdateParentProcess $TargetExecutable $ParentPidText
    if ($null -eq $process) {
        return
    }
    try {
        $process.WaitForExit()
    }
    catch {
        if (-not $process.HasExited) {
            throw "ASKBRIDGE_UPDATE_PARENT_PID process could not be awaited: $($_.Exception.Message)"
        }
    }
}

function Start-UpdatedAskBridge {
    param(
        [string]$ExecutablePath,
        [string]$InstallDirectory
    )

    # Do not leak the one-shot updater contract into the restarted application.
    $parentPid = $env:ASKBRIDGE_UPDATE_PARENT_PID
    $restartMarker = $env:ASKBRIDGE_RESTART_AFTER_INSTALL
    $env:ASKBRIDGE_UPDATE_PARENT_PID = $null
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $null
    try {
        $process = Start-Process -FilePath $ExecutablePath -WorkingDirectory $InstallDirectory -PassThru
        if ($null -eq $process) {
            throw "Start-Process did not return a restarted AskBridge process."
        }
        Write-Host "AskBridge restarted after the update (PID $($process.Id))."
    }
    finally {
        $env:ASKBRIDGE_UPDATE_PARENT_PID = $parentPid
        $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $restartMarker
    }
}

function New-UpdateBackup {
    param(
        [string]$InstallDirectory,
        [string[]]$PayloadFiles
    )

    $backupRoot = Join-Path $InstallDirectory (".askbridge-update-backup-{0}" -f $PID)
    New-Item -ItemType Directory -Path $backupRoot -Force | Out-Null
    try {
        foreach ($file in $PayloadFiles) {
            $source = Join-Path $InstallDirectory $file
            if (Test-Path -LiteralPath $source -PathType Leaf) {
                Copy-Item -LiteralPath $source -Destination (Join-Path $backupRoot $file) -Force
            }
        }
        $manifest = Join-Path $InstallDirectory "install-manifest.json"
        if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
            throw "The existing AskBridge install manifest disappeared before backup."
        }
        Copy-Item -LiteralPath $manifest -Destination (Join-Path $backupRoot "install-manifest.json") -Force
        return $backupRoot
    }
    catch {
        if (Test-Path -LiteralPath $backupRoot) {
            Remove-Item -LiteralPath $backupRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
        throw "Could not back up the existing AskBridge installation before update: $($_.Exception.Message)"
    }
}

function Restore-UpdateBackup {
    param(
        [string]$InstallDirectory,
        [string]$BackupRoot,
        [string[]]$PayloadFiles
    )

    foreach ($file in $PayloadFiles) {
        $destination = Join-Path $InstallDirectory $file
        $backup = Join-Path $BackupRoot $file
        if (Test-Path -LiteralPath $backup -PathType Leaf) {
            Copy-Item -LiteralPath $backup -Destination $destination -Force
        }
        elseif (Test-Path -LiteralPath $destination -PathType Leaf) {
            Remove-Item -LiteralPath $destination -Force
        }
    }
    $manifestBackup = Join-Path $BackupRoot "install-manifest.json"
    $manifestDestination = Join-Path $InstallDirectory "install-manifest.json"
    if (-not (Test-Path -LiteralPath $manifestBackup -PathType Leaf)) {
        throw "The AskBridge update backup is missing install-manifest.json."
    }
    Copy-Item -LiteralPath $manifestBackup -Destination $manifestDestination -Force
}

function Invoke-UpdateTestFailure {
    param([int]$FileIndex)

    $requested = [string]$env:ASKBRIDGE_TEST_FAIL_AFTER_UPDATE_FILE
    if ([string]::IsNullOrWhiteSpace($requested)) {
        return
    }
    if ($requested -notmatch '^[1-9][0-9]*$' -or [Int64]$requested -gt [Int32]::MaxValue) {
        throw "ASKBRIDGE_TEST_FAIL_AFTER_UPDATE_FILE must be a positive decimal file index."
    }
    if ([Int32]$requested -eq $FileIndex) {
        throw "Test hook: simulated update failure after file $FileIndex."
    }
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
    "WebView2Loader.dll",
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

$updateRequested = Test-UpdateRequested
if ($updateRequested -and [string]::IsNullOrWhiteSpace([string]$env:ASKBRIDGE_UPDATE_PARENT_PID)) {
    throw "ASKBRIDGE_UPDATE_PARENT_PID is required for an update install."
}
$resolvedInstallRoot = Resolve-SafeInstallRoot $InstallRoot $packageRoot
$targetExecutable = Join-Path $resolvedInstallRoot "askbridge.exe"
if ($updateRequested) {
    Assert-ExistingUpdateInstall $resolvedInstallRoot $targetExecutable
    Wait-ForUpdateParentExit $targetExecutable ([string]$env:ASKBRIDGE_UPDATE_PARENT_PID)
}
$running = @(Get-RunningInstalledProcess $targetExecutable)
if ($running.Count -gt 0) {
    throw "Close the installed AskBridge process before installing or upgrading."
}

$updateBackupRoot = $null
if ($updateRequested) {
    $updateBackupRoot = New-UpdateBackup $resolvedInstallRoot $requiredFiles
}
try {
    New-Item -ItemType Directory -Path $resolvedInstallRoot -Force | Out-Null
    $fileIndex = 0
    foreach ($file in $requiredFiles) {
        Install-FileAtomically -Source (Join-Path $packageRoot $file) -Destination (Join-Path $resolvedInstallRoot $file)
        $fileIndex++
        if ($updateRequested) {
            Invoke-UpdateTestFailure $fileIndex
        }
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
    if ($updateRequested) {
        Start-UpdatedAskBridge $targetExecutable $resolvedInstallRoot
    }
}
catch {
    $failure = $_
    if ($updateRequested -and $null -ne $updateBackupRoot) {
        try {
            Restore-UpdateBackup $resolvedInstallRoot $updateBackupRoot $requiredFiles
        }
        catch {
            throw "AskBridge update failed and rollback failed: $($_.Exception.Message). Original failure: $($failure.Exception.Message)"
        }
    }
    throw $failure
}
finally {
    if ($null -ne $updateBackupRoot -and (Test-Path -LiteralPath $updateBackupRoot)) {
        Remove-Item -LiteralPath $updateBackupRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
