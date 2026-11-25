#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$OutputPath,
    [ValidateRange(1, 100)][int]$Runs = 30
)

$ErrorActionPreference = 'Stop'
$exe = Get-Item -LiteralPath $Executable
if ($exe.PSIsContainer -or $exe.Name -ine 'powerdock.exe') {
    throw 'Select a PowerDock executable.'
}
if (Test-Path -LiteralPath $OutputPath) {
    throw "Output already exists: $OutputPath"
}

$startedAt = [DateTimeOffset]::UtcNow
$sha256 = (Get-FileHash -LiteralPath $exe.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$windows = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$start = [Diagnostics.ProcessStartInfo]::new($exe.FullName)
$start.ArgumentList.Add('--list-devices')
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.Environment['POWERDOCK_DIAGNOSTICS_ROOT'] = Join-Path ([IO.Path]::GetTempPath()) "powerdock-discovery-$([Guid]::NewGuid())"
$results = @(
    for ($run = 1; $run -le $Runs; $run++) {
        $candidateCount = $null
        $exitCode = $null
        $errorCode = $null
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $process = [Diagnostics.Process]::Start($start)
        try {
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
            if ($process.WaitForExit(10000)) {
                $exitCode = $process.ExitCode
                $text = $stdout.GetAwaiter().GetResult()
                $null = $stderr.GetAwaiter().GetResult()
                $clock.Stop()
                if ($exitCode -ne 0) {
                    $errorCode = 'nonzero_exit'
                } else {
                    try {
                        $candidates = ConvertFrom-Json -InputObject $text -NoEnumerate
                        if ($candidates -isnot [Array]) { throw 'Expected an array.' }
                        $candidateCount = $candidates.Count
                    } catch {
                        $errorCode = 'invalid_output'
                    }
                }
            } else {
                $clock.Stop()
                $process.Kill()
                $errorCode = 'timeout'
            }
        } finally {
            $process.Dispose()
        }
        [pscustomobject]@{
            run = $run
            elapsed_ms = $clock.Elapsed.TotalMilliseconds
            exit_code = $exitCode
            error = $errorCode
            candidate_count = $candidateCount
        }
    }
)

$successful = @($results | Where-Object { $null -eq $_.error } | Sort-Object elapsed_ms)
$p95 = if ($successful.Count) { $successful[[int][Math]::Ceiling(0.95 * $successful.Count) - 1].elapsed_ms }
$result = [ordered]@{
    schema_version = 1
    started_at_utc = $startedAt.ToString('o')
    windows_build = "$($windows.CurrentBuildNumber).$($windows.UBR)"
    product_version = $exe.VersionInfo.ProductVersion
    executable_sha256 = $sha256
    command = '--list-devices'
    requested_runs = $Runs
    successful_runs = $successful.Count
    successful_p95_ms = $p95
    samples = $results
}
$result | ConvertTo-Json -Depth 4 | Out-File -LiteralPath $OutputPath -Encoding utf8 -NoClobber
if ($successful.Count -ne $Runs) {
    throw "Metadata discovery failed in $($Runs - $successful.Count) of $Runs runs. See $OutputPath."
}
Write-Output "Metadata discovery completed $Runs runs (p95 $([Math]::Round($p95, 2)) ms)."
