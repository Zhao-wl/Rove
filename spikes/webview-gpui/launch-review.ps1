param([ValidateSet('Baseline','Hwnd','Transparent')][string]$Mode='Hwnd')
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$run=Join-Path $root ('runs/manual-'+$Mode+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory $run | Out-Null
$oldDcomp=$env:GPUI_DISABLE_DIRECT_COMPOSITION
$oldTransparent=$env:ROVE_TRANSPARENT_PREVIEW
try {
 $env:GPUI_DISABLE_DIRECT_COMPOSITION=$null
 $env:ROVE_TRANSPARENT_PREVIEW=$null
 if($Mode -eq 'Hwnd'){$env:GPUI_DISABLE_DIRECT_COMPOSITION='1'}
 if($Mode -eq 'Transparent'){$env:ROVE_TRANSPARENT_PREVIEW='1'}
 $exe=Join-Path $root 'target/release/webview-gpui.exe'
 [pscustomobject]@{mode=$Mode;exeSha256=(Get-FileHash $exe).Hash;startedUtc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json | Set-Content (Join-Path $run 'run.json')
 Start-Process -FilePath $exe -WindowStyle Normal -RedirectStandardOutput (Join-Path $run 'events.jsonl') -RedirectStandardError (Join-Path $run 'stderr.log') | Out-Null
} finally {
 $env:GPUI_DISABLE_DIRECT_COMPOSITION=$oldDcomp
 $env:ROVE_TRANSPARENT_PREVIEW=$oldTransparent
}
Write-Output $run
