param([Parameter(Mandatory)][string]$Executable)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$scratch=Join-Path $root ('target\validation\diagnostics-smoke-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch -Force | Out-Null
$settingsFile=Join-Path $env:LOCALAPPDATA 'PowerDock\settings.json'
$before=if(Test-Path -LiteralPath $settingsFile){(Get-FileHash -LiteralPath $settingsFile).Hash}else{$null}
function Run-DiagnosticsProcess([string[]]$Arguments,[bool]$ExpectSuccess=$true){
    $start=[Diagnostics.ProcessStartInfo]::new($Executable);$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $start.Environment['POWERDOCK_DIAGNOSTICS_ROOT']=$scratch
    foreach($argument in $Arguments){$start.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::Start($start)
    try{
        $output=$process.StandardOutput.ReadToEndAsync();$errorOutput=$process.StandardError.ReadToEndAsync()
        if(-not $process.WaitForExit(20000)){$process.Kill($true);throw 'Diagnostic fixture timed out'}
        $text=$output.GetAwaiter().GetResult();$errors=$errorOutput.GetAwaiter().GetResult()
        if(($process.ExitCode -eq 0) -ne $ExpectSuccess){throw "Unexpected diagnostic exit $($process.ExitCode): $text $errors"}
        return $text
    }finally{$process.Dispose()}
}
$null=Run-DiagnosticsProcess @('--diagnostics-smoke',$scratch)
$null=Run-DiagnosticsProcess @('--diagnostics-smoke',$scratch,'--native-crash-fixture')
$null=Run-DiagnosticsProcess @('--diagnostics-smoke',$scratch,'--panic-fixture') $false
$events=@(Get-ChildItem -LiteralPath $scratch -Filter '*.jsonl' | ForEach-Object {Get-Content -LiteralPath $_.FullName | ForEach-Object {ConvertFrom-Json -InputObject $_}})
if(-not @($events | Where-Object {$_.fields.event -eq 'application_stopped'}).Count){throw 'Graceful log flush failed'}
if(-not @($events | Where-Object {$_.event -eq 'rust_panic' -and $_.backtrace}).Count){throw 'Aborting panic did not preserve an independent stack report'}
if(-not @($events | Where-Object {$_.fields.event -eq 'native_error' -and $_.fields.details -match 'E0005044'}).Count){throw 'Native fatal exception was not captured by the parent logger'}
$bundle=Join-Path $scratch 'report.zip'
$reply=Run-DiagnosticsProcess @('agent','diagnostics','export','--output',$bundle) | ConvertFrom-Json
if(-not $reply.ok -or $reply.data.uploaded){throw 'Diagnostic export failed or attempted upload'}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive=[IO.Compression.ZipFile]::OpenRead($bundle)
try{
    $entry=$archive.GetEntry('manifest.json');if(-not $entry){throw 'Missing manifest'}
    $reader=[IO.StreamReader]::new($entry.Open());try{$text=$reader.ReadToEnd()}finally{$reader.Dispose()}
    $manifest=$text | ConvertFrom-Json
    if($manifest.schema_version -ne 1 -or $manifest.privacy.device_names -or $manifest.privacy.automatic_upload){throw 'Diagnostic manifest contract changed'}
    if($text.Contains($env:USERPROFILE)){throw 'Diagnostic manifest exposed a profile path'}
    foreach($entry in @($archive.Entries | Where-Object FullName -Like 'logs/*')){
        $reader=[IO.StreamReader]::new($entry.Open());try{$lines=$reader.ReadToEnd()}finally{$reader.Dispose()}
        foreach($line in ($lines -split "`n" | Where-Object {$_})){$null=ConvertFrom-Json -InputObject $line}
    }
}finally{$archive.Dispose()}
$denied=Run-DiagnosticsProcess @('agent','--preview','diagnostics','export','--output',(Join-Path $scratch 'forbidden.zip')) $false | ConvertFrom-Json
if($denied.error.code -ne 'preview_read_only' -or (Test-Path (Join-Path $scratch 'forbidden.zip'))){throw 'Preview exported production logs'}
$after=if(Test-Path -LiteralPath $settingsFile){(Get-FileHash -LiteralPath $settingsFile).Hash}else{$null}
if($before -cne $after){throw 'Diagnostic fixtures changed user settings'}
@{structured_logs=$true;normal_flush=$true;panic_report=$true;native_fault=$true;zip=$true;redaction=$true;preview_isolation=$true} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'target/diagnostics-test-results.json') -Encoding utf8
Write-Output 'Diagnostic events, flush, aborting panic, native exception capture, ZIP, redaction and preview isolation passed.'
