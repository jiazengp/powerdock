param([ValidateSet('Debug','Release')][string]$Configuration='Release', [string]$OutputDirectory)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
& (Join-Path $PSScriptRoot 'generate-resources.ps1')
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if(-not(Test-Path -LiteralPath $vswhere)){throw 'Visual Studio 2026 C++ Build Tools are required.'}
$install=& $vswhere -latest -products '*' -version '[18.0,19.0)' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if($LASTEXITCODE -ne 0 -or -not $install){throw 'Visual Studio C++ toolchain was not found.'}
$msbuild=Join-Path $install 'MSBuild\Current\Bin\MSBuild.exe'
$output=Join-Path $projectRoot ("target\{0}\ui\" -f $Configuration.ToLowerInvariant())
if($OutputDirectory){$output=[IO.Path]::GetFullPath($OutputDirectory).TrimEnd('\','/')+[IO.Path]::DirectorySeparatorChar}
$arguments=@((Join-Path $projectRoot 'ui\PowerDockUI.vcxproj'),'/restore','/m',"/p:Configuration=$Configuration",'/p:Platform=x64',"/p:OutDir=$output",'/p:RestoreLockedMode=true','/v:minimal','/nologo')
if($OutputDirectory){
    $intermediate=Join-Path ([IO.Path]::GetDirectoryName($output.TrimEnd('\','/'))) 'ui-obj/'
    $generated=Join-Path $intermediate 'Generated Files/'
    $arguments+=@("/p:IntDir=$intermediate","/p:GeneratedFilesDir=$generated")
}
& $msbuild @arguments
if($LASTEXITCODE -ne 0){throw "WinUI build failed ($LASTEXITCODE)."}
$lock=Get-Content -LiteralPath (Join-Path $projectRoot 'ui\packages.lock.json') -Raw | ConvertFrom-Json -AsHashtable
foreach($package in $lock.dependencies['native,Version=v0.0'].GetEnumerator()){
    $packageRoot=Join-Path $projectRoot ("ui\packages\{0}\{1}" -f $package.Key.ToLowerInvariant(),$package.Value.resolved)
    $notices=Get-ChildItem -LiteralPath $packageRoot -File | Where-Object Name -Match '(?i)license|notice|copyright'
    $noticeRoot=Join-Path $output ("licenses\{0}-{1}" -f $package.Key,$package.Value.resolved)
    New-Item -ItemType Directory -Path $noticeRoot -Force | Out-Null
    foreach($notice in $notices){Copy-Item -LiteralPath $notice.FullName -Destination (Join-Path $noticeRoot $notice.Name) -Force}
}
Copy-Item -LiteralPath (Join-Path $projectRoot 'ui\UPSTREAM-LICENSE.txt') -Destination (Join-Path $output 'licenses\Microsoft-WindowsAppSDK-Samples-LICENSE.txt') -Force
$assetOutput=Join-Path $output 'Assets'
New-Item -ItemType Directory -Path $assetOutput -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/PowerDock.ico') -Destination (Join-Path $assetOutput 'PowerDock.ico') -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/FLUENT-ICONS-LICENSE.txt') -Destination (Join-Path $output 'licenses/Fluent-System-Icons-LICENSE.txt') -Force
Write-Output "WinUI frontend: $(Join-Path $output 'PowerDockUI.exe')"
