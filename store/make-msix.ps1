# Builds the Microsoft Store package (MSIX) of the Windows version.
#
#   store\make-msix.ps1 -Version 1.8.0 -IdentityName <Package/Identity/Name> -Publisher <Package/Identity/Publisher> `
#                       -PublisherDisplayName <Package/Properties/PublisherDisplayName> -DisplayName <reserved name>
#
# The three identity values come from Partner Center (the app's Product identity page), and the display name must be
# one of the app's reserved names there; without them the package gets a test identity that the Store would refuse. The package is left unsigned: the Store signs it.
#
# Steps: build JuleHygge.exe with the version, lay out the package in obj\store\layout (the exe, the manifest and
# the logos, drawn like the app icon), and pack it with makeappx.exe from the Windows SDK into
# obj\store\JuleHygge-<version>.msix. Without the SDK (it is not on the development PC) it stops after the layout;
# the Store workflow (.github\workflows\store.yml) packs on GitHub's Windows runner, which has the SDK.
#
# -Register installs the layout for the current user for a local test (needs Developer Mode in Windows Settings,
# System, For developers); Get-AppxPackage *JuleHygge* | Remove-AppxPackage removes it again.
param(
    [string]$Version = '0.0.1',
    [string]$IdentityName = 'JuleHygge.Test',
    [string]$Publisher = 'CN=JuleHygge Test',
    [string]$PublisherDisplayName = 'JuleHygge',
    [string]$DisplayName = 'JuleHygge Test',   # the Store needs a name reserved in Partner Center

    [switch]$Register
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { Write-Error "Version must look like 1.2.3, got '$Version'"; exit 1 }
$root = Split-Path $PSScriptRoot -Parent
. (Join-Path $root 'tools\AppIcon.ps1')   # New-IconBitmap

& (Join-Path $root 'build.ps1') -Version $Version
if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { exit 1 }

$out = Join-Path $root 'obj\store'
$layout = Join-Path $out 'layout'
if (Test-Path $layout) { Remove-Item -Recurse -Force $layout }
New-Item -ItemType Directory -Force (Join-Path $layout 'Assets') | Out-Null
Copy-Item (Join-Path $root 'JuleHygge.exe') $layout

# The logos: the snowflake icon centred on a transparent canvas of each size Windows and the Store ask for.
function Save-Logo([string]$name, [int]$w, [int]$h, [double]$share) {
    $size = [int]([Math]::Min($w, $h) * $share)
    $icon = New-IconBitmap $size
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($icon, [int](($w - $size) / 2), [int](($h - $size) / 2), $size, $size)
    $g.Dispose(); $icon.Dispose()
    $bmp.Save((Join-Path $layout "Assets\$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
Save-Logo 'Square44x44Logo' 44 44 1.0
Save-Logo 'Square150x150Logo' 150 150 0.66
Save-Logo 'Wide310x150Logo' 310 150 0.66
Save-Logo 'StoreLogo' 50 50 1.0

function Escape-Xml([string]$s) { return [System.Security.SecurityElement]::Escape($s) }
$manifest = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot 'AppxManifest.xml'))
$manifest = $manifest.Replace('$IdentityName$', (Escape-Xml $IdentityName)).Replace('$Publisher$', (Escape-Xml $Publisher)).
    Replace('$PublisherDisplayName$', (Escape-Xml $PublisherDisplayName)).Replace('$DisplayName$', (Escape-Xml $DisplayName)).
    Replace('$Version$', "$Version.0")
[System.IO.File]::WriteAllText((Join-Path $layout 'AppxManifest.xml'), $manifest, (New-Object System.Text.UTF8Encoding $false))
Write-Host "Package layout: $layout"

if ($Register) {
    Get-AppxPackage -Name $IdentityName | Remove-AppxPackage
    Add-AppxPackage -Register (Join-Path $layout 'AppxManifest.xml')
    Write-Host "Registered $IdentityName $Version for this user (start JuleHygge from the Start menu)."
}

# makeappx.exe from the newest Windows SDK, if there is one.
$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [Version]$_.Directory.Parent.Name } -ErrorAction SilentlyContinue | Select-Object -Last 1
if (-not $makeappx) { Write-Host 'No Windows SDK (makeappx.exe): stopped after the layout.'; exit 0 }
$msix = Join-Path $out "JuleHygge-$Version.msix"
& $makeappx.FullName pack /o /d $layout /p $msix
if ($LASTEXITCODE -ne 0) { Write-Error 'makeappx failed'; exit 1 }
Write-Host "Built: $msix"
