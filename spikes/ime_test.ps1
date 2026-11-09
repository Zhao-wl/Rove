# Drive the Microsoft Pinyin IME against a spike prototype and capture the result.
# Safety: no key is ever sent unless GetForegroundWindow() is confirmed to be the
# prototype's own window, so stray keystrokes cannot land in other applications.
#   powershell -File ime_test.ps1 -Exe <exe> -File <testfile> -Tag <name> [-ForceChinese]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$File,
    [Parameter(Mandatory = $true)][string]$Tag,
    [switch]$ForceChinese,
    [ValidateSet('shift','ctrl','ctrlspace')][string]$SwitchKey = 'ctrlspace'
)

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ImeDrv {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint threadId);
    [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint mapType);
    [DllImport("imm32.dll")] public static extern IntPtr ImmGetContext(IntPtr h);
    [DllImport("imm32.dll")] public static extern bool ImmReleaseContext(IntPtr h, IntPtr himc);
    [DllImport("imm32.dll")] public static extern bool ImmGetConversionStatus(IntPtr himc, out uint conv, out uint sentence);
    [DllImport("imm32.dll")] public static extern bool ImmSetConversionStatus(IntPtr himc, uint conv, uint sentence);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@

$shot = "D:\Projects\Research\Rove\spikes\ime-$Tag"
$p = Start-Process -FilePath $Exe -ArgumentList $File -PassThru -RedirectStandardOutput "$shot.out" -RedirectStandardError "$shot.err" -NoNewWindow
$deadline = (Get-Date).AddSeconds(20)
while ((Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300; $p.Refresh(); if ($p.MainWindowHandle -ne [IntPtr]::Zero) { break } }
if ($p.MainWindowHandle -eq [IntPtr]::Zero) { Write-Output "RESULT: no window"; if (-not $p.HasExited) { $p.Kill() }; exit 1 }
$h = $p.MainWindowHandle

# --- try hard to make our window the foreground window -----------------------
$me = [ImeDrv]::GetCurrentThreadId()
$target = [ImeDrv]::GetWindowThreadProcessId($h, [IntPtr]::Zero)
$fg = [ImeDrv]::GetForegroundWindow()
$fgThread = [ImeDrv]::GetWindowThreadProcessId($fg, [IntPtr]::Zero)
[ImeDrv]::AttachThreadInput($me, $fgThread, $true) | Out-Null
[ImeDrv]::AttachThreadInput($target, $fgThread, $true) | Out-Null
[ImeDrv]::ShowWindow($h, 5) | Out-Null
[ImeDrv]::BringWindowToTop($h) | Out-Null
[ImeDrv]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 1200
[ImeDrv]::AttachThreadInput($me, $fgThread, $false) | Out-Null
[ImeDrv]::AttachThreadInput($target, $fgThread, $false) | Out-Null

$now = [ImeDrv]::GetForegroundWindow()
if ($now -ne $h) {
    Write-Output "RESULT: could not make the prototype window foreground (got 0x$("{0:X}" -f $now.ToInt64()), wanted 0x$("{0:X}" -f $h.ToInt64()))."
    Write-Output "RESULT: ABORTING - no keystrokes sent, to avoid typing into another application."
    if (-not $p.HasExited) { $p.Kill() }
    exit 2
}
Write-Output "foreground OK; keyboard layout of UI thread: 0x$("{0:X}" -f ([ImeDrv]::GetKeyboardLayout($target)).ToInt64())"

$himc = [ImeDrv]::ImmGetContext($h)
$conv = 0; $sent = 0
if ($himc -ne [IntPtr]::Zero) {
    [ImeDrv]::ImmGetConversionStatus($himc, [ref]$conv, [ref]$sent) | Out-Null
    Write-Output ("IMM conversion mode = 0x{0:X} (bit0 NATIVE={1}), sentence = 0x{2:X}" -f $conv, (($conv -band 1) -ne 0), $sent)
    if ($ForceChinese -and (($conv -band 1) -eq 0)) {
        [ImeDrv]::ImmSetConversionStatus($himc, ($conv -bor 1), $sent) | Out-Null
        [ImeDrv]::ImmGetConversionStatus($himc, [ref]$conv, [ref]$sent) | Out-Null
        Write-Output ("after ImmSetConversionStatus: 0x{0:X}" -f $conv)
    }
    [ImeDrv]::ImmReleaseContext($h, $himc) | Out-Null
} else {
    Write-Output "ImmGetContext returned NULL (window has no IMM context)"
}

# click inside the bottom input strip to focus the editable buffer
$cr = New-Object ImeDrv+RECT
[ImeDrv]::GetClientRect($h, [ref]$cr) | Out-Null
$org = New-Object ImeDrv+POINT
[ImeDrv]::ClientToScreen($h, [ref]$org) | Out-Null
[ImeDrv]::SetCursorPos(($org.X + 200), ($org.Y + $cr.Bottom - 15)) | Out-Null
Start-Sleep -Milliseconds 300
if ([ImeDrv]::GetForegroundWindow() -eq $h) {
    [ImeDrv]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [ImeDrv]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
}
Start-Sleep -Milliseconds 500

function SafeKey($vk) {
    if ([ImeDrv]::GetForegroundWindow() -ne $h) { Write-Output "RESULT: lost foreground, stopping key injection"; return $false }
    $scan = [byte]([ImeDrv]::MapVirtualKey([uint32]$vk, 0))
    [ImeDrv]::keybd_event($vk, $scan, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 70
    [ImeDrv]::keybd_event($vk, $scan, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    return $true
}
function Screen($name) {
    $w = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap($w.Width, $w.Height)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($w.Left, $w.Top, 0, 0, (New-Object System.Drawing.Size($w.Width, $w.Height)))
    $g.Dispose(); $bmp.Save($name, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}

function SafeCombo($mod, $vk) {
    if ([ImeDrv]::GetForegroundWindow() -ne $h) { Write-Output "RESULT: lost foreground, stopping key injection"; return $false }
    $ms = [byte]([ImeDrv]::MapVirtualKey([uint32]$mod, 0))
    $ks = [byte]([ImeDrv]::MapVirtualKey([uint32]$vk, 0))
    [ImeDrv]::keybd_event($mod, $ms, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [ImeDrv]::keybd_event($vk, $ks, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [ImeDrv]::keybd_event($vk, $ks, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [ImeDrv]::keybd_event($mod, $ms, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    return $true
}

# Toggle Microsoft Pinyin into Chinese mode. The same toggle is applied again at
# the end, so the user's original mode is restored whichever way round it started.
# SwitchKey: shift | ctrl | ctrlspace
if ($ForceChinese) {
    switch ($SwitchKey) {
        'shift' { SafeKey 0x10 | Out-Null }
        'ctrl' { SafeKey 0x11 | Out-Null }
        default { SafeCombo 0x11 0x20 | Out-Null }
    }
    Start-Sleep -Milliseconds 900
}

foreach ($c in 'N', 'I', 'H', 'A', 'O') { if (-not (SafeKey ([byte][char]$c))) { break } }
Start-Sleep -Milliseconds 1200
Screen "$shot-preedit.png"
Write-Output "saved $shot-preedit.png"

SafeKey 0x20 | Out-Null   # space = commit first candidate
Start-Sleep -Milliseconds 900
Screen "$shot-commit.png"
Write-Output "saved $shot-commit.png"

if ($ForceChinese) {
    Start-Sleep -Milliseconds 300
    switch ($SwitchKey) {
        'shift' { SafeKey 0x10 | Out-Null }
        'ctrl' { SafeKey 0x11 | Out-Null }
        default { SafeCombo 0x11 0x20 | Out-Null }
    }
}

Start-Sleep -Milliseconds 300
if (-not $p.HasExited) { $p.Kill() }
Start-Sleep -Milliseconds 400
Write-Output "--- stdout ---"
Get-Content "$shot.out" | Select-Object -First 60
