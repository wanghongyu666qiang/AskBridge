[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AcceptanceRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target"))
if (-not [IO.Path]::IsPathRooted($AcceptanceRoot)) {
    throw "AcceptanceRoot must be an explicit absolute path."
}
$root = [IO.Path]::GetFullPath($AcceptanceRoot).TrimEnd('\')
if (-not $root.StartsWith($targetRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw "AcceptanceRoot must be a new child of the repository target directory."
}
if (Test-Path -LiteralPath $root) {
    throw "AcceptanceRoot already exists; refusing to overwrite it."
}

$fixture = Join-Path $root "package"
$installRoot = Join-Path $root "installed"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$previousRunEntry = Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
$previousRunValue = if ($null -eq $previousRunEntry) { $null } else { [string]$previousRunEntry.AskBridge }
$previousDataEnvironment = $env:ASKBRIDGE_DATA_DIR
$previousUpdateParent = $env:ASKBRIDGE_UPDATE_PARENT_PID
$previousRestartAfterInstall = $env:ASKBRIDGE_RESTART_AFTER_INSTALL
$previousTestFailure = $env:ASKBRIDGE_TEST_FAIL_AFTER_UPDATE_FILE
$updateParentProcess = $null
$updateRestartedProcess = $null
$updateStopper = $null
$rollbackParentProcess = $null

function Get-FileSnapshot {
    param([string]$Path)

    return [Convert]::ToBase64String([IO.File]::ReadAllBytes($Path))
}

try {
    New-Item -ItemType Directory -Path $fixture -Force | Out-Null
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
        Copy-Item -LiteralPath $entry.Key -Destination (Join-Path $fixture $entry.Value)
    }
    [ordered]@{
        product = "AskBridge"
        version = "0.9.0-acceptance"
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture "package.json") -Encoding UTF8

    Write-Host "[1/12] Reject unsafe package metadata"
    [ordered]@{
        product = "AskBridge"
        version = "0.9.0-acceptance"
        architecture = "windows-x64"
        auto_submit = $true
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture "package.json") -Encoding UTF8
    try {
        & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    }
    catch {
        if (-not $_.Exception.Message.StartsWith("package.json property 'auto_submit' must be false.", [StringComparison]::Ordinal)) {
            throw
        }
    }
    if (Test-Path -LiteralPath $installRoot) {
        throw "Unsafe package metadata created an install directory."
    }
    [ordered]@{
        product = "AskBridge"
        version = "0.9.0-acceptance"
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture "package.json") -Encoding UTF8

    Write-Host "[2/12] Fresh user-level install with persistent startup"
    & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot -StartOnLogin
    foreach ($file in @("askbridge.exe", "WebView2Loader.dll", "install-manifest.json", "Uninstall-AskBridge.ps1")) {
        if (-not (Test-Path -LiteralPath (Join-Path $installRoot $file) -PathType Leaf)) {
            throw "Fresh install did not create $file."
        }
    }
    $installedConfig = Get-Content -LiteralPath (Join-Path $installRoot "data\config.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    $expectedRunValue = '"' + (Join-Path $installRoot "askbridge.exe") + '"'
    $installedRunValue = [string](Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
    if (-not $installedConfig.general.start_on_login -or $installedRunValue -ne $expectedRunValue) {
        throw "StartOnLogin did not update both config.json and the current-user Run value."
    }

    Write-Host "[3/12] First launch preserves installer-selected startup"
    # This acceptance install lives below the repository's target directory. Explicitly select
    # the installed data directory so the development-tree detector cannot redirect the child to
    # the repository's normal data directory.
    $env:ASKBRIDGE_DATA_DIR = Join-Path $installRoot "data"
    $installedProcess = Start-Process -FilePath (Join-Path $installRoot "askbridge.exe") -PassThru
    try {
        Start-Sleep -Seconds 2
        $installedProcess.Refresh()
        if ($installedProcess.HasExited) { throw "The freshly installed AskBridge process exited during startup acceptance." }
        $installedRunValue = [string](Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction Stop).AskBridge
        if ($installedRunValue -ne $expectedRunValue) {
            throw "First launch removed or changed the installer-selected startup value."
        }
    }
    finally {
        if (-not $installedProcess.HasExited) {
            Stop-Process -Id $installedProcess.Id -Force -ErrorAction SilentlyContinue
            Wait-Process -Id $installedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
        }
        $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
    }

    Write-Host "[4/12] In-place upgrade preserves data"
    $dataRoot = Join-Path $installRoot "data"
    New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null
    $sentinel = Join-Path $dataRoot "upgrade-preservation.txt"
    Set-Content -LiteralPath $sentinel -Value "preserve-me" -Encoding ASCII
    [ordered]@{
        product = "AskBridge"
        version = "0.9.1-acceptance"
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture "package.json") -Encoding UTF8
    & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    $manifest = Get-Content -LiteralPath (Join-Path $installRoot "install-manifest.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$manifest.version -ne "0.9.1-acceptance" -or -not (Test-Path -LiteralPath $sentinel)) {
        throw "Upgrade did not update the version while preserving data."
    }

    Write-Host "[5/12] Update waits for the owning process, preserves data, and restarts AskBridge"
    [ordered]@{
        product = "AskBridge"
        version = "0.9.2-acceptance"
        architecture = "windows-x64"
        auto_submit = $false
        chrome_bundled = $false
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture "package.json") -Encoding UTF8
    $env:ASKBRIDGE_DATA_DIR = $dataRoot
    $updateParentProcess = Start-Process -FilePath (Join-Path $installRoot "askbridge.exe") -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Seconds 1
        $updateParentProcess.Refresh()
        if ($updateParentProcess.HasExited) {
            throw "The update parent process exited before the updater test began."
        }
        $env:ASKBRIDGE_UPDATE_PARENT_PID = [string]$updateParentProcess.Id
        $env:ASKBRIDGE_RESTART_AFTER_INSTALL = "1"
        $stopCommand = "Start-Sleep -Milliseconds 750; Stop-Process -Id $($updateParentProcess.Id) -Force -ErrorAction SilentlyContinue"
        $updateStopper = Start-Process -FilePath "powershell.exe" -ArgumentList @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", $stopCommand
        ) -PassThru -WindowStyle Hidden
        & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
        $updateParentProcess.Refresh()
        if (-not $updateParentProcess.HasExited) {
            throw "Updater returned before the owning AskBridge process exited."
        }
        $manifest = Get-Content -LiteralPath (Join-Path $installRoot "install-manifest.json") -Raw -Encoding UTF8 | ConvertFrom-Json
        if ([string]$manifest.version -ne "0.9.2-acceptance" -or -not (Test-Path -LiteralPath $sentinel)) {
            throw "Updater did not preserve user data while replacing the application files."
        }
        $targetExecutable = [IO.Path]::GetFullPath((Join-Path $installRoot "askbridge.exe"))
        $restartDeadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            $updateRestartedProcess = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue |
                Where-Object {
                    $_.Path -and $_.Path.Equals($targetExecutable, [StringComparison]::OrdinalIgnoreCase) -and
                    $_.Id -ne $updateParentProcess.Id
                } | Select-Object -First 1)
            if ($updateRestartedProcess.Count -gt 0) { break }
            Start-Sleep -Milliseconds 250
        } while ([DateTime]::UtcNow -lt $restartDeadline)
        if ($updateRestartedProcess.Count -eq 0) {
            throw "Updater did not restart AskBridge after a successful in-place upgrade."
        }
        $updateRestartedProcess = $updateRestartedProcess[0]
    }
    finally {
        $env:ASKBRIDGE_UPDATE_PARENT_PID = $previousUpdateParent
        $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $previousRestartAfterInstall
        if ($null -ne $updateStopper) {
            $updateStopper.Refresh()
            if (-not $updateStopper.HasExited) {
                Stop-Process -Id $updateStopper.Id -Force -ErrorAction SilentlyContinue
            }
            $updateStopper = $null
        }
        foreach ($ownedProcess in @($updateParentProcess, $updateRestartedProcess)) {
            if ($null -ne $ownedProcess) {
                $ownedProcess.Refresh()
                if (-not $ownedProcess.HasExited) {
                    Stop-Process -Id $ownedProcess.Id -Force -ErrorAction SilentlyContinue
                    Wait-Process -Id $ownedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
                }
            }
        }
        $updateParentProcess = $null
        $updateRestartedProcess = $null
    }

    Write-Host "[6/12] Reject invalid updater parent PID"
    $env:ASKBRIDGE_UPDATE_PARENT_PID = "not-a-pid"
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = "1"
    try {
        & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    }
    catch {
        if (-not $_.Exception.Message.StartsWith("ASKBRIDGE_UPDATE_PARENT_PID must be a positive decimal process ID.", [StringComparison]::Ordinal)) {
            throw
        }
    }
    finally {
        $env:ASKBRIDGE_UPDATE_PARENT_PID = $previousUpdateParent
        $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $previousRestartAfterInstall
    }
    if ([string](Get-Content -LiteralPath (Join-Path $installRoot "install-manifest.json") -Raw -Encoding UTF8 | ConvertFrom-Json).version -ne "0.9.2-acceptance") {
        throw "Invalid updater PID changed the installed package."
    }

    Write-Host "[7/12] Failed update restores old program files and manifest"
    $rollbackFiles = @(
        "askbridge.exe",
        "WebView2Loader.dll",
        "README.md",
        "PRIVACY.md",
        "TROUBLESHOOTING.md",
        "Uninstall-AskBridge.ps1",
        "package.json"
    )
    $rollbackSnapshot = @{}
    foreach ($file in $rollbackFiles) {
        $rollbackSnapshot[$file] = Get-FileSnapshot (Join-Path $installRoot $file)
    }
    $rollbackManifestPath = Join-Path $installRoot "install-manifest.json"
    $rollbackManifestSnapshot = Get-FileSnapshot $rollbackManifestPath
    $rollbackParentProcess = Start-Process -FilePath "powershell.exe" -ArgumentList @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "exit 0"
    ) -PassThru -WindowStyle Hidden
    if (-not $rollbackParentProcess.WaitForExit(10000)) {
        throw "Rollback test helper process did not exit."
    }
    $env:ASKBRIDGE_UPDATE_PARENT_PID = [string]$rollbackParentProcess.Id
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = "1"
    $env:ASKBRIDGE_TEST_FAIL_AFTER_UPDATE_FILE = "2"
    $rollbackFailed = $false
    try {
        & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    }
    catch {
        if (-not $_.Exception.Message.StartsWith("Test hook: simulated update failure after file 2.", [StringComparison]::Ordinal)) {
            throw
        }
        $rollbackFailed = $true
    }
    finally {
        $env:ASKBRIDGE_UPDATE_PARENT_PID = $previousUpdateParent
        $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $previousRestartAfterInstall
        $env:ASKBRIDGE_TEST_FAIL_AFTER_UPDATE_FILE = $previousTestFailure
    }
    if (-not $rollbackFailed) {
        throw "The simulated mid-update failure unexpectedly succeeded."
    }
    foreach ($file in $rollbackFiles) {
        if ((Get-FileSnapshot (Join-Path $installRoot $file)) -ne $rollbackSnapshot[$file]) {
            throw "Failed update did not restore $file."
        }
    }
    if ((Get-FileSnapshot $rollbackManifestPath) -ne $rollbackManifestSnapshot) {
        throw "Failed update did not restore install-manifest.json."
    }
    if (-not (Test-Path -LiteralPath $sentinel -PathType Leaf)) {
        throw "Failed update removed user data."
    }
    if (Get-ChildItem -LiteralPath $installRoot -Force -Directory -Filter ".askbridge-update-backup-*" -ErrorAction SilentlyContinue) {
        throw "Failed update left a temporary rollback backup behind."
    }

    Write-Host "[8/12] Default-safe uninstall preserves data"
    & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -PreserveData
    if (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe")) {
        throw "Uninstall left the application executable behind."
    }
    if (-not (Test-Path -LiteralPath $sentinel)) {
        throw "PreserveData uninstall removed user data."
    }

    Write-Host "[9/12] Reject malformed uninstall manifest shape"
    & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    $manifestPath = Join-Path $installRoot "install-manifest.json"
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $manifest | Add-Member -MemberType NoteProperty -Name unexpected_field -Value "reject"
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    try {
        & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -PreserveData
    }
    catch {
        if (-not $_.Exception.Message.StartsWith("The install manifest does not match the expected AskBridge field set.", [StringComparison]::Ordinal)) {
            throw
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe") -PathType Leaf)) {
        throw "Malformed uninstall manifest shape partially removed the installed executable."
    }
    Remove-Item -LiteralPath $manifestPath -Force

    Write-Host "[10/12] Reject unexpected uninstall manifest file list"
    & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    $outsideSentinel = Join-Path $root "outside-sentinel.txt"
    Set-Content -LiteralPath $outsideSentinel -Encoding ASCII -Value "must-not-delete"
    $manifestPath = Join-Path $installRoot "install-manifest.json"
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $manifest.files += "..\outside-sentinel.txt"
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    try {
        & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -PreserveData
    }
    catch {
        if (-not $_.Exception.Message.StartsWith("The install manifest file list does not match the expected AskBridge payload.", [StringComparison]::Ordinal)) {
            throw
        }
    }
    if (-not (Test-Path -LiteralPath $outsideSentinel -PathType Leaf)) {
        throw "Out-of-scope uninstall manifest entry removed a file outside the install root."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe") -PathType Leaf)) {
        throw "Out-of-scope uninstall manifest entry partially removed the installed executable."
    }
    Remove-Item -LiteralPath $manifestPath -Force

    Write-Host "[11/12] Reject out-of-scope start menu shortcut manifest entry"
    & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    $shortcutSentinel = Join-Path $root "shortcut-sentinel.lnk"
    Set-Content -LiteralPath $shortcutSentinel -Encoding ASCII -Value "must-not-delete"
    $manifestPath = Join-Path $installRoot "install-manifest.json"
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $manifest.start_menu_shortcut = $shortcutSentinel
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    try {
        & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -PreserveData
    }
    catch {
        if (-not $_.Exception.Message.StartsWith("The install manifest contains an out-of-scope start menu shortcut path.", [StringComparison]::Ordinal)) {
            throw
        }
    }
    if (-not (Test-Path -LiteralPath $shortcutSentinel -PathType Leaf)) {
        throw "Out-of-scope start menu shortcut manifest entry removed a file outside the allowed shortcut path."
    }
    if (-not (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe") -PathType Leaf)) {
        throw "Out-of-scope start menu shortcut manifest entry partially removed the installed executable."
    }
    Remove-Item -LiteralPath $manifestPath -Force

    Write-Host "[12/12] Explicit data-removal uninstall"
    & (Join-Path $fixture "Install-AskBridge.ps1") -InstallRoot $installRoot
    & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -RemoveData
    if (Test-Path -LiteralPath (Join-Path $installRoot "data")) {
        throw "RemoveData uninstall left user data behind."
    }
    if (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe")) {
        throw "RemoveData uninstall left the application executable behind."
    }
    Write-Host "Installer acceptance passed."
}
finally {
    $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
    $env:ASKBRIDGE_UPDATE_PARENT_PID = $previousUpdateParent
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $previousRestartAfterInstall
    $env:ASKBRIDGE_TEST_FAIL_AFTER_UPDATE_FILE = $previousTestFailure
    foreach ($ownedProcess in @($updateParentProcess, $updateRestartedProcess, $updateStopper)) {
        if ($null -ne $ownedProcess) {
            $ownedProcess.Refresh()
            if (-not $ownedProcess.HasExited) {
                Stop-Process -Id $ownedProcess.Id -Force -ErrorAction SilentlyContinue
                Wait-Process -Id $ownedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
            }
        }
    }
    if ($null -ne $rollbackParentProcess) {
        $rollbackParentProcess.Refresh()
        if (-not $rollbackParentProcess.HasExited) {
            Stop-Process -Id $rollbackParentProcess.Id -Force -ErrorAction SilentlyContinue
            Wait-Process -Id $rollbackParentProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
        }
    }
    if ($null -eq $previousRunValue) {
        Remove-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
    }
    else {
        New-Item -Path $runKey -Force | Out-Null
        New-ItemProperty -Path $runKey -Name "AskBridge" -Value $previousRunValue -PropertyType String -Force | Out-Null
    }
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
