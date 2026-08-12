[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$DesktopReportPath,
    [string]$ChromeReportPath,
    [string]$TimingsReportPath,
    [string]$ExecutablePath,
    [string]$ExpectedChromeProfilePath,
    [ValidateRange(1, 3600)]
    [int]$MinimumDesktopDurationSeconds = 300,
    [ValidateRange(1, 1800)]
    [int]$MinimumChromeDurationSeconds = 300
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ChromeReportPath)) {
    throw "ChromeReportPath is required for final performance validation."
}
if ([string]::IsNullOrWhiteSpace($TimingsReportPath)) {
    throw "TimingsReportPath is required for final performance validation."
}
if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
    throw "ExecutablePath is required for final performance validation."
}
if ([string]::IsNullOrWhiteSpace($ExpectedChromeProfilePath)) {
    throw "ExpectedChromeProfilePath is required for final performance validation."
}

function Resolve-RequiredFile {
    param([string]$Path, [string]$Label)

    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathRooted($Path)) {
        throw "$Label must be an explicit absolute path."
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "$Label does not exist: $resolved"
    }
    return $resolved
}

function Read-JsonFile {
    param([string]$Path)

    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Require-NumberAtMost {
    param([object]$Object, [string]$Name, [double]$Maximum)

    $value = [double](Get-RequiredPropertyValue $Object @($Name) $Name)
    if ($value -gt $Maximum) {
        throw "$Name is $value, expected at most $Maximum."
    }
}

function Require-NumberGreaterThan {
    param([object]$Object, [string]$Name, [double]$Minimum)

    $value = [double](Get-RequiredPropertyValue $Object @($Name) $Name)
    if ($value -le $Minimum) {
        throw "$Name is $value, expected greater than $Minimum."
    }
}

function Require-NumberAtLeast {
    param([object]$Object, [string]$Name, [double]$Minimum)

    $value = [double](Get-RequiredPropertyValue $Object @($Name) $Name)
    if ($value -lt $Minimum) {
        throw "$Name is $value, expected at least $Minimum."
    }
}

function Get-OptionalProperty {
    param([object]$Object, [string]$Name)

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }
    return $property.Value
}

function Get-RequiredPropertyValue {
    param([object]$Object, [string[]]$Names, [string]$Label)

    foreach ($name in $Names) {
        $value = Get-OptionalProperty $Object $name
        if ($null -ne $value) {
            return $value
        }
    }
    throw "$Label is missing."
}

function Assert-ReportExecutablePath {
    param([object]$Report, [string]$ExpectedPath, [string]$Label)

    $reported = [string](Get-RequiredPropertyValue $Report @("executable") "$Label executable path")
    if ([string]::IsNullOrWhiteSpace($reported) -or -not [IO.Path]::IsPathRooted($reported)) {
        throw "$Label executable path must be an explicit absolute path."
    }
    $resolvedReported = [IO.Path]::GetFullPath($reported)
    if (-not $resolvedReported.Equals($ExpectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label executable path does not match the expected Release EXE. report=$resolvedReported expected=$ExpectedPath"
    }
}

function Assert-ReportMeasuredAt {
    param([object]$Report, [string]$Label)

    $measuredAt = [string](Get-OptionalProperty $Report "measured_at")
    if ([string]::IsNullOrWhiteSpace($measuredAt)) {
        throw "$Label must include measured_at."
    }
    try {
        [DateTimeOffset]::Parse($measuredAt) | Out-Null
    }
    catch {
        throw "$Label measured_at must be an ISO 8601 timestamp."
    }
}

$desktopPath = Resolve-RequiredFile $DesktopReportPath "DesktopReportPath"
$desktop = Read-JsonFile $desktopPath

Assert-ReportMeasuredAt $desktop "Desktop report"
if ([string]::IsNullOrWhiteSpace([string]$desktop.executable_sha256)) {
    throw "Desktop report does not include executable_sha256."
}
if ($null -eq $desktop.samples -or @($desktop.samples).Count -lt 2) {
    throw "Desktop report must include at least two samples."
}
Require-NumberGreaterThan $desktop "cold_start_ms" 0
Require-NumberAtLeast $desktop "actual_duration_seconds" $MinimumDesktopDurationSeconds
Require-NumberAtMost $desktop "idle_cpu_percent_machine" 0.2

$desktopWorkingSetMax = [double](Get-RequiredPropertyValue $desktop @("working_set_max_bytes", "working_set_bytes_max") "desktop working set max")
if ($desktopWorkingSetMax -le 0 -or $desktopWorkingSetMax -gt 35MB) {
    throw "desktop working set max is outside the acceptance bound: $desktopWorkingSetMax"
}

$desktopExternalConnections = [int](Get-RequiredPropertyValue $desktop @("external_tcp_connection_count_max", "external_tcp_connection_count") "desktop external TCP connection count")
if ($desktopExternalConnections -ne 0) {
    throw "desktop external TCP connection count is $desktopExternalConnections, expected 0."
}

$desktopProcessCount = [int](Get-RequiredPropertyValue $desktop @("process_count_max", "process_count") "desktop process count")
if ($desktopProcessCount -ne 1) {
    throw "desktop process count is $desktopProcessCount, expected 1."
}

$executable = Resolve-RequiredFile $ExecutablePath "ExecutablePath"
Assert-ReportExecutablePath $desktop $executable "Desktop report"
$actualHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
if ($actualHash -ne [string]$desktop.executable_sha256) {
    throw "Desktop report executable hash is stale. report=$($desktop.executable_sha256) actual=$actualHash"
}

$chromePath = Resolve-RequiredFile $ChromeReportPath "ChromeReportPath"
$chrome = Read-JsonFile $chromePath
Assert-ReportMeasuredAt $chrome "Chrome report"
Assert-ReportExecutablePath $chrome $executable "Chrome report"
if (-not [IO.Path]::IsPathRooted($ExpectedChromeProfilePath)) {
    throw "ExpectedChromeProfilePath must be an explicit absolute path."
}
$expectedChromeProfile = [IO.Path]::GetFullPath($ExpectedChromeProfilePath).TrimEnd('\')
$reportedChromeProfile = [string](Get-RequiredPropertyValue $chrome @("profile_path") "Chrome profile path")
if ([string]::IsNullOrWhiteSpace($reportedChromeProfile) -or -not [IO.Path]::IsPathRooted($reportedChromeProfile)) {
    throw "Chrome report profile_path must be an explicit absolute path."
}
$actualChromeProfile = [IO.Path]::GetFullPath($reportedChromeProfile).TrimEnd('\')
if (-not $actualChromeProfile.Equals($expectedChromeProfile, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Chrome report profile path does not match the expected AskBridge profile. report=$actualChromeProfile expected=$expectedChromeProfile"
}
$chromeExecutableHash = [string](Get-OptionalProperty $chrome "executable_sha256")
if ([string]::IsNullOrWhiteSpace($chromeExecutableHash)) {
    throw "Chrome report does not include executable_sha256."
}
if ($actualHash -ne $chromeExecutableHash) {
    throw "Chrome report executable hash is stale. report=$chromeExecutableHash actual=$actualHash"
}
if ($null -eq $chrome.samples -or @($chrome.samples).Count -lt 2) {
    throw "Chrome report must include at least two samples."
}
Require-NumberAtLeast $chrome "actual_duration_seconds" $MinimumChromeDurationSeconds
Require-NumberGreaterThan $chrome "working_set_average_bytes" 0
Require-NumberGreaterThan $chrome "process_count_max" 0

$timingsPath = Resolve-RequiredFile $TimingsReportPath "TimingsReportPath"
$timings = Read-JsonFile $timingsPath
$timingsAutoSubmit = Get-OptionalProperty $timings "auto_submit"
$managedBrowserClosed = Get-OptionalProperty $timings "managed_browser_closed"
if ($timingsAutoSubmit -ne $false -or $managedBrowserClosed -ne $true) {
    throw "Preparation timings must record auto_submit=false and managed_browser_closed=true."
}
$timingsProvider = [string](Get-OptionalProperty $timings "provider")
if ([string]::IsNullOrWhiteSpace($timingsProvider)) {
    throw "Preparation timings must include provider."
}
Require-NumberGreaterThan $timings "measured_at_unix_ms" 0
Require-NumberGreaterThan $timings "browser_launch_ms" 0
Require-NumberGreaterThan $timings "first_preparation_ms" 0
Require-NumberGreaterThan $timings "continuous_preparation_ms" 0

Write-Host "Performance reports are internally valid."
