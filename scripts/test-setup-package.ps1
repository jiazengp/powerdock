[CmdletBinding()]
param([string]$MetadataPath)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if(-not $MetadataPath){$MetadataPath=Join-Path $root 'target/managed-package.json'}
$meta=Get-Content -LiteralPath $MetadataPath -Raw | ConvertFrom-Json
$version=[semver]$meta.version
$channel=if($version.PreReleaseLabel){'win-x64-preview'}else{'win-x64-stable'}
if($meta.channel -cne $channel){throw 'Setup channel does not match package version'}
$directory=[IO.Path]::GetFullPath($meta.directory)
$expected=Join-Path $directory "PowerDock-$channel-Setup.exe"
if(-not $meta.setup -or -not $meta.setupChecksum -or
    -not ([IO.Path]::GetFullPath($meta.setup)).Equals($expected,[StringComparison]::OrdinalIgnoreCase) -or
    -not ([IO.Path]::GetFullPath($meta.setupChecksum)).Equals($expected+'.sha256',[StringComparison]::OrdinalIgnoreCase)){
    throw 'Setup metadata must identify the channel installer and checksum inside the output directory'
}
if(-not(Test-Path -LiteralPath $expected -PathType Leaf)){throw 'Missing Setup installer'}
$hash=(Get-FileHash -LiteralPath $expected -Algorithm SHA256).Hash.ToLowerInvariant()
if($meta.setupSha256 -notmatch '^[a-fA-F0-9]{64}$' -or $hash -ine $meta.setupSha256){throw 'Setup SHA-256 does not match metadata'}
$leaf=[IO.Path]::GetFileName($expected)
$checksum="$hash  $leaf"
foreach($path in @($meta.setupChecksum,(Join-Path $directory 'SHA256SUMS'))){
    $text=Get-Content -LiteralPath $path -Raw
    if($text.Contains("`r") -or -not $text.EndsWith("`n")){throw 'Setup checksum files must use LF line endings for release verification'}
}
if((Get-Content -LiteralPath $meta.setupChecksum -Raw).Trim() -cne $checksum){throw 'Setup checksum file does not match the installer'}
$entries=@(Get-Content -LiteralPath (Join-Path $directory 'SHA256SUMS') | Where-Object {$_ -match ('^[a-fA-F0-9]{64}  '+[regex]::Escape($leaf)+'$')})
if($entries.Count -ne 1 -or $entries[0] -cne $checksum){throw 'Setup is missing or inconsistent in channel SHA256SUMS'}
# Windows reads PE version resources without starting the installer.
$info=[Diagnostics.FileVersionInfo]::GetVersionInfo($expected)
if($info.ProductName -cne 'PowerDock' -or $info.ProductVersion -cne $meta.version){throw 'Setup embedded product/version does not match package metadata'}
$signature=(Get-AuthenticodeSignature -LiteralPath $expected).Status.ToString()
if($signature -notin @('Valid','NotSigned') -or ($meta.signed -and $signature -ne 'Valid')){throw "Setup signature validation failed: $signature"}
& (Join-Path $PSScriptRoot 'test-app-icons.ps1') -Executables @($expected)
$result=[ordered]@{version=$meta.version;channel=$channel;setup=$expected;sha256=$hash;bytes=(Get-Item -LiteralPath $expected).Length;signature=$signature;artifact_valid=$true;installer_executed=$false;install_upgrade_uninstall_tested=$false}
$result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'target/setup-test-results.json') -Encoding utf8
$result | ConvertTo-Json
