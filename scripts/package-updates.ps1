param(
    [Parameter(Mandatory)][string]$MetadataPath,
    [string]$OutputDirectory,
    [switch]$FetchPrevious,
    [string]$Repository='jiazengp/powerdock',
    [string]$VpkPath
)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$meta=Get-Content -LiteralPath $MetadataPath -Raw | ConvertFrom-Json
$version=[semver]$meta.version
$channel=if($version.PreReleaseLabel){'win-x64-preview'}else{'win-x64-stable'}
if(-not $OutputDirectory){$OutputDirectory=Join-Path $root "dist\updates\$channel"}
$OutputDirectory=[IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
if($FetchPrevious){& (Join-Path $PSScriptRoot 'fetch-update-base.ps1') -OutputDirectory $OutputDirectory -Channel $channel -Repository $Repository}
$oldFeed=Join-Path $OutputDirectory "releases.$channel.json"
if(Test-Path -LiteralPath $oldFeed){
    $existing=Get-Content -LiteralPath $oldFeed -Raw | ConvertFrom-Json
    if($existing.Assets | Where-Object {[semver]$_.Version -ge $version}){throw 'Output already contains this version or a newer one; use a new version and fresh output directory'}
}
$arguments=@('pack','--packId','PowerDock','--packTitle','PowerDock','--icon',(Join-Path $root 'assets/PowerDock.ico'),'--packVersion',$meta.version,'--packDir',$meta.directory,'--mainExe','powerdock.exe','--outputDir',$OutputDirectory,'--channel',$channel,'--runtime','win-x64','--delta','BestSpeed','--releaseNotes',(Join-Path $root 'CHANGELOG.md'))
# A release signing command can be passed to vpk before it computes packages and deltas.
if($env:POWERDOCK_SIGN_TEMPLATE){$arguments+=@('--signTemplate',$env:POWERDOCK_SIGN_TEMPLATE)}
if($VpkPath){& $VpkPath @arguments;$packExit=$LASTEXITCODE}else{
    Push-Location -LiteralPath $root
    try{& dotnet tool run vpk -- @arguments;$packExit=$LASTEXITCODE}finally{Pop-Location}
}
if($packExit -ne 0){throw "Velopack failed ($packExit)"}
$setup=Join-Path $OutputDirectory "PowerDock-$channel-Setup.exe"
$portable=Join-Path $OutputDirectory "PowerDock-$channel-Portable.zip"
if(-not(Test-Path -LiteralPath $setup -PathType Leaf)){throw 'Missing Setup installer from Velopack output'}
if(-not(Test-Path -LiteralPath $portable -PathType Leaf)){throw 'Missing managed portable from Velopack output'}
$setupHash=(Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
$setupChecksum=$setup+'.sha256'
[IO.File]::WriteAllText($setupChecksum, "$setupHash  $([IO.Path]::GetFileName($setup))`n", [Text.Encoding]::ASCII)
$feed=Get-Content -LiteralPath $oldFeed -Raw | ConvertFrom-Json
$current=@($feed.Assets | Where-Object Version -CEQ $meta.version)
if(@($current | Where-Object Type -eq 'Full').Count -ne 1){throw 'Missing full update package'}
foreach($asset in $current){
    if($asset.PackageId -cne 'PowerDock' -or $asset.FileName -notmatch '^[A-Za-z0-9_.-]+\.nupkg$'){throw 'Invalid generated update asset'}
    if((Get-FileHash -LiteralPath (Join-Path $OutputDirectory $asset.FileName) -Algorithm SHA256).Hash -ine $asset.SHA256){throw 'Generated package checksum mismatch'}
}
$checksums=Get-ChildItem -LiteralPath $OutputDirectory -File | Where-Object Extension -in '.exe','.zip','.nupkg','.json' | Sort-Object Name | ForEach-Object {"$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $($_.Name)"}
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'SHA256SUMS'), ($checksums -join "`n")+"`n", [Text.Encoding]::ASCII)
$result=[ordered]@{version=$meta.version;channel=$channel;directory=$OutputDirectory;setup=$setup;setupSha256=$setupHash;setupChecksum=$setupChecksum;portable=$portable;portableSha256=(Get-FileHash -LiteralPath $portable -Algorithm SHA256).Hash.ToLowerInvariant();delta=@($current | Where-Object Type -eq 'Delta').Count -gt 0;signed=[bool]$env:POWERDOCK_SIGN_TEMPLATE}
$result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'target\managed-package.json') -Encoding utf8
$result | ConvertTo-Json
