param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][ValidateSet('win-x64-stable','win-x64-preview')][string]$Channel,
    [string]$Repository='jiazengp/powerdock'
)
$ErrorActionPreference='Stop'
if($Repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$'){throw 'Invalid repository'}
$headers=@{Accept='application/vnd.github+json';'User-Agent'='PowerDock-release'}
if($env:GH_TOKEN){$headers.Authorization="Bearer $env:GH_TOKEN"}
# Never turn an authentication failure or network outage into a first release.
$releases=Invoke-RestMethod -Uri "https://api.github.com/repos/$Repository/releases?per_page=100" -Headers $headers -TimeoutSec 30
$ordered=@($releases | Where-Object {-not $_.draft -and $_.tag_name -match '^v\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?$'} | Sort-Object { [semver]($_.tag_name.TrimStart('v')) } -Descending)
$feedName="releases.$Channel.json"
$release=$ordered | Where-Object {@($_.assets | Where-Object name -CEQ $feedName).Count -eq 1} | Select-Object -First 1
if(-not $release){Write-Output "No previous published $Channel feed; generating the first full release.";return}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$feedAsset=$release.assets | Where-Object name -CEQ $feedName
$downloadHeaders=@{Accept='application/octet-stream';'User-Agent'='PowerDock-release'}
if($env:GH_TOKEN){$downloadHeaders.Authorization="Bearer $env:GH_TOKEN"}
$feedFile=Join-Path $OutputDirectory $feedName
Invoke-WebRequest -Uri $feedAsset.url -Headers $downloadHeaders -OutFile $feedFile -TimeoutSec 60
$feed=Get-Content -LiteralPath $feedFile -Raw | ConvertFrom-Json
$full=$feed.Assets | Where-Object {$_.Type -eq 'Full' -and $_.PackageId -ceq 'PowerDock'} | Sort-Object {[semver]$_.Version} -Descending | Select-Object -First 1
if(-not $full -or $full.FileName -notmatch '^[A-Za-z0-9_.-]+\.nupkg$' -or $full.SHA256 -notmatch '^[A-Fa-f0-9]{64}$'){throw 'Previous release feed is invalid'}
$asset=$release.assets | Where-Object name -CEQ $full.FileName
if(@($asset).Count -ne 1){throw 'Previous full package is not attached to its release'}
$target=Join-Path $OutputDirectory $full.FileName
Invoke-WebRequest -Uri $asset.url -Headers $downloadHeaders -OutFile $target -TimeoutSec 300
if((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ine $full.SHA256){throw 'Previous full package checksum mismatch'}
Write-Output "Verified previous $Channel package $($full.Version)."
