param(
    [switch]$SkipBuild,
    [string]$Version
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    if (-not $SkipBuild) {
        cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $metadataJson = cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Cannot collect locked package metadata.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $appPackage = @($metadata.packages | Where-Object name -eq 'lexwisp-app')
    if ($appPackage.Count -ne 1) { throw 'Expected exactly one lexwisp-app package.' }
    $manifestVersion = $appPackage[0].version
    if ([string]::IsNullOrWhiteSpace($Version)) { $Version = $manifestVersion }
    if ($Version -notmatch '^v?(?<semver>\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$') {
        throw "Invalid release version: $Version"
    }
    $releaseVersion = $Matches.semver
    if ($releaseVersion -ne $manifestVersion) {
        throw "Release version $releaseVersion does not match Cargo version $manifestVersion."
    }
    $releaseTag = "v$releaseVersion"
    $packageName = "LexWisp-$releaseTag-windows-x64"
    $stageRoot = Join-Path $repo 'target/package'
    $stage = Join-Path $stageRoot $packageName
    $resolvedStageRoot = [IO.Path]::GetFullPath($stageRoot).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if (-not $resolvedStage.StartsWith($resolvedStageRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolvedStage) -ne $packageName) {
        throw "Refusing to clean unexpected staging path: $resolvedStage"
    }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    $binary = Join-Path $repo 'target/x86_64-pc-windows-msvc/release/LexWisp.exe'
    Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'LexWisp.exe') -Force
    Copy-Item -LiteralPath (Join-Path $repo 'README.md') -Destination (Join-Path $stage 'README.md') -Force
    [IO.File]::WriteAllText((Join-Path $stage 'portable.flag'), '')
    # The verified PE imports VCRUNTIME140.dll. Use the developer redist, never System32.
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $vs) { throw 'Cannot locate the MSVC redistributable.' }
    $redistVersion = (Get-Content -LiteralPath (Join-Path $vs 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt') -Raw).Trim()
    $crt = @(Get-ChildItem -Path (Join-Path $vs "VC/Redist/MSVC/$redistVersion/x64/Microsoft.VC*.CRT/vcruntime140.dll") -File)
    if ($crt.Count -ne 1) { throw 'Expected one x64 VCRUNTIME140.dll redistributable.' }
    Copy-Item -LiteralPath $crt[0].FullName -Destination (Join-Path $stage 'vcruntime140.dll') -Force
    $notices = [Text.StringBuilder]::new()
    [void]$notices.AppendLine("LexWisp $releaseTag - third-party notices")
    [void]$notices.AppendLine('Inventory includes build/test dependencies, not all of which ship. Package sources are unmodified; Windows fonts are not redistributed.')
    [void]$notices.AppendLine("Microsoft Visual C++ Runtime $redistVersion (vcruntime140.dll), Copyright Microsoft Corporation. App-local redistributable from Visual Studio Build Tools. Redistribution list: https://aka.ms/vs/18/redistribution")
    $texts = [Collections.Generic.Dictionary[string,int]]::new([StringComparer]::Ordinal)
    foreach ($package in ($metadata.packages | Where-Object source | Sort-Object name,version)) {
        [void]$notices.AppendLine("`n=== $($package.name) $($package.version) ===")
        [void]$notices.AppendLine("License: $($package.license)")
        [void]$notices.AppendLine("Source: $($package.repository)")
        $packageDir = Split-Path $package.manifest_path
        $licenses = Get-ChildItem -LiteralPath $packageDir -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|UNLICENSE)([.-]|$)' }
        foreach ($license in $licenses) {
            $body = [IO.File]::ReadAllText($license.FullName)
            if (-not $texts.ContainsKey($body)) { $texts.Add($body, $texts.Count + 1) }
            [void]$notices.AppendLine("$($license.Name): see license text $($texts[$body])")
        }
    }
    foreach ($entry in ($texts.GetEnumerator() | Sort-Object Value)) {
        [void]$notices.AppendLine("`n=== License text $($entry.Value) ===")
        [void]$notices.AppendLine($entry.Key)
    }
    [IO.File]::WriteAllText((Join-Path $stage 'THIRD-PARTY-NOTICES.txt'), $notices.ToString())
    # Explicit allowlist: never archive a directory that might contain user data.
    $files = @('LexWisp.exe', 'vcruntime140.dll', 'portable.flag', 'README.md', 'THIRD-PARTY-NOTICES.txt') | ForEach-Object { Join-Path $stage $_ }
    $dist = Join-Path $repo 'dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $archive = Join-Path $dist "$packageName.zip"
    Compress-Archive -LiteralPath $files -DestinationPath $archive -Force
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii
    Get-Item -LiteralPath $binary, $archive | Select-Object FullName, Length
} finally {
    Pop-Location
}
