param([string]$Exe, [ValidateSet('Hwnd','Transparent')][string]$Mode='Transparent', [int]$Trials=30, [int]$GapMs=200, [string]$Out, [switch]$Prep)
$ErrorActionPreference='Stop'
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Diagnostics; using System.Threading;
public static class K {
  [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk; public ushort scan; public uint flags; public uint time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit, Size=40)] struct IN { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KI ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, IN[] i, int cb);
  [DllImport("user32.dll")] static extern uint MapVirtualKey(uint c, uint t);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr pid);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("user32.dll")] static extern bool BringWindowToTop(IntPtr h);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool r);
  // Foreground lock: borrow the current foreground thread's input state only for the switch.
  public static bool Activate(IntPtr h){ uint fg=GetWindowThreadProcessId(GetForegroundWindow(),IntPtr.Zero); uint me=GetCurrentThreadId(); bool att= fg!=me && AttachThreadInput(me,fg,true); BringWindowToTop(h); bool ok=SetForegroundWindow(h); if(att) AttachThreadInput(me,fg,false); return ok; }
  public static void Key(ushort vk, bool up){ var a=new IN[1]; a[0].type=1; a[0].ki.vk=vk; a[0].ki.scan=(ushort)MapVirtualKey(vk,0); a[0].ki.flags= up?2u:0u; if(SendInput(1,a,Marshal.SizeOf(typeof(IN)))!=1) throw new Exception("SendInput failed"); }
  public static double Tap(ushort vk){ var sw=Stopwatch.StartNew(); Key(vk,false); Key(vk,true); return sw.Elapsed.TotalMilliseconds; }
  public static void SpinUntil(Stopwatch sw, double ms){ while(sw.Elapsed.TotalMilliseconds < ms - 2) Thread.Sleep(1); while(sw.Elapsed.TotalMilliseconds < ms) {} }
}
"@
New-Item -ItemType Directory -Force $Out | Out-Null
$events = Join-Path $Out 'events.jsonl'
$env:GPUI_DISABLE_DIRECT_COMPOSITION=$null; $env:ROVE_TRANSPARENT_PREVIEW=$null
if($Mode -eq 'Hwnd'){$env:GPUI_DISABLE_DIRECT_COMPOSITION='1'} else {$env:ROVE_TRANSPARENT_PREVIEW='1'}
$env:ROVE_WEBVIEW_PROFILE = Join-Path $Out 'profile'
$p = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $events -RedirectStandardError (Join-Path $Out 'stderr.log')
function Count($kind){ (Get-Content $events -ErrorAction SilentlyContinue | Select-String -SimpleMatch "`"event`":`"$kind`"").Count }
function WaitCount($kind, $n, $ms){ $sw=[Diagnostics.Stopwatch]::StartNew(); while($sw.ElapsedMilliseconds -lt $ms){ if((Count $kind) -ge $n){return $true}; Start-Sleep -Milliseconds 50 }; return $false }
try {
  if(-not (WaitCount 'first_render' 1 60000)){ throw 'no first_render' }
  $p.Refresh(); $h = $p.MainWindowHandle
  [K]::Activate($h) | Out-Null; Start-Sleep -Milliseconds 300
  function Guard(){ $fg=[K]::GetForegroundWindow(); if($fg -ne $h){ throw "foreground is $fg, not spike $h; aborting injection" } }
  Guard; [K]::Tap(0x4F) | Out-Null
  if(-not (WaitCount 'ready' 1 20000)){ throw 'initial ready missing' }
  if($Prep){ [K]::MoveWindow($h, 200, 120, 1300, 860, $true) | Out-Null; Start-Sleep -Milliseconds 500; Guard; [K]::Tap(0x54) | Out-Null; Start-Sleep -Milliseconds 500 }
  $results=@()
  for($i=1;$i -le $Trials;$i++){
    $ready = Count 'ready'; $opens = (Count 'open_requested') + (Count 'reshown'); $reshown = Count 'reshown'
    Start-Sleep -Milliseconds 400
    Guard
    $sw=[Diagnostics.Stopwatch]::StartNew()
    $xh=[K]::Tap(0x58)
    [K]::SpinUntil($sw, $GapMs)
    $fgBeforeO=[K]::GetForegroundWindow()
    if($fgBeforeO -ne $h){ throw "foreground changed before o: $fgBeforeO" }
    $oAt=$sw.Elapsed.TotalMilliseconds; $oh=[K]::Tap(0x4F)
    $sw2=[Diagnostics.Stopwatch]::StartNew(); while($sw2.ElapsedMilliseconds -lt 3000 -and ((Count 'open_requested')+(Count 'reshown')) -le $opens){ Start-Sleep -Milliseconds 20 }
    $ok = ((Count 'open_requested')+(Count 'reshown')) -gt $opens
    $wasReshown = (Count 'reshown') -gt $reshown
    $readyOk = $wasReshown -or (WaitCount 'ready' ($ready+1) 25000)
    $r=[pscustomobject]@{trial=$i; ok=$ok; gapMs=[math]::Round($oAt,2); xHold=[math]::Round($xh,3); oHold=[math]::Round($oh,3); ready=$readyOk; reshown=$wasReshown}
    $results+=$r; $r | ConvertTo-Json -Compress | Add-Content (Join-Path $Out 'trials.jsonl')
    if(-not $readyOk){ Guard; [K]::Tap(0x4F) | Out-Null; if(-not (WaitCount 'ready' ($ready+1) 25000)){ throw "recovery failed at trial $i" } }
  }
  "{0}: {1}/{2} ok" -f $Mode, ($results|?{$_.ok}).Count, $Trials
} finally { if(-not $p.HasExited){ $p.Kill() } }
