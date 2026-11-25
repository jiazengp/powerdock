[CmdletBinding()]
param([Parameter(Mandatory)][string]$MetadataPath)

$ErrorActionPreference = 'Stop'
$metadata = Get-Content -LiteralPath $MetadataPath -Raw | ConvertFrom-Json
$archiveHash = (Get-FileHash -LiteralPath $metadata.zip -Algorithm SHA256).Hash
if ($archiveHash -cne $metadata.sha256) { throw 'Archive SHA-256 does not match metadata.' }
$checksumContent = Get-Content -LiteralPath $metadata.checksum -Raw
if ($checksumContent.Contains("`r") -or -not $checksumContent.EndsWith("`n")) {
    throw 'Archive checksum must use LF line endings for release verification.'
}
$checksum = $checksumContent.Trim()
if ($checksum -cne "$($archiveHash.ToLowerInvariant())  $([IO.Path]::GetFileName($metadata.zip))") {
    throw 'Checksum file does not match the archive.'
}

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$scratch = Join-Path $projectRoot ('target\validation\package-smoke-' + [Guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $metadata.zip -DestinationPath $scratch
$roots = @(Get-ChildItem -LiteralPath $scratch -Directory)
if ($roots.Count -ne 1) { throw 'Archive must contain a single application directory.' }
$application = $roots[0].FullName
$required = @(
    'powerdock.exe', 'VCRUNTIME140.dll', 'README.md', 'LICENSE', 'CONTRIBUTING-DEVICES.md',
    'THIRD_PARTY_NOTICES.md', 'ui\PowerDockUI.exe', 'ui\Microsoft.ui.xaml.dll',
    'ui\Microsoft.WindowsAppRuntime.dll', 'ui\msvcp140.dll', 'ui\vcruntime140_1.dll',
    'ui\licenses\Microsoft-WindowsAppSDK-Samples-LICENSE.txt',
    'ui\licenses\Microsoft-Visual-CPP-Runtime.txt',
    'ui\Assets\PowerDock.ico', 'ui\licenses\Fluent-System-Icons-LICENSE.txt'
)
foreach ($relative in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $application $relative) -PathType Leaf)) {
        throw "Required package file is missing: $relative"
    }
}
foreach ($relative in @('docs', 'plans', 'skills', 'experiments', 'SUPPORTED-DEVICES.md', 'assets\screenshots')) {
    if (Test-Path -LiteralPath (Join-Path $application $relative)) {
        throw "Portable package must not contain local-only files: $relative"
    }
}
$developmentFiles = @(Get-ChildItem -LiteralPath $application -File -Recurse | Where-Object Extension -In @('.pdb', '.lib', '.exp', '.ilk', '.obj', '.log'))
if ($developmentFiles.Count) { throw 'Portable package contains development artifacts.' }
& (Join-Path $PSScriptRoot 'test-app-icons.ps1') -Executables @((Join-Path $application 'powerdock.exe'),(Join-Path $application 'ui/PowerDockUI.exe')) -IconFile (Join-Path $application 'ui/Assets/PowerDock.ico')

function Invoke-PackagedCommand {
    param([string[]]$NativeArgs)

    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = Join-Path $application 'powerdock.exe'
    $start.WorkingDirectory = $application
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $NativeArgs) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($start)
    try {
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            throw 'Packaged command timed out.'
        }
        $result = $stdout.GetAwaiter().GetResult()
        $errorText = $stderr.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) { throw "Packaged command failed: $errorText" }
        return $result
    } finally {
        $process.Dispose()
    }
}

$version = Invoke-PackagedCommand -NativeArgs @('--version')
if ($version.Trim() -cne "powerdock $($metadata.version)") {
    throw 'Packaged executable version does not match Cargo.toml.'
}
# Enumeration is metadata-only; no receiver or notification interaction is required.
$diagnostic = Invoke-PackagedCommand -NativeArgs @('--diagnose')
$devices = ConvertFrom-Json -InputObject $diagnostic -NoEnumerate
if ($devices -isnot [array]) { throw 'HID diagnostic must return a JSON array.' }
$discovery = Invoke-PackagedCommand -NativeArgs @('--list-devices')
$candidates = ConvertFrom-Json -InputObject $discovery -NoEnumerate
if ($candidates -isnot [array]) { throw 'Multi-device discovery must return a JSON array.' }
$icons = Join-Path $scratch 'tray icons.bmp'
$null = Invoke-PackagedCommand -NativeArgs @('--export-icons', $icons)
$bytes = [IO.File]::ReadAllBytes($icons)
if ($bytes.Length -lt 54 -or $bytes[0] -ne 0x42 -or $bytes[1] -ne 0x4D) {
    throw 'Packaged icon renderer did not create a BMP.'
}
& (Join-Path $PSScriptRoot 'test-ui.ps1') -Executable (Join-Path $application 'ui\PowerDockUI.exe')
& (Join-Path $PSScriptRoot 'test-agent.ps1') -Executable (Join-Path $application 'powerdock.exe')
Write-Output "Portable archive, runtime files, version, enumeration, Agent CLI and icon renderer passed: $($metadata.version)"
