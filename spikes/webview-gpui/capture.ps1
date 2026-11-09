param([Parameter(Mandatory=$true)][int]$ProcessId,[Parameter(Mandatory=$true)][string]$Output,[long]$WindowHandle=0)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class RoveShot {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out RECT rect);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,IntPtr pid);
 [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
 [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a,uint b,bool attach);
}
"@
$p=Get-Process -Id $ProcessId
$hwnd=if($WindowHandle -ne 0){[IntPtr]$WindowHandle}else{$p.MainWindowHandle}
if($hwnd -eq [IntPtr]::Zero){throw 'No visible prototype window'}
$r=[RoveShot+RECT]::new()
if(-not [RoveShot]::GetWindowRect($hwnd,[ref]$r)){throw 'GetWindowRect failed'}
$bitmap=[Drawing.Bitmap]::new($r.Right-$r.Left,$r.Bottom-$r.Top)
$graphics=[Drawing.Graphics]::FromImage($bitmap)
try {
 $current=[RoveShot]::GetCurrentThreadId()
 $foreground=[RoveShot]::GetWindowThreadProcessId([RoveShot]::GetForegroundWindow(),[IntPtr]::Zero)
 $target=[RoveShot]::GetWindowThreadProcessId($hwnd,[IntPtr]::Zero)
 [void][RoveShot]::AttachThreadInput($current,$foreground,$true)
 [void][RoveShot]::AttachThreadInput($target,$foreground,$true)
 try { [void][RoveShot]::SetForegroundWindow($hwnd) }
 finally {
  [void][RoveShot]::AttachThreadInput($target,$foreground,$false)
  [void][RoveShot]::AttachThreadInput($current,$foreground,$false)
 }
 Start-Sleep -Milliseconds 150
 if([RoveShot]::GetForegroundWindow() -ne $hwnd){Write-Warning 'Prototype is not foreground: image requires visual inspection for occlusion'}
 $graphics.CopyFromScreen($r.Left,$r.Top,0,0,$bitmap.Size)
 $bitmap.Save($Output,[Drawing.Imaging.ImageFormat]::Png)
} finally {$graphics.Dispose();$bitmap.Dispose()}
