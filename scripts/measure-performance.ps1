[CmdletBinding()]
param(
    [string]$ExecutablePath,
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,
    [ValidateRange(30, 3600)]
    [int]$DurationSeconds = 300,
    [ValidateRange(1, 10)]
    [int]$SampleIntervalSeconds = 1
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
    $scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
    $ExecutablePath = Join-Path (Split-Path -Parent $scriptDirectory) "target\release\askbridge.exe"
}
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
if (-not [IO.Path]::IsPathRooted($OutputPath)) {
    throw "OutputPath must be absolute so the report location is explicit."
}
$resolvedOutput = [IO.Path]::GetFullPath($OutputPath)
$outputParent = Split-Path -Parent $resolvedOutput
New-Item -ItemType Directory -Path $outputParent -Force | Out-Null

$existing = @(Get-Process -Name askbridge -ErrorAction SilentlyContinue)
if ($existing.Count -gt 0) {
    throw "Close every AskBridge process before measuring a clean cold start."
}

$logicalProcessors = [Math]::Max(1, [Environment]::ProcessorCount)
$startClock = [Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath $executable -PassThru -WindowStyle Hidden
try {
    $process.WaitForInputIdle(5000) | Out-Null
    $coldStartMs = $startClock.Elapsed.TotalMilliseconds
    Start-Sleep -Seconds 2
    $samples = [Collections.Generic.List[object]]::new()
    $measurementStart = [DateTimeOffset]::Now
    $deadline = $measurementStart.AddSeconds($DurationSeconds)
    $processName = [IO.Path]::GetFileNameWithoutExtension($executable)
    while ([DateTimeOffset]::Now -lt $deadline) {
        $current = Get-Process -Id $process.Id -ErrorAction Stop
        $sameExecutableProcesses = @(Get-Process -Name $processName -ErrorAction SilentlyContinue | Where-Object {
            try {
                $_.Path -and $_.Path.Equals($executable, [StringComparison]::OrdinalIgnoreCase)
            }
            catch {
                $false
            }
        })
        $connections = @(Get-NetTCPConnection -OwningProcess $process.Id -ErrorAction SilentlyContinue |
            Where-Object { $_.State -notin @("Closed", "TimeWait") })
        $externalConnections = @($connections | Where-Object {
            $_.RemoteAddress -notin @("0.0.0.0", "127.0.0.1", "::", "::1")
        })
        $samples.Add([pscustomobject]@{
            timestamp = [DateTimeOffset]::Now.ToString("o")
            cpu_seconds = $current.TotalProcessorTime.TotalSeconds
            working_set_bytes = [int64]$current.WorkingSet64
            private_bytes = [int64]$current.PrivateMemorySize64
            handle_count = $current.HandleCount
            thread_count = $current.Threads.Count
            process_count = $sameExecutableProcesses.Count
            external_tcp_connection_count = $externalConnections.Count
        })
        Start-Sleep -Seconds $SampleIntervalSeconds
    }

    if ($samples.Count -lt 2) {
        throw "Desktop performance measurement produced fewer than two samples."
    }
    $measurementEnd = [DateTimeOffset]::Now
    $firstSampleTime = [DateTimeOffset]::Parse([string]$samples[0].timestamp)
    $lastSampleTime = [DateTimeOffset]::Parse([string]$samples[-1].timestamp)
    $elapsed = [Math]::Max(0.001, ($lastSampleTime - $firstSampleTime).TotalSeconds)
    $cpuDelta = [Math]::Max(0, $samples[-1].cpu_seconds - $samples[0].cpu_seconds)
    $cpuPercentOfMachine = 100 * $cpuDelta / $elapsed / $logicalProcessors
    $report = [ordered]@{
        measured_at = [DateTimeOffset]::Now.ToString("o")
        executable = $executable
        executable_sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
        requested_duration_seconds = $DurationSeconds
        actual_duration_seconds = [Math]::Round(($measurementEnd - $measurementStart).TotalSeconds, 2)
        sample_interval_seconds = $SampleIntervalSeconds
        logical_processors = $logicalProcessors
        cold_start_ms = [Math]::Round($coldStartMs, 2)
        idle_cpu_percent_machine = [Math]::Round($cpuPercentOfMachine, 4)
        working_set_average_bytes = [int64](($samples | Measure-Object working_set_bytes -Average).Average)
        working_set_max_bytes = [int64](($samples | Measure-Object working_set_bytes -Maximum).Maximum)
        private_bytes_average = [int64](($samples | Measure-Object private_bytes -Average).Average)
        external_tcp_connection_count_max = [int](($samples | Measure-Object external_tcp_connection_count -Maximum).Maximum)
        process_count_max = [int](($samples | Measure-Object process_count -Maximum).Maximum)
        samples = $samples
        targets = [ordered]@{
            idle_cpu_percent_max = 0.2
            idle_working_set_target_bytes = 20MB
            idle_working_set_acceptance_bytes = 35MB
            idle_external_network_connections = 0
        }
    }
    $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $resolvedOutput -Encoding UTF8
    Write-Host "Performance report written to $resolvedOutput"
}
finally {
    if (-not $process.HasExited) {
        $process.CloseMainWindow() | Out-Null
        Start-Sleep -Milliseconds 500
    }
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
}
