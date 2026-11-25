param([Parameter(Mandatory)][string[]]$Executables, [string]$IconFile)
$ErrorActionPreference='Stop'
# Inspect PE resources without launching apps or consulting Explorer's icon cache.
if(-not ('PowerDockIconResources' -as [type])){
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PowerDockIconResources {
    [DllImport("shell32.dll", CharSet=CharSet.Unicode)]
    public static extern uint ExtractIconExW(string file, int index, IntPtr large, IntPtr small, uint count);
    [DllImport("shell32.dll", CharSet=CharSet.Unicode, EntryPoint="ExtractIconExW")]
    public static extern uint ExtractIcons(string file, int index, out IntPtr large, out IntPtr small, uint count);
    [DllImport("user32.dll")]
    public static extern bool DestroyIcon(IntPtr icon);
}
'@
}
foreach($executable in $Executables){
    $path=[IO.Path]::GetFullPath($executable)
    if(-not(Test-Path -LiteralPath $path -PathType Leaf)){throw "Missing executable: $path"}
    $count=[PowerDockIconResources]::ExtractIconExW($path,-1,[IntPtr]::Zero,[IntPtr]::Zero,0)
    if($count -eq 0 -or $count -eq [uint32]::MaxValue){throw "Missing embedded application icon: $path"}
}
if($IconFile){
    $bytes=[IO.File]::ReadAllBytes([IO.Path]::GetFullPath($IconFile))
    if($bytes.Length -lt 6 -or [BitConverter]::ToUInt16($bytes,0) -ne 0 -or [BitConverter]::ToUInt16($bytes,2) -ne 1){throw 'Invalid ICO header'}
    $count=[BitConverter]::ToUInt16($bytes,4)
    if($bytes.Length -lt 6+16*$count){throw 'Truncated ICO directory'}
    $sizes=for($index=0;$index -lt $count;$index++){
        $offset=6+16*$index
        $width=if($bytes[$offset]){[int]$bytes[$offset]}else{256}
        $height=if($bytes[$offset+1]){[int]$bytes[$offset+1]}else{256}
        if($width -ne $height){throw 'Application icon must be square'}
        $dataLength=[BitConverter]::ToUInt32($bytes,$offset+8)
        $dataOffset=[BitConverter]::ToUInt32($bytes,$offset+12)
        if($dataLength -eq 0 -or [long]$dataOffset+$dataLength -gt $bytes.Length){throw 'Truncated icon image'}
        $width
    }
    if(Compare-Object @(16,20,24,30,32,36,40,48,60,64,72,80,96,128,256) @($sizes | Sort-Object -Unique)){throw 'Application icon sizes do not cover Windows DPI scales'}
    Add-Type -AssemblyName System.Drawing
    function Read-IconPixels {
        param([string]$Path)
        $large=[IntPtr]::Zero
        $small=[IntPtr]::Zero
        try {
            $count=[PowerDockIconResources]::ExtractIcons([IO.Path]::GetFullPath($Path),0,[ref]$large,[ref]$small,1)
            if($count -eq 0 -or $count -eq [uint32]::MaxValue -or $large -eq [IntPtr]::Zero -or $small -eq [IntPtr]::Zero){throw "Cannot extract icon images: $Path"}
            foreach($handle in @($large,$small)){
                $icon=[Drawing.Icon]::FromHandle($handle)
                $bitmap=$icon.ToBitmap()
                try {
                    "$($bitmap.Width)x$($bitmap.Height)"
                    for($row=0;$row -lt $bitmap.Height;$row++){
                        for($column=0;$column -lt $bitmap.Width;$column++){
                            $pixel=$bitmap.GetPixel($column,$row)
                            if($pixel.A -eq 0){0}else{$pixel.ToArgb()}
                        }
                    }
                } finally {$bitmap.Dispose();$icon.Dispose()}
            }
        } finally {
            if($large -ne [IntPtr]::Zero){[void][PowerDockIconResources]::DestroyIcon($large)}
            if($small -ne [IntPtr]::Zero){[void][PowerDockIconResources]::DestroyIcon($small)}
        }
    }
    $expected=@(Read-IconPixels -Path $IconFile)
    foreach($executable in $Executables){
        $actual=@(Read-IconPixels -Path $executable)
        if(Compare-Object $expected $actual -SyncWindow 0){throw "Embedded application icon differs from $IconFile : $executable"}
    }
}
Write-Output 'Embedded application icons, image consistency and native DPI image sizes passed.'
