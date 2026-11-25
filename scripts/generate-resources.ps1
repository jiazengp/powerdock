param()
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$catalogs = [ordered]@{}
$baseline = $null

function Get-Placeholders {
    param([string]$Text, [string]$Location)
    $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $offset = 0
    while (($start = $Text.IndexOf('%{', $offset, [StringComparison]::Ordinal)) -ge 0) {
        $end = $Text.IndexOf('}', $start + 2)
        if ($end -lt 0) { throw "${Location}: unclosed placeholder" }
        $name = $Text.Substring($start + 2, $end - $start - 2)
        if ($name -cnotmatch '\A[A-Za-z_][A-Za-z0-9_]*\z') { throw "${Location}: invalid placeholder %{$name}" }
        [void]$names.Add($name)
        $offset = $end + 1
    }
    # Preserve the set even when it is empty or contains one item.
    return ,$names
}

# Validate every input before creating or overwriting generated outputs.
foreach ($language in @('en-US', 'zh-CN')) {
    $strings = Get-Content -LiteralPath (Join-Path $root "locales\$language.json") -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    if ($strings -isnot [Collections.IDictionary]) { throw "${language}: expected a JSON object" }
    if ($strings['_version'] -isnot [long] -or $strings['_version'] -ne 1) { throw "${language}: _version must be 1" }
    $keys = @($strings.Keys | Where-Object { $_ -cne '_version' } | Sort-Object -CaseSensitive)
    if (-not $keys.Count) { throw "${language}: catalog has no translations" }
    if ($null -ne $baseline) {
        $difference = @(Compare-Object $baseline $keys -CaseSensitive)
        if ($difference.Count) { throw "${language}: keys differ from en-US: $($difference.InputObject -join ', ')" }
    }
    foreach ($key in $keys) {
        if ($strings[$key] -isnot [string] -or [string]::IsNullOrWhiteSpace($strings[$key])) { throw "${language}/${key}: expected a nonempty string" }
        $names = Get-Placeholders -Text $strings[$key] -Location "$language/$key"
        if ($null -ne $baseline) {
            $expected = Get-Placeholders -Text $catalogs['en-US'][$key] -Location "en-US/$key"
            if (-not $names.SetEquals($expected)) { throw "${language}/${key}: placeholders differ from en-US" }
        }
    }
    $catalogs[$language] = $strings
    if ($null -eq $baseline) { $baseline = $keys }
}

$manifest = Get-Content -LiteralPath (Join-Path $root 'Cargo.toml') -Raw
if ($manifest -notmatch '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?)"') { throw 'Cannot read product version' }
$productVersion = $Matches[1]
$version = [semver]$productVersion
$flags = if ($version.PreReleaseLabel) { '0x2L' } else { '0x0L' }

foreach ($language in $catalogs.Keys) {
    $strings = $catalogs[$language]
    $dir = Join-Path $root "ui\Strings\$language"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $settings = [Xml.XmlWriterSettings]::new()
    $settings.Indent = $true
    $settings.Encoding = [Text.UTF8Encoding]::new($false)
    $writer = [Xml.XmlWriter]::Create((Join-Path $dir 'Resources.resw'), $settings)
    try {
        $writer.WriteStartDocument()
        $writer.WriteStartElement('root')
        foreach ($header in ([ordered]@{resmimetype='text/microsoft-resx';version='2.0';reader='System.Resources.ResXResourceReader, System.Windows.Forms';writer='System.Resources.ResXResourceWriter, System.Windows.Forms'}).GetEnumerator()) {
            $writer.WriteStartElement('resheader')
            $writer.WriteAttributeString('name', $header.Key)
            $writer.WriteElementString('value', $header.Value)
            $writer.WriteEndElement()
        }
        foreach ($key in $baseline) {
            $writer.WriteStartElement('data')
            $writer.WriteAttributeString('name', $key)
            $writer.WriteAttributeString('xml', 'space', $null, 'preserve')
            $writer.WriteElementString('value', $strings[$key])
            $writer.WriteEndElement()
        }
        $writer.WriteEndElement()
        $writer.WriteEndDocument()
    } finally { $writer.Dispose() }
}
Write-Output "Generated WinUI resources for $($baseline.Count) shared locale keys."

$generated = Join-Path $root 'ui/Generated Files'
New-Item -ItemType Directory -Path $generated -Force | Out-Null
@("#define PD_FILE_VERSION $($version.Major),$($version.Minor),$($version.Patch),0", ('#define PD_VERSION_STRING "' + $productVersion + '\0"'), "#define PD_FILE_FLAGS $flags") | Set-Content -LiteralPath (Join-Path $generated 'ProductVersion.g.h') -Encoding ascii
