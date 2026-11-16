<#
.SYNOPSIS
  在目标机器上验证 C# 索引组件（#43）：版本、全量索引、常驻握手与一次文件变更。

.DESCRIPTION
  给 Windows Sandbox / 干净虚拟机用：机器上不需要 .NET。依次做
    1. `--version`：组件自述（版本、协议区间、索引格式）；
    2. 全量索引一个项目（默认用脚本生成的最小项目；`-Project` 可指向真实 Unity 项目的副本）；
    3. `--serve`：读 hello、发 initialize、发 changed（改一个源文件）、读取并验证覆盖层段、确认 shutdown 成功。
  任何一步失败都以非零码退出；最后打印一行 `SMOKE OK`。

.PARAMETER Component
  版本目录（里面有 rove-indexer-cs.exe 与 component.json）。
.PARAMETER Project
  Unity 项目副本的根目录（会追加一行注释）；省略时在临时目录生成最小项目。
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Component,
    [string]$Project
)

$ErrorActionPreference = 'Stop'
$exe = Join-Path $Component 'rove-indexer-cs.exe'
if (-not (Test-Path $exe)) { throw "找不到 $exe" }
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("rove-smoke-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null

$about = (& $exe --version) -join ''
if ($LASTEXITCODE -ne 0) { throw "版本查询失败（$LASTEXITCODE）" }
Write-Output "version: $about"

if (-not $Project) {
    $Project = Join-Path $work 'Game'
    New-Item -ItemType Directory -Force -Path (Join-Path $Project 'Assets\Scripts') | Out-Null
    Set-Content -Path (Join-Path $Project 'Game.csproj') -Value '<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><Compile Include="Assets\Scripts\A.cs" /><Compile Include="Assets\Scripts\B.cs" /></ItemGroup></Project>'
    Set-Content -Path (Join-Path $Project 'Assets\Scripts\A.cs') -Value 'public class A { public int Value; public void Run() { Value++; } }'
    Set-Content -Path (Join-Path $Project 'Assets\Scripts\B.cs') -Value 'public class B { void Go() { new A().Run(); } }'
}
$Project = (Resolve-Path -LiteralPath $Project).ProviderPath
$changed = Get-ChildItem -LiteralPath (Join-Path $Project 'Assets') -File -Recurse -Filter *.cs | Select-Object -First 1
if (-not $changed) { throw '项目里没有 .cs 文件' }
$changedRel = $changed.FullName.Substring($Project.TrimEnd('\').Length + 1).Replace('\', '/')

# 2. 全量索引
$index = Join-Path $work 'base.rvcsidx'
$sw = [Diagnostics.Stopwatch]::StartNew()
& $exe $Project $index | Out-Null
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $index)) { throw "全量索引失败（$LASTEXITCODE）" }
Write-Output ('full index: {0:N1}s, {1:N1} MB' -f $sw.Elapsed.TotalSeconds, ((Get-Item $index).Length / 1MB))

# 3. 常驻模式
$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $exe
$psi.Arguments = '--serve'
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.StandardOutputEncoding = [Text.Encoding]::UTF8
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$proc = [Diagnostics.Process]::Start($psi)
$null = $proc.StandardError.ReadToEndAsync()
function Read-Line([int]$timeoutMs) {
    $task = $proc.StandardOutput.ReadLineAsync()
    if (-not $task.Wait($timeoutMs)) { throw '常驻进程没有在限定时间内回复' }
    if ($null -eq $task.Result) { throw '常驻进程在回复前关闭了输出' }
    return $task.Result
}

function Send-Request([string]$json) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($json + [char]10)
    $stream = $proc.StandardInput.BaseStream
    $stream.Write($bytes, 0, $bytes.Length)
    $stream.Flush()
}
# JSON-RPC 通知没有 ID。等待目标响应时使用同一个超时预算。
function Read-Reply([int]$id, [int]$timeoutMs) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        $left = $timeoutMs - $timer.ElapsedMilliseconds
        if ($left -le 0) { throw "等待 RPC $id 超时" }
        $reply = Read-Line $left | ConvertFrom-Json
        if ($reply.id -eq $id) {
            if ($reply.error) { throw "RPC ${id} 失败：$($reply.error.message)" }
            if ($reply.jsonrpc -ne '2.0' -or $null -eq $reply.result) { throw "RPC $id 回复无效" }
            return $reply.result
        }
    } while ($true)
}

# 覆盖层 v1 段头和尾部校验和见 docs/CS_INDEX_FORMAT.md。
function Assert-JournalSegment($segment, [string]$journal, [string]$index) {
    if ($null -eq $segment -or $null -eq $segment.offset -or
        $null -eq $segment.length -or $null -eq $segment.sequence) { throw 'changed 缺少覆盖层段' }
    $offset = [long]$segment.offset
    $length = [long]$segment.length
    $sequence = [long]$segment.sequence
    if ($offset -lt 0 -or $offset % 8 -ne 0 -or $length -lt 72 -or
        $length % 8 -ne 0 -or $sequence -lt 1) { throw 'changed 覆盖层段位置、长度或序号无效' }

    $base = [IO.BinaryReader]::new([IO.File]::OpenRead($index))
    try {
        $base.BaseStream.Position = 88
        $baseChecksum = $base.ReadUInt64()
        $base.BaseStream.Position = 16
        $baseLength = $base.ReadUInt64()
    }
    finally { $base.Dispose() }

    $stream = [IO.File]::Open($journal, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($offset -gt $stream.Length -or $length -gt $stream.Length - $offset) {
            throw '覆盖层日志未完整写入所报告的段'
        }
        $stream.Position = $offset
        $magic = [Text.Encoding]::ASCII.GetString($reader.ReadBytes(8))
        if ($magic -cne "RVCSOVL$([char]0)" -or $reader.ReadUInt32() -ne 1 -or
            $reader.ReadUInt32() -ne 0 -or $reader.ReadUInt64() -ne $length -or
            $reader.ReadUInt64() -ne $baseChecksum -or $reader.ReadUInt64() -ne $baseLength -or
            $reader.ReadUInt64() -ne $sequence) { throw '覆盖层段头与回复或基础索引不匹配' }

        # FNV-1a-64 用两个 32 位字计算，避免 PowerShell 溢出后转 double。
        $stream.Position = $offset
        $high = 3421674724L
        $low = 2216829733L
        $remaining = $length - 8
        while ($remaining -gt 0) {
            $chunk = $reader.ReadBytes([int][Math]::Min($remaining, 65536L))
            if ($chunk.Length -eq 0) { throw '读取覆盖层段时遇到文件结束' }
            foreach ($byte in $chunk) {
                $low = $low -bxor $byte
                $product = $low * 435L
                $high = (($high * 435L) + ($low -shl 8) + ($product -shr 32)) -band 4294967295L
                $low = $product -band 4294967295L
            }
            $remaining -= $chunk.Length
        }
        if ($reader.ReadUInt32() -ne $low -or $reader.ReadUInt32() -ne $high) {
            throw '覆盖层段校验和不匹配'
        }
    }
    finally { $reader.Dispose() }
}
try {
    $hello = Read-Line 60000 | ConvertFrom-Json
    if ($hello.method -ne 'hello') { throw "第一条消息不是 hello：$($hello.method)" }
    Write-Output "hello: protocol=$($hello.params.protocol) min=$($hello.params.protocol_min) version=$($hello.params.indexer_version)"
    $journal = "$index.journal"
    $init = @{ jsonrpc = '2.0'; id = 0; method = 'initialize'; params = @{
        protocol = $hello.params.protocol; project = $Project; base = $index; journal = $journal } } | ConvertTo-Json -Compress -Depth 5
    Send-Request $init
    $null = Read-Reply 0 300000
    Add-Content -LiteralPath $changed.FullName -Value '// smoke edit'
    $req = @{ jsonrpc = '2.0'; id = 1; method = 'changed'; params = @{ changes = @(@{ path = $changedRel }) } } | ConvertTo-Json -Compress -Depth 5
    Send-Request $req
    $result = Read-Reply 1 300000
    if ($result.rebuild_required) { throw 'changed 要求全量重建' }
    if ($result.locked -contains $changedRel) { throw "changed 目标文件被锁定：$changedRel" }
    if ($result.files -notcontains $changedRel) { throw "changed 缺少目标文件：$changedRel" }
    Assert-JournalSegment $result.segment $journal $index
    Write-Output "changed: files=$($result.files -join ',') segment=$($result.segment.sequence) elapsed_ms=$($result.elapsed_ms)"
    Send-Request '{"jsonrpc":"2.0","id":2,"method":"shutdown"}'
    $null = Read-Reply 2 30000
    if (-not $proc.WaitForExit(30000)) { throw '常驻进程没有退出' }
    if ($proc.ExitCode -ne 0) { throw "常驻进程退出失败（$($proc.ExitCode)）" }
}
finally {
    if (-not $proc.HasExited) { $proc.Kill() }
    $proc.Dispose()
}
Write-Output 'SMOKE OK'
