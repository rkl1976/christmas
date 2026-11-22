# The app icon at a given pixel size, as a Bitmap. Dot-sourced by build.ps1 (the .ico) and store\make-msix.ps1
# (the package logos).
#
# Get-AppIconBitmap uses assets\icon\desktop-hygge.png (a mug of cocoa in a knitted sleeve, made with an image
# model and cleaned of stray pixels; the one image file in the app, see AGENTS.md) and falls back to the drawn
# snowflake (New-IconBitmap, the same design as the tray icon) when the file is missing.
Add-Type -AssemblyName System.Drawing
$script:AppIconPng = Join-Path $PSScriptRoot '..\assets\icon\desktop-hygge.png'

function Get-AppIconBitmap([int]$size) {
    if (-not (Test-Path $script:AppIconPng)) { return New-IconBitmap $size }
    $src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $script:AppIconPng))
    # halve step by step down to twice the target, then the last step: sharper small icons than one big jump
    $cur = $src
    while ($cur.Width -gt $size * 2) {
        $n = [int][Math]::Max($size * 2, $cur.Width / 2)
        $next = New-Object System.Drawing.Bitmap $n, $n, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [System.Drawing.Graphics]::FromImage($next); $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'
        $g.DrawImage($cur, 0, 0, $n, $n); $g.Dispose()
        if ($cur -ne $src) { $cur.Dispose() }
        $cur = $next
    }
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp); $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'
    $g.DrawImage($cur, 0, 0, $size, $size); $g.Dispose()
    if ($cur -ne $src) { $cur.Dispose() }
    $src.Dispose()
    return $bmp
}

function New-IconBitmap([int]$size) {
    $k = $size / 32.0
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Transparent)
    $rect = New-Object System.Drawing.RectangleF (1 * $k), (1 * $k), (30 * $k), (30 * $k)
    $bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, ([System.Drawing.Color]::FromArgb(225, 45, 55)), ([System.Drawing.Color]::FromArgb(160, 15, 25)), 90.0
    $g.FillEllipse($bg, $rect)
    $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::White), ([Math]::Max(1.2, 2.2 * $k))
    $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
    $c = 16 * $k
    for ($i = 0; $i -lt 3; $i++) {
        $a = $i * [Math]::PI / 3 + [Math]::PI / 2
        $dx = [Math]::Cos($a) * 11 * $k; $dy = [Math]::Sin($a) * 11 * $k
        $g.DrawLine($pen, [float]($c - $dx), [float]($c - $dy), [float]($c + $dx), [float]($c + $dy))
        if ($size -lt 24) { continue }   # branches are just noise at tiny sizes
        foreach ($sgn in -1, 1) {
            $ex = $c + $sgn * $dx * 0.65; $ey = $c + $sgn * $dy * 0.65
            $a1 = $a + $(if ($sgn -gt 0) { 0 } else { [Math]::PI })
            foreach ($side in -1, 1) {
                $ba = $a1 + $side * 2.4
                $g.DrawLine($pen, [float]$ex, [float]$ey, [float]($ex + [Math]::Cos($ba) * 4 * $k), [float]($ey + [Math]::Sin($ba) * 4 * $k))
            }
        }
    }
    $g.Dispose()
    return $bmp
}
