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

try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null

    $acceptanceRootScripts = @(
        "test-package.ps1",
        "test-package-artifact-validator.ps1",
        "test-performance-report-validator.ps1",
        "test-install-metadata-validator.ps1",
        "test-installer.ps1",
        "test-setup.ps1",
        "test-release-local.ps1"
    )

    Write-Host "[1/5] Reject relative acceptance roots"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-release-local.ps1")
    } "AcceptanceRoot must be an explicit absolute path."
    foreach ($scriptName in $acceptanceRootScripts) {
        Invoke-ExpectedFailure {
            & (Join-Path $repoRoot "scripts\$scriptName") -AcceptanceRoot "relative-acceptance-root"
        } "AcceptanceRoot must be an explicit absolute path."
    }
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-hotkey-ui.ps1") -AcceptanceDataRoot "relative-acceptance-root"
    } "AcceptanceDataRoot must be an explicit absolute path."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-settings-ui.ps1") `
            -AcceptanceDataRoot "relative-acceptance-root" `
            -ScreenshotPath "relative-screenshot.png"
    } "AcceptanceDataRoot and ScreenshotPath must be explicit absolute paths."

    Write-Host "[2/5] Reject acceptance roots outside repository target"
    $outsideRoot = Join-Path (Split-Path -Parent $repoRoot) "askbridge-outside-acceptance"
    foreach ($scriptName in $acceptanceRootScripts) {
        Invoke-ExpectedFailure {
            & (Join-Path $repoRoot "scripts\$scriptName") -AcceptanceRoot $outsideRoot
        } "AcceptanceRoot must be a new child of the repository target directory."
    }
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-hotkey-ui.ps1") -AcceptanceDataRoot $outsideRoot
    } "AcceptanceDataRoot must be a new child of the repository target directory."

    Write-Host "[3/5] Reject existing acceptance roots"
    $existingRoot = Join-Path $root "existing"
    New-Item -ItemType Directory -Path $existingRoot -Force | Out-Null
    foreach ($scriptName in $acceptanceRootScripts) {
        Invoke-ExpectedFailure {
            & (Join-Path $repoRoot "scripts\$scriptName") -AcceptanceRoot $existingRoot
        } "AcceptanceRoot already exists; refusing to overwrite it."
    }
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-hotkey-ui.ps1") -AcceptanceDataRoot $existingRoot
    } "AcceptanceDataRoot already exists; refusing to overwrite it."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-settings-ui.ps1") `
            -AcceptanceDataRoot $existingRoot `
            -ScreenshotPath (Join-Path $existingRoot "settings.png")
    } "AcceptanceDataRoot already exists; refusing to overwrite it."

    Write-Host "[4/5] Reject settings UI roots outside repository target"
    $settingsOutsideDataRoot = Join-Path (Split-Path -Parent $repoRoot) "askbridge-settings-outside-data"
    $settingsOutsideScreenshotPath = Join-Path $settingsOutsideDataRoot "settings.png"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-settings-ui.ps1") `
            -AcceptanceDataRoot $settingsOutsideDataRoot `
            -ScreenshotPath $settingsOutsideScreenshotPath
    } "AcceptanceDataRoot must be a new child of the repository target directory."

    Write-Host "[5/5] Reject settings UI screenshots outside the isolated data root"
    $settingsDataRoot = Join-Path $root "settings-data"
    $settingsOutsideScreenshot = Join-Path $root "outside-screenshot\settings.png"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\test-settings-ui.ps1") `
            -AcceptanceDataRoot $settingsDataRoot `
            -ScreenshotPath $settingsOutsideScreenshot
    } "ScreenshotPath must be inside AcceptanceDataRoot."

    if (Test-Path -LiteralPath $settingsDataRoot) {
        throw "Settings UI guard created the acceptance data root before rejecting the screenshot path."
    }
    if (Test-Path -LiteralPath (Split-Path -Parent $settingsOutsideScreenshot)) {
        throw "Settings UI guard created a screenshot directory outside the acceptance data root."
    }

    Write-Host "Acceptance root guard validation passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
