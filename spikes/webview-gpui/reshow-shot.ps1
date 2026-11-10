param([string]$Exe,[string]$Out)
$ErrorActionPreference='Stop'
. { $src = Get-Content (Join-Path $PSScriptRoot "shortpress.ps1") -Raw; $start=$src.IndexOf('Add-Type @"'); $end=$src.IndexOf('"@',$start)+2; Invoke-Expression $src.Substring($start,$end-$start) }
Add-Type -AssemblyName System.Drawing
Add-Type 'using System;using System.Runtime.InteropServices;public struct R{public int l,t,r,b;} public static class G{[DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out R r);}'
New-Item -ItemType Directory -Force $Out | Out-Null
$env:ROVE_TRANSPARENT_PREVIEW='1'; $env:ROVE_WEBVIEW_PROFILE=Join-Path $Out 'profile'
$ev=Join-Path $Out 'events.jsonl'
$p=Start-Process $Exe -PassThru -RedirectStandardOutput $ev -RedirectStandardError (Join-Path $Out 'err.log')
function C($k){ (Get-Content $ev -ErrorAction SilentlyContinue | Select-String -SimpleMatch "`"event`":`"$k`"").Count }
function W($k,$n){ for($i=0;$i -lt 600;$i++){ if((C $k) -ge $n){return}; Start-Sleep -Milliseconds 100 }; throw "timeout $k" }
function Shot($name){ $r=New-Object R; [G]::GetWindowRect($h,[ref]$r)|Out-Null; $b=New-Object Drawing.Bitmap ($r.r-$r.l),($r.b-$r.t); $g=[Drawing.Graphics]::FromImage($b); $g.CopyFromScreen($r.l,$r.t,0,0,$b.Size); $b.Save((Join-Path $Out "$name.png")); $g.Dispose(); $b.Dispose() }
try {
 W 'first_render' 1; $p.Refresh(); $h=$p.MainWindowHandle; [K]::Activate($h)|Out-Null; Start-Sleep -Milliseconds 400
 function Guard(){ if([K]::GetForegroundWindow() -ne $h){ throw 'foreground lost; abort' } }
 Guard; [K]::Tap(0x4F)|Out-Null; W 'ready' 1; Start-Sleep -Milliseconds 800; Shot '1-open'
 Guard; [K]::Tap(0x58)|Out-Null; Start-Sleep -Milliseconds 600; Shot '2-dismissed'
 Guard; [K]::Tap(0x4F)|Out-Null; Start-Sleep -Milliseconds 300; Shot '3-reshown-300ms'
} finally { if(-not $p.HasExited){$p.Kill()} }
