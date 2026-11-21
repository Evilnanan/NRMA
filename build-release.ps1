param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'releases'),
    [ValidateSet('standalone','mmv')][string]$Variant = 'standalone',
    [string]$MmvRoot = $env:NRHEX_MMV,
    [string]$OriginalSfx = $env:NRHEX_ORIGINAL_SFX
)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    $variantName = $Variant.ToLowerInvariant()
    $version = (Select-String -LiteralPath 'Cargo.toml' -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $name = if ($variantName -eq 'mmv') { "Mayhem Augments-MMV-$version" } else { "Mayhem Augments-$version" }
    foreach ($item in @($name, "$name.zip")) {
        if (Test-Path -LiteralPath (Join-Path $OutputDirectory $item)) {
            throw "Release already exists: $item. Choose another output directory or prepare the next version before testing."
        }
    }
    $action = & python -X utf8 tools/tested_build.py plan $variantName
    if ($LASTEXITCODE -ne 0) { throw 'Could not check the installed test build.' }
    if ($action -ne 'reuse') {
        $mmvArgs = @{}
        if ($variantName -eq 'mmv') {
            if ($MmvRoot) { $mmvArgs.MmvRoot = $MmvRoot }
            if (!$OriginalSfx) {
                $record = "work/build/tested-mmv.json"
                if (Test-Path $record) { $OriginalSfx = (Get-Content $record -Raw | ConvertFrom-Json).originalSfx.path }
                if (!$OriginalSfx -and (Test-Path 'work/compat/mmv/build-report.json')) {
                    $OriginalSfx = (Get-Content 'work/compat/mmv/build-report.json' -Raw | ConvertFrom-Json).extraOriginalSfx.path
                }
            }
            if (!$OriginalSfx -or !(Test-Path -LiteralPath $OriginalSfx -PathType Leaf)) {
                throw 'Rebuilding MMV requires its original SFX binder. Pass -OriginalSfx or set NRHEX_ORIGINAL_SFX.'
            }
            $mmvArgs.OriginalSfx = $OriginalSfx
        }
        $buildShared = $action -in @('resources','initial')
        if ($buildShared -and $variantName -eq 'mmv') {
            $shared = & python -X utf8 tools/tested_build.py plan standalone
            if ($LASTEXITCODE -ne 0) { throw 'Could not check shared resource inputs.' }
            $buildShared = $shared -in @('resources','initial')
            if ($shared -eq 'texts') { & ./build-texts.ps1 }
        }
        if ($buildShared) {
            & ./build.ps1
        } elseif ($action -eq 'texts') {
            & ./build-texts.ps1
        }
        if ($variantName -eq 'mmv') { & ./build.ps1 -Variant MMV @mmvArgs }
        elseif (!$buildShared) { & ./build.ps1 -DllOnly }
    } else {
        Write-Host "Reusing verified $variantName test build $version."
    }
    & ./build-texts.ps1 -Check
    if ($LASTEXITCODE -ne 0) { throw 'Release texts are out of date.' }
    $releaseArgs = @('--output-directory', $OutputDirectory, '--variant', $variantName)
    if ($MmvRoot) { $releaseArgs += @('--mmv-root', $MmvRoot) }
    python -X utf8 tools/package_release.py @releaseArgs
    if ($LASTEXITCODE -ne 0) { throw 'Release packaging failed.' }
} finally { Pop-Location }
