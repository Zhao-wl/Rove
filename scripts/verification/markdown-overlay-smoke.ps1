# Run only on an isolated GitHub-hosted Windows runner. The executable must be built
# with cargo build -p rove-ui --example markdown_probe.
[CmdletBinding()]
param(
    [string]$Executable = 'target/debug/examples/markdown_probe.exe',
    [string]$OutputDirectory = 'target/markdown-overlay-smoke'
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows') {
    throw 'The native pointer smoke requires an isolated GitHub-hosted Windows runner.'
}

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class OverlaySmokeWin32
{
    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("dwmapi.dll")]
    public static extern int DwmFlush();
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
    [DllImport("user32.dll")]
    public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    public static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")]
    public static extern bool IsChild(IntPtr parent, IntPtr child);
    [DllImport("user32.dll")]
    public static extern IntPtr GetParent(IntPtr hwnd);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")]
    public static extern int GetWindowRgn(IntPtr hwnd, IntPtr region);
    [DllImport("user32.dll")]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr hwnd, uint message, UIntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, uint x, uint y, int data, UIntPtr extra);
    [DllImport("gdi32.dll")]
    public static extern IntPtr CreateRectRgn(int left, int top, int right, int bottom);
    [DllImport("gdi32.dll")]
    public static extern bool PtInRegion(IntPtr region, int x, int y);
    [DllImport("gdi32.dll")]
    public static extern bool DeleteObject(IntPtr handle);

    public static void Key(byte key, bool control)
    {
        if (control) keybd_event(0x11, 0, 0, UIntPtr.Zero);
        keybd_event(key, 0, 0, UIntPtr.Zero);
        keybd_event(key, 0, 2, UIntPtr.Zero);
        if (control) keybd_event(0x11, 0, 2, UIntPtr.Zero);
    }

    public static void Click()
    {
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }

    public static void ScrollDown()
    {
        mouse_event(0x0800, 0, 0, -600, UIntPtr.Zero);
    }
}
'@
$previousDpi = [OverlaySmokeWin32]::SetThreadDpiAwarenessContext([IntPtr](-4))
if ($previousDpi -eq [IntPtr]::Zero) { throw 'Cannot enable per-monitor DPI for native hit testing.' }

$executablePath = (Resolve-Path -LiteralPath $Executable).Path
[void](New-Item -ItemType Directory -Force -Path $OutputDirectory)
$outputPath = (Resolve-Path -LiteralPath $OutputDirectory).Path
$stdout = Join-Path $outputPath 'probe.stdout.log'
$stderr = Join-Path $outputPath 'probe.stderr.log'
$env:ROVE_MARKDOWN_HOSTED_OVERLAY_PROBE = '1'
$probe = Start-Process -FilePath $executablePath -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr

function Wait-ProbeState([string]$Description, [scriptblock]$Ready, [int]$Seconds = 8) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    $last = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($probe.HasExited) {
            throw "markdown_probe exited before $Description (exit $($probe.ExitCode)). See $stderr."
        }
        $line = Get-Content -LiteralPath $stdout -Tail 20 |
            Where-Object { $_.StartsWith('MARKDOWN_OVERLAY {') } |
            Select-Object -Last 1
        if ($line) {
            try { $last = $line.Substring('MARKDOWN_OVERLAY '.Length) | ConvertFrom-Json }
            catch { Start-Sleep -Milliseconds 100; continue }
            if (& $Ready $last) { return $last }
        }
        Start-Sleep -Milliseconds 150
    }
    throw "Timed out waiting for $Description. Last state: $($last | ConvertTo-Json -Compress -Depth 5)"
}

function Wait-SettledOverlay($State, [string]$Kind) {
    $fresh = Wait-ProbeState "$Kind next presented sample" { param($s)
        $s.sample -gt $State.sample -and $s.layout.$Kind -and $s.occlusions -gt 0
    }
    $bounds = $fresh.layout.$Kind -join ','
    Wait-ProbeState "$Kind stable second sample" { param($s)
        $s.sample -gt $fresh.sample -and $s.occlusions -gt 0 -and
        ($s.layout.$Kind -join ',') -ceq $bounds
    }
}

function Assert-Foreground {
    [uint32]$windowProcess = 0
    [void][OverlaySmokeWin32]::GetWindowThreadProcessId($hwnd, [ref]$windowProcess)
    if ($windowProcess -ne $probe.Id -or [OverlaySmokeWin32]::GetForegroundWindow() -ne $hwnd) {
        throw 'The probe does not own the active desktop window. Refusing to inject input.'
    }
}

function Convert-ScreenPoint($State, [double]$X, [double]$Y) {
    $point = [OverlaySmokeWin32+Point]::new()
    $point.X = [int][Math]::Round($X * $State.layout.scale)
    $point.Y = [int][Math]::Round($Y * $State.layout.scale)
    if (![OverlaySmokeWin32]::ClientToScreen($hwnd, [ref]$point)) {
        throw 'Could not convert probe coordinates to the interactive desktop.'
    }
    return $point
}

function Get-BodyPoint($State) {
    $b = $State.layout.preview
    Convert-ScreenPoint $State (0.30 * $b[0] + 0.70 * $b[2]) (0.20 * $b[1] + 0.80 * $b[3])
}

function Move-Pointer($Point) {
    Assert-Foreground
    $target = [OverlaySmokeWin32]::WindowFromPoint($Point)
    if ($target -ne $hwnd -and ![OverlaySmokeWin32]::IsChild($hwnd, $target)) {
        throw "Input point $($Point.X),$($Point.Y) is outside the probe window (HWND $target)."
    }
    if (![OverlaySmokeWin32]::SetCursorPos($Point.X, $Point.Y)) {
        throw 'Could not move the pointer on the interactive runner desktop.'
    }
}

function Click-Point($Point) {
    Move-Pointer $Point
    Assert-Foreground
    [OverlaySmokeWin32]::Click()
}

function Assert-BodyHit($Point) {
    $hit = [OverlaySmokeWin32]::WindowFromPoint($Point)
    if (![OverlaySmokeWin32]::IsChild($hwnd, $hit)) {
        throw "Uncovered preview did not hit a native child (point $($Point.X),$($Point.Y), HWND $hit)."
    }
    return $hit
}

function Save-Screenshot([string]$Name) {
    Assert-Foreground
    $result = [OverlaySmokeWin32]::DwmFlush()
    if ($result -lt 0) { throw "DwmFlush failed with HRESULT $result." }
    $rect = [OverlaySmokeWin32+Rect]::new()
    if (![OverlaySmokeWin32]::GetWindowRect($hwnd, [ref]$rect)) { throw 'Cannot get probe bounds.' }
    $bitmap = [System.Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    try {
        $display = [System.Drawing.Graphics]::FromImage($bitmap)
        try { $display.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size) }
        finally { $display.Dispose() }
        $bitmap.Save((Join-Path $outputPath "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
        if ($Name -in @('quick-open', 'quick-open-typed', 'settings-menu', 'maximize-tooltip')) {
            $width = [Math]::Min(640, $bitmap.Width)
            $height = [int][Math]::Round($bitmap.Height * $width / $bitmap.Width)
            $thumbnail = [System.Drawing.Bitmap]::new($bitmap, $width, $height)
            try {
                $encoded = [System.IO.MemoryStream]::new()
                try {
                    $thumbnail.Save($encoded, [System.Drawing.Imaging.ImageFormat]::Jpeg)
                    Write-Host "MARKDOWN_OVERLAY_SCREENSHOT $Name $([Convert]::ToBase64String($encoded.ToArray()))"
                }
                finally { $encoded.Dispose() }
            }
            finally { $thumbnail.Dispose() }
        }
    }
    finally { $bitmap.Dispose() }
}

function Assert-Overlay($State, $Overlay, [string]$Name) {
    if (!$State.visible -or !$State.rendered -or $State.occlusions -lt 1) {
        throw "$Name did not retain visible WebView2 with applied clipping."
    }
    $b = $State.layout.preview
    $left = [Math]::Max($Overlay[0], $b[0])
    $top = [Math]::Max($Overlay[1], $b[1])
    $right = [Math]::Min($Overlay[2], $b[2])
    $bottom = [Math]::Min($Overlay[3], $b[3])
    if ($right -le $left -or $bottom -le $top) { throw "$Name does not overlap the preview." }
    $cut = Convert-ScreenPoint $State (($left + $right) / 2) (($top + $bottom) / 2)
    if ([OverlaySmokeWin32]::WindowFromPoint($cut) -ne $hwnd) {
        throw "$Name's overlap does not hit the GPUI host."
    }

    $body = Get-BodyPoint $State
    $child = Assert-BodyHit $body
    while (($parent = [OverlaySmokeWin32]::GetParent($child)) -ne $hwnd) {
        if ($parent -eq [IntPtr]::Zero) { throw 'Could not locate the WebView2 container.' }
        $child = $parent
    }
    $rect = [OverlaySmokeWin32+Rect]::new()
    if (![OverlaySmokeWin32]::GetWindowRect($child, [ref]$rect)) { throw 'Cannot get WebView2 bounds.' }
    $region = [OverlaySmokeWin32]::CreateRectRgn(0, 0, 0, 0)
    if ($region -eq [IntPtr]::Zero) { throw 'Could not create the validation region.' }
    try {
        if ([OverlaySmokeWin32]::GetWindowRgn($child, $region) -eq 0 -or
            [OverlaySmokeWin32]::PtInRegion($region, $cut.X - $rect.Left, $cut.Y - $rect.Top) -or
            ![OverlaySmokeWin32]::PtInRegion($region, $body.X - $rect.Left, $body.Y - $rect.Top)) {
            throw "$Name native clip region does not match covered and uncovered hit points."
        }
    }
    finally { [void][OverlaySmokeWin32]::DeleteObject($region) }
    Save-Screenshot $Name
    Write-Host "$($Name): native HRGN and Win32 hit points passed"
    return $cut
}

function Assert-Restored($State, [string]$Name, $PreviouslyCoveredPoint) {
    if ($State.pid -ne $baseline.pid -or $null -eq $State.position -or
        [Math]::Abs($State.position - $baseline.position) -gt 1 -or $State.occlusions -ne 0) {
        throw "$Name lost the preview process or reading position: $($State | ConvertTo-Json -Compress -Depth 5)"
    }
    [void](Assert-BodyHit (Get-BodyPoint $State))
    [void](Assert-BodyHit $PreviouslyCoveredPoint)
    Save-Screenshot "$Name-restored"
    Write-Host "$Name restored source line $($State.position), PID $($State.pid)"
}

try {
    $initial = Wait-ProbeState 'actual WebView2 render' { param($s)
        $s.rendered -and $s.pid -and $s.layout.hwnd -and $s.layout.gear -and
        $s.layout.maximize -and $s.layout.preview -and $null -ne $s.position
    } 35
    $hwnd = [IntPtr][Int64]$initial.layout.hwnd
    Assert-Foreground
    $body = Get-BodyPoint $initial
    [void](Assert-BodyHit $body)
    Click-Point $body
    Assert-Foreground
    [OverlaySmokeWin32]::ScrollDown()
    [void](Wait-ProbeState 'native wheel scroll' { param($s) $s.position -gt ($initial.position + 3) })
    Start-Sleep -Milliseconds 750
    $baseline = Wait-ProbeState 'stable reading position' { param($s) $s.position -gt 3 }
    Save-Screenshot 'baseline'

    Assert-Foreground
    [OverlaySmokeWin32]::Key(0x50, $true)
    $quick = Wait-ProbeState 'Ctrl+P' { param($s) $s.layout.quick -and $s.occlusions -gt 0 }
    $quick = Wait-SettledOverlay $quick 'quick'
    [void](Assert-Overlay $quick $quick.layout.quick 'quick-open')
    Assert-Foreground
    Set-Clipboard -Value 'markdown-overlay-clipboard-sentinel'
    [OverlaySmokeWin32]::Key(0x4C, $false)
    Start-Sleep -Milliseconds 250
    [OverlaySmokeWin32]::Key(0x41, $true)
    [OverlaySmokeWin32]::Key(0x43, $true)
    $copyDeadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        Start-Sleep -Milliseconds 150
        $typed = (Get-Clipboard -Raw).Trim()
    } until ($typed -ieq 'l' -or [DateTime]::UtcNow -ge $copyDeadline)
    if ($typed -ine 'l') { throw "Quick input did not accept and copy the typed letter: '$typed'." }
    Write-Host 'quick-open: typed letter copied from the real focus target'
    $typedState = Wait-SettledOverlay $quick 'quick'
    $quickCovered = Assert-Overlay $typedState $typedState.layout.quick 'quick-open-typed'
    Assert-Foreground
    [OverlaySmokeWin32]::Key(0x1B, $false)
    $closed = Wait-ProbeState 'Escape closing quick input' { param($s)
        $s.occlusions -eq 0 -and $null -ne $s.position
    }
    Assert-Restored $closed 'quick-open' $quickCovered

    Assert-Foreground
    [OverlaySmokeWin32]::Key(0x42, $true)
    [void](Wait-ProbeState 'side bar closed' { param($s)
        $s.layout.preview[0] -lt ($initial.layout.preview[0] - 20)
    })
    Start-Sleep -Milliseconds 500
    $baseline = Wait-ProbeState 'post-resize reading position' { param($s) $null -ne $s.position }
    $gear = $baseline.layout.gear
    Click-Point (Convert-ScreenPoint $baseline (($gear[0] + $gear[2]) / 2) (($gear[1] + $gear[3]) / 2))
    $menu = Wait-ProbeState 'settings gear' { param($s) $s.layout.menu -and $s.occlusions -gt 0 }
    $menu = Wait-SettledOverlay $menu 'menu'
    $menuCovered = Assert-Overlay $menu $menu.layout.menu 'settings-menu'
    $item = $menu.layout.menu_command
    if (!$item) { throw 'The command-palette item in the settings menu was not rendered.' }
    Click-Point (Convert-ScreenPoint $menu (($item[0] + $item[2]) / 2) (($item[1] + $item[3]) / 2))
    $menuCommand = Wait-ProbeState 'settings item opening the command palette' { param($s)
        $s.sample -gt $menu.sample -and $s.layout.quick -and $s.occlusions -gt 0 -and
        [OverlaySmokeWin32]::IsChild($hwnd, [OverlaySmokeWin32]::WindowFromPoint($menuCovered))
    }
    $menuCommand = Wait-SettledOverlay $menuCommand 'quick'
    [void](Assert-Overlay $menuCommand $menuCommand.layout.quick 'settings-command-palette')
    Assert-Foreground
    [OverlaySmokeWin32]::Key(0x1B, $false)
    $closed = Wait-ProbeState 'settings command closed' { param($s)
        $s.occlusions -eq 0 -and $null -ne $s.position
    }
    Assert-Restored $closed 'settings-command' $menuCovered
    Write-Host 'settings-menu: real menu item opened the command palette'

    Click-Point (Convert-ScreenPoint $closed (($gear[0] + $gear[2]) / 2) (($gear[1] + $gear[3]) / 2))
    $menu = Wait-ProbeState 'settings gear reopened' { param($s)
        $s.layout.menu -and $s.occlusions -gt 0
    }
    Click-Point (Get-BodyPoint $menu)
    $closed = Wait-ProbeState 'native preview click closing settings menu' { param($s)
        $s.occlusions -eq 0 -and $null -ne $s.position
    }
    Assert-Restored $closed 'settings-menu' $menuCovered

    $max = $closed.layout.maximize
    Move-Pointer (Convert-ScreenPoint $closed (($max[0] + $max[2]) / 2) ($max[3] - 2))
    $tip = Wait-ProbeState 'maximize tooltip' { param($s) $s.layout.tooltip -and $s.occlusions -gt 0 }
    $tip = Wait-SettledOverlay $tip 'tooltip'
    $tipCovered = Assert-Overlay $tip $tip.layout.tooltip 'maximize-tooltip'
    Move-Pointer (Get-BodyPoint $tip)
    $closed = Wait-ProbeState 'native mouse entry clearing tooltip' { param($s)
        !$s.layout.tooltip -and $s.occlusions -eq 0 -and $null -ne $s.position
    }
    Assert-Restored $closed 'maximize-tooltip' $tipCovered
    Write-Host 'MARKDOWN_HOSTED_OVERLAY_SMOKE_PASS'
}
finally {
    if (!$probe.HasExited) {
        if ($hwnd) {
            [uint32]$windowProcess = 0
            [void][OverlaySmokeWin32]::GetWindowThreadProcessId($hwnd, [ref]$windowProcess)
            if ($windowProcess -eq $probe.Id) {
                [void][OverlaySmokeWin32]::PostMessage($hwnd, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
                [void]$probe.WaitForExit(5000)
            }
        }
        if (!$probe.HasExited) { Stop-Process -Id $probe.Id }
    }
    [void][OverlaySmokeWin32]::SetThreadDpiAwarenessContext($previousDpi)
    Remove-Item Env:ROVE_MARKDOWN_HOSTED_OVERLAY_PROBE -ErrorAction SilentlyContinue
}
