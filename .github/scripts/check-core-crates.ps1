<#
.SYNOPSIS
  核心层禁止依赖 gpui（issue #3）。

.DESCRIPTION
  - 核心层名单： .github/core-crates.txt
  - 允许依赖 gpui 的名单： .github/ui-crates.txt（rove-ui / rove-app）

  对每个核心层 crate 执行 `cargo tree -p <crate> -i gpui`，输出非空即失败。
  检查范围来自 `cargo metadata`（workspace 全部成员），**未登记的 crate 也会按核心层检查**，
  所以 A 链新增 crate 后不需要任何改动就会被覆盖；同时会给出 ::warning:: 提示补充名单。

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

$repoRoot = & git rev-parse --show-toplevel
if ($LASTEXITCODE -ne 0) {
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

    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo metadata 失败，无法确定 workspace 成员。'
    }
    $members = @(($metadataJson | ConvertFrom-Json).packages | ForEach-Object { $_.name })

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
        # --all-features：否则 optional 依赖 + feature 的写法会让 cargo 报
        #   "did not match any packages"，检查会静默放过。
        # --target all：否则平台条件依赖（[target.'cfg(unix)'.dependencies]）
        #   会打印空的依赖树，排查时容易误导。
        $output = (& cmd /c "cargo tree --package $crate --invert gpui --all-features --target all 2>&1") -join "`n"
        $code = $LASTEXITCODE
        $checked++
        if ($code -eq 0) {
            if ([string]::IsNullOrWhiteSpace($output)) {
                $output = '（cargo tree 没有输出，无法定位依赖路径；请人工确认）'
            }
            $violations += [pscustomobject]@{
                Crate   = $crate
                Summary = "核心层 crate `"$crate`" 依赖了 gpui"
                Output  = $output
            }
        }
        elseif ($output -match 'did not match any packages|does not depend on') {
            # gpui 不在该 crate 的依赖图里，符合预期。
        }
        else {
            $violations += [pscustomobject]@{
                Crate   = $crate
                Summary = "无法确认 `"$crate`" 是否依赖 gpui（cargo tree 退出码 $code）"
                Output  = $output
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
}
