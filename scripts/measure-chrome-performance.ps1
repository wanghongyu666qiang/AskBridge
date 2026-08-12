[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ProfilePath,
    [Parameter(Mandatory = $true)]
    [string]$ExecutablePath,
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [ValidateRange(30, 1800)]
    [int]$DurationSeconds = 60,
    [ValidateRange(1, 10)]
    [int]$SampleIntervalSeconds = 1
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not [IO.Path]::IsPathRooted($ProfilePath) -or -not [IO.Path]::IsPathRooted($OutputPath)) {
    throw "ProfilePath and OutputPath must both be explicit absolute paths."
}
$resolvedProfile = [IO.Path]::GetFullPath($ProfilePath).TrimEnd('\')
$resolvedOutput = [IO.Path]::GetFullPath($OutputPath)
$resolvedExecutable = $null
if (-not [IO.Path]::IsPathRooted($ExecutablePath)) {
    throw "ExecutablePath must be an explicit absolute path."
}
$resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$executableHash = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash
New-Item -ItemType Directory -Path (Split-Path -Parent $resolvedOutput) -Force | Out-Null

function Get-ManagedChromeProcessIds {
    $processes = @(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'")
    $managedIds = [Collections.Generic.HashSet[int]]::new()
    foreach ($process in $processes) {
        if ($process.CommandLine -and
            $process.CommandLine.IndexOf("--user-data-dir", [StringComparison]::OrdinalIgnoreCase) -ge 0 -and
            $process.CommandLine.IndexOf($resolvedProfile, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            $managedIds.Add([int]$process.ProcessId) | Out-Null
        }
    }
    do {
        $added = $false
        foreach ($process in $processes) {
            if ($managedIds.Contains([int]$process.ParentProcessId) -and $managedIds.Add([int]$process.ProcessId)) {
                $added = $true
            }
        }
    } while ($added)
    return @($managedIds)
}

$initialIds = @(Get-ManagedChromeProcessIds)
if ($initialIds.Count -eq 0) {
    throw "No Chrome process using the exact AskBridge profile path is running."
}

$samples = [Collections.Generic.List[object]]::new()
$measurementStart = [DateTimeOffset]::Now
$deadline = $measurementStart.AddSeconds($DurationSeconds)
while ([DateTimeOffset]::Now -lt $deadline) {
    $ids = @(Get-ManagedChromeProcessIds)
    $processes = @($ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
    if ($processes.Count -eq 0) {
        throw "The AskBridge Chrome process tree ended before the measurement deadline."
    }
    [int64]$workingSetBytes = 0
    [int64]$privateBytes = 0
    [double]$cpuSeconds = 0
    foreach ($process in $processes) {
        $workingSetBytes += $process.WorkingSet64
        $privateBytes += $process.PrivateMemorySize64
        $cpuSeconds += $process.TotalProcessorTime.TotalSeconds
    }
    $samples.Add([pscustomobject]@{
        timestamp = [DateTimeOffset]::Now.ToString("o")
        process_count = $processes.Count
        working_set_bytes = $workingSetBytes
        private_bytes = $privateBytes
        cpu_seconds = $cpuSeconds
    })
    Start-Sleep -Seconds $SampleIntervalSeconds
}

if ($samples.Count -lt 2) {
    throw "Dedicated Chrome measurement produced fewer than two samples."
}
$measurementEnd = [DateTimeOffset]::Now

$report = [ordered]@{
    measured_at = [DateTimeOffset]::Now.ToString("o")
    profile_path = $resolvedProfile
    executable = $resolvedExecutable
    executable_sha256 = $executableHash
    requested_duration_seconds = $DurationSeconds
    actual_duration_seconds = [Math]::Round(($measurementEnd - $measurementStart).TotalSeconds, 2)
    sample_interval_seconds = $SampleIntervalSeconds
    working_set_average_bytes = [int64](($samples | Measure-Object working_set_bytes -Average).Average)
    working_set_max_bytes = [int64](($samples | Measure-Object working_set_bytes -Maximum).Maximum)
    private_bytes_average = [int64](($samples | Measure-Object private_bytes -Average).Average)
    process_count_max = [int](($samples | Measure-Object process_count -Maximum).Maximum)
    samples = $samples
}
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $resolvedOutput -Encoding UTF8
Write-Host "Dedicated Chrome performance report written to $resolvedOutput"
