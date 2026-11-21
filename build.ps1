# Builds JuleHygge.exe with the C# compiler that ships with Windows (.NET Framework 4.x).
# The app icon is drawn here at build time, so the repo contains no image files.
# -Version x.y.z stamps the exe (the GitHub workflow passes the release tag). Without it the exe is 0.0.0,
# a development build that never updates itself.
param([string]$Version = '0.0.0')
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { Write-Error "Version must look like 1.2.3, got '$Version'"; exit 1 }
Add-Type -AssemblyName System.Drawing

$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path $csc)) { $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe' }
$out = Join-Path $PSScriptRoot 'JuleHygge.exe'
$ico = Join-Path $PSScriptRoot 'JuleHygge.ico'

. (Join-Path $PSScriptRoot 'tools\AppIcon.ps1')   # New-IconBitmap: the snowflake icon, shared with store\make-msix.ps1

# Encodes one icon entry: PNG for 256 px, classic 32-bit DIB (BGRA + AND mask) for smaller sizes,
# because many readers (including System.Drawing.Icon) only understand PNG in the 256 px entry.
function Get-IconEntryBytes([int]$size) {
    $bmp = New-IconBitmap $size
    $ms = New-Object System.IO.MemoryStream
    if ($size -ge 256) {
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    } else {
        $w = New-Object System.IO.BinaryWriter $ms
        $maskStride = [int]([Math]::Ceiling($size / 32.0) * 4)
        # BITMAPINFOHEADER; height is doubled to cover the XOR image plus the AND mask
        $w.Write([uint32]40); $w.Write([int32]$size); $w.Write([int32]($size * 2))
        $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]0)
        $w.Write([uint32]($size * $size * 4 + $maskStride * $size))
        $w.Write([int32]0); $w.Write([int32]0); $w.Write([uint32]0); $w.Write([uint32]0)
        for ($y = $size - 1; $y -ge 0; $y--) {          # rows are stored bottom-up
            for ($x = 0; $x -lt $size; $x++) {
                $c = $bmp.GetPixel($x, $y)
                $w.Write([byte]$c.B); $w.Write([byte]$c.G); $w.Write([byte]$c.R); $w.Write([byte]$c.A)
            }
        }
        $w.Write((New-Object byte[] ($maskStride * $size)))   # all-zero mask; alpha channel does the work
        $w.Flush()
    }
    $bmp.Dispose()
    return , $ms.ToArray()
}

# Writes a multi-size .ico file.
function Write-Ico([string]$path, [int[]]$sizes) {
    $pngs = @($sizes | ForEach-Object { , (Get-IconEntryBytes $_) })
    $fs = [System.IO.File]::Create($path)
    $w = New-Object System.IO.BinaryWriter $fs
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $s = $sizes[$i]; $dim = if ($s -ge 256) { 0 } else { $s }
        $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([uint16]1); $w.Write([uint16]32)
        $w.Write([uint32]$pngs[$i].Length); $w.Write([uint32]$offset)
        $offset += $pngs[$i].Length
    }
    foreach ($p in $pngs) { $w.Write($p) }
    $w.Close()
}

Write-Ico $ico @(16, 24, 32, 48, 64, 256)

# Version attributes, generated so the version lives only in the release tag.
$obj = Join-Path $PSScriptRoot 'obj'
New-Item -ItemType Directory -Force $obj | Out-Null
$versionFile = Join-Path $obj 'Version.cs'
@"
using System.Reflection;
[assembly: AssemblyTitle("JuleHygge")]
[assembly: AssemblyProduct("JuleHygge")]
[assembly: AssemblyVersion("$Version.0")]
[assembly: AssemblyFileVersion("$Version.0")]
[assembly: AssemblyInformationalVersion("$Version")]
"@ | Set-Content -Path $versionFile -Encoding UTF8

# The menu's languages (assets\sprog\README.md): every file there goes into the exe as a resource "sprog.xx.ini".
$sprog = Get-ChildItem (Join-Path $PSScriptRoot 'assets\sprog\*.ini') | ForEach-Object { "/resource:$($_.FullName),sprog.$($_.Name)" }

& $csc /nologo /codepage:65001 /target:winexe /optimize+ /unsafe /platform:anycpu /out:$out `
    /win32icon:$ico `
    "/resource:$(Join-Path $PSScriptRoot 'assets\lyd\gaes-flok.wav'),gaes-flok.wav" `
    $sprog `
    /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Web.Extensions.dll `
    (Join-Path $PSScriptRoot 'src\*.cs') $versionFile
if ($LASTEXITCODE -ne 0) { Write-Error "Build failed"; exit 1 }
Write-Host "Built: $out (version $Version)"
