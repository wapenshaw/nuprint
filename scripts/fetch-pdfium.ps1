<#
.SYNOPSIS
  Download the pinned PDFium build (bblanchon/pdfium-binaries) into vendor/pdfium/<platform>/.
.DESCRIPTION
  Pinned to chromium/7881, the release that pdfium-render 0.9.x binds with its `pdfium_latest`
  feature. The archive SHA-256 is verified before extraction. Result on Windows:
  vendor/pdfium/win-x64/bin/pdfium.dll (plus LICENSE and include/).
.PARAMETER Platform
  win-x64 (default), win-arm64, mac-arm64, mac-x64, linux-x64, linux-arm64.
.PARAMETER Force
  Re-download even if the library is already present.
#>
param(
    [string]$Platform = "win-x64",
    [switch]$Force
)
$ErrorActionPreference = "Stop"

$Tag = "chromium/7881"
$Hashes = @{
    "win-x64"     = "73cc0de638ac2095e7445bf56a38200a5b7c7ca0e9f4ba144598f2457377ac08"
    "win-arm64"   = "d3035d4d2cacac6ecd1a2ece197a3d702a1b2a58466276b9f870b8cb278a9d84"
    "mac-arm64"   = "52e94ca5aa8847934330daf3f8150c190682c5ca93831468794f8b90d4392e40"
    "mac-x64"     = "6dedf83990e0e3d6b7c93c9e7589c5a126b0ae14b7464d76120cff7a26afb18b"
    "linux-x64"   = "1470e21b8b4a3b4ad7f85684e2da11d94f3b69a86d81dee11b9b6709d927ac1d"
    "linux-arm64" = "ee7f7b7d5468958336a818c1cd580bdd20972846b7377b13f9a923d92d1d4674"
}
if (-not $Hashes.ContainsKey($Platform)) {
    throw "Unknown platform '$Platform'. Choose one of: $($Hashes.Keys -join ', ')"
}

$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root "vendor/pdfium/$Platform"
$Library = switch -Wildcard ($Platform) {
    "win-*"   { Join-Path $Dest "bin/pdfium.dll" }
    "mac-*"   { Join-Path $Dest "lib/libpdfium.dylib" }
    default   { Join-Path $Dest "lib/libpdfium.so" }
}
if ((Test-Path $Library) -and -not $Force) {
    Write-Host "PDFium already present: $Library"
    exit 0
}

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("nuprint-pdfium-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Work | Out-Null
try {
    $Archive = Join-Path $Work "pdfium.tgz"
    $Url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$Tag/pdfium-$Platform.tgz"
    Write-Host "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $Archive -UseBasicParsing

    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    if ($Actual -ne $Hashes[$Platform]) {
        throw "SHA-256 mismatch for pdfium-$Platform.tgz: expected $($Hashes[$Platform]) but got $Actual"
    }

    $Extract = Join-Path $Work "extract"
    New-Item -ItemType Directory -Path $Extract | Out-Null
    # Prefer the Windows bsdtar: GNU tar (e.g. from Git Bash) treats "C:\..." as a remote host.
    $Tar = Join-Path $env:SystemRoot "System32/tar.exe"
    if (-not (Test-Path $Tar)) { $Tar = "tar" }
    & $Tar -xzf $Archive -C $Extract
    if ($LASTEXITCODE -ne 0) { throw "tar failed with exit code $LASTEXITCODE" }

    if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
    New-Item -ItemType Directory -Path (Split-Path -Parent $Dest) -Force | Out-Null
    Move-Item $Extract $Dest
    Set-Content -Path (Join-Path $Dest "VERSION.txt") -Value "bblanchon/pdfium-binaries $Tag ($Platform)"
    if (-not (Test-Path $Library)) { throw "Expected library not found after extraction: $Library" }
    Write-Host "PDFium $Tag installed: $Library"
}
finally {
    Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
}
