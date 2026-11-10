param([ValidateRange(1,10)][int]$Runs=3)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$exe=Join-Path $root 'target/release/webview-gpui.exe'
$runRoot=Join-Path $root ('runs/review-'+(Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory $runRoot | Out-Null
$hash=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
$oldProfile=$env:ROVE_WEBVIEW_PROFILE
try {
 for($i=1;$i -le $Runs;$i++) {
  $env:ROVE_WEBVIEW_PROFILE=Join-Path $runRoot "profile-$i"
  $out=Join-Path $runRoot "auto-$i.jsonl"
  $err=Join-Path $runRoot "auto-$i.stderr.log"
  $started=[DateTime]::UtcNow.ToString('o')
  $p=Start-Process -FilePath $exe -ArgumentList '--auto' -WindowStyle Normal -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
  if(-not $p.WaitForExit(90000)) { $p.Kill(); throw "auto-$i timed out" }
  $p.Refresh()
  [pscustomobject]@{trial="auto-$i";startedUtc=$started;exeSha256=$hash;exitCode=$p.ExitCode;mode='fresh-profile continuous-frame auto probe';profile=$env:ROVE_WEBVIEW_PROFILE} | ConvertTo-Json | Set-Content (Join-Path $runRoot "auto-$i.run.json")
  & python (Join-Path $root 'verify_run.py') $out
  if($LASTEXITCODE -ne 0){throw "auto-$i runtime verification failed; evidence retained at $runRoot"}
 }
} finally {$env:ROVE_WEBVIEW_PROFILE=$oldProfile}
& python (Join-Path $root 'summarize.py') $runRoot | Set-Content (Join-Path $runRoot 'summary.json')
if($LASTEXITCODE -ne 0){throw 'Summarization failed'}
Write-Output $runRoot
