param([string]$OutputRoot, [string]$MetadataPath, [string]$BuildDirectory)
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $OutputRoot){$OutputRoot=Join-Path $projectRoot 'dist'}
$OutputRoot=[IO.Path]::GetFullPath($OutputRoot)
$manifest=Get-Content -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -Raw
if($manifest -notmatch '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?)"'){throw 'Cannot read package version'}
$version=$Matches[1]
$name="powerdock-$version-windows-x64"
$destination=Join-Path $OutputRoot $name
if(Test-Path -LiteralPath $destination){$destination+='-'+[DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,6)}
if(-not $BuildDirectory){$BuildDirectory=Join-Path $projectRoot 'target/release'}
$BuildDirectory=[IO.Path]::GetFullPath($BuildDirectory)
$exe=Join-Path $BuildDirectory 'powerdock.exe'
$runtime=Join-Path $BuildDirectory 'ui'
foreach ($binary in @($exe, (Join-Path $runtime 'PowerDockUI.exe'))) {
    if (!(Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "Build the Rust core and WinUI frontend before packaging. Missing $binary"
    }
    $actual = [Diagnostics.FileVersionInfo]::GetVersionInfo($binary).ProductVersion
    if ($actual -cne $version) {
        throw "Expected $version in $binary, found '$actual'. Rebuild both components before packaging."
    }
}
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination $destination
foreach($file in Get-ChildItem -LiteralPath $runtime -Recurse -File){
    if($file.Extension -in @('.pdb','.lib','.exp','.ilk','.obj')){continue}
    $relative=[IO.Path]::GetRelativePath($runtime,$file.FullName)
    $target=[IO.Path]::GetFullPath((Join-Path (Join-Path $destination 'ui') $relative))
    if(-not $target.StartsWith($destination+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Package path escapes destination'}
    New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
    Copy-Item -LiteralPath $file.FullName -Destination $target
}
# Self-contained WinUI does not include the dynamically linked Visual C++ CRT.
# App-local deployment keeps this portable package independent of a machine-wide installer.
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$install=& $vswhere -latest -products '*' -version '[18.0,19.0)' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if($LASTEXITCODE -ne 0 -or -not $install){throw 'Cannot locate Visual C++ redistributable files'}
$redistVersion=Get-ChildItem -LiteralPath (Join-Path $install 'VC\Redist\MSVC') -Directory | Where-Object Name -Match '^\d+\.\d+\.\d+$' | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if(-not $redistVersion){throw 'Visual C++ redistributable version not found'}
$crt=Get-ChildItem -LiteralPath (Join-Path $redistVersion.FullName 'x64') -Directory | Where-Object Name -Like 'Microsoft.VC*.CRT' | Select-Object -First 1
if(-not $crt){throw 'x64 Visual C++ CRT redistributable not found'}
foreach($file in Get-ChildItem -LiteralPath $crt.FullName -File -Filter '*.dll'){
    Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $destination 'ui')
}
Copy-Item -LiteralPath (Join-Path $crt.FullName 'vcruntime140.dll') -Destination $destination
$vcNotice=Join-Path $destination 'ui\licenses\Microsoft-Visual-CPP-Runtime.txt'
@"
Microsoft Visual C++ Runtime, version $($redistVersion.Name), x64.
Copyright Microsoft Corporation. All rights reserved.
These unmodified DLLs are redistributed from the licensed Visual Studio VC/Redist/MSVC directory.
They are Microsoft distributable code and are not covered by this project's MIT license.
Deployment documentation and applicable license terms:
https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files
https://visualstudio.microsoft.com/license-terms/
"@ | Set-Content -LiteralPath $vcNotice
foreach($file in @('SOURCE.md','UPSTREAM-LICENSE.txt')){Copy-Item -LiteralPath (Join-Path $projectRoot "ui\$file") -Destination (Join-Path $destination 'ui')}
foreach($name in @('README.md','CONTRIBUTING.md','CONTRIBUTING-DEVICES.md','CHANGELOG.md','LICENSE','THIRD_PARTY_NOTICES.md')){Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $destination}
foreach($folder in @('vendor','assets')){
    foreach($file in Get-ChildItem -LiteralPath (Join-Path $projectRoot $folder) -File -Recurse){
        $relative=[IO.Path]::GetRelativePath($projectRoot,$file.FullName)
        if($relative.StartsWith('assets\screenshots\',[StringComparison]::OrdinalIgnoreCase)){continue}
        $target=[IO.Path]::GetFullPath((Join-Path $destination $relative))
        if(-not $target.StartsWith($destination+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Notice path escapes destination'}
        New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $target
    }
}
$zip=$destination+'.zip'
Compress-Archive -LiteralPath $destination -DestinationPath $zip -CompressionLevel Optimal
$total=(Get-ChildItem -LiteralPath $destination -File -Recurse | Measure-Object Length -Sum).Sum
$hash=(Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash
$checksum=$zip+'.sha256'
# GNU checksum readers on the release runner require portable LF line endings.
[IO.File]::WriteAllText($checksum, "$($hash.ToLowerInvariant())  $([IO.Path]::GetFileName($zip))`n", [Text.Encoding]::ASCII)
$metadata=[ordered]@{version=$version;directory=$destination;zip=$zip;checksum=$checksum;uncompressedBytes=$total;zipBytes=(Get-Item -LiteralPath $zip).Length;sha256=$hash} | ConvertTo-Json
if(-not $MetadataPath){$MetadataPath=$zip+'.json'}
$MetadataPath=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($MetadataPath)
New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($MetadataPath)) -Force | Out-Null
$metadata | Set-Content -LiteralPath $MetadataPath -Encoding utf8
$metadata
