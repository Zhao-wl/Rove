<#
.SYNOPSIS
  行尾检查（issue #3）：提交进仓库的文本文件必须是 LF，`.ps1` 在 .gitattributes 里声明为 CRLF。

.DESCRIPTION
  `.gitattributes` 声明了 `* text=auto eol=lf` 与 `*.ps1 text eol=crlf`。本脚本用
  `git ls-files --eol` 读取**索引**里的行尾（也就是真正会被别人 checkout 的内容），检查：

  1. 文本文件在索引里是 LF（`i/lf`）；二进制文件（`i/none` 或 attr 含 `-text`）跳过；
  2. 每个文本文件都被 .gitattributes 覆盖（attr 不为空），新增文件类型不会悄悄逃逸；
  3. `*.ps1` 声明了 `eol=crlf`。

  工作区里已有的 CRLF 不检查：那是本地 checkout 的历史状态，提交时 git 会按属性归一化。

  为了让 Windows PowerShell 5.1 在中文代码页（CP936）下也能工作，git 的输出重定向到
  临时文件并按 UTF-8 读取，控制台输出编码也临时切成 UTF-8。

.PARAMETER Fix
  运行 `git add --renormalize .` 后重新检查（本地修复索引里的 CRLF）。
#>
[CmdletBinding()]
param(
    [switch]$Fix
)

$ErrorActionPreference = 'Stop'

$originalOutputEncoding = [Console]::OutputEncoding
try { [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) } catch { }

$tempFiles = New-Object System.Collections.Generic.List[string]
$stdoutFile = Join-Path ([System.IO.Path]::GetTempPath()) ("rove-eol-out-{0}.tmp" -f [guid]::NewGuid())
$stderrFile = Join-Path ([System.IO.Path]::GetTempPath()) ("rove-eol-err-{0}.tmp" -f [guid]::NewGuid())
[void]$tempFiles.Add($stdoutFile)
[void]$tempFiles.Add($stderrFile)

try {
    $repoRoot = (& git rev-parse --show-toplevel)
    if ($LASTEXITCODE -ne 0 -or -not $repoRoot) {
        throw '必须在 git 仓库内运行。'
    }
    Push-Location $repoRoot
    try {
        if ($Fix) {
            & git add --renormalize .
            if ($LASTEXITCODE -ne 0) { throw 'git add --renormalize 失败。' }
            Write-Host '已执行 git add --renormalize .'
        }

        $proc = Start-Process -FilePath 'git' -NoNewWindow -Wait -PassThru `
            -ArgumentList @('-c', 'core.quotepath=false', 'ls-files', '--eol') `
            -RedirectStandardOutput $stdoutFile -RedirectStandardError $stderrFile
        if ($proc.ExitCode -ne 0) {
            throw ("git ls-files --eol 失败：`n{0}" -f (Get-Content -LiteralPath $stderrFile -Encoding UTF8 -Raw))
        }
        $lines = @(Get-Content -LiteralPath $stdoutFile -Encoding UTF8)

        $problems = @()
        $checked = 0
        foreach ($line in $lines) {
            if ($line -notmatch '^(i/\S+)\s+(w/\S+)\s+attr/(.*?)\t(.+)$') {
                $problems += "无法解析 git ls-files --eol 输出：$line"
                continue
            }
            $indexEol = $Matches[1]
            $attr = $Matches[3].Trim()
            $path = $Matches[4].Trim()

            if ($indexEol -eq 'i/none') { continue }                      # 二进制
            if ($attr -match '(^|\s)(-text|binary)(\s|$)') { continue }   # 显式声明为二进制

            $checked++
            $isPowerShell = $path -match '\.ps1$'

            if ([string]::IsNullOrWhiteSpace($attr)) {
                $problems += "$path：没有被 .gitattributes 覆盖（attr 为空），行尾行为不可控"
                continue
            }

            if ($isPowerShell) {
                if ($attr -notmatch 'eol=crlf') {
                    $problems += "$path：.ps1 必须以 CRLF 检出，但 .gitattributes 没有声明 eol=crlf（attr：$attr）"
                }
                continue
            }

            if ($indexEol -ne 'i/lf') {
                $problems += "$path：索引里的行尾是 $indexEol，必须是 LF（用 -Fix 重新归一化）"
            }
        }

        Write-Host "已检查 $checked 个文本文件（共 $($lines.Count) 个受跟踪文件）。"

        if ($problems.Count -gt 0) {
            foreach ($problem in $problems) {
                Write-Host "::error::$problem"
            }
            [Console]::Error.WriteLine(
                ("行尾检查失败（{0} 项）：`n{1}" -f $problems.Count, ($problems -join "`n"))
            )
            exit 1
        }

        Write-Host 'OK：行尾符合 .gitattributes。'
        exit 0
    }
    finally {
        Pop-Location
    }
}
finally {
    foreach ($path in $tempFiles) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }
    }
    try { [Console]::OutputEncoding = $originalOutputEncoding } catch { }
}
