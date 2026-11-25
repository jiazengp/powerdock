[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location -LiteralPath $projectRoot
try {
    & (Join-Path $PSScriptRoot 'test-locales.ps1')
    & cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust formatting check failed.' }
    & cargo clippy --locked --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Rust static analysis failed.' }
    & cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
} finally {
    Pop-Location
}
