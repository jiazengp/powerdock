#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateRange(1, 2147483647)][int]$CoreProcessId,
    [Parameter(Mandatory)][string]$OutputPath,
    [ValidateRange(1, 86400)][int]$DurationSeconds = 600,
    [ValidateRange(1, 60)][int]$SampleIntervalSeconds = 5
)

$ErrorActionPreference = 'Stop'
if ($DurationSeconds -lt $SampleIntervalSeconds) {
    throw 'Duration must be at least one sample interval.'
}
if (Test-Path -LiteralPath $OutputPath) {
    throw "Output already exists: $OutputPath"
}

$core = Get-Process -Id $CoreProcessId
try {
    if ($core.ProcessName -ine 'powerdock' -or -not $core.Path) {
        throw 'Select a running PowerDock core with a readable executable.'
    }

    $executable = $core.Path
    $version = (Get-Item -LiteralPath $executable).VersionInfo.ProductVersion
    $sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
    $sessionId = $core.SessionId
    $windows = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $samples = [Collections.Generic.List[object]]::new()
    $intervals = [Collections.Generic.List[double]]::new()
    $cpuPercents = [Collections.Generic.List[double]]::new()
    $startedAt = [DateTimeOffset]::UtcNow
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $firstSampleTime = 0.0
    $nextSample = 0.0

    while ($true) {
        $delay = $nextSample - $clock.Elapsed.TotalSeconds
        if ($delay -gt 0) {
            Start-Sleep -Milliseconds ([Math]::Ceiling($delay * 1000))
        }

        $core.Refresh()
        if ($core.HasExited) {
            throw 'PowerDock core exited during sampling.'
        }
        $cpu = $core.TotalProcessorTime.TotalSeconds
        $sampleTime = $clock.Elapsed.TotalSeconds
        if ($samples.Count -eq 0) { $firstSampleTime = $sampleTime }
        $elapsed = $sampleTime - $firstSampleTime
        $uiProcesses = [Diagnostics.Process]::GetProcessesByName('PowerDockUI')
        try {
            $uiCount = @($uiProcesses | Where-Object { $_.SessionId -eq $sessionId }).Count
        } finally {
            foreach ($ui in $uiProcesses) { $ui.Dispose() }
        }
        if ($samples.Count -gt 0) {
            $previous = $samples[$samples.Count - 1]
            $gap = $elapsed - $previous.elapsed_seconds
            $intervals.Add($gap)
            $cpuPercents.Add(100 * ($cpu - $previous.cpu_seconds) / $gap)
        }
        $samples.Add([pscustomobject]@{
            elapsed_seconds = $elapsed
            cpu_seconds = $cpu
            private_bytes = $core.PrivateMemorySize64
            working_set_bytes = $core.WorkingSet64
            threads = $core.Threads.Count
            handles = $core.HandleCount
            ui_processes_in_session = $uiCount
        })
        if ($elapsed -ge $DurationSeconds) { break }
        $nextSample = $sampleTime + $SampleIntervalSeconds
    }
} finally {
    $core.Dispose()
}

$first = $samples[0]
$last = $samples[$samples.Count - 1]
$measuredSeconds = $last.elapsed_seconds - $first.elapsed_seconds
$sortedCpu = @($cpuPercents | Sort-Object)
$p95Index = [int][Math]::Ceiling(0.95 * $sortedCpu.Count) - 1
$result = [ordered]@{
    schema_version = 1
    started_at_utc = $startedAt.ToString('o')
    windows_build = "$($windows.CurrentBuildNumber).$($windows.UBR)"
    product_version = $version
    executable_sha256 = $sha256
    requested_duration_seconds = $DurationSeconds
    measured_seconds = $measuredSeconds
    sample_interval_seconds = $SampleIntervalSeconds
    max_gap_seconds = ($intervals | Measure-Object -Maximum).Maximum
    core_cpu_percent_single_logical_cpu_mean = 100 * ($last.cpu_seconds - $first.cpu_seconds) / $measuredSeconds
    core_cpu_percent_single_logical_cpu_p95 = $sortedCpu[$p95Index]
    ui_present_samples = @($samples | Where-Object { $_.ui_processes_in_session -gt 0 }).Count
    samples = $samples.ToArray()
}
$result | ConvertTo-Json -Depth 4 | Out-File -LiteralPath $OutputPath -Encoding utf8 -NoClobber
Write-Output "PowerDock runtime sampled for $([Math]::Round($measuredSeconds, 2)) seconds ($($samples.Count) samples)."
