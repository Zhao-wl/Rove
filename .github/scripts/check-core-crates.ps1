<#
.SYNOPSIS
  核心层禁止依赖 gpui（issue #3）。

.DESCRIPTION
  - 允许依赖 gpui 的名单： .github/ui-crates.txt（rove-ui / rove-app）
  - 核心层名册： .github/core-crates.txt

  判定采用**默认拒绝**：`cargo metadata` 里除 `ui-crates.txt` 之外的所有 workspace 成员
  都按核心层检查，所以 A 链新增 crate 不需要任何改动就会被覆盖；`core-crates.txt`
  只用于名册与 `::warning::` 提示，漏登记不会让新 crate 逃过检查。
  新增 crate 后请把它追加到 `core-crates.txt`。

  对每个核心层 crate 执行：

      cargo tree --package <crate> --invert gpui --all-features --target all

  - `--all-features`：否则 `optional = true` + feature 的写法会让 cargo 报
    "did not match any packages"，检查会静默放过；
  - `--target all`：否则平台条件依赖（`[target.'cfg(unix)'.dependencies]`）打印空树。
  - `dev-dependencies` 也计入：核心层的测试同样不应该依赖 gpui。

  输出非空即失败。为了让 Windows PowerShell 5.1 在中文代码页（CP936）下也能工作，
  所有原生命令的输出都重定向到临时文件并按 UTF-8 读取（CP936 会把 cargo 的 UTF-8
  JSON 解码坏，`ConvertFrom-Json` 直接抛异常）。

.PARAMETER Update
  把未登记的核心层 crate 追加到 .github/core-crates.txt。

.PARAMETER Strict
  名单与 workspace 不一致时直接失败（维护者本地用，CI 默认只警告）。
#>
[CmdletBinding()]
param(
    [switch]$Update,
    [switch]$Strict
)

$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 在中文代码页下会把原生命令的 UTF-8 输出按 CP936 解码；
# 统一切成 UTF-8，退出时还原（git rev-parse 之类的短输出也依赖它）。
$originalOutputEncoding = [Console]::OutputEncoding
try { [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) } catch { }

$tempFiles = New-Object System.Collections.Generic.List[string]

function New-TempFile([string]$Prefix) {
    $path = Join-Path ([System.IO.Path]::GetTempPath()) ("{0}-{1}.tmp" -f $Prefix, [guid]::NewGuid())
    [void]$tempFiles.Add($path)
    return $path
}

# 用 Start-Process 把 stdout/stderr 分别重定向到文件，避免两件事：
# 1) PS 5.1 把原生命令的 stderr 当成 ErrorRecord，在 Stop 下直接抛出、拿不到退出码；
# 2) 重新解析 Cargo.lock 时几百行 "Locking/Adding" 混进违规详情。
function Invoke-Captured([string]$Exe, [string[]]$Arguments) {
    $stdoutFile = New-TempFile 'rove-cap-out'
    $stderrFile = New-TempFile 'rove-cap-err'
    $proc = Start-Process -FilePath $Exe -ArgumentList $Arguments -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $stdoutFile -RedirectStandardError $stderrFile
    $stdout = if (Test-Path -LiteralPath $stdoutFile) { Get-Content -LiteralPath $stdoutFile -Encoding UTF8 -Raw } else { '' }
    $stderr = if (Test-Path -LiteralPath $stderrFile) { Get-Content -LiteralPath $stderrFile -Encoding UTF8 -Raw } else { '' }
    return [pscustomobject]@{
        ExitCode = $proc.ExitCode
        StdOut   = $stdout
        StdErr   = $stderr
    }
}

$repoRoot = (& git rev-parse --show-toplevel)
if ($LASTEXITCODE -ne 0 -or -not $repoRoot) {
    throw '必须在 git 仓库内运行。'
}
Push-Location $repoRoot
try {
    function Read-CrateList([string]$Path) {
        if (-not (Test-Path -LiteralPath $Path)) { return @() }
        # 显式 UTF-8：Windows PowerShell 5.1 默认按 ANSI 读取，会破坏中文注释与行边界。
        return @(
            Get-Content -LiteralPath $Path -Encoding UTF8 |
                ForEach-Object { $_.Trim() } |
                Where-Object { $_ -and -not $_.StartsWith('#') }
        )
    }

    $uiCrates = Read-CrateList '.github/ui-crates.txt'
    $listedCore = Read-CrateList '.github/core-crates.txt'

    $metadata = Invoke-Captured 'cargo' @('metadata', '--no-deps', '--format-version', '1')
    if ($metadata.ExitCode -ne 0) {
        throw ("cargo metadata 失败，无法确定 workspace 成员：`n{0}" -f $metadata.StdErr)
    }
    $members = @(($metadata.StdOut | ConvertFrom-Json).packages | ForEach-Object { $_.name })

    # 默认按核心层处理：只有 ui-crates.txt 里的 crate 允许依赖 gpui。
    $coreCrates = @($members | Where-Object { $uiCrates -notcontains $_ })

    $missingFromList = @($coreCrates | Where-Object { $listedCore -notcontains $_ })
    $staleInList = @($listedCore | Where-Object { $members -notcontains $_ })
    $staleUi = @($uiCrates | Where-Object { $members -notcontains $_ })

    if ($Update -and $missingFromList.Count -gt 0) {
        $appendix = (($missingFromList | ForEach-Object { $_ }) -join "`n") + "`n"
        [System.IO.File]::AppendAllText(
            (Join-Path $repoRoot '.github/core-crates.txt'),
            $appendix,
            [System.Text.UTF8Encoding]::new($false)
        )
        Write-Host "已追加到 .github/core-crates.txt：$($missingFromList -join ', ')"
        $listedCore = @($listedCore + $missingFromList)
        $missingFromList = @()
    }

    if ($missingFromList.Count -gt 0) {
        $message = "未登记的核心层 crate：$($missingFromList -join ', ')。" +
            ' 请把它们追加到 .github/core-crates.txt（本地可运行 `pwsh -File .github/scripts/check-core-crates.ps1 -Update`）。' +
            ' 本次已按核心层检查，不影响结论。'
        Write-Host "::warning::$message"
        Write-Host "警告：$message"
    }
    foreach ($stale in $staleInList) {
        $message = ".github/core-crates.txt 里的 `"$stale`" 不是 workspace 成员，请删除或改名。"
        Write-Host "::warning::$message"
        Write-Host "警告：$message"
    }
    foreach ($stale in $staleUi) {
        $message = ".github/ui-crates.txt 里的 `"$stale`" 不是 workspace 成员，请删除或改名。"
        Write-Host "::warning::$message"
        Write-Host "警告：$message"
    }
    if ($Strict -and ($missingFromList.Count -gt 0 -or $staleInList.Count -gt 0 -or $staleUi.Count -gt 0)) {
        throw '名单与 workspace 成员不一致（-Strict）。'
    }

    $violations = @()
    $checked = 0
    foreach ($crate in $coreCrates) {
        $tree = Invoke-Captured 'cargo' @(
            'tree', '--package', $crate, '--invert', 'gpui', '--all-features', '--target', 'all'
        )
        $checked++
        if ($tree.ExitCode -eq 0) {
            $detail = $tree.StdOut.Trim()
            if ([string]::IsNullOrWhiteSpace($detail)) {
                $detail = '（cargo tree 没有输出，无法定位依赖路径；请人工确认）'
            }
            $violations += [pscustomobject]@{
                Crate   = $crate
                Summary = "核心层 crate `"$crate`" 依赖了 gpui"
                Output  = $detail
            }
        }
        elseif (($tree.StdOut + "`n" + $tree.StdErr) -match 'did not match any packages|does not depend on') {
            # gpui 不在该 crate 的依赖图里，符合预期。
        }
        else {
            $detail = ($tree.StdErr.Trim() -split "`n" | Select-Object -Last 15) -join "`n"
            if ([string]::IsNullOrWhiteSpace($detail)) { $detail = $tree.StdOut.Trim() }
            $violations += [pscustomobject]@{
                Crate   = $crate
                Summary = "无法确认 `"$crate`" 是否依赖 gpui（cargo tree 退出码 $($tree.ExitCode)）"
                Output  = $detail
            }
        }
    }

    Write-Host ("核心层 crate（{0} 个，已检查 {1} 个）：{2}" -f $coreCrates.Count, $checked, ($coreCrates -join ', '))
    Write-Host ("允许依赖 gpui（{0} 个）：{1}" -f $uiCrates.Count, ($uiCrates -join ', '))

    if ($violations.Count -gt 0) {
        $details = @()
        foreach ($violation in $violations) {
            # ::error:: 注解必须是单行，依赖树单独打印在日志里。
            Write-Host "::error::$($violation.Summary)"
            Write-Host $violation.Summary
            Write-Host $violation.Output
            $details += ("{0}`n{1}" -f $violation.Summary, $violation.Output)
        }
        [Console]::Error.WriteLine(
            ("核心层禁止依赖 gpui 的检查失败（{0} 项）：`n{1}" -f $violations.Count, ($details -join "`n"))
        )
        exit 1
    }

    Write-Host 'OK：没有任何核心层 crate 依赖 gpui。'
    exit 0
}
finally {
    Pop-Location
    foreach ($path in $tempFiles) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
    }
    try { [Console]::OutputEncoding = $originalOutputEncoding } catch { }
}
