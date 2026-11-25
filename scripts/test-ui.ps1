param([Parameter(Mandatory)][string]$Executable,
  [ValidateSet('battery','settings','tray','navigation','charging','notification')][string[]]$Pages=@('battery','settings','tray','navigation','charging','notification'))
$ErrorActionPreference='Stop'
foreach($theme in @('light','dark')){
  foreach($page in $Pages){
    $scenarios = if ($page -ne 'notification') {
        @{}
    } elseif ($theme -eq 'light') {
        @{ Position = 'bottom-left'; Transparency = 20 }
    } else {
        @{ Position = 'top-right'; Transparency = 50 }
        @{ Position = 'bottom-right'; Transparency = 20 }
        @{ Position = 'top-left'; Transparency = 0 }
        @{ Position = 'bottom-left'; Transparency = 20 }
        @{ Position = 'top-right'; Transparency = 100 }
    }
    foreach($scenario in $scenarios){
    $position = $scenario.Position
    $start=[Diagnostics.ProcessStartInfo]::new([IO.Path]::GetFullPath($Executable))
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $mode=if($page -eq 'navigation'){'--smoke-navigation'}elseif($page -eq 'charging'){'--smoke-charging'}else{'--smoke-test'}
    $launchPage=if($page -eq 'navigation'){'tray'}elseif($page -eq 'charging'){'battery'}else{$page}
    $notificationTitle='PowerDock 提醒测试'
    $notificationBody='Mouse · 15% / 设备低电量。请连接充电。'
    if($page -eq 'notification' -and $position -eq 'bottom-right'){
        $notificationTitle='PowerDock · 多设备低电量提醒 / Low battery'
        $notificationBody='MCHOSE A7 V3 Pro · 15%。请连接充电。MOMENTUM 4 Wireless headphones · 10%. Connect your devices to power. 长设备名称和中英文文案应自动换行。'
    }
    if ($scenario.Transparency -eq 100) {
        $notificationTitle = 'PowerDock · 全透明背景测试'
    }
    $launchArguments=if($page -eq 'notification'){@('--notification','--smoke-test','--title',$notificationTitle,'--body',$notificationBody)}else{@('--preview',$mode,'--theme',$theme,'--page',$launchPage)}
    if($page -eq 'notification'){
        $launchArguments+=@('--position',$position,'--transparency',[string]$scenario.Transparency)
    }
    if($page -eq 'notification' -and $theme -eq 'light'){$launchArguments+=@('--reduce-motion','--primary-monitor')}
    foreach($arg in $launchArguments){$start.ArgumentList.Add($arg)}
    $process=[Diagnostics.Process]::Start($start)
    try{
        $stdout=$process.StandardOutput.ReadToEndAsync();$stderr=$process.StandardError.ReadToEndAsync()
        if(-not $process.WaitForExit(15000)){$process.Kill($true);throw "WinUI $theme $page startup timed out"}
        $output=$stdout.GetAwaiter().GetResult();$errorText=$stderr.GetAwaiter().GetResult()
        if($page -eq 'notification'){Write-Output ($output.Trim())}
        if($process.ExitCode -ne 0 -or $output -notmatch 'WinUI (startup|navigation|charging|notification) passed'){throw "WinUI $theme $page startup failed ($($process.ExitCode)): $output $errorText"}
        if($page -eq 'notification' -and $position -eq 'bottom-right'){
            if($output -notmatch 'Notification layout: [\d.]+ x ([\d.]+)' -or [double]$Matches[1] -le 80){
                throw 'Long notification did not expand its compact layout.'
            }
        }
    }finally{$process.Dispose()}
    }
  }
}
Write-Output "Native WinUI light/dark scenarios passed: $($Pages -join ', ')."
