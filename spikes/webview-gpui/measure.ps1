param([int]$Runs = 3)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$exe = Join-Path $root 'target/release/webview-gpui.exe'
$runRoot = Join-Path $root ('runs/' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory $runRoot -Force | Out-Null
$previousProfile = $env:ROVE_WEBVIEW_PROFILE
try {
  foreach ($mode in @('fresh-profile','reused-profile')) {
    for ($i=1; $i -le $Runs; $i++) {
      $name = "$mode-$i"
      $profile = if ($mode -eq 'fresh-profile') { Join-Path $runRoot "profile-$i" } else { Join-Path $runRoot 'profile-1' }
      $env:ROVE_WEBVIEW_PROFILE = $profile
      $out = Join-Path $runRoot "$name.jsonl"
      $err = Join-Path $runRoot "$name.stderr.log"
      $p = Start-Process -FilePath $exe -ArgumentList '--auto' -PassThru -WindowStyle Normal -RedirectStandardOutput $out -RedirectStandardError $err
      $watch = [Diagnostics.Stopwatch]::StartNew()
      $samples = [Collections.Generic.List[object]]::new()
      while (-not $p.HasExited -and $watch.Elapsed.TotalSeconds -lt 45) {
        # Build the descendant tree, rather than count unrelated WebView2 apps.
        $processes = @(Get-CimInstance Win32_Process)
        $ids = [Collections.Generic.HashSet[int]]::new()
        [void]$ids.Add($p.Id)
        do {
          $added = $false
          foreach ($entry in $processes) {
            if ($ids.Contains([int]$entry.ParentProcessId) -and $ids.Add([int]$entry.ProcessId)) { $added=$true }
          }
        } while ($added)
        $live = @(Get-Process -Id @($ids) -ErrorAction SilentlyContinue)
        $samples.Add([pscustomobject]@{seconds=$watch.Elapsed.TotalSeconds;processCount=$live.Count;workingSetBytes=($live|Measure-Object WorkingSet64 -Sum).Sum;privateBytes=($live|Measure-Object PrivateMemorySize64 -Sum).Sum;ids=@($ids)})
        Start-Sleep -Milliseconds 300
        $p.Refresh()
      }
      if (-not $p.HasExited) { $p.Kill(); throw "Prototype exceeded timeout: $name" }
      $samples | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $runRoot "$name.memory.json")
      [pscustomobject]@{name=$name;exitCode=$p.ExitCode;seconds=$watch.Elapsed.TotalSeconds;profile=$profile} | ConvertTo-Json | Set-Content (Join-Path $runRoot "$name.run.json")
      Start-Sleep -Seconds 2
    }
  }
} finally { $env:ROVE_WEBVIEW_PROFILE = $previousProfile }
Write-Output $runRoot
