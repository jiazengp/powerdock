param([string]$VpkPath)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$test = Join-Path $root ('target/validation/update-test-' + [guid]::NewGuid().ToString('N'))
$payload = Join-Path $test 'payload'
$releases = Join-Path $test 'releases'
$null = New-Item -ItemType Directory -Path $payload

& {
    # Invoke-RestMethod returns a JSON array as one output object. Preserve that
    # behavior here to catch a redundant array wrapper in the release script.
    function Invoke-RestMethod {
        param($Uri, $Headers, $TimeoutSec)
        $items = @(
            @{ tag_name = 'v1.0.0'; draft = $false; assets = @(
                @{ name = 'releases.win-x64-stable.json'; url = 'fixture-feed' }
                @{ name = 'PowerDock-1.0.0-full.nupkg'; url = 'fixture-package' }
            )}
            @{ tag_name = 'v1.0.1'; draft = $true; assets = @(
                @{ name = 'releases.win-x64-stable.json'; url = 'unexpected-draft-feed' }
            )}
        )
        return ,$items
    }

    function Invoke-WebRequest {
        param($Uri, $Headers, $OutFile, $TimeoutSec)
        switch ($Uri) {
            'fixture-feed' { Copy-Item -LiteralPath $fixtureFeed -Destination $OutFile }
            'fixture-package' { Copy-Item -LiteralPath $fixturePackage -Destination $OutFile }
            default { throw "Unexpected fixture request $Uri" }
        }
    }

    $fixturePackage = Join-Path $test 'fixture-base.nupkg'
    $fixtureFeed = Join-Path $test 'fixture-feed.json'
    [IO.File]::WriteAllText($fixturePackage, 'verified fixture payload')
    $expectedHash = (Get-FileHash -LiteralPath $fixturePackage).Hash
    @{ Assets = @(@{
        Type = 'Full'; PackageId = 'PowerDock'; Version = '1.0.0'
        FileName = 'PowerDock-1.0.0-full.nupkg'; SHA256 = $expectedHash
    }) } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $fixtureFeed
    $output = Join-Path $test 'release-base'
    & (Join-Path $PSScriptRoot 'fetch-update-base.ps1') -OutputDirectory $output -Channel win-x64-stable -Repository fixture/powerdock
    if ((Get-FileHash -LiteralPath (Join-Path $output 'PowerDock-1.0.0-full.nupkg')).Hash -cne $expectedHash) {
        throw 'Published base package was not selected and verified'
    }
}

Push-Location -LiteralPath $root
try {
    $build = & cargo test --locked --bin powerdock --no-run --message-format=json
    if ($LASTEXITCODE -ne 0) { throw 'Updater fixture build failed' }
    $fixture = @($build | ConvertFrom-Json | Where-Object {
        $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'powerdock' -and $_.profile.test -and $_.executable
    })
    if ($fixture.Count -ne 1) { throw 'Cannot locate the PowerDock test executable' }
    Copy-Item -LiteralPath $fixture[0].executable -Destination (Join-Path $payload 'powerdock.exe')

    foreach ($version in @('1.0.0', '1.0.1', '1.0.2')) {
        $value = ('constant payload line ' + ('x' * 80) + "`n") * 12000 + "version=$version"
        [IO.File]::WriteAllText((Join-Path $payload 'fixture-data.txt'), $value)
        $pack = @(
            'pack', '--packId', 'PowerDock', '--packVersion', $version, '--packDir', $payload,
            '--mainExe', 'powerdock.exe', '--packTitle', 'PowerDock update fixture',
            '--outputDir', $releases, '--channel', 'win-x64-stable', '--runtime', 'win-x64', '--noInst'
        )
        if ($VpkPath) { & $VpkPath @pack } else { & dotnet tool run vpk -- @pack }
        if ($LASTEXITCODE -ne 0) { throw "Fixture packaging failed for $version" }
        if ($version -eq '1.0.0') {
            $savedPortable = Join-Path $test 'old-portable.zip'
            $portable = Join-Path $releases 'PowerDock-win-x64-stable-Portable.zip'
            Copy-Item -LiteralPath $portable -Destination $savedPortable
        }
    }
} finally { Pop-Location }

$feedFile = Join-Path $releases 'releases.win-x64-stable.json'
$feed = Get-Content -LiteralPath $feedFile -Raw | ConvertFrom-Json
$base = $feed.Assets | Where-Object { $_.Version -eq '1.0.0' -and $_.Type -eq 'Full' }
$full = $feed.Assets | Where-Object { $_.Version -eq '1.0.1' -and $_.Type -eq 'Full' }
$deltas = @($feed.Assets | Where-Object Type -eq 'Delta')
if (!$base -or !$full -or @($deltas | Where-Object Version -eq '1.0.1').Count -ne 1 -or
    @($deltas | Where-Object Version -eq '1.0.2').Count -ne 1) {
    throw 'Expected the base, an intermediate full package and two delta packages'
}
$chainCases = @('delta-chain', 'missing-link')
$restartCases = @('delta', 'fallback', 'full', 'interrupted') + $chainCases

$results = @(
    foreach ($case in ($restartCases + @('corrupt-full', 'stale-cache', 'changed-after-download', 'on-exit'))) {
        $targetVersion = if ($case -in $chainCases) { '1.0.2' } else { '1.0.1' }
        $targetFull = $feed.Assets | Where-Object { $_.Version -eq $targetVersion -and $_.Type -eq 'Full' }
        if (!$targetFull) { throw "Missing target full metadata for $case" }
        $caseRoot = Join-Path $test $case
        $source = Join-Path $caseRoot 'source'
        $app = Join-Path $caseRoot 'app'
        $profile = Join-Path $caseRoot 'profile'
        $config = Join-Path $profile 'PowerDock'
        $settings = Join-Path $config 'settings.json'
        $devices = Join-Path $config 'devices.json'
        $null = New-Item -ItemType Directory -Path $source
        $null = New-Item -ItemType Directory -Path $config
        $null = New-Item -ItemType File -Path (Join-Path $caseRoot 'powerdock-update-fixture')
        $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        @{
            version = 1
            interval_secs = 120
            device_thresholds = @{ 'fixture-unit' = 10 }
            device_alerts = @{ 'fixture-unit' = @{ latched = $true; last_seen = $now; last_sent = $now } }
            providers = @{ disabled_readers = @('mchose_hid') }
        } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $settings -Encoding utf8
        @{
            version = 1
            records = @{ 'fixture-unit' = @{ aliases = @('fixture-alias'); evidence = @() } }
        } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $devices -Encoding utf8
        $settingsHash = (Get-FileHash -LiteralPath $settings).Hash
        $devicesHash = (Get-FileHash -LiteralPath $devices).Hash
        Expand-Archive -LiteralPath $savedPortable -DestinationPath $app
        $assets = @($feed.Assets | Where-Object { [version]$_.Version -le [version]$targetVersion })
        @{ Assets = $assets } | ConvertTo-Json -Depth 12 |
            Set-Content -LiteralPath (Join-Path $source 'releases.win-x64-stable.json') -Encoding utf8
        foreach ($delta in $deltas | Where-Object { [version]$_.Version -le [version]$targetVersion }) {
            if ($case -eq 'missing-link' -and $delta.Version -eq '1.0.2') { continue }
            Copy-Item -LiteralPath (Join-Path $releases $delta.FileName) -Destination $source
            if ($case -eq 'fallback') {
                [IO.File]::WriteAllText((Join-Path $source $delta.FileName), 'corrupt delta')
            }
        }
        if ($case -notin 'delta', 'delta-chain') {
            Copy-Item -LiteralPath (Join-Path $releases $targetFull.FileName) -Destination $source
        }
        if ($case -in (@('delta', 'fallback') + $chainCases)) {
            $packages = Join-Path $app 'packages'
            $null = New-Item -ItemType Directory -Path $packages -Force
            Copy-Item -LiteralPath (Join-Path $releases $base.FileName) -Destination $packages
        }
        if ($case -eq 'corrupt-full') {
            [IO.File]::WriteAllText((Join-Path $source $targetFull.FileName), 'corrupt full')
        }

        $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $app 'current/powerdock.exe'))
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.Environment['LOCALAPPDATA'] = $profile
        foreach ($argument in @('--exact', 'updates::tests::package::worker_apply_restart', '--ignored', '--nocapture')) {
            $start.ArgumentList.Add($argument)
        }
        $process = [Diagnostics.Process]::Start($start)
        try {
            $stdout = $process.StandardOutput.ReadToEndAsync()
            $stderr = $process.StandardError.ReadToEndAsync()
            if (!$process.WaitForExit(90000)) {
                $process.Kill($true)
                throw "Updater fixture $case timed out"
            }
            if ($process.ExitCode -ne 0) {
                throw "Updater fixture $case failed: $($stdout.GetAwaiter().GetResult()) $($stderr.GetAwaiter().GetResult())"
            }
        } finally { $process.Dispose() }

        $result = Get-Content -LiteralPath (Join-Path $caseRoot 'result.json') -Raw | ConvertFrom-Json
        $restart = $case -in $restartCases
        $marker = Join-Path $caseRoot 'restarted'
        $installedFile = Join-Path $app 'current/fixture-data.txt'
        $deadline = [DateTime]::UtcNow.AddSeconds(60)
        if ($restart) {
            while (!(Test-Path -LiteralPath $marker) -and [DateTime]::UtcNow -lt $deadline) {
                Start-Sleep -Milliseconds 250
            }
            if (!(Test-Path -LiteralPath $marker) -or (Get-Content -LiteralPath $marker -Raw) -cne $targetVersion) {
                throw "Updater fixture $case did not restart at version $targetVersion"
            }
        }
        $expectedPayload = $value.Replace('version=1.0.2', "version=$targetVersion")
        $applyOnExit = $case -eq 'on-exit'
        if ($applyOnExit) {
            while ([DateTime]::UtcNow -lt $deadline) {
                if ((Get-Content -LiteralPath $installedFile -Raw -ErrorAction SilentlyContinue) -ceq $expectedPayload) { break }
                Start-Sleep -Milliseconds 250
            }
            if (Test-Path -LiteralPath $marker) { throw 'Exit-only update restarted the test application' }
        }
        if (($restart -or $applyOnExit) -and [IO.File]::ReadAllText($installedFile) -cne $expectedPayload) {
            throw "Updater fixture $case installed different bytes from the target"
        }
        if ((Get-FileHash -LiteralPath $settings).Hash -cne $settingsHash -or
            (Get-FileHash -LiteralPath $devices).Hash -cne $devicesHash) {
            throw "Updater fixture $case modified the isolated configuration"
        }
        $result | Add-Member -NotePropertyName configuration_reloaded -NotePropertyValue $restart
        $result | Add-Member -NotePropertyName restarted -NotePropertyValue $restart -PassThru
    }
)

$json = $results | ConvertTo-Json -Depth 5
$json | Set-Content -LiteralPath (Join-Path $root 'target/update-test-results.json') -Encoding utf8
$json
