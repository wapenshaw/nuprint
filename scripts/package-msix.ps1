<#
.SYNOPSIS
  Package the built NuPrint desktop app as an MSIX for the Microsoft Store.
.DESCRIPTION
  Run after `npx tauri build --no-bundle` (or a full build) in apps/tauri. Lays out NuPrint.exe,
  PDFium and the tile images beside AppxManifest.xml, with the identity from
  apps/tauri/src-tauri/msix/identity.json and the version from tauri.conf.json, then packs it with
  the Windows SDK's makeappx. The package is unsigned: the Store signs it. To install it locally,
  turn on Developer Mode and register the layout folder instead:
  Add-AppxPackage -Register target\msix\layout\AppxManifest.xml
.PARAMETER Exe
  The release executable (default: the newer of target\release\nuprint-desktop.exe and the
  x86_64-pc-windows-msvc target's).
.PARAMETER Out
  The .msix to write (default: target\msix\NuPrint_<version>_x64.msix).
#>
param(
    [string]$Exe,
    [string]$Out
)
$ErrorActionPreference = "Stop"

$Root = Split-Path $PSScriptRoot -Parent
$Tauri = Join-Path $Root "apps\tauri\src-tauri"
$Conf = Get-Content (Join-Path $Tauri "tauri.conf.json") -Raw | ConvertFrom-Json
$Identity = Get-Content (Join-Path $Tauri "msix\identity.json") -Raw | ConvertFrom-Json
$Version = $Conf.version

if (-not $Exe) {
    # The newest build, so one left over from another target is never packed.
    $Exe = @("target\x86_64-pc-windows-msvc\release\nuprint-desktop.exe", "target\release\nuprint-desktop.exe") |
        ForEach-Object { Join-Path $Root $_ } | Where-Object { Test-Path $_ } |
        Sort-Object { (Get-Item $_).LastWriteTime } | Select-Object -Last 1
    if (-not $Exe) { throw "No release build of nuprint-desktop.exe; run npx tauri build --no-bundle in apps\tauri." }
}
if (-not $Out) { $Out = Join-Path $Root "target\msix\NuPrint_${Version}_x64.msix" }
$Pdfium = Join-Path $Root "vendor\pdfium\win-x64"
if (-not (Test-Path "$Pdfium\bin\pdfium.dll")) { throw "PDFium is missing; run scripts\fetch-pdfium.ps1." }

$MakeAppx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]$_.Directory.Parent.Name } | Select-Object -Last 1
if (-not $MakeAppx) { throw "makeappx.exe not found; install the Windows SDK." }

# The same layout the installer makes: resources sit beside the executable.
$Layout = Join-Path $Root "target\msix\layout"
Remove-Item $Layout -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$Layout\pdfium", "$Layout\Assets" | Out-Null
Copy-Item $Exe "$Layout\NuPrint.exe"
Copy-Item "$Pdfium\bin\pdfium.dll", "$Pdfium\LICENSE" "$Layout\pdfium\"

# Tile and Store images, scaled from the app icon.
Add-Type -AssemblyName System.Drawing
$Icon = [System.Drawing.Image]::FromFile((Join-Path $Tauri "icons\icon.png"))
try {
    foreach ($image in @(@("Square44x44Logo", 44), @("Square150x150Logo", 150), @("StoreLogo", 50))) {
        $size = $image[1]
        $bitmap = New-Object System.Drawing.Bitmap $size, $size
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.DrawImage($Icon, 0, 0, $size, $size)
        $graphics.Dispose()
        $bitmap.Save("$Layout\Assets\$($image[0]).png", [System.Drawing.Imaging.ImageFormat]::Png)
        $bitmap.Dispose()
    }
} finally {
    $Icon.Dispose()
}

$Escape = { param($text) [System.Security.SecurityElement]::Escape($text) }
$Manifest = (Get-Content (Join-Path $Tauri "msix\AppxManifest.xml") -Raw).
    Replace("{{NAME}}", (& $Escape $Identity.name)).
    Replace("{{PUBLISHER}}", (& $Escape $Identity.publisher)).
    Replace("{{PUBLISHER_DISPLAY_NAME}}", (& $Escape $Identity.publisherDisplayName)).
    Replace("{{VERSION}}", $Version).
    Replace("{{DESCRIPTION}}", (& $Escape $Conf.bundle.shortDescription))
Set-Content "$Layout\AppxManifest.xml" $Manifest -Encoding utf8NoBOM

New-Item -ItemType Directory -Force (Split-Path $Out -Parent) | Out-Null
& $MakeAppx.FullName pack /o /d $Layout /p $Out | Out-Host
if ($LASTEXITCODE -ne 0) { throw "makeappx failed with exit code $LASTEXITCODE" }
Write-Host "Packaged $Out"
