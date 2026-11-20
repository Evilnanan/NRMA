param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'releases'),
    [ValidateSet('standalone','mmv')][string]$Variant = 'standalone',
    [string]$MmvRoot = $env:NRHEX_MMV
)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    & ./build-texts.ps1 -Check
    if ($LASTEXITCODE -ne 0) { throw 'Release texts are out of date.' }
    $releaseArgs = @('--output-directory', $OutputDirectory, '--variant', $Variant.ToLowerInvariant())
    if ($MmvRoot) { $releaseArgs += @('--mmv-root', $MmvRoot) }
    python -X utf8 tools/package_release.py @releaseArgs
    if ($LASTEXITCODE -ne 0) { throw 'Release packaging failed.' }
} finally { Pop-Location }
