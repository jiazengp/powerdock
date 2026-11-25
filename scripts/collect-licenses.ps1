[CmdletBinding()]
param()
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location -LiteralPath $root
try{
    $json=& cargo metadata --locked --format-version 1
    if($LASTEXITCODE -ne 0){throw 'Cannot read locked dependency metadata'}
    $metadata=$json | ConvertFrom-Json
    $copied=0
    foreach($package in $metadata.packages | Where-Object {$_.source -like 'registry+*'}){
        $source=[IO.Path]::GetDirectoryName($package.manifest_path)
        $notices=@(Get-ChildItem -LiteralPath $source -File | Where-Object Name -Match '(?i)^(license|copying|notice|copyright)')
        if($package.license_file){$licensePath=Join-Path $source $package.license_file;if(Test-Path -LiteralPath $licensePath -PathType Leaf){$notices+=Get-Item -LiteralPath $licensePath}}
        if(-not $notices.Count){continue}
        $destination=Join-Path $root ('vendor/dependency-licenses/'+$package.name+'-'+$package.version)
        New-Item -ItemType Directory -Path $destination -Force | Out-Null
        foreach($notice in $notices){$target=Join-Path $destination $notice.Name;if(-not(Test-Path -LiteralPath $target)){Copy-Item -LiteralPath $notice.FullName -Destination $target;$copied++}}
    }
    Write-Output "Collected $copied missing license notices from locked registry dependencies."
}finally{Pop-Location}
