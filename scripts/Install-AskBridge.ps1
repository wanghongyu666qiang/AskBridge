[CmdletBinding()]
param(
    [string]$InstallRoot,
    [switch]$StartOnLogin,
    [switch]$CreateStartMenuShortcut,
    [switch]$CreateDesktopShortcut
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$startOnLoginConfigured = $PSBoundParameters.ContainsKey("StartOnLogin")

function ConvertFrom-Utf8Base64 {
    param([string]$Value)

    return [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($Value))
}

function Test-EnabledEnvironmentFlag {
    param([string]$Name)

    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $false
    }
    if ($value -notin @("0", "1")) {
        throw "$Name must be 0 or 1 when set."
    }
    return $value -eq "1"
}

function Show-InstallOptionsDialog {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [Windows.Forms.Application]::EnableVisualStyles()

    $form = New-Object Windows.Forms.Form
    $form.Text = ConvertFrom-Utf8Base64 "5a6J6KOFIEFza0JyaWRnZQ=="
    $form.ClientSize = New-Object Drawing.Size(560, 270)
    $form.FormBorderStyle = [Windows.Forms.FormBorderStyle]::FixedDialog
    $form.StartPosition = [Windows.Forms.FormStartPosition]::CenterScreen
    $form.MaximizeBox = $false
    $form.MinimizeBox = $false
    $form.ShowIcon = $true

    $title = New-Object Windows.Forms.Label
    $title.Text = ConvertFrom-Utf8Base64 "5a6J6KOFIEFza0JyaWRnZQ=="
    $title.Font = New-Object Drawing.Font($form.Font.FontFamily, 14, [Drawing.FontStyle]::Bold)
    $title.AutoSize = $true
    $title.Location = New-Object Drawing.Point(24, 20)
    $form.Controls.Add($title)

    $description = New-Object Windows.Forms.Label
    $description.Text = ConvertFrom-Utf8Base64 "6K+36YCJ5oup54us56uL55qE5a6J6KOF55uu5b2V77yM5bm25Yaz5a6a5piv5ZCm5Yib5bu65b+r5o235pa55byP44CC5LiN5Lya6buY6K6k5YaZ5YWlIEMg55uY44CC"
    $description.AutoSize = $true
    $description.Location = New-Object Drawing.Point(26, 58)
    $form.Controls.Add($description)

    $pathLabel = New-Object Windows.Forms.Label
    $pathLabel.Text = ConvertFrom-Utf8Base64 "5a6J6KOF55uu5b2V77ya"
    $pathLabel.AutoSize = $true
    $pathLabel.Location = New-Object Drawing.Point(26, 94)
    $form.Controls.Add($pathLabel)

    $pathText = New-Object Windows.Forms.TextBox
    $pathText.Location = New-Object Drawing.Point(26, 116)
    $pathText.Size = New-Object Drawing.Size(405, 24)
    $form.Controls.Add($pathText)

    $browseButton = New-Object Windows.Forms.Button
    $browseButton.Text = ConvertFrom-Utf8Base64 "5rWP6KeILi4u"
    $browseButton.Location = New-Object Drawing.Point(443, 114)
    $browseButton.Size = New-Object Drawing.Size(88, 28)
    $browseButton.Add_Click({
        $folderDialog = New-Object Windows.Forms.FolderBrowserDialog
        $folderDialog.Description = ConvertFrom-Utf8Base64 "6YCJ5oupIEFza0JyaWRnZSDlronoo4Xnm67lvZU="
        $folderDialog.ShowNewFolderButton = $true
        if ($folderDialog.ShowDialog($form) -eq [Windows.Forms.DialogResult]::OK) {
            $pathText.Text = $folderDialog.SelectedPath
        }
        $folderDialog.Dispose()
    })
    $form.Controls.Add($browseButton)

    $desktopCheck = New-Object Windows.Forms.CheckBox
    $desktopCheck.Text = ConvertFrom-Utf8Base64 "5Yib5bu65qGM6Z2i5b+r5o235pa55byP"
    $desktopCheck.Checked = $true
    $desktopCheck.AutoSize = $true
    $desktopCheck.Location = New-Object Drawing.Point(28, 157)
    $form.Controls.Add($desktopCheck)

    $startMenuCheck = New-Object Windows.Forms.CheckBox
    $startMenuCheck.Text = ConvertFrom-Utf8Base64 "5Yib5bu65byA5aeL6I+c5Y2V5b+r5o235pa55byP"
    $startMenuCheck.Checked = $true
    $startMenuCheck.AutoSize = $true
    $startMenuCheck.Location = New-Object Drawing.Point(205, 157)
    $form.Controls.Add($startMenuCheck)

    $startupCheck = New-Object Windows.Forms.CheckBox
    $startupCheck.Text = ConvertFrom-Utf8Base64 "55m75b2VIFdpbmRvd3Mg5ZCO6Ieq5Yqo5ZCv5Yqo"
    $startupCheck.Checked = $false
    $startupCheck.AutoSize = $true
    $startupCheck.Location = New-Object Drawing.Point(28, 187)
    $form.Controls.Add($startupCheck)

    $installButton = New-Object Windows.Forms.Button
    $installButton.Text = ConvertFrom-Utf8Base64 "5a6J6KOF"
    $installButton.Location = New-Object Drawing.Point(350, 224)
    $installButton.Size = New-Object Drawing.Size(86, 30)
    $installButton.Add_Click({
        $candidate = $pathText.Text.Trim()
        if ([string]::IsNullOrWhiteSpace($candidate) -or -not [IO.Path]::IsPathRooted($candidate)) {
            [Windows.Forms.MessageBox]::Show(
                $form,
                (ConvertFrom-Utf8Base64 "6K+36YCJ5oup5LiA5Liq57ud5a+55a6J6KOF6Lev5b6E77yM5L6L5aaCIEQ6XEFwcHNcQXNrQnJpZGdl44CC"),
                (ConvertFrom-Utf8Base64 "5a6J6KOF55uu5b2V5peg5pWI"),
                [Windows.Forms.MessageBoxButtons]::OK,
                [Windows.Forms.MessageBoxIcon]::Warning
            ) | Out-Null
            return
        }
        $form.DialogResult = [Windows.Forms.DialogResult]::OK
        $form.Close()
    })
    $form.Controls.Add($installButton)

    $cancelButton = New-Object Windows.Forms.Button
    $cancelButton.Text = ConvertFrom-Utf8Base64 "5Y+W5raI"
    $cancelButton.Location = New-Object Drawing.Point(445, 224)
    $cancelButton.Size = New-Object Drawing.Size(86, 30)
    $cancelButton.DialogResult = [Windows.Forms.DialogResult]::Cancel
    $form.Controls.Add($cancelButton)

    $form.AcceptButton = $installButton
    $form.CancelButton = $cancelButton
    $pathText.Select()

    try {
        if ($form.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) {
            return $null
        }
        return [pscustomobject]@{
            InstallRoot = $pathText.Text.Trim()
            CreateDesktopShortcut = [bool]$desktopCheck.Checked
            CreateStartMenuShortcut = [bool]$startMenuCheck.Checked
            StartOnLogin = [bool]$startupCheck.Checked
        }
    }
    finally {
        $form.Dispose()
    }
}

function Get-ShortcutDirectory {
    param([ValidateSet("Desktop", "StartMenu")][string]$Kind)

    $testRoot = [string]$env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT
    if (-not [string]::IsNullOrWhiteSpace($testRoot)) {
        if ([string]$env:ASKBRIDGE_INSTALLER_TEST_MODE -ne "1" -or -not [IO.Path]::IsPathRooted($testRoot)) {
            throw "ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT requires test mode and an absolute path."
        }
        $leaf = if ($Kind -eq "Desktop") { "Desktop" } else { "StartMenuPrograms" }
        return [IO.Path]::GetFullPath((Join-Path $testRoot $leaf))
    }

    if ($Kind -eq "Desktop") {
        $directory = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
    }
    else {
        $directory = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    }
    if ([string]::IsNullOrWhiteSpace($directory) -or -not [IO.Path]::IsPathRooted($directory)) {
        throw "Windows did not provide a valid $Kind shortcut directory."
    }
    return [IO.Path]::GetFullPath($directory).TrimEnd('\')
}

function New-AskBridgeShortcut {
    param(
        [string]$ShortcutPath,
        [string]$ExecutablePath,
        [string]$WorkingDirectory
    )

    $parent = Split-Path -Parent $ShortcutPath
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $shell = New-Object -ComObject WScript.Shell
    if (Test-Path -LiteralPath $ShortcutPath -PathType Leaf) {
        $existing = $shell.CreateShortcut($ShortcutPath)
        if ([string]::IsNullOrWhiteSpace([string]$existing.TargetPath) -or
            -not ([IO.Path]::GetFullPath([string]$existing.TargetPath)).Equals([IO.Path]::GetFullPath($ExecutablePath), [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to overwrite an existing shortcut that does not belong to this AskBridge installation: $ShortcutPath"
        }
    }
    $shortcut = $shell.CreateShortcut($ShortcutPath)
    $shortcut.TargetPath = $ExecutablePath
    $shortcut.WorkingDirectory = $WorkingDirectory
    $shortcut.IconLocation = "$ExecutablePath,0"
    $shortcut.Description = "AskBridge"
    $shortcut.Save()
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
    $target = [IO.Path]::GetFullPath([string]$shortcut.TargetPath)
    if ($target.Equals([IO.Path]::GetFullPath($ExecutablePath), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $ShortcutPath -Force
    }
}

function Get-ValidatedManifestShortcut {
    param(
        [psobject]$Manifest,
        [string]$PropertyName,
        [ValidateSet("Desktop", "StartMenu")][string]$Kind
    )

    $property = $Manifest.PSObject.Properties[$PropertyName]
    if ($null -eq $property -or $null -eq $property.Value) {
        return $null
    }
    if ($property.Value -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$property.Value)) {
        throw "The existing install manifest property '$PropertyName' must be null or a non-empty string."
    }
    $actual = [IO.Path]::GetFullPath([string]$property.Value)
    $expected = [IO.Path]::GetFullPath((Join-Path (Get-ShortcutDirectory $Kind) "AskBridge.lnk"))
    if (-not $actual.Equals($expected, [StringComparison]::OrdinalIgnoreCase)) {
        throw "The existing install manifest contains an out-of-scope $Kind shortcut path."
    }
    return $actual
}

function Resolve-SafeInstallRoot {
    param([string]$RequestedPath, [string]$PackageRoot)

    if ([string]::IsNullOrWhiteSpace($RequestedPath)) {
        $RequestedPath = $env:ASKBRIDGE_INSTALL_ROOT
    }
    if ([string]::IsNullOrWhiteSpace($RequestedPath)) {
        $RequestedPath = Read-Host (ConvertFrom-Utf8Base64 "6K+36L6T5YWlIEFza0JyaWRnZSDlronoo4Xnm67lvZXnmoTnu53lr7not6/lvoTvvIjkuI3kvJrpu5jorqTlhpnlhaUgQyDnm5jvvIk=")
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

function Set-StartOnLoginInConfig {
    param(
        [string]$InstallDirectory,
        [bool]$Enabled
    )

    $dataDirectory = Join-Path $InstallDirectory "data"
    $configPath = Join-Path $dataDirectory "config.json"
    New-Item -ItemType Directory -Path $dataDirectory -Force | Out-Null
    if (Test-Path -LiteralPath $configPath -PathType Leaf) {
        $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($null -eq $config.general) {
            $config | Add-Member -MemberType NoteProperty -Name general -Value ([PSCustomObject]@{})
        }
        if ($null -eq $config.general.PSObject.Properties["start_on_login"]) {
            $config.general | Add-Member -MemberType NoteProperty -Name start_on_login -Value $Enabled
        }
        else {
            $config.general.start_on_login = $Enabled
        }
        if ($null -ne $config.general.PSObject.Properties["auto_submit"]) {
            $config.general.auto_submit = $false
        }
    }
    else {
        $config = [ordered]@{
            schema_version = 3
            general = [ordered]@{
                start_on_login = $Enabled
                auto_submit = $false
            }
        }
    }

    $temporary = "$configPath.install-$PID"
    $json = $config | ConvertTo-Json -Depth 16
    [IO.File]::WriteAllText($temporary, $json, (New-Object Text.UTF8Encoding($false)))
    try {
        if (Test-Path -LiteralPath $configPath -PathType Leaf) {
            $backup = "$configPath.install-backup-$PID"
            try {
                [IO.File]::Replace($temporary, $configPath, $backup, $true)
            }
            finally {
                if (Test-Path -LiteralPath $backup) {
                    Remove-Item -LiteralPath $backup -Force
                }
            }
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

function Remove-StartOnLoginOwnedByInstall {
    param(
        [string]$RunKey,
        [string]$ExecutablePath
    )

    $entry = Get-ItemProperty -Path $RunKey -Name "AskBridge" -ErrorAction SilentlyContinue
    if ($null -eq $entry) {
        return
    }
    $actualValue = [string]$entry.AskBridge
    $expectedValue = '"' + [IO.Path]::GetFullPath($ExecutablePath) + '"'
    if ($actualValue.Equals($expectedValue, [StringComparison]::OrdinalIgnoreCase)) {
        Remove-ItemProperty -Path $RunKey -Name "AskBridge" -Force
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
$requestedInstallRoot = $InstallRoot
if ([string]::IsNullOrWhiteSpace($requestedInstallRoot)) {
    $requestedInstallRoot = [string]$env:ASKBRIDGE_INSTALL_ROOT
}
if (-not $updateRequested -and [string]::IsNullOrWhiteSpace($requestedInstallRoot)) {
    $options = Show-InstallOptionsDialog
    if ($null -eq $options) {
        Write-Host "AskBridge installation cancelled."
        return
    }
    $InstallRoot = [string]$options.InstallRoot
    $CreateDesktopShortcut = [bool]$options.CreateDesktopShortcut
    $CreateStartMenuShortcut = [bool]$options.CreateStartMenuShortcut
    $StartOnLogin = [bool]$options.StartOnLogin
    $startOnLoginConfigured = $true
}
elseif (-not $updateRequested) {
    if (Test-EnabledEnvironmentFlag "ASKBRIDGE_CREATE_DESKTOP_SHORTCUT") {
        $CreateDesktopShortcut = $true
    }
    if (Test-EnabledEnvironmentFlag "ASKBRIDGE_CREATE_START_MENU_SHORTCUT") {
        $CreateStartMenuShortcut = $true
    }
    if (-not $startOnLoginConfigured) {
        $startOnLoginEnvironment = [Environment]::GetEnvironmentVariable("ASKBRIDGE_START_ON_LOGIN")
        if (-not [string]::IsNullOrWhiteSpace($startOnLoginEnvironment)) {
            $StartOnLogin = Test-EnabledEnvironmentFlag "ASKBRIDGE_START_ON_LOGIN"
            $startOnLoginConfigured = $true
        }
    }
}
$resolvedInstallRoot = Resolve-SafeInstallRoot $InstallRoot $packageRoot
$targetExecutable = Join-Path $resolvedInstallRoot "askbridge.exe"
$existingManifest = $null
if ($updateRequested) {
    Assert-ExistingUpdateInstall $resolvedInstallRoot $targetExecutable
    $existingManifest = Get-Content -LiteralPath (Join-Path $resolvedInstallRoot "install-manifest.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    $existingStartMenuShortcut = Get-ValidatedManifestShortcut $existingManifest "start_menu_shortcut" "StartMenu"
    $existingDesktopShortcut = Get-ValidatedManifestShortcut $existingManifest "desktop_shortcut" "Desktop"
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
    if (-not $updateRequested -and $startOnLoginConfigured) {
        Set-StartOnLoginInConfig $resolvedInstallRoot ([bool]$StartOnLogin)
        if ($StartOnLogin) {
            New-Item -Path $runKey -Force | Out-Null
            New-ItemProperty -Path $runKey -Name "AskBridge" -Value ('"' + $targetExecutable + '"') -PropertyType String -Force | Out-Null
        }
        else {
            Remove-StartOnLoginOwnedByInstall $runKey $targetExecutable
        }
    }

    $startMenuShortcutPath = $null
    $desktopShortcutPath = $null
    if ($updateRequested) {
        $startMenuShortcutPath = $existingStartMenuShortcut
        $desktopShortcutPath = $existingDesktopShortcut
        if (-not [string]::IsNullOrWhiteSpace($startMenuShortcutPath)) {
            New-AskBridgeShortcut $startMenuShortcutPath $targetExecutable $resolvedInstallRoot
        }
        if (-not [string]::IsNullOrWhiteSpace($desktopShortcutPath)) {
            New-AskBridgeShortcut $desktopShortcutPath $targetExecutable $resolvedInstallRoot
        }
    }
    else {
        $startMenuShortcutPath = Join-Path (Get-ShortcutDirectory "StartMenu") "AskBridge.lnk"
        if ($CreateStartMenuShortcut) {
            New-AskBridgeShortcut $startMenuShortcutPath $targetExecutable $resolvedInstallRoot
        }
        else {
            Remove-ShortcutOwnedByInstall $startMenuShortcutPath $targetExecutable
            $startMenuShortcutPath = $null
        }

        $desktopShortcutPath = Join-Path (Get-ShortcutDirectory "Desktop") "AskBridge.lnk"
        if ($CreateDesktopShortcut) {
            New-AskBridgeShortcut $desktopShortcutPath $targetExecutable $resolvedInstallRoot
        }
        else {
            Remove-ShortcutOwnedByInstall $desktopShortcutPath $targetExecutable
            $desktopShortcutPath = $null
        }
    }

    $installManifest = [ordered]@{
        product = "AskBridge"
        version = [string]$packageMetadata.version
        install_root = $resolvedInstallRoot
        installed_at = [DateTimeOffset]::Now.ToString("o")
        files = $requiredFiles
        data_directory = (Join-Path $resolvedInstallRoot "data")
        desktop_shortcut = $desktopShortcutPath
        start_menu_shortcut = $startMenuShortcutPath
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
