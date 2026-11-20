param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'releases'))
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    & ./build-texts.ps1 -Check
    if ($LASTEXITCODE -ne 0) { throw 'Release texts are out of date.' }
    python -X utf8 tools/package_release.py --output-directory $OutputDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Release packaging failed.' }
} finally { Pop-Location }
