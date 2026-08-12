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

function Write-JsonFile {
    param([string]$Path, [object]$Value)

    $Value | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Path -Encoding UTF8
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

function Invoke-PerformanceValidator {
    param(
        [string]$Desktop = $desktopPath,
        [string]$Chrome = $chromePath,
        [string]$Timings = $timingsPath,
        [string]$Executable = $exePath,
        [string]$ExpectedChromeProfile = $expectedChromeProfilePath
    )

    & (Join-Path $repoRoot "scripts\validate-performance-report.ps1") `
        -DesktopReportPath $Desktop `
        -ChromeReportPath $Chrome `
        -TimingsReportPath $Timings `
        -ExecutablePath $Executable `
        -ExpectedChromeProfilePath $ExpectedChromeProfile
}

try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null

    $exePath = Join-Path $root "askbridge.exe"
    Set-Content -LiteralPath $exePath -Encoding ASCII -Value "validator-fixture"
    $exeHash = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash

    $desktopPath = Join-Path $root "desktop.json"
    $chromePath = Join-Path $root "chrome.json"
    $timingsPath = Join-Path $root "timings.json"
    $expectedChromeProfilePath = Join-Path $root "BrowserProfile"
    $measuredAt = "2026-08-12T00:00:00.0000000+08:00"
    Write-JsonFile $desktopPath ([ordered]@{
        measured_at = $measuredAt
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Write-JsonFile $chromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = $exeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Write-JsonFile $timingsPath ([ordered]@{
        measured_at_unix_ms = 1786304522013
        provider = "chatgpt"
        auto_submit = $false
        managed_browser_closed = $true
        browser_launch_ms = 600
        first_preparation_ms = 6000
        continuous_preparation_ms = 3800
    })

    Write-Host "[1/11] Accept valid reports with matching executable paths and hashes"
    Invoke-PerformanceValidator

    Write-Host "[2/11] Reject missing final evidence paths"
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\validate-performance-report.ps1") `
            -DesktopReportPath $desktopPath `
            -TimingsReportPath $timingsPath `
            -ExecutablePath $exePath `
            -ExpectedChromeProfilePath $expectedChromeProfilePath
    } "ChromeReportPath is required for final performance validation."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\validate-performance-report.ps1") `
            -DesktopReportPath $desktopPath `
            -ChromeReportPath $chromePath `
            -ExecutablePath $exePath `
            -ExpectedChromeProfilePath $expectedChromeProfilePath
    } "TimingsReportPath is required for final performance validation."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\validate-performance-report.ps1") `
            -DesktopReportPath $desktopPath `
            -ChromeReportPath $chromePath `
            -TimingsReportPath $timingsPath `
            -ExpectedChromeProfilePath $expectedChromeProfilePath
    } "ExecutablePath is required for final performance validation."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\validate-performance-report.ps1") `
            -DesktopReportPath $desktopPath `
            -ChromeReportPath $chromePath `
            -TimingsReportPath $timingsPath `
            -ExecutablePath $exePath
    } "ExpectedChromeProfilePath is required for final performance validation."

    Write-Host "[3/11] Reject stale desktop executable identity"
    $desktopWithoutExecutablePath = Join-Path $root "desktop-without-executable.json"
    Write-JsonFile $desktopWithoutExecutablePath ([ordered]@{
        measured_at = $measuredAt
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $desktopWithoutExecutablePath
    } "Desktop report executable path is missing."

    $wrongExePath = Join-Path $root "wrong-askbridge.exe"
    Set-Content -LiteralPath $wrongExePath -Encoding ASCII -Value "validator-fixture"
    $desktopWrongExecutablePath = Join-Path $root "desktop-wrong-executable.json"
    Write-JsonFile $desktopWrongExecutablePath ([ordered]@{
        measured_at = $measuredAt
        executable = $wrongExePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $desktopWrongExecutablePath
    } "Desktop report executable path does not match the expected Release EXE."

    Set-Content -LiteralPath $exePath -Encoding ASCII -Value "changed-fixture"
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator
    } "Desktop report executable hash is stale."

    Write-Host "[4/11] Reject stale Chrome executable hashes"
    $freshExePath = Join-Path $root "fresh-askbridge.exe"
    Set-Content -LiteralPath $freshExePath -Encoding ASCII -Value "validator-fixture"
    $freshExeHash = (Get-FileHash -LiteralPath $freshExePath -Algorithm SHA256).Hash
    Set-Content -LiteralPath $exePath -Encoding ASCII -Value "validator-fixture"
    $chromeWithoutExecutablePath = Join-Path $root "chrome-without-executable.json"
    Write-JsonFile $chromeWithoutExecutablePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Chrome $chromeWithoutExecutablePath
    } "Chrome report executable path is missing."

    $chromeWrongExecutablePath = Join-Path $root "chrome-wrong-executable.json"
    Write-JsonFile $chromeWrongExecutablePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $wrongExePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Chrome $chromeWrongExecutablePath
    } "Chrome report executable path does not match the expected Release EXE."

    Write-JsonFile $chromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = "0" * 64
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator
    } "Chrome report executable hash is stale."
    Write-JsonFile $chromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })

    Write-Host "[5/11] Reject relative report and measurement paths"
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop "desktop.json"
    } "DesktopReportPath must be an explicit absolute path."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\measure-performance.ps1") `
            -ExecutablePath "target\release\askbridge.exe" `
            -OutputPath (Join-Path $root "desktop-measurement.json")
    } "ExecutablePath must be an explicit absolute path."
    Invoke-ExpectedFailure {
        & (Join-Path $repoRoot "scripts\measure-chrome-performance.ps1") `
            -ProfilePath $expectedChromeProfilePath `
            -ExecutablePath "target\release\askbridge.exe" `
            -OutputPath (Join-Path $root "chrome-measurement.json")
    } "ExecutablePath must be an explicit absolute path."

    Write-Host "[6/11] Reject wrong Chrome profile paths"
    Write-JsonFile $chromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = (Join-Path $root "OtherProfile")
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator
    } "Chrome report profile path does not match the expected AskBridge profile."
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -ExpectedChromeProfile "relative-profile"
    } "ExpectedChromeProfilePath must be an explicit absolute path."
    Write-JsonFile $chromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = "relative-profile"
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator
    } "Chrome report profile_path must be an explicit absolute path."
    Write-JsonFile $chromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })

    Write-Host "[7/11] Reject unsafe desktop bounds"
    $badDesktopPath = Join-Path $root "desktop-bad.json"
    Write-JsonFile $badDesktopPath ([ordered]@{
        measured_at = $measuredAt
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.5
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $badDesktopPath
    } "idle_cpu_percent_machine is"

    Write-Host "[8/11] Reject missing desktop external connection evidence"
    $missingExternalPath = Join-Path $root "desktop-missing-external.json"
    Write-JsonFile $missingExternalPath ([ordered]@{
        measured_at = $measuredAt
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $missingExternalPath
    } "desktop external TCP connection count is missing."

    Write-Host "[9/11] Reject missing timestamps or under-duration performance reports"
    $missingDesktopTimestampPath = Join-Path $root "desktop-missing-timestamp.json"
    Write-JsonFile $missingDesktopTimestampPath ([ordered]@{
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $missingDesktopTimestampPath
    } "Desktop report must include measured_at."

    $invalidDesktopTimestampPath = Join-Path $root "desktop-invalid-timestamp.json"
    Write-JsonFile $invalidDesktopTimestampPath ([ordered]@{
        measured_at = "not-a-timestamp"
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 300
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $invalidDesktopTimestampPath
    } "Desktop report measured_at must be an ISO 8601 timestamp."

    $missingChromeTimestampPath = Join-Path $root "chrome-missing-timestamp.json"
    Write-JsonFile $missingChromeTimestampPath ([ordered]@{
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Chrome $missingChromeTimestampPath
    } "Chrome report must include measured_at."

    $invalidChromeTimestampPath = Join-Path $root "chrome-invalid-timestamp.json"
    Write-JsonFile $invalidChromeTimestampPath ([ordered]@{
        measured_at = "not-a-timestamp"
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Chrome $invalidChromeTimestampPath
    } "Chrome report measured_at must be an ISO 8601 timestamp."

    $missingDurationPath = Join-Path $root "desktop-missing-duration.json"
    Write-JsonFile $missingDurationPath ([ordered]@{
        measured_at = $measuredAt
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $missingDurationPath
    } "actual_duration_seconds is missing."

    $shortDesktopPath = Join-Path $root "desktop-short.json"
    Write-JsonFile $shortDesktopPath ([ordered]@{
        measured_at = $measuredAt
        executable = $exePath
        executable_sha256 = $exeHash
        cold_start_ms = 120.5
        actual_duration_seconds = 299
        idle_cpu_percent_machine = 0.05
        working_set_max_bytes = 15MB
        external_tcp_connection_count_max = 0
        process_count_max = 1
        samples = @(
            @{ sample = 1; working_set_bytes = 14MB },
            @{ sample = 2; working_set_bytes = 15MB }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Desktop $shortDesktopPath
    } "actual_duration_seconds is"
    $shortChromePath = Join-Path $root "chrome-short.json"
    Write-JsonFile $shortChromePath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        executable_sha256 = $freshExeHash
        actual_duration_seconds = 299
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Chrome $shortChromePath
    } "actual_duration_seconds is"

    Write-Host "[10/11] Reject incomplete preparation timings"
    $missingProviderTimingsPath = Join-Path $root "timings-missing-provider.json"
    Write-JsonFile $missingProviderTimingsPath ([ordered]@{
        measured_at_unix_ms = 1786304522013
        auto_submit = $false
        managed_browser_closed = $true
        browser_launch_ms = 600
        first_preparation_ms = 6000
        continuous_preparation_ms = 3800
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Timings $missingProviderTimingsPath
    } "Preparation timings must include provider."

    $missingTimestampTimingsPath = Join-Path $root "timings-missing-timestamp.json"
    Write-JsonFile $missingTimestampTimingsPath ([ordered]@{
        provider = "chatgpt"
        auto_submit = $false
        managed_browser_closed = $true
        browser_launch_ms = 600
        first_preparation_ms = 6000
        continuous_preparation_ms = 3800
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Timings $missingTimestampTimingsPath
    } "measured_at_unix_ms is missing."

    Write-Host "[11/11] Reject Chrome reports without executable hashes during final validation"
    $chromeWithoutHashPath = Join-Path $root "chrome-without-hash.json"
    Write-JsonFile $chromeWithoutHashPath ([ordered]@{
        measured_at = $measuredAt
        profile_path = $expectedChromeProfilePath
        executable = $exePath
        actual_duration_seconds = 300
        working_set_average_bytes = 800MB
        process_count_max = 8
        samples = @(
            @{ sample = 1; process_count = 8 },
            @{ sample = 2; process_count = 8 }
        )
    })
    Invoke-ExpectedFailure {
        Invoke-PerformanceValidator -Chrome $chromeWithoutHashPath
    } "Chrome report does not include executable_sha256."

    Write-Host "Performance report validator acceptance passed."
}
finally {
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
