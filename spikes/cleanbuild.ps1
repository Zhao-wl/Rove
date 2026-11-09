# Times a from-scratch release build of each prototype (dependencies already in
# the local cargo registry cache, i.e. no network download time included).
param([Parameter(Mandatory=$true)][string]$Dir)
$vc = "D:\Tools\Visual Studio\2026\VC\Auxiliary\Build\vcvars64.bat"
Push-Location $Dir
cargo clean | Out-Null
$sw = [System.Diagnostics.Stopwatch]::StartNew()
cmd /c "`"$vc`" >nul 2>&1 && cargo build --release 2>&1" | Out-Null
$sw.Stop()
Pop-Location
Write-Output ("CLEAN_BUILD {0} : {1:N1} s" -f (Split-Path $Dir -Leaf), $sw.Elapsed.TotalSeconds)
