$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing, System.Windows.Forms

$p = Start-Process -FilePath 'Q:\gitroot\Game\native-browser\target\release\browser.exe' -PassThru
Start-Sleep -Seconds 9

$sig = @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
public struct RECT { public int L, T, R, B; }
'@
Add-Type -MemberDefinition $sig -Name W -Namespace U

$p.Refresh()
$h = $p.MainWindowHandle
"hwnd=$h title=$($p.MainWindowTitle)"
[U.W]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800

$r = New-Object U.W+RECT
[U.W]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $hh = $r.B - $r.T
"rect=${w}x${hh} at $($r.L),$($r.T)"

$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$g.Dispose()
$bmp.Save('Q:\gitroot\Game\native-browser\build\live.png', [System.Drawing.Imaging.ImageFormat]::Png)

$colors = @{}
for ($y = 0; $y -lt $hh; $y += 10) {
  for ($x = 0; $x -lt $w; $x += 10) {
    $c = $bmp.GetPixel($x, $y)
    $k = '{0:X2}{1:X2}{2:X2}' -f $c.R, $c.G, $c.B
    $colors[$k] = 1 + $colors[$k]
  }
}
"distinct_sampled_colors=$($colors.Count)"
$colors.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 6 | ForEach-Object { "  $($_.Key) $($_.Value)" }
$bmp.Dispose()

Stop-Process -Id $p.Id -Force