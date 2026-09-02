[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AcceptanceRoot,
    [string]$UpdateSigningKeyFile
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
# Fall back to the conventional gitignored key location next to the repository.
$UpdateSigningKeyFile = [string]$UpdateSigningKeyFile
if ([string]::IsNullOrWhiteSpace($UpdateSigningKeyFile)) {
    $defaultKeyFile = Join-Path $repoRoot ".update-signing-key.json"
    if (Test-Path -LiteralPath $defaultKeyFile -PathType Leaf) {
        $UpdateSigningKeyFile = $defaultKeyFile
    }
}
if (Test-Path -LiteralPath $root) {
    throw "AcceptanceRoot already exists; refusing to overwrite it."
}

$artifactRoot = Join-Path $root "package"
$installRoot = Join-Path $root "installed"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$previousRunEntry = Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
$previousRunValue = if ($null -eq $previousRunEntry) { $null } else { [string]$previousRunEntry.AskBridge }
$previousInstallEnvironment = $env:ASKBRIDGE_INSTALL_ROOT
$previousDataEnvironment = $env:ASKBRIDGE_DATA_DIR
$previousUpdateParent = $env:ASKBRIDGE_UPDATE_PARENT_PID
$previousRestartAfterInstall = $env:ASKBRIDGE_RESTART_AFTER_INSTALL
$previousSetupNoDialog = $env:ASKBRIDGE_SETUP_NO_DIALOG
$previousCreateDesktopShortcut = $env:ASKBRIDGE_CREATE_DESKTOP_SHORTCUT
$previousCreateStartMenuShortcut = $env:ASKBRIDGE_CREATE_START_MENU_SHORTCUT
$previousInstallerTestMode = $env:ASKBRIDGE_INSTALLER_TEST_MODE
$previousShortcutRoot = $env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT
$shortcutRoot = Join-Path $root "shortcuts"
$desktopShortcut = Join-Path $shortcutRoot "Desktop\AskBridge.lnk"
$startMenuShortcut = Join-Path $shortcutRoot "StartMenuPrograms\AskBridge.lnk"
$setupProcess = $null
$installedProcess = $null
$updateParentProcess = $null
$updateParentPid = $null
$updateRestartedProcess = $null
$updateStopper = $null

function Get-AskBridgePackageVersion {
    Push-Location $repoRoot
    try {
        $metadata = cargo metadata --locked --offline --no-deps --format-version 1 | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed with exit code $LASTEXITCODE." }
        $package = $metadata.packages | Where-Object { $_.name -eq "askbridge-win" } | Select-Object -First 1
        if ($null -eq $package) { throw "askbridge-win metadata was not found." }
        return [string]$package.version
    }
    finally {
        Pop-Location
    }
}

try {
    Write-Host "[1/5] Build portable and self-extracting packages"
    & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot $artifactRoot -UpdateSigningKeyFile $UpdateSigningKeyFile
    if ($LASTEXITCODE -ne 0) { throw "package.ps1 failed with exit code $LASTEXITCODE." }
    $expectedVersion = Get-AskBridgePackageVersion
    Push-Location $repoRoot
    try {
        cargo run --package xtask --locked --offline -- validate-package-artifacts `
            --artifact-root $artifactRoot `
            --expected-version $expectedVersion `
            --expected-release-exe-path (Join-Path $repoRoot "target\release\askbridge.exe") `
            --expected-source-root $repoRoot `
            --require-update-signature
        if ($LASTEXITCODE -ne 0) {
            throw "package artifact validator failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }

    $setup = @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "*-Setup.exe")
    if ($setup.Count -ne 1) { throw "Packaging did not produce exactly one Setup.exe." }

    Write-Host "[2/5] Run Setup.exe and verify clean exit"
    $env:ASKBRIDGE_INSTALL_ROOT = $installRoot
    $env:ASKBRIDGE_UPDATE_PARENT_PID = $null
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $null
    $env:ASKBRIDGE_SETUP_NO_DIALOG = "1"
    $env:ASKBRIDGE_CREATE_DESKTOP_SHORTCUT = "1"
    $env:ASKBRIDGE_CREATE_START_MENU_SHORTCUT = "1"
    $env:ASKBRIDGE_INSTALLER_TEST_MODE = "1"
    $env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT = $shortcutRoot
    $initialSetupStdout = Join-Path $root "setup-install.stdout.log"
    $initialSetupStderr = Join-Path $root "setup-install.stderr.log"
    $setupProcess = Start-Process -FilePath $setup[0].FullName -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $initialSetupStdout -RedirectStandardError $initialSetupStderr
    $manifestPath = Join-Path $installRoot "install-manifest.json"
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Start-Sleep -Milliseconds 200
    }
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Setup.exe did not produce an install manifest within 45 seconds."
    }
    if (-not $setupProcess.WaitForExit(30000)) {
        throw "Setup.exe did not exit after installation."
    }
    $setupProcess.WaitForExit()
    $setupProcess.Refresh()
    $setupExitCode = $setupProcess.ExitCode
    if ($null -ne $setupExitCode -and $setupExitCode -ne 0) {
        $setupError = Get-Content -LiteralPath $initialSetupStderr -Raw -ErrorAction SilentlyContinue
        $setupOutput = Get-Content -LiteralPath $initialSetupStdout -Raw -ErrorAction SilentlyContinue
        throw "Setup.exe installation failed with exit code $setupExitCode. stderr: $setupError stdout: $setupOutput"
    }
    $setupProcess = $null
    foreach ($file in @("askbridge.exe", "WebView2Loader.dll", "install-manifest.json", "Uninstall-AskBridge.ps1")) {
        if (-not (Test-Path -LiteralPath (Join-Path $installRoot $file) -PathType Leaf)) {
            throw "Setup.exe install is missing $file."
        }
    }
    foreach ($shortcutPath in @($desktopShortcut, $startMenuShortcut)) {
        if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
            throw "Setup.exe did not create shortcut $shortcutPath."
        }
        $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
        if (-not ([IO.Path]::GetFullPath([string]$shortcut.TargetPath)).Equals((Join-Path $installRoot "askbridge.exe"), [StringComparison]::OrdinalIgnoreCase) -or
            -not ([IO.Path]::GetFullPath([string]$shortcut.WorkingDirectory)).Equals($installRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Setup.exe created a shortcut with an incorrect target or working directory."
        }
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$manifest.desktop_shortcut -ne $desktopShortcut -or [string]$manifest.start_menu_shortcut -ne $startMenuShortcut) {
        throw "Setup.exe manifest did not record both shortcut paths."
    }

    Write-Host "[3/5] Run Setup.exe in update mode and restart the installed application"
    # The acceptance root is below the repository target directory, so explicitly bind the
    # child process to its installed data directory instead of the development-tree data root.
    $env:ASKBRIDGE_DATA_DIR = Join-Path $installRoot "data"
    $updateSentinel = Join-Path $installRoot "data\setup-update-preserves-data.txt"
    New-Item -ItemType Directory -Path (Split-Path -Parent $updateSentinel) -Force | Out-Null
    Set-Content -LiteralPath $updateSentinel -Value "preserve-me" -Encoding ASCII
    $updateParentProcess = @(Start-Process -FilePath (Join-Path $installRoot "askbridge.exe") -PassThru -WindowStyle Hidden)[0]
    $updateParentPid = [int]$updateParentProcess.Id
    Start-Sleep -Seconds 2
    if ($null -eq (Get-Process -Id $updateParentPid -ErrorAction SilentlyContinue)) {
        throw "Setup-installed AskBridge exited before update acceptance."
    }
    $env:ASKBRIDGE_UPDATE_PARENT_PID = [string]$updateParentPid
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = "1"
    $stopCommand = "Start-Sleep -Milliseconds 750; Stop-Process -Id $updateParentPid -Force -ErrorAction SilentlyContinue"
    $updateStopper = Start-Process -FilePath "powershell.exe" -ArgumentList @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", $stopCommand
    ) -PassThru -WindowStyle Hidden
    $updateSetupStdout = Join-Path $root "setup-update.stdout.log"
    $updateSetupStderr = Join-Path $root "setup-update.stderr.log"
    $setupProcess = Start-Process -FilePath $setup[0].FullName -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $updateSetupStdout -RedirectStandardError $updateSetupStderr
    if (-not $setupProcess.WaitForExit(30000)) {
        throw "Setup.exe update mode did not exit after installation."
    }
    $setupProcess.WaitForExit()
    $setupProcess.Refresh()
    $setupExitCode = $setupProcess.ExitCode
    if ($null -ne $setupExitCode -and $setupExitCode -ne 0) {
        $setupError = Get-Content -LiteralPath $updateSetupStderr -Raw -ErrorAction SilentlyContinue
        $setupOutput = Get-Content -LiteralPath $updateSetupStdout -Raw -ErrorAction SilentlyContinue
        throw "Setup.exe update mode failed with exit code $setupExitCode. stderr: $setupError stdout: $setupOutput"
    }
    $setupProcess = $null
    $updateParentProcess.Refresh()
    if (-not $updateParentProcess.HasExited) {
        throw "Setup.exe update mode returned before its owning AskBridge process exited."
    }
    if (-not (Test-Path -LiteralPath $updateSentinel -PathType Leaf)) {
        throw "Setup.exe update mode removed user data."
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]$manifest.desktop_shortcut -ne $desktopShortcut -or
        [string]$manifest.start_menu_shortcut -ne $startMenuShortcut -or
        -not (Test-Path -LiteralPath $desktopShortcut -PathType Leaf) -or
        -not (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf)) {
        throw "Setup.exe update mode did not preserve the shortcut contract."
    }
    $targetExecutable = [IO.Path]::GetFullPath((Join-Path $installRoot "askbridge.exe"))
    $restartDeadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $updateRestartedProcess = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue |
            Where-Object {
                $_.Path -and $_.Path.Equals($targetExecutable, [StringComparison]::OrdinalIgnoreCase)
            } | Select-Object -First 1)
        if ($updateRestartedProcess.Count -gt 0) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $restartDeadline)
    if ($updateRestartedProcess.Count -eq 0) {
        $setupError = Get-Content -LiteralPath $updateSetupStderr -Raw -ErrorAction SilentlyContinue
        $setupOutput = Get-Content -LiteralPath $updateSetupStdout -Raw -ErrorAction SilentlyContinue
        $applicationLog = Get-Content -LiteralPath (Join-Path $installRoot "data\logs\askbridge.log") -Raw -ErrorAction SilentlyContinue
        $observedProcesses = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue | ForEach-Object {
            "PID=$($_.Id) Path=$($_.Path)"
        }) -join "; "
        throw "Setup.exe update mode did not restart AskBridge. stderr: $setupError stdout: $setupOutput application log: $applicationLog observed processes: $observedProcesses"
    }
    $updateRestartedProcess = $updateRestartedProcess[0]
    $env:ASKBRIDGE_UPDATE_PARENT_PID = $previousUpdateParent
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $previousRestartAfterInstall
    $env:ASKBRIDGE_SETUP_NO_DIALOG = $previousSetupNoDialog
    Stop-Process -Id $updateRestartedProcess.Id -Force -ErrorAction SilentlyContinue
    Wait-Process -Id $updateRestartedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
    $updateRestartedProcess = $null

    Write-Host "[4/5] Start the installed application"
    $installedProcess = Start-Process -FilePath (Join-Path $installRoot "askbridge.exe") -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
    $installedProcess.Refresh()
    if ($installedProcess.HasExited) { throw "Setup-installed AskBridge exited during startup acceptance." }
    Stop-Process -Id $installedProcess.Id -Force
    Wait-Process -Id $installedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
    $installedProcess = $null

    Write-Host "[5/5] Uninstall and remove isolated data"
    & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -RemoveData
    if (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe")) {
        throw "Setup smoke uninstall left askbridge.exe behind."
    }
    if ((Test-Path -LiteralPath $desktopShortcut -PathType Leaf) -or (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf)) {
        throw "Setup smoke uninstall left an AskBridge shortcut behind."
    }
    Write-Host "Setup.exe version, hashes, Release EXE identity, extraction, install, update wait/restart, clean exit, first launch, and uninstall acceptance passed."
}
finally {
    foreach ($ownedProcess in @($installedProcess, $updateParentProcess, $updateRestartedProcess, $updateStopper, $setupProcess)) {
        if ($null -ne $ownedProcess) {
            try {
                $ownedProcess.Refresh()
                if (-not $ownedProcess.HasExited) {
                    Stop-Process -InputObject $ownedProcess -Force -ErrorAction SilentlyContinue
                    $ownedProcess.WaitForExit(5000) | Out-Null
                }
            }
            catch {
                # The owned process may already have exited and released its process handle.
            }
        }
    }
    $env:ASKBRIDGE_INSTALL_ROOT = $previousInstallEnvironment
    $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
    $env:ASKBRIDGE_UPDATE_PARENT_PID = $previousUpdateParent
    $env:ASKBRIDGE_RESTART_AFTER_INSTALL = $previousRestartAfterInstall
    $env:ASKBRIDGE_SETUP_NO_DIALOG = $previousSetupNoDialog
    $env:ASKBRIDGE_CREATE_DESKTOP_SHORTCUT = $previousCreateDesktopShortcut
    $env:ASKBRIDGE_CREATE_START_MENU_SHORTCUT = $previousCreateStartMenuShortcut
    $env:ASKBRIDGE_INSTALLER_TEST_MODE = $previousInstallerTestMode
    $env:ASKBRIDGE_INSTALLER_TEST_SHORTCUT_ROOT = $previousShortcutRoot
    if ($null -eq $previousRunValue) {
        Remove-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
    }
    else {
        New-Item -Path $runKey -Force | Out-Null
        New-ItemProperty -Path $runKey -Name "AskBridge" -Value $previousRunValue -PropertyType String -Force | Out-Null
    }

    $cleanupDeadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        try {
            if (Test-Path -LiteralPath $root) {
                Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
            }
            break
        }
        catch [IO.IOException], [UnauthorizedAccessException] {
            if ([DateTime]::UtcNow -ge $cleanupDeadline) { throw }
            Start-Sleep -Milliseconds 250
        }
    } while ($true)
}
