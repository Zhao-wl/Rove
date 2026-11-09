# Launch a spike prototype, wait, capture its window to a PNG, then kill it.
#   powershell -File capture.ps1 -Exe <exe> -File <testfile> -Png <out.png> [-WaitSec 3] [-ExtraArgs @()]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$File,
    [Parameter(Mandatory = $true)][string]$Png,
    [int]$WaitSec = 3,
    [string[]]$ExtraArgs = @()
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Cap {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$args2 = @($File) + $ExtraArgs
$p = Start-Process -FilePath $Exe -ArgumentList $args2 -PassThru -RedirectStandardOutput "$Png.out" -RedirectStandardError "$Png.err" -NoNewWindow
$deadline = (Get-Date).AddSeconds(15)
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 300
    $p.Refresh()
    if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break }
}
if ($p.MainWindowHandle -eq [IntPtr]::Zero) { Write-Output "no window"; if (-not $p.HasExited) { $p.Kill() }; exit 1 }
$h = $p.MainWindowHandle
[Win32Cap]::ShowWindow($h, 5) | Out-Null
[Win32Cap]::SetForegroundWindow($h) | Out-Null
# keep it on top so nothing overlaps the capture
[Win32Cap]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
Start-Sleep -Seconds $WaitSec

$r = New-Object Win32Cap+RECT
[Win32Cap]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
Write-Output "window rect: $($r.Left),$($r.Top) ${w}x${hh}"

$bmp = New-Object System.Drawing.Bitmap($w, $hh)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $hh)))
$g.Dispose()
$bmp.Save($Png, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "saved $Png"
if (-not $p.HasExited) { $p.Kill() }
Get-Content "$Png.out" | Select-Object -First 15
