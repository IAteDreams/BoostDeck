# Draws the BoostDeck icon and writes assets/boostdeck.ico (16-256 px, PNG
# entries) and assets/boostdeck-256.png (the window icon). Each size is
# drawn on its own so the small ones stay crisp. Run from the repo root:
#   powershell -File assets/make-icon.ps1 -OutDir assets
param([string]$OutDir)
Add-Type -AssemblyName System.Drawing
function Render([int]$n) {
  $bmp = New-Object System.Drawing.Bitmap $n, $n, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'; $g.Clear([System.Drawing.Color]::Transparent)
  $s = $n / 256.0
  # Rounded square background, blue gradient.
  $r = 56 * $s; $m = 8 * $s; $w = $n - 2 * $m
  $path = New-Object System.Drawing.Drawing2D.GraphicsPath
  $path.AddArc($m, $m, 2*$r, 2*$r, 180, 90); $path.AddArc($m + $w - 2*$r, $m, 2*$r, 2*$r, 270, 90)
  $path.AddArc($m + $w - 2*$r, $m + $w - 2*$r, 2*$r, 2*$r, 0, 90); $path.AddArc($m, $m + $w - 2*$r, 2*$r, 2*$r, 90, 90); $path.CloseFigure()
  $grad = New-Object System.Drawing.Drawing2D.LinearGradientBrush ([System.Drawing.PointF]::new(0,0)), ([System.Drawing.PointF]::new($n,$n)), ([System.Drawing.Color]::FromArgb(255, 47, 128, 237)), ([System.Drawing.Color]::FromArgb(255, 18, 52, 128))
  $g.FillPath($grad, $path)
  # Speaker: a box and a cone.
  $white = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::White)
  $pts = @(
    [System.Drawing.PointF]::new(40*$s, 104*$s), [System.Drawing.PointF]::new(76*$s, 104*$s),
    [System.Drawing.PointF]::new(120*$s, 64*$s), [System.Drawing.PointF]::new(120*$s, 192*$s),
    [System.Drawing.PointF]::new(76*$s, 140*$s), [System.Drawing.PointF]::new(40*$s, 140*$s))
  $g.FillPolygon($white, $pts)
  # Sound waves; the outer one orange, for the boost. Small sizes get fewer.
  $waves = if ($n -le 24) { 2 } else { 3 }
  for ($i = 1; $i -le $waves; $i++) {
    $rad = (22 + 26 * $i) * $s
    $color = if ($i -eq $waves) { [System.Drawing.Color]::FromArgb(255, 255, 170, 40) } else { [System.Drawing.Color]::White }
    $pen = New-Object System.Drawing.Pen $color, ([Math]::Max(1.5, 15 * $s))
    $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
    $g.DrawArc($pen, 120*$s - $rad, 128*$s - $rad, 2*$rad, 2*$rad, -45, 90)
  }
  $g.Dispose(); return $bmp
}
$sizes = 16, 24, 32, 48, 64, 128, 256
$pngs = @()
foreach ($n in $sizes) {
  $bmp = Render $n; $ms = New-Object System.IO.MemoryStream
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $pngs += ,($ms.ToArray())
  if ($n -eq 256) { $bmp.Save("$OutDir\boostdeck-256.png", [System.Drawing.Imaging.ImageFormat]::Png) }
  $bmp.Dispose()
}
# ICO: header, one directory entry per size, then the PNG images.
$out = New-Object System.IO.MemoryStream; $bw = New-Object System.IO.BinaryWriter $out
$bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $n = $sizes[$i]; $d = if ($n -ge 256) { 0 } else { $n }
  $bw.Write([byte]$d); $bw.Write([byte]$d); $bw.Write([byte]0); $bw.Write([byte]0)
  $bw.Write([UInt16]1); $bw.Write([UInt16]32); $bw.Write([UInt32]$pngs[$i].Length); $bw.Write([UInt32]$offset)
  $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $bw.Write($p) }
[System.IO.File]::WriteAllBytes("$OutDir\boostdeck.ico", $out.ToArray()); "wrote icon, $($out.Length) bytes"
