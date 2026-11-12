// #35 常驻模式：`rove-indexer-cs --serve [--idle-exit-seconds N] [--trim-seconds N]`
//
// 传输：stdin / stdout 上逐行的 JSON-RPC 2.0（每行一个 JSON 对象，UTF-8，不用 Content-Length 头）。stderr 仍是日志。
// 选它而不是命名管道：进程本来就是 Rove 的子进程（#36 拉起与监控），标准流是这对进程私有的通道，不需要管道名、
// 访问控制与连接握手；Rove 退出时 stdin 关闭，进程随即退出，不会留下孤儿进程。
//
// 消息（完整说明见 docs/CS_INDEX_FORMAT.md「常驻模式与覆盖层」）：
//   进程 → Rove 第一行：hello 通知，带 protocol 与 indexer_version（版本握手的规则由 #43 定稿）。
//   initialize { protocol, project, base, journal, ... }   之后才能发其他请求
//   changed    { changes: [{ path, kind }] }               文件已变更 / 新增 / 删除 → 前台重绑，回复覆盖层段位置
//   index      { output, rebase?, journal? }               全量索引，期间发 progress 通知
//   status                                                 正在更新的文件、队列、覆盖层数量
//   wait_idle                                              后台队列清空后回复（测试与基准用）
//   set_trim   { trim_seconds?, idle_exit_seconds? }       运行中调整空闲策略
//   shutdown                                               落盘后退出（stdin 关闭同样处理）
//   进程 → Rove 通知：overlay（后台重绑写了新段）、status、progress、exiting（空闲退出）

using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Rove.Indexer;

static class Server
{
    public const int Protocol = 1;
    static readonly object WriteLock = new();
    static readonly Stream Out = Console.OpenStandardOutput();

    sealed record Request(JsonNode? Id, string Method, JsonObject Params);

    public static int Run(string[] args)
    {
        // stdout 只给协议用：其他代码误写的 Console.Out 改到 stderr。
        Console.SetOut(Console.Error);
        string? Value(string name)
        {
            int i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
        }
        var idleExit = TimeSpan.FromSeconds(double.Parse(Value("--idle-exit-seconds") ?? "600"));
        var trimAfter = TimeSpan.FromSeconds(double.Parse(Value("--trim-seconds") ?? "120"));
        string version = typeof(Server).Assembly.GetName().Version!.ToString();
        Notify("hello", new JsonObject { ["protocol"] = Protocol, ["indexer_version"] = version, ["pid"] = Environment.ProcessId });

        var inbox = new BlockingCollection<Request>();
        var shuttingDown = new CancellationTokenSource();
        var reader = new Thread(() =>
        {
            using var input = new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false));
            while (input.ReadLine() is { } line)
            {
                if (line.Length == 0) continue;
                Request request;
                try
                {
                    var node = JsonNode.Parse(line)!.AsObject();
                    request = new Request(node["id"]?.DeepClone(), (string?)node["method"] ?? "",
                        node["params"] as JsonObject is { } p ? (JsonObject)p.DeepClone() : new JsonObject());
                }
                catch (Exception e) when (e is JsonException or InvalidOperationException)
                {
                    Error(null, -32700, $"parse error: {e.Message}");
                    continue;
                }
                if (request.Method == "status") { Reply(request.Id, StatusSnapshot); continue; }
                if (request.Method == "shutdown") shuttingDown.Cancel();
                inbox.Add(request);
            }
            shuttingDown.Cancel();
            inbox.Add(new Request(null, "shutdown", new JsonObject()));
        }) { IsBackground = true, Name = "stdin" };
        reader.Start();

        Engine? engine = null;
        var waiters = new List<JsonNode?>();
        var lastActivity = Stopwatch.StartNew();
        bool trimmed = true;
        try
        {
            while (true)
            {
                PublishStatus(engine, null);
                if (engine != null && engine.Queued == 0)
                {
                    foreach (var id in waiters) Reply(id, new JsonObject { ["queued"] = 0, ["locked"] = engine.LockedCount });
                    waiters.Clear();
                }
                // 前台请求优先；没有请求时做一批后台工作；都没有就等（同时计时空闲修剪与退出）。
                Request? request = null;
                var batch = engine != null && !shuttingDown.IsCancellationRequested && !inbox.TryTake(out request) ? engine.NextBatch() : [];
                if (request == null && batch.Count > 0)
                {
                    var watch = Stopwatch.StartNew();
                    var previous = Thread.CurrentThread.Priority;
                    Thread.CurrentThread.Priority = ThreadPriority.BelowNormal;
                    RebindResult result;
                    try { result = engine!.Rebind(batch, background: true); }
                    finally { Thread.CurrentThread.Priority = previous; }
                    if (result.Segment != null) Notify("overlay", Describe(result, background: true));
                    lastActivity.Restart();
                    trimmed = false;
                    // 限速：按占空比休眠；有前台请求时立即醒来
                    var rest = watch.Elapsed * (1 / Math.Clamp(engine.BackgroundDuty, 0.05, 1) - 1);
                    if (rest > TimeSpan.Zero && inbox.TryTake(out request, rest)) { }
                    if (request == null) continue;
                }
                if (request == null)
                {
                    var wait = engine == null ? idleExit : trimmed ? idleExit - lastActivity.Elapsed : trimAfter - lastActivity.Elapsed;
                    if (engine?.NextLockedRetry() is { } retry && retry < wait) wait = retry;
                    if (wait < TimeSpan.Zero) wait = TimeSpan.Zero;
                    if (!inbox.TryTake(out request, wait))
                    {
                        if (engine != null && engine.NextLockedRetry() is { } r && r <= TimeSpan.Zero) continue;
                        if (!trimmed && lastActivity.Elapsed >= trimAfter && engine != null)
                        {
                            engine.Trim();
                            trimmed = true;
                            Program.Log($"idle {trimAfter.TotalSeconds:F0}s: dropped warm compilations, ws={Environment.WorkingSet >> 20}MB");
                            continue;
                        }
                        if (lastActivity.Elapsed >= idleExit)
                        {
                            Program.Log($"idle {idleExit.TotalSeconds:F0}s: exiting");
                            Notify("exiting", new JsonObject { ["reason"] = "idle" });
                            return 0;
                        }
                        continue;
                    }
                }
                lastActivity.Restart();
                trimmed = false;
                switch (request.Method)
                {
                    case "initialize":
                        try
                        {
                            engine?.Dispose();
                            var p = request.Params;
                            if ((int?)p["protocol"] is { } protocol && protocol != Protocol)
                            {
                                Error(request.Id, -32001, $"unsupported protocol {protocol} (indexer speaks {Protocol})");
                                break;
                            }
                            if ((double?)p["idle_exit_seconds"] is { } ie) idleExit = TimeSpan.FromSeconds(ie);
                            if ((double?)p["trim_seconds"] is { } ts) trimAfter = TimeSpan.FromSeconds(ts);
                            var options = new EngineOptions();
                            if ((double?)p["background_duty"] is { } duty) options.BackgroundDuty = duty;
                            if ((int?)p["max_warm_files"] is { } warm) options.MaxWarmFiles = warm;
                            if ((int?)p["background_batch"] is { } bb) options.BackgroundBatch = bb;
                            engine = new Engine((string)p["project"]!, (string)p["base"]!, (string)p["journal"]!, options);
                            Reply(request.Id, new JsonObject
                            {
                                ["protocol"] = Protocol, ["indexer_version"] = version,
                                ["segments"] = engine.Segments, ["journal_bytes"] = engine.JournalBytes, ["overlay_files"] = engine.OverlayFiles,
                            });
                        }
                        catch (Exception e)
                        {
                            engine = null;
                            Error(request.Id, -32002, $"initialize failed: {e.Message}");
                        }
                        break;
                    case "changed":
                    {
                        if (engine == null) { Error(request.Id, -32003, "not initialized"); break; }
                        var paths = (request.Params["changes"] as JsonArray ?? []).Select(c => (string?)c?["path"]).OfType<string>().ToList();
                        PublishStatus(engine, paths);
                        try { Reply(request.Id, Describe(engine.Rebind(paths, background: false), background: false)); }
                        catch (Exception e)
                        {
                            Program.Log($"rebind failed: {e}");
                            Error(request.Id, -32004, $"rebind failed: {e.Message}");
                        }
                        break;
                    }
                    case "index":
                    {
                        var p = request.Params;
                        var output = (string?)p["output"];
                        var root = (string?)p["project"];
                        if (output == null || root == null) { Error(request.Id, -32602, "index needs output and project"); break; }
                        var id = request.Id;
                        int code;
                        try
                        {
                            code = Program.FullIndex(Path.GetFullPath(root), Path.GetFullPath(output), new IndexOptions(), null, reconcile: true,
                                2200L << 20, (done, total) => Notify("progress", new JsonObject { ["id"] = id?.DeepClone(), ["done"] = done, ["total"] = total }),
                                printReport: false);
                        }
                        catch (Exception e) { Program.Log($"full index failed: {e}"); code = -1; }
                        GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);
                        if (code != 0) { Error(id, -32005, $"full index failed ({code})"); break; }
                        if ((bool?)p["rebase"] == true && engine != null && (string?)p["journal"] is { } journal)
                            engine.Rebase(Path.GetFullPath(output), journal);
                        Reply(id, new JsonObject { ["output"] = output, ["files"] = Program.LastFullIndex.files, ["symbols"] = Program.LastFullIndex.symbols });
                        break;
                    }
                    case "set_trim":
                        // 运行中调整空闲策略（测量空闲内存用）
                        if ((double?)request.Params["trim_seconds"] is { } trim) trimAfter = TimeSpan.FromSeconds(trim);
                        if ((double?)request.Params["idle_exit_seconds"] is { } exit) idleExit = TimeSpan.FromSeconds(exit);
                        Reply(request.Id, new JsonObject());
                        break;
                    case "wait_idle":
                        waiters.Add(request.Id);
                        break;
                    case "shutdown":
                        engine?.Sync();
                        if (request.Id != null) Reply(request.Id, new JsonObject());
                        return 0;
                    default:
                        Error(request.Id, -32601, $"unknown method {request.Method}");
                        break;
                }
            }
        }
        finally
        {
            engine?.Dispose();
        }
    }

    static JsonObject StatusSnapshot = new() { ["initialized"] = false };

    static void PublishStatus(Engine? engine, List<string>? updating)
    {
        if (engine == null) return;
        var status = new JsonObject
        {
            ["initialized"] = true,
            ["updating"] = new JsonArray((updating ?? []).Select(p => (JsonNode)p).ToArray()),
            ["queued"] = engine.Queued,
            ["locked"] = engine.LockedCount,
            ["overlay_files"] = engine.OverlayFiles,
            ["segments"] = engine.Segments,
            ["journal_bytes"] = engine.JournalBytes,
            ["warm_projects"] = new JsonArray(engine.WarmProjects.Select(p => (JsonNode)p).ToArray()),
            ["working_set_mb"] = Environment.WorkingSet >> 20,
            ["gc_heap_mb"] = GC.GetTotalMemory(false) >> 20,
        };
        bool changed = status.ToJsonString() != StatusSnapshot.ToJsonString();
        StatusSnapshot = status;
        if (changed && (int?)status["queued"] is int q && (q > 0 || updating != null)) Notify("status", (JsonObject)status.DeepClone());
    }

    static JsonObject Describe(RebindResult r, bool background) => new()
    {
        ["background"] = background,
        ["files"] = new JsonArray(r.Files.Select(f => (JsonNode)f).ToArray()),
        ["segment"] = r.Segment is { } s ? new JsonObject { ["sequence"] = s.sequence, ["offset"] = s.offset, ["length"] = s.length } : null,
        ["impact"] = new JsonObject
        {
            ["changed"] = new JsonArray(r.Changed.Select(f => (JsonNode)f).ToArray()),
            ["added_names"] = new JsonArray(r.AddedNames.Select(f => (JsonNode)f).ToArray()),
            ["queued"] = r.Queued,
        },
        ["locked"] = new JsonArray(r.Locked.Select(f => (JsonNode)f).ToArray()),
        ["rebuild_required"] = r.RebuildRequired,
        ["elapsed_ms"] = r.ElapsedMs,
    };

    static void Send(JsonObject message)
    {
        message["jsonrpc"] = "2.0";
        var bytes = Encoding.UTF8.GetBytes(message.ToJsonString() + "\n");
        lock (WriteLock)
        {
            Out.Write(bytes);
            Out.Flush();
        }
    }

    static void Notify(string method, JsonObject p) => Send(new JsonObject { ["method"] = method, ["params"] = p });

    static void Reply(JsonNode? id, JsonObject result)
    {
        if (id == null) return;
        Send(new JsonObject { ["id"] = id.DeepClone(), ["result"] = result.DeepClone() });
    }

    static void Error(JsonNode? id, int code, string message) =>
        Send(new JsonObject { ["id"] = id?.DeepClone(), ["error"] = new JsonObject { ["code"] = code, ["message"] = message } });
}
