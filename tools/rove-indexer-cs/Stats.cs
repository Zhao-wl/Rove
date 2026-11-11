using System.Collections.Concurrent;
using System.Diagnostics;

namespace Rove.Indexer;

/// 计数与分段计时。流水线里各阶段相互重叠，所以除墙钟时间外，按"线程秒"累计每个阶段的 CPU 去向。
static class Stats
{
    public static long Read, Parse, Compile, Generators, Emit, BindTotal, SymbolInfo, DeclaredSymbol, SymbolKey, Relations;
    public static long Names, UnresolvedNames, CandidateNames, ReadBytes, Images, ImageFailures, GeneratedTrees;
    public static readonly ConcurrentDictionary<string, int> Unresolved = new();
    public static readonly ConcurrentDictionary<string, int> UnresolvedByProject = new();
    public static readonly ConcurrentQueue<string> GeneratorIssues = new();

    public static long Now() => Stopwatch.GetTimestamp();
    public static void Add(ref long counter, long start) => Interlocked.Add(ref counter, Stopwatch.GetTimestamp() - start);
    public static double Seconds(long ticks) => Math.Round((double)ticks / Stopwatch.Frequency, 2);

    public static object ThreadSeconds() => new
    {
        read = Seconds(Read),
        parse = Seconds(Parse),
        create_compilations = Seconds(Compile),
        generators = Seconds(Generators),
        emit_metadata_images = Seconds(Emit),
        bind_and_collect = Seconds(BindTotal),
        bind_get_symbol_info = Seconds(SymbolInfo),
        bind_get_declared_symbol = Seconds(DeclaredSymbol),
        bind_symbol_key = Seconds(SymbolKey),
        bind_relations = Seconds(Relations),
    };
}

/// 后台采样内存：工作集峰值之外，记录 GC 堆与已提交内存的峰值，用于区分"存活对象"与"GC 余量"。
/// 同时充当软上限：工作集超过 softLimit 时做一次激进的完整 GC（压缩并把空闲段还给系统）。
/// 服务器 GC 的各代预算按 16 个堆放大，存活 1.9 GB 时工作集能涨到 3.5 GB 以上；软上限把余量压回去，
/// 只在超限时触发，绝大多数时间没有额外停顿。
sealed class MemorySampler : IDisposable
{
    readonly Timer _timer;
    readonly bool _log;
    readonly long _softLimit;
    long _lastCompact;
    int _ticks;
    public long PeakHeap, PeakCommitted, PeakWorkingSet;
    public int SoftLimitCollections;

    public MemorySampler(bool log, long softLimit)
    {
        _log = log;
        _softLimit = softLimit;
        _timer = new Timer(_ => Sample(), null, 0, 500);
    }

    public void Sample()
    {
        var info = GC.GetGCMemoryInfo();
        long ws = Environment.WorkingSet;
        lock (this)
        {
            PeakHeap = Math.Max(PeakHeap, info.HeapSizeBytes);
            PeakCommitted = Math.Max(PeakCommitted, info.TotalCommittedBytes);
            PeakWorkingSet = Math.Max(PeakWorkingSet, ws);
        }
        if (_softLimit > 0 && ws > _softLimit && Stopwatch.GetElapsedTime(_lastCompact).TotalSeconds > 3)
        {
            GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);
            _lastCompact = Stopwatch.GetTimestamp();
            Interlocked.Increment(ref SoftLimitCollections);
            if (_log) Program.Log($"MEM soft limit: ws={ws >> 20}MB -> {Environment.WorkingSet >> 20}MB");
        }
        if (_log && ++_ticks % 6 == 0)
        {
            // 剖析模式：每 3 秒强制一次完整 GC，得到真实存活量（堆大小里含尚未回收的垃圾）
            long live = GC.GetTotalMemory(forceFullCollection: true);
            Program.Log($"MEM live={live >> 20}MB heap={info.HeapSizeBytes >> 20}MB committed={info.TotalCommittedBytes >> 20}MB ws={ws >> 20}MB");
        }
    }

    public void Dispose() => _timer.Dispose();
}
