<#
.SYNOPSIS
  在目标机器上验证 C# 索引组件（#43）：版本、全量索引、常驻握手与一次文件变更。

.DESCRIPTION
  给 Windows Sandbox / 干净虚拟机用：机器上不需要 .NET。依次做
    1. `--version`：组件自述（版本、协议区间、索引格式）；
    2. 全量索引一个项目（默认用脚本生成的最小项目；`-Project` 可指向真实 Unity 项目的副本）；
    3. `--serve`：读 hello、发 initialize、发 changed（改一个源文件）、收到覆盖层回复、shutdown。
  任何一步失败都以非零码退出；最后打印一行 `SMOKE OK`。

.PARAMETER Component
  版本目录（里面有 rove-indexer-cs.exe 与 component.json）。
.PARAMETER Project
  Unity 项目根目录；省略时在临时目录生成最小项目。
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
Write-Output "version: $about"

if (-not $Project) {
    $Project = Join-Path $work 'Game'
    New-Item -ItemType Directory -Force -Path (Join-Path $Project 'Assets\Scripts') | Out-Null
    Set-Content -Path (Join-Path $Project 'Game.csproj') -Value '<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><Compile Include="Assets\Scripts\A.cs" /><Compile Include="Assets\Scripts\B.cs" /></ItemGroup></Project>'
    Set-Content -Path (Join-Path $Project 'Assets\Scripts\A.cs') -Value 'public class A { public int Value; public void Run() { Value++; } }'
    Set-Content -Path (Join-Path $Project 'Assets\Scripts\B.cs') -Value 'public class B { void Go() { new A().Run(); } }'
}
$changed = Get-ChildItem (Join-Path $Project 'Assets') -Recurse -Filter *.cs | Select-Object -First 1
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
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$proc = [Diagnostics.Process]::Start($psi)
$null = $proc.StandardError.ReadToEndAsync()
function Read-Line([int]$timeoutMs) {
    $task = $proc.StandardOutput.ReadLineAsync()
    if (-not $task.Wait($timeoutMs)) { throw '常驻进程没有在限定时间内回复' }
    return $task.Result
}
try {
    $hello = Read-Line 60000 | ConvertFrom-Json
    if ($hello.method -ne 'hello') { throw "第一条消息不是 hello：$($hello.method)" }
    Write-Output "hello: protocol=$($hello.params.protocol) min=$($hello.params.protocol_min) version=$($hello.params.indexer_version)"
    $init = @{ jsonrpc = '2.0'; id = 0; method = 'initialize'; params = @{
        protocol = $hello.params.protocol; project = $Project; base = $index; journal = "$index.journal" } } | ConvertTo-Json -Compress -Depth 5
    $proc.StandardInput.WriteLine($init); $proc.StandardInput.Flush()
    $reply = Read-Line 300000 | ConvertFrom-Json
    if ($reply.error) { throw "initialize 失败：$($reply.error.message)" }
    Add-Content -Path $changed.FullName -Value '// smoke edit'
    $req = @{ jsonrpc = '2.0'; id = 1; method = 'changed'; params = @{ changes = @(@{ path = $changedRel }) } } | ConvertTo-Json -Compress -Depth 5
    $proc.StandardInput.WriteLine($req); $proc.StandardInput.Flush()
    do { $line = Read-Line 300000 | ConvertFrom-Json } while ($line.id -ne 1)
    if ($line.error) { throw "changed 失败：$($line.error.message)" }
    Write-Output "changed: files=$($line.result.files -join ',') elapsed_ms=$($line.result.elapsed_ms)"
    $proc.StandardInput.WriteLine('{"jsonrpc":"2.0","id":2,"method":"shutdown"}'); $proc.StandardInput.Flush()
    if (-not $proc.WaitForExit(30000)) { throw '常驻进程没有退出' }
}
finally {
    if (-not $proc.HasExited) { $proc.Kill() }
}
Write-Output 'SMOKE OK'
