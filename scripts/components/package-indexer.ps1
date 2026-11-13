<#
.SYNOPSIS
  打包 C# 索引组件（#43）：dotnet publish（self-contained、trim）→ 生成 component.json → 自检 → 压成 zip。

.DESCRIPTION
  产物是 `<OutDir>\rove-indexer-cs-<版本>-win-x64.zip`，压缩包的顶层目录是版本号，因此用户把它解压到
  `%LOCALAPPDATA%\Rove\components\indexer-cs\` 即得到 `<版本>\rove-indexer-cs.exe` 与 `<版本>\component.json`，
  和 Rove 的发现规则一致（见 docs/CS_INDEX_FORMAT.md「组件、协议与索引格式的版本」）。发布流水线（#76）直接调用本脚本，
  并同时发布 zip 的 SHA256。

  component.json 由 `rove-indexer-cs --version` 的输出原样写成：版本、协议区间、索引格式版本都来自进程自己，
  不在脚本里重复写一遍。

.PARAMETER OutDir
  输出目录，默认 target\components。
.PARAMETER NoTrim
  关闭 trim（排查用；正式包默认 trim，结论与数据见 docs/CS_INDEX_FORMAT.md）。
.PARAMETER ReadyToRun
  开启 ReadyToRun（默认关闭，数据见文档）。
#>
[CmdletBinding()]
param(
    [string]$OutDir = 'target\components',
    [switch]$NoTrim,
    [switch]$ReadyToRun
)

$ErrorActionPreference = 'Stop'
$repo = (& git rev-parse --show-toplevel)
if ($LASTEXITCODE -ne 0 -or -not $repo) { throw '必须在 git 仓库内运行。' }
$project = Join-Path $repo 'tools\rove-indexer-cs\RoveIndexer.csproj'
if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $repo $OutDir }
$stage = Join-Path $OutDir 'stage'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

$publishArgs = @('publish', $project, '-c', 'Release', '-r', 'win-x64', '--self-contained', '-o', $stage)
if (-not $NoTrim) { $publishArgs += '-p:PublishTrimmed=true' }
if ($ReadyToRun) { $publishArgs += '-p:PublishReadyToRun=true' }
& dotnet @publishArgs
if ($LASTEXITCODE -ne 0) { throw "dotnet publish 失败（$LASTEXITCODE）" }

$exe = Join-Path $stage 'rove-indexer-cs.exe'
$info = (& $exe --version) -join ''
if ($LASTEXITCODE -ne 0) { throw "--version 失败（$LASTEXITCODE）" }
$manifest = $info | ConvertFrom-Json
if ($manifest.component -ne 'indexer-cs') { throw "意外的组件名：$($manifest.component)" }
# 去掉组件不需要的文件：调试符号与文档
Get-ChildItem $stage -Include *.pdb, *.xml -Recurse | Remove-Item -Force
[System.IO.File]::WriteAllText((Join-Path $stage 'component.json'), $info, [System.Text.UTF8Encoding]::new($false))

# 第三方许可：Roslyn 与 .NET 运行时都是 MIT，随包放一份说明（完整清单由 #76 汇总）
$notice = @"
rove-indexer-cs $($manifest.version) 包含：
- .NET 9 运行时与基础类库（MIT，Copyright (c) .NET Foundation and Contributors）
- Microsoft.CodeAnalysis (Roslyn) 5.9.0（MIT，Copyright (c) .NET Foundation and Contributors）
- System.IO.Hashing（MIT，Copyright (c) .NET Foundation and Contributors）
本组件运行期不发起任何网络请求。
"@
[System.IO.File]::WriteAllText((Join-Path $stage 'THIRD_PARTY_NOTICES.txt'), $notice, [System.Text.UTF8Encoding]::new($false))

# 自检：影响分析规则等纯函数（真实项目上的全量索引、常驻握手见 scripts\components\smoke-indexer.ps1）
& $exe --self-test | Out-Null
if ($LASTEXITCODE -ne 0) { throw "--self-test 失败（$LASTEXITCODE）" }

$version = $manifest.version
$versionDir = Join-Path $OutDir $version
if (Test-Path $versionDir) { Remove-Item -Recurse -Force $versionDir }
Move-Item $stage $versionDir
$zip = Join-Path $OutDir "rove-indexer-cs-$version-win-x64.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $versionDir -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText("$zip.sha256", "$hash  $(Split-Path -Leaf $zip)`n", [System.Text.UTF8Encoding]::new($false))

$unpacked = (Get-ChildItem $versionDir -Recurse -File | Measure-Object Length -Sum).Sum
'{0}: 解压后 {1:N1} MB，zip {2:N1} MB，sha256 {3}' -f (Split-Path -Leaf $zip), ($unpacked / 1MB), ((Get-Item $zip).Length / 1MB), $hash
Write-Output $zip
