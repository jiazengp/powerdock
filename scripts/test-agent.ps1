[CmdletBinding()]
param([Parameter(Mandatory)][string]$Executable)
$ErrorActionPreference='Stop'
$Executable=[IO.Path]::GetFullPath($Executable)
function Invoke-AgentCommand {
    param([string[]]$NativeArgs,[int]$ExpectedExit=0)
    $start=[Diagnostics.ProcessStartInfo]::new($Executable)
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true
    $start.RedirectStandardError=$true
    foreach($argument in $NativeArgs){$start.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::Start($start)
    try {
        $stdout=$process.StandardOutput.ReadToEndAsync()
        $stderr=$process.StandardError.ReadToEndAsync()
        if(-not $process.WaitForExit(10000)){$process.Kill();throw 'Agent command exceeded its deadline'}
        $output=$stdout.GetAwaiter().GetResult()
        $errorText=$stderr.GetAwaiter().GetResult()
        if($process.ExitCode -ne $ExpectedExit){throw "Agent command exited $($process.ExitCode), expected $ExpectedExit. $output $errorText"}
        $json=ConvertFrom-Json -InputObject $output
        if($json.schema_version -ne 1){throw 'Unexpected Agent schema version'}
        return $json
    } finally {$process.Dispose()}
}
$capabilities=Invoke-AgentCommand @('agent','capabilities')
if(-not $capabilities.ok -or -not $capabilities.data.no_mcp){throw 'CLI capabilities are invalid'}
if($capabilities.data.max_devices -ne 16 -or $capabilities.data.snapshot_reply_byte_limit -ne 131072){throw '16-device preview capacity is not advertised'}
$invalid=Invoke-AgentCommand @('agent','settings','set','--threshold','101') -ExpectedExit 2
if($invalid.error.code -ne 'invalid_arguments'){throw 'Invalid argument contract changed'}
foreach($diagnostic in @(@('agent','--preview','diagnose'),@('agent','--preview','diagnose','--bluetooth'))){
    $blocked=Invoke-AgentCommand -NativeArgs $diagnostic -ExpectedExit 3
    if($blocked.error.code -ne 'preview_read_only'){throw 'Preview accessed hardware diagnostics'}
}
# Never interrupt another preview. Its presence is an actionable precondition failure.
$missing=Invoke-AgentCommand @('agent','--preview','status') -ExpectedExit 4
if($missing.error.code -ne 'not_running'){throw 'An isolated preview is already active'}
$settingsFile=Join-Path $env:LOCALAPPDATA 'PowerDock\settings.json'
$before=if(Test-Path -LiteralPath $settingsFile){(Get-FileHash -LiteralPath $settingsFile -Algorithm SHA256).Hash}else{$null}
$hostProcess=$null
try {
    $hostProcess=Start-Process -FilePath $Executable -ArgumentList @('--preview','--headless','--run-for','45') -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 700
    $status=Invoke-AgentCommand @('agent','--preview','status')
    if(-not $status.ok -or -not $status.data.demo -or $status.data.devices.Count -ne 4 -or $status.data.percent -ne 18){throw 'Preview snapshot or lowest-battery selection is incorrect'}
    $devices=Invoke-AgentCommand @('agent','--preview','devices','list')
    if($devices.data.devices.Count -ne 4 -or $devices.data.selected_id -ne 'demo-headset'){throw 'Device-list snapshot differs from the host'}
    $all=Invoke-AgentCommand @('agent','--preview','devices','list','--all')
    if($all.data.devices.Count -ne 4 -or @($devices.data.devices | Where-Object {$_.connected -ne $true}).Count){throw 'Connected filter or diagnostic all flag changed'}
    $paged=Invoke-AgentCommand @('agent','--preview','devices','list','--all','--page-size','2')
    if($paged.data.total -ne 4 -or $paged.data.devices.Count -ne 2 -or $paged.data.selected_id -ne $all.data.selected_id -or -not $paged.data.next_cursor){throw 'Active snapshot first page changed'}
    $deviceCursor=$paged.data.next_cursor
    $pageTwo=Invoke-AgentCommand @('agent','--preview','devices','list','--all','--cursor',$deviceCursor)
    if($pageTwo.data.revision -cne $paged.data.revision -or $pageTwo.data.offset -ne 2 -or $pageTwo.data.next_cursor){throw 'Active snapshot continuation changed'}
    if(((@($paged.data.devices.id)+@($pageTwo.data.devices.id)) -join ',') -cne ($all.data.devices.id -join ',')){throw 'Active snapshot pagination lost, duplicated or reordered devices'}
    $wrongFilter=Invoke-AgentCommand @('agent','--preview','devices','list','--cursor',$deviceCursor) -ExpectedExit 3
    if($wrongFilter.error.code -ne 'stale_revision'){throw 'Active snapshot cursor was accepted with a different filter'}
    $catalog=Invoke-AgentCommand @('agent','--preview','devices','catalog','--page-size','1')
    if($catalog.data.total -ne 4 -or $catalog.data.active_limit -ne $capabilities.data.max_devices -or $catalog.data.max_parallel -ne 4 -or $catalog.data.candidates.Count -ne 1 -or -not $catalog.data.next_cursor){throw 'Discovery catalog pagination contract changed'}
    $catalogIds=@($catalog.data.candidates.id)
    $cursor=$catalog.data.next_cursor
    while($cursor){
        $page=Invoke-AgentCommand @('agent','--preview','devices','catalog','--page-size','1','--cursor',$cursor)
        if($page.data.revision -cne $catalog.data.revision){throw 'Catalog mixed revisions'}
        $catalogIds+=@($page.data.candidates.id)
        $cursor=$page.data.next_cursor
        if($catalogIds.Count -gt 4){throw 'Catalog failed to terminate'}
    }
    if((($catalogIds | Sort-Object) -join ',') -cne (($devices.data.devices.id | Sort-Object) -join ',')){throw 'Catalog silently lost or duplicated a candidate'}
    $invalidCursor=Invoke-AgentCommand @('agent','--preview','devices','catalog','--cursor','invalid') -ExpectedExit 3
    if($invalidCursor.error.code -ne 'invalid_cursor'){throw 'Malformed catalog cursor was accepted'}
    $staleCursor=('0'*64)+'.1'
    $stale=Invoke-AgentCommand @('agent','--preview','devices','catalog','--cursor',$staleCursor) -ExpectedExit 3
    if($stale.error.code -ne 'stale_revision'){throw 'Stale catalog revision was accepted'}
    $denied=Invoke-AgentCommand @('agent','--preview','settings','set','--threshold','0') -ExpectedExit 3
    if($denied.error.code -ne 'preview_read_only'){throw 'Preview accepted a real mutation'}
    $noticeDenied=Invoke-AgentCommand @('agent','--preview','notification-test') -ExpectedExit 3
    if($noticeDenied.error.code -ne 'preview_read_only'){throw 'Preview sent a real notification'}
    $overlayDenied=Invoke-AgentCommand @('agent','--preview','settings','general','--overlay-duration','10','--overlay-position','bottom-left','--overlay-transparency','50') -ExpectedExit 3
    if($overlayDenied.error.code -ne 'preview_read_only'){throw 'Preview changed notification preferences'}
    $reminder=Invoke-AgentCommand @('agent','--preview','devices','reminder','get','--id','demo-mouse')
    if(-not $reminder.data.inherits_global){throw 'Device reminder did not inherit global settings'}
    if($reminder.data.effective_threshold -ne 20){throw 'Device reminder threshold differs from the global default'}
    $unknownReminder=Invoke-AgentCommand @('agent','--preview','devices','reminder','get','--id','unknown-device') -ExpectedExit 3
    if($unknownReminder.error.code -ne 'device_not_found'){throw 'Unknown reminder target was accepted'}
    $reminderDenied=Invoke-AgentCommand @('agent','--preview','devices','reminder','set','--id','demo-mouse','--threshold','10') -ExpectedExit 3
    if($reminderDenied.error.code -ne 'preview_read_only'){throw 'Preview accepted a per-device reminder mutation'}
    $sortDenied=Invoke-AgentCommand @('agent','--preview','settings','set','--sort','type','--order','desc') -ExpectedExit 3
    if($sortDenied.error.code -ne 'preview_read_only'){throw 'Preview accepted a persisted sort mutation'}
    foreach($arguments in @(@('settings','general','--language','en-US'),@('settings','general','--preview-updates','true'),@('updates','check'),@('updates','download'),@('updates','apply'))){
        $denied=Invoke-AgentCommand (@('agent','--preview')+$arguments) -ExpectedExit 3
        if($denied.error.code -ne 'preview_read_only'){throw 'Preview accepted a general/update mutation'}
    }
    $update=Invoke-AgentCommand @('agent','--preview','updates','status')
    if($update.data.state -ne 'demo' -or $update.data.managed){throw 'Preview started a real update manager'}
    $settings=Invoke-AgentCommand @('agent','--preview','settings','get')
    if(-not $settings.ok -or $settings.data.interval_secs -notin @(30,60,120) -or $settings.data.sort_by -ne 'battery' -or $settings.data.sort_order -ne 'asc'){throw 'Settings response is invalid'}
    $after=if(Test-Path -LiteralPath $settingsFile){(Get-FileHash -LiteralPath $settingsFile -Algorithm SHA256).Hash}else{$null}
    if($before -cne $after){throw 'Isolated Agent validation changed real settings'}
} finally {
    if($hostProcess -and -not $hostProcess.HasExited){
        $quit=Start-Process -FilePath $Executable -ArgumentList @('--preview','--quit') -WindowStyle Hidden -PassThru -Wait
        if(-not $hostProcess.WaitForExit(10000)){throw 'Isolated Agent host did not exit normally'}
    }
}
$missing=Invoke-AgentCommand @('agent','--preview','status') -ExpectedExit 4
if($missing.error.code -ne 'not_running'){throw 'Stopped host was still reported as running'}
Write-Output 'Agent JSON, usage errors, live IPC, device snapshot, lowest selection, preview write isolation and missing-host exit code passed.'
