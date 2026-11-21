# Draws the app icon (a white snowflake on a red disc, the same design as the tray icon) at the given pixel size
# and returns a Bitmap. Dot-sourced by build.ps1 (the .ico) and store\make-msix.ps1 (the package logos).
Add-Type -AssemblyName System.Drawing

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
