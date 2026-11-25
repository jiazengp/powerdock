param([string]$MetadataPath)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $MetadataPath){$MetadataPath=Join-Path $root 'target\managed-package.json'}
$meta=Get-Content -LiteralPath $MetadataPath -Raw | ConvertFrom-Json
$destination=Join-Path $root ('target\validation\managed-smoke-'+[guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $meta.portable -DestinationPath $destination
foreach($relative in @('Update.exe','powerdock.exe','current\powerdock.exe','current\sq.version','current\ui\PowerDockUI.exe','current\ui\PowerDockUI.pri','current\ui\Assets\PowerDock.ico')){
    if(-not(Test-Path -LiteralPath (Join-Path $destination $relative) -PathType Leaf)){throw "Missing managed package file $relative"}
}
$exe=Join-Path $destination 'current\powerdock.exe'
& (Join-Path $PSScriptRoot 'test-app-icons.ps1') -Executables @((Join-Path $destination 'powerdock.exe'),$exe,(Join-Path $destination 'current/ui/PowerDockUI.exe')) -IconFile (Join-Path $destination 'current/ui/Assets/PowerDock.ico')
$versionStart=[Diagnostics.ProcessStartInfo]::new($exe)
$versionStart.UseShellExecute=$false;$versionStart.CreateNoWindow=$true
$versionStart.RedirectStandardOutput=$true;$versionStart.RedirectStandardError=$true
$versionStart.ArgumentList.Add('--version')
$versionProcess=[Diagnostics.Process]::Start($versionStart)
try{
    $output=$versionProcess.StandardOutput.ReadToEndAsync();$errorOutput=$versionProcess.StandardError.ReadToEndAsync()
    if(-not $versionProcess.WaitForExit(10000)){$versionProcess.Kill();throw 'Managed version check timed out'}
    $actualVersion=$output.GetAwaiter().GetResult().Trim()
    if($versionProcess.ExitCode -ne 0 -or $actualVersion -cne "powerdock $($meta.version)"){throw "Managed version check failed: $actualVersion $($errorOutput.GetAwaiter().GetResult())"}
    Write-Output $actualVersion
}finally{$versionProcess.Dispose()}
& (Join-Path $PSScriptRoot 'test-ui.ps1') -Executable (Join-Path $destination 'current\ui\PowerDockUI.exe')
& (Join-Path $PSScriptRoot 'test-agent.ps1') -Executable $exe
& (Join-Path $PSScriptRoot 'test-diagnostics.ps1') -Executable $exe
@{version=$meta.version;layout=$true;agent=$true} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'target\managed-test-results.json') -Encoding utf8
Write-Output 'Managed portable layout and isolated Agent CLI passed.'
