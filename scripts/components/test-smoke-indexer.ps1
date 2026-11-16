# 通过独立进程测试冒烟脚本，覆盖真实入口的 RPC、日志和退出码验证。
$ErrorActionPreference = 'Stop'
$smoke = Join-Path $PSScriptRoot 'smoke-indexer.ps1'
$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $csc)) { throw "找不到测试夹具编译器：$csc" }

$root = Join-Path ([IO.Path]::GetTempPath()) ('rove-smoke-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$source = Join-Path $root 'FakeIndexer.cs'
$exe = Join-Path $root 'rove-indexer-cs.exe'
$previous = $env:ROVE_SMOKE_TEST_CASE
try {
    @'
using System;
using System.IO;
using System.Text;

class FakeIndexer
{
    static int Main(string[] args)
    {
        string mode = Environment.GetEnvironmentVariable("ROVE_SMOKE_TEST_CASE");
        string indexPathFile = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "index-path.txt");
        if (args[0] == "--version")
        {
            Console.WriteLine("{}");
            return 0;
        }
        if (args[0] != "--serve")
        {
            var index = new byte[96];
            index[16] = 96;
            File.WriteAllBytes(args[1], index);
            File.WriteAllText(indexPathFile, args[1]);
            return 0;
        }

        Console.WriteLine("{\"jsonrpc\":\"2.0\",\"method\":\"hello\",\"params\":{\"protocol\":1}}");
        if (Console.ReadLine() == null) return 2;
        Reply(0, "{}");
        if (Console.ReadLine() == null) return 2;
        Console.WriteLine("{\"jsonrpc\":\"2.0\",\"method\":\"status\"}");
        string journal = File.ReadAllText(indexPathFile) + ".journal";
        var segment = new byte[72];
        Encoding.ASCII.GetBytes("RVCSOVL").CopyTo(segment, 0);
        segment[8] = 1;
        segment[16] = 72;
        segment[32] = 96;
        segment[40] = 1;
        ulong hash = 14695981039346656037UL;
        for (int i = 0; i < 64; i++)
            hash = unchecked((hash ^ segment[i]) * 1099511628211UL);
        BitConverter.GetBytes(hash).CopyTo(segment, 64);
        if (mode == "checksum") segment[50] = 42;
        if (mode == "truncated") Array.Resize(ref segment, 71);
        File.WriteAllBytes(journal, segment);
        const string path = "\"Assets/Scripts/A.cs\"";
        string files = mode == "missing-file" ? "" : path;
        string locked = mode == "locked" ? path : "";
        string metadata = mode == "missing-segment" ? "null" : "{\"sequence\":1,\"offset\":0,\"length\":72}";
        Reply(1, "{\"files\":[" + files + "],\"locked\":[" + locked + "],\"segment\":" + metadata +
                 ",\"rebuild_required\":" + (mode == "rebuild" ? "true" : "false") + "}");
        if (Console.ReadLine() == null) return 2;
        if (mode == "shutdown-error")
            Console.WriteLine("{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-1,\"message\":\"shutdown failed\"}}");
        else if (mode != "shutdown-eof")
            Reply(2, "{}");
        return mode == "exit" ? 7 : 0;
    }

    static void Reply(int id, string result)
    {
        Console.WriteLine("{\"jsonrpc\":\"2.0\",\"id\":" + id + ",\"result\":" + result + "}");
    }
}
'@ | Set-Content -LiteralPath $source -Encoding UTF8
    & $csc /nologo /target:exe "/out:$exe" $source
    if ($LASTEXITCODE -ne 0) { throw '测试夹具编译失败' }

    $cases = [ordered]@{
        normal = 'SMOKE OK'
        'missing-file' = 'changed 缺少目标文件'
        'missing-segment' = 'changed 缺少覆盖层段'
        locked = 'changed 目标文件被锁定'
        rebuild = 'changed 要求全量重建'
        truncated = '覆盖层日志未完整写入'
        checksum = '覆盖层段校验和不匹配'
        'shutdown-error' = 'shutdown failed'
        'shutdown-eof' = '常驻进程在回复前关闭了输出'
        exit = '常驻进程退出失败'
    }
    $hostExe = Join-Path $PSHOME 'powershell.exe'
    if (-not (Test-Path -LiteralPath $hostExe)) { $hostExe = Join-Path $PSHOME 'pwsh.exe' }
    foreach ($mode in $cases.Keys) {
        $env:ROVE_SMOKE_TEST_CASE = $mode
        $run = & {
            # Windows PowerShell turns native stderr into ErrorRecords; expected failures are still test data.
            $ErrorActionPreference = 'Continue'
            $output = (& $hostExe -NoProfile -File $smoke -Component $root 2>&1 | Out-String)
            @{ Output = $output; Code = $LASTEXITCODE }
        }
        $output = $run.Output
        $code = $run.Code
        if ($mode -eq 'normal') {
            if ($code -ne 0 -or $output -notmatch '(?m)^SMOKE OK\r?$') { throw "正常样例失败：$output" }
        }
        elseif ($code -eq 0 -or $output -match '(?m)^SMOKE OK\r?$') {
            throw "$mode 样例误报成功：$output"
        }
        if (-not $output.Contains($cases[$mode])) { throw "$mode 样例失败原因不符：$output" }
        Write-Output "PASS $mode"
    }
}
finally {
    $env:ROVE_SMOKE_TEST_CASE = $previous
    # 只清理本测试写入的已知文件。
    foreach ($path in @($source, $exe, (Join-Path $root 'index-path.txt'))) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
    Remove-Item -LiteralPath $root
}
