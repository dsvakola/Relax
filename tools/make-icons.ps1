# Makes every Relax logo from the one master picture assets\relax-icon-1024.png:
#   - the Microsoft Store logos in store\Assets (every scale, 100% to 400%)
#   - src-tauri\icons\icon.ico with 16, 24, 32, 48, 64, 128 and 256 px
# Run only when the master picture changes:  powershell -ExecutionPolicy Bypass -File tools\make-icons.ps1
# Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path -Parent $PSScriptRoot
$master = [Drawing.Image]::FromFile((Join-Path $root 'assets\relax-icon-1024.png'))

function New-Square([int]$size) {
    # Several halving steps keep small sizes sharp (one big jump blurs).
    $cur = New-Object Drawing.Bitmap $master
    while ($cur.Width / 2 -ge $size -and $cur.Width -gt $size * 2) {
        $half = New-Object Drawing.Bitmap ([int]($cur.Width / 2)), ([int]($cur.Height / 2)), ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [Drawing.Graphics]::FromImage($half)
        $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'; $g.SmoothingMode = 'HighQuality'
        $g.DrawImage($cur, 0, 0, $half.Width, $half.Height); $g.Dispose(); $cur.Dispose(); $cur = $half
    }
    $out = New-Object Drawing.Bitmap $size, $size, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($out)
    $g.InterpolationMode = 'HighQualityBicubic'; $g.PixelOffsetMode = 'HighQuality'; $g.SmoothingMode = 'HighQuality'
    $g.Clear([Drawing.Color]::Transparent); $g.DrawImage($cur, 0, 0, $size, $size); $g.Dispose(); $cur.Dispose()
    return $out
}
function Save-Square([int]$size, [string]$path) { $b = New-Square $size; $b.Save($path, [Drawing.Imaging.ImageFormat]::Png); $b.Dispose() }
function Save-Wide([int]$w, [int]$h, [string]$path) {
    # wide tile: the logo centred, as tall as the tile allows (with a small margin)
    $s = [int]($h * 0.8); $sq = New-Square $s
    $out = New-Object Drawing.Bitmap $w, $h, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($out); $g.Clear([Drawing.Color]::Transparent)
    $g.DrawImageUnscaled($sq, [int](($w - $s) / 2), [int](($h - $s) / 2)); $g.Dispose(); $sq.Dispose()
    $out.Save($path, [Drawing.Imaging.ImageFormat]::Png); $out.Dispose()
}

$dir = Join-Path $root 'store\Assets'
if (Test-Path $dir) { Get-ChildItem $dir -Filter *.png | Remove-Item -Force } else { New-Item -ItemType Directory $dir | Out-Null }
$scales = @{ 100 = 1.0; 125 = 1.25; 150 = 1.5; 200 = 2.0; 400 = 4.0 }
foreach ($k in $scales.Keys) {
    $f = $scales[$k]
    Save-Square ([int][Math]::Round(44 * $f))  "$dir\Square44x44Logo.scale-$k.png"
    Save-Square ([int][Math]::Round(71 * $f))  "$dir\SmallTile.scale-$k.png"
    Save-Square ([int][Math]::Round(150 * $f)) "$dir\Square150x150Logo.scale-$k.png"
    Save-Square ([int][Math]::Round(310 * $f)) "$dir\LargeTile.scale-$k.png"
    Save-Square ([int][Math]::Round(50 * $f))  "$dir\StoreLogo.scale-$k.png"
    Save-Wide ([int][Math]::Round(310 * $f)) ([int][Math]::Round(150 * $f)) "$dir\Wide310x150Logo.scale-$k.png"
}
foreach ($t in 16, 20, 24, 30, 32, 36, 40, 44, 48, 60, 64, 72, 80, 96, 256) {
    foreach ($alt in '', '_altform-unplated', '_altform-lightunplated') {
        Save-Square $t "$dir\Square44x44Logo.targetsize-$t$alt.png"
    }
}

# icon.ico: PNG frames
$sizes = 16, 24, 32, 48, 64, 128, 256
$frames = foreach ($s in $sizes) { $b = New-Square $s; $ms = New-Object IO.MemoryStream; $b.Save($ms, [Drawing.Imaging.ImageFormat]::Png); $b.Dispose(); , $ms.ToArray() }
$ico = New-Object IO.MemoryStream; $w = New-Object IO.BinaryWriter $ico
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]; $dimByte = if ($s -ge 256) { 0 } else { $s }
    $w.Write([byte]$dimByte); $w.Write([byte]$dimByte); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$frames[$i].Length); $w.Write([uint32]$offset)
    $offset += $frames[$i].Length
}
foreach ($f in $frames) { $w.Write($f) }
$w.Flush()
[IO.File]::WriteAllBytes((Join-Path $root 'src-tauri\icons\icon.ico'), $ico.ToArray())
$master.Dispose()
Write-Host "Done: $((Get-ChildItem $dir).Count) Store logos and icon.ico ($($sizes -join ', ') px)."
