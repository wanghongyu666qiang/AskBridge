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

$artifactRoot = Join-Path $root "package"
$installRoot = Join-Path $root "installed"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$previousRunEntry = Get-ItemProperty -Path $runKey -Name "AskBridge" -ErrorAction SilentlyContinue
$previousRunValue = if ($null -eq $previousRunEntry) { $null } else { [string]$previousRunEntry.AskBridge }
$previousInstallEnvironment = $env:ASKBRIDGE_INSTALL_ROOT
$previousDataEnvironment = $env:ASKBRIDGE_DATA_DIR
$setupProcess = $null
$installedProcess = $null

try {
    Write-Host "[1/4] Build portable and self-extracting packages"
    & (Join-Path $repoRoot "scripts\package.ps1") -ArtifactRoot $artifactRoot
    if ($LASTEXITCODE -ne 0) { throw "package.ps1 failed with exit code $LASTEXITCODE." }

    $setup = @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "*-Setup.exe")
    $hashFiles = @(Get-ChildItem -LiteralPath $artifactRoot -File -Filter "*-SHA256SUMS.txt")
    if ($setup.Count -ne 1 -or $hashFiles.Count -ne 1) {
        throw "Packaging did not produce exactly one Setup.exe and one SHA256SUMS file."
    }
    foreach ($line in Get-Content -LiteralPath $hashFiles[0].FullName -Encoding ASCII) {
        if ($line -notmatch '^([0-9A-F]{64})  (.+)$') { throw "Malformed SHA256SUMS line: $line" }
        $expectedHash = $Matches[1]
        $leaf = $Matches[2]
        $matches = @(Get-ChildItem -LiteralPath $artifactRoot -Recurse -File | Where-Object Name -EQ $leaf)
        if ($matches.Count -ne 1) { throw "Hash target '$leaf' is missing or ambiguous." }
        $actualHash = (Get-FileHash -LiteralPath $matches[0].FullName -Algorithm SHA256).Hash
        if ($actualHash -ne $expectedHash) { throw "Hash verification failed for '$leaf'." }
    }

    Write-Host "[2/4] Run Setup.exe and verify clean exit"
    $env:ASKBRIDGE_INSTALL_ROOT = $installRoot
    $setupProcess = Start-Process -FilePath $setup[0].FullName -PassThru -WindowStyle Hidden
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
    $setupProcess = $null
    foreach ($file in @("askbridge.exe", "install-manifest.json", "Uninstall-AskBridge.ps1")) {
        if (-not (Test-Path -LiteralPath (Join-Path $installRoot $file) -PathType Leaf)) {
            throw "Setup.exe install is missing $file."
        }
    }

    Write-Host "[3/4] Start the installed application"
    # The acceptance root is below the repository target directory, so explicitly bind the
    # child process to its installed data directory instead of the development-tree data root.
    $env:ASKBRIDGE_DATA_DIR = Join-Path $installRoot "data"
    $installedProcess = Start-Process -FilePath (Join-Path $installRoot "askbridge.exe") -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2
    $installedProcess.Refresh()
    if ($installedProcess.HasExited) { throw "Setup-installed AskBridge exited during startup acceptance." }
    Stop-Process -Id $installedProcess.Id -Force
    Wait-Process -Id $installedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
    $installedProcess = $null

    Write-Host "[4/4] Uninstall and remove isolated data"
    & (Join-Path $installRoot "Uninstall-AskBridge.ps1") -InstallRoot $installRoot -RemoveData
    if (Test-Path -LiteralPath (Join-Path $installRoot "askbridge.exe")) {
        throw "Setup smoke uninstall left askbridge.exe behind."
    }
    Write-Host "Setup.exe hashes, extraction, install, clean exit, first launch, and uninstall acceptance passed."
}
finally {
    foreach ($ownedProcess in @($installedProcess, $setupProcess)) {
        if ($null -ne $ownedProcess) {
            $ownedProcess.Refresh()
            if (-not $ownedProcess.HasExited) {
                Stop-Process -Id $ownedProcess.Id -Force -ErrorAction SilentlyContinue
                Wait-Process -Id $ownedProcess.Id -Timeout 5 -ErrorAction SilentlyContinue
            }
        }
    }
    $env:ASKBRIDGE_INSTALL_ROOT = $previousInstallEnvironment
    $env:ASKBRIDGE_DATA_DIR = $previousDataEnvironment
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
