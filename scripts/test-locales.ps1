[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixture = Join-Path $root ("target/locale-tests-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $fixture 'scripts'), (Join-Path $fixture 'locales') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'generate-resources.ps1') -Destination (Join-Path $fixture 'scripts/generate-resources.ps1')
$generator = Join-Path $fixture 'scripts/generate-resources.ps1'
$outputs = @('ui/Strings/en-US/Resources.resw', 'ui/Strings/zh-CN/Resources.resw', 'ui/Generated Files/ProductVersion.g.h')
$validEn = @{_version=1;Battery='%{name}: %{percent}%';Literal='A & B < C';NoParameters='PowerDock'}
$validZh = @{_version=1;Battery='%{percent}% · %{name} %{name}';Literal='甲 & 乙 < 丙';NoParameters='电量'}
function Write-Fixture {
    param($English, $Chinese, [string]$Version='0.3.0-preview.2')
    $English | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fixture 'locales/en-US.json') -Encoding utf8
    $Chinese | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $fixture 'locales/zh-CN.json') -Encoding utf8
    "[package]`nversion = `"$Version`"" | Set-Content -LiteralPath (Join-Path $fixture 'Cargo.toml') -Encoding utf8
}
function Assert-Rejected {
    param($English, $Chinese, [string]$Expected, [string]$Version='0.3.0-preview.2')
    Write-Fixture -English $English -Chinese $Chinese -Version $Version
    foreach ($output in $outputs) {
        $path = Join-Path $fixture $output
        New-Item -ItemType Directory -Path (Split-Path -Parent $path) -Force | Out-Null
        Set-Content -LiteralPath $path -Value 'unchanged output sentinel' -Encoding utf8
    }
    $before = @($outputs | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $fixture $_)).Hash })
    $failure = $null
    try { & $generator | Out-Null } catch { $failure = $_.Exception.Message }
    if (-not $failure -or $failure -notmatch $Expected) { throw "Expected rejection '$Expected', received '$failure'" }
    $after = @($outputs | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $fixture $_)).Hash })
    if (Compare-Object $before $after -SyncWindow 0) { throw "Invalid input changed generated output: $failure" }
}
try {
    Write-Fixture -English $validEn -Chinese $validZh
    & $generator
    foreach ($language in @('en-US', 'zh-CN')) {
        [xml]$resw = Get-Content -LiteralPath (Join-Path $fixture "ui/Strings/$language/Resources.resw") -Raw
        $source = if ($language -eq 'en-US') { $validEn } else { $validZh }
        if ($resw.root.data.Count -ne 3) { throw 'Incorrect resource count' }
        foreach ($entry in $resw.root.data) {
            if ($entry.value -cne $source[$entry.name]) { throw "Resource text changed: $language/$($entry.name)" }
        }
    }
    $header = Get-Content -LiteralPath (Join-Path $fixture $outputs[2]) -Raw
    if ($header -notmatch 'PD_FILE_FLAGS 0x2L' -or $header -notmatch 'PD_FILE_VERSION 0,3,0,0') { throw 'Prerelease version metadata changed' }

    $cases = @(
        @{Value=@{_version=1;Other='text'};Expected='zh-CN: keys differ'},
        @{Value=@{_version=1;Battery='%{name}';Literal='text';NoParameters='text'};Expected='zh-CN/Battery: placeholders differ'},
        @{Value=@{_version=1;Battery='   ';Literal='text';NoParameters='text'};Expected='zh-CN/Battery: expected a nonempty string'},
        @{Value=@{_version=1;Battery=15;Literal='text';NoParameters='text'};Expected='zh-CN/Battery: expected a nonempty string'},
        @{Value=@{_version=2;Battery='text'};Expected='zh-CN: _version must be 1'},
        @{Value=@{Battery='text'};Expected='zh-CN: _version must be 1'},
        @{Value=@{_version=1};Expected='zh-CN: catalog has no translations'}
    )
    foreach ($case in $cases) { Assert-Rejected -English $validEn -Chinese $case.Value -Expected $case.Expected }
    foreach ($template in @('%{', '%{name', '%{}', '%{na me}', '%{0name}', '%{%{name}}', "%{name`n}")) {
        $invalid = $validZh.Clone()
        $invalid.Battery = $template
        Assert-Rejected -English $validEn -Chinese $invalid -Expected 'zh-CN/Battery: (unclosed|invalid) placeholder'
    }
    $invalidEnglish = $validEn.Clone()
    $invalidEnglish.Battery = '%{name'
    Assert-Rejected -English $invalidEnglish -Chinese $validZh -Expected 'en-US/Battery: unclosed placeholder'
    Assert-Rejected -English $validEn -Chinese $validZh -Version 'invalid' -Expected 'Cannot read product version'
    Write-Output 'Locale generation passed: valid XML, matching parameters, invalid inputs rejected without output changes.'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    $expectedParent = [IO.Path]::GetFullPath((Join-Path $root 'target'))
    if ((Split-Path -Parent $resolved) -ne $expectedParent -or (Split-Path -Leaf $resolved) -notmatch '^locale-tests-[0-9a-f]{32}$') { throw 'Unexpected locale test cleanup path' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
