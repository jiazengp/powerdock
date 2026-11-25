[CmdletBinding()]
param([string]$TargetDirectory)

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
& (Join-Path $PSScriptRoot 'check.ps1')
Push-Location -LiteralPath $projectRoot
try {
    $buildArguments = @('build', '--locked', '--release')
    if ($TargetDirectory) {
        $TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory)
        $buildArguments += @('--target-dir', $TargetDirectory)
    }
    & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
} finally {
    Pop-Location
}
$uiArguments = @{ Configuration = 'Release' }
if ($TargetDirectory) { $uiArguments.OutputDirectory = Join-Path $TargetDirectory 'release/ui' }
& (Join-Path $PSScriptRoot 'build-ui.ps1') @uiArguments
& (Join-Path $PSScriptRoot 'collect-licenses.ps1')
