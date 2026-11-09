# Rove M0 UI spike - measurement harness.
#   powershell -File measure.ps1 -Exe <path-to-exe> -File <path-to-testfile> -Label <name> [-Runs 5] [-Step 3] [-Frames 300]
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$File,
    [Parameter(Mandatory = $true)][string]$Label,
    [int]$Runs = 5,
    [double]$Step = 3,
    [int]$Frames = 300,
    [int]$MemWaitSec = 8
)

$tmp = Join-Path $env:TEMP "rove-spike"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$out = Join-Path $tmp "$Label.out"
$err = Join-Path $tmp "$Label.err"

Write-Output "===== $Label ====="
Write-Output "exe:  $Exe"
Write-Output "file: $File  ($([math]::Round((Get-Item $File).Length / 1MB, 1)) MB)"

$spawnToFirstFrame = @()
$scrollAvg = @()
$scrollP99 = @()
$scrollMax = @()
$cpuAvg = @()
$cpuP99 = @()
$openMs = @()
$fullIdx = @()

for ($i = 1; $i -le $Runs; $i++) {
    $t0 = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $p = Start-Process -FilePath $Exe -ArgumentList $File, '--bench-scroll', $Frames, '--bench-step', $Step `
        -PassThru -RedirectStandardOutput $out -RedirectStandardError $err -NoNewWindow
    $p.WaitForExit(120000) | Out-Null
    if (-not $p.HasExited) { $p.Kill(); Write-Output "  run $i TIMEOUT"; continue }
    $text = Get-Content $out -Raw
    if ($text -match 'FIRSTFRAME_EPOCH_MS (\d+)') { $spawnToFirstFrame += ([int64]$Matches[1] - $t0) }
    if ($text -match 'SCROLL_INTERVAL_MS avg=([\d.]+) p99=([\d.]+) max=([\d.]+)') {
        $scrollAvg += [double]$Matches[1]; $scrollP99 += [double]$Matches[2]; $scrollMax += [double]$Matches[3]
    }
    if ($text -match 'SCROLL_CPU_US avg=(\d+) p99=(\d+)') { $cpuAvg += [double]$Matches[1]; $cpuP99 += [double]$Matches[2] }
    if ($text -match 'DOC_OPEN_MS ([\d.]+)') { $openMs += [double]$Matches[1] }
    if ($text -match 'FULL_INDEX_MS (\d+)') { $fullIdx += [double]$Matches[1] }
    Write-Output "  run $i ok"
}

function Show($name, $arr, $unit) {
    if ($arr.Count -eq 0) { Write-Output ("{0,-34}: not measured" -f $name); return }
    $min = ($arr | Measure-Object -Minimum).Minimum
    $med = ($arr | Sort-Object)[[int]([math]::Floor($arr.Count / 2))]
    $max = ($arr | Measure-Object -Maximum).Maximum
    Write-Output ("{0,-34}: min={1} median={2} max={3} {4}  (n={5})" -f $name, [math]::Round($min, 2), [math]::Round($med, 2), [math]::Round($max, 2), $unit, $arr.Count)
}

Show "spawn -> first frame" $spawnToFirstFrame "ms"
Show "Doc::open (mmap+head index)" $openMs "ms"
Show "full line index" $fullIdx "ms"
Show "scroll frame interval avg" $scrollAvg "ms"
Show "scroll frame interval p99" $scrollP99 "ms"
Show "scroll frame interval max" $scrollMax "ms"
Show "scroll CPU per frame avg" $cpuAvg "us"
Show "scroll CPU per frame p99" $cpuP99 "us"

# --- idle memory -------------------------------------------------------------
$p = Start-Process -FilePath $Exe -ArgumentList $File -PassThru -RedirectStandardOutput "$out.mem" -RedirectStandardError "$err.mem" -NoNewWindow
$deadline = (Get-Date).AddSeconds($MemWaitSec)
$ws = 0; $priv = 0
while ((Get-Date) -lt $deadline -and -not $p.HasExited) {
    Start-Sleep -Milliseconds 500
    try { $p.Refresh(); $ws = $p.WorkingSet64; $priv = $p.PrivateMemorySize64 } catch {}
}
if (-not $p.HasExited) { $p.Kill() }
Write-Output ("{0,-34}: working set {1} MB / private {2} MB (after {3}s idle)" -f "idle memory", [math]::Round($ws / 1MB, 1), [math]::Round($priv / 1MB, 1), $MemWaitSec)
Write-Output ("{0,-34}: {1} KB" -f "release binary", [math]::Round((Get-Item $Exe).Length / 1KB, 0))
Write-Output ""
