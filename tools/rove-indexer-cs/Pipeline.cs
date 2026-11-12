// 按项目依赖拓扑的流水线：读文件 → 解析 → 构造编译 → 输出元数据镜像（解锁下游）→ 绑定 → 释放。
//
// 下游项目只引用上游的元数据镜像（metadata-only emit），不持有上游的 Compilation，
// 所以一个项目绑定完，它的语法树、编译与语义缓存都能立即回收；同时活着的只有正在处理的几个项目。

using System.Collections.Concurrent;
using System.Collections.Immutable;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Runtime.Loader;
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Diagnostics;
using Microsoft.CodeAnalysis.Emit;

namespace Rove.Indexer;

sealed class IndexOptions
{
    public bool Generators;
    public string? DiagAssembly;
    /// 同时读文件的线程数。实测（docs/M2_INDEXER_PERF_REPORT.md）机械硬盘与 SSD 都是并发越深越快：
    /// 深队列让 NCQ 与系统 I/O 调度合并寻道；32 之后不再改善。
    public int IoConcurrency = 32;
    /// 同时处于"已解析未释放"状态的项目，其文件数之和的上限。单个项目超限时独占运行；
    /// 关键路径上的项目（优先级不低于任何尚未开工的项目）不受此限。
    public int MaxActiveFiles = 6000;
    /// 预读缓冲上限（字节）。
    public long PrefetchBytes = 256L << 20;
    public int BindConcurrency = Environment.ProcessorCount;
    /// 每释放这么多文件的项目就发起一次后台完整 GC。
    public int GcEveryFiles = 1500;
    public bool Profile;
}

sealed class IndexContext
{
    public required string Root;
    public required IndexOptions Options;
    public required Dictionary<string, ProjectInfo> Projects;
    /// 文件 → 所属项目（第一个列出它的项目）。
    public required Dictionary<string, string> FileOwner;
    public required Dictionary<string, CSharpParseOptions> ParseOptions;
    /// 磁盘文件的临时号（即最终路径序）。
    public required Dictionary<string, uint> FileIds;
    public required FileResult?[] Results;
    public required HashSet<string> ProjectAssemblies;
    public readonly ConcurrentBag<FileResult> Generated = new();
    public int NextGeneratedId;
    public readonly SymbolTable Symbols = new();
    public readonly ConcurrentBag<(SymbolKey from, SymbolKey to, RelKind kind)> Relations = new();
    /// DLL 引用缓存。读不出来的 DLL（被占用、损坏）记为 null，只警告一次，项目照常编译。
    public readonly ConcurrentDictionary<string, MetadataReference?> MetadataRefs = new(StringComparer.OrdinalIgnoreCase);
    /// 被引用到的外部类型，流水线结束后生成元数据视图（#34）。
    public readonly MetadataRequests Metadata = new();
    public readonly ConcurrentDictionary<string, AnalyzerFileReference> AnalyzerRefs = new(StringComparer.OrdinalIgnoreCase);
    public readonly AnalyzerLoader Loader = new();
    public required SemaphoreSlim Io;
    public required FilePrefetcher Prefetch;
    public int Missing;
    int _boundFiles;

    /// 进度协议：每跨过 1% 在 stderr 上输出一行 `progress <已绑定文件> <磁盘文件总数>`（不带时间前缀），
    /// 供 Rove 的索引生命周期换算进度条（`crates/rove-lang-cs/src/lifecycle/process.rs` 的 `Progress::parse`）。
    public void FileBound()
    {
        int total = Results.Length;
        int done = Interlocked.Increment(ref _boundFiles);
        if (total > 0 && done <= total && (long)done * 100 / total != (long)(done - 1) * 100 / total)
            Console.Error.WriteLine($"progress {done} {total}");
    }

    public string RelativePath(string fullPath) => Path.GetRelativePath(Root, fullPath).Replace('\\', '/');
}

static class Pipeline
{
    enum Event { ImageReady, Done }

    public static void Run(IndexContext ctx)
    {
        var projects = ctx.Projects.Values.OrderBy(p => p.Path, StringComparer.OrdinalIgnoreCase).ToArray();
        int n = projects.Length;
        var index = projects.Select((p, i) => (p.Path, i)).ToDictionary(x => x.Path, x => x.i, StringComparer.OrdinalIgnoreCase);
        var deps = projects.Select((p, i) => p.ProjectRefs.Where(index.ContainsKey).Select(r => index[r]).Where(d => d != i)
            .Distinct().ToList()).ToArray();
        BreakCycles(projects, deps);
        var dependents = Enumerable.Range(0, n).Select(_ => new List<int>()).ToArray();
        for (int i = 0; i < n; i++) foreach (int d in deps[i]) dependents[d].Add(i);
        var owned = Enumerable.Range(0, n).Select(_ => new List<string>()).ToArray();
        foreach (var (file, owner) in ctx.FileOwner) owned[index[owner]].Add(file);
        foreach (var list in owned) list.Sort(StringComparer.OrdinalIgnoreCase);
        var weight = owned.Select(o => Math.Max(1, o.Count)).ToArray();

        // 优先级 = 自身文件数 + 下游最长链上的文件数：关键路径上的项目先开工。
        var topo = TopoOrder(n, deps, dependents);
        var priority = new long[n];
        for (int k = n - 1; k >= 0; k--)
        {
            int i = topo[k];
            priority[i] = weight[i] + dependents[i].Select(d => priority[d]).DefaultIfEmpty(0).Max();
        }
        ctx.Prefetch.Start(SimulatedOrder(n, deps, dependents, priority).SelectMany(i => owned[i]));
        using var workers = new BindWorkers(ctx.Options.BindConcurrency);

        var images = new MetadataReference?[n];
        var pendingDependents = dependents.Select(d => d.Count).ToArray();
        var pendingDeps = deps.Select(d => d.Count).ToArray();
        var events = new BlockingCollection<(Event kind, int project)>();
        var ready = new PriorityQueue<int, (long, int)>();
        for (int i = 0; i < n; i++) if (pendingDeps[i] == 0) ready.Enqueue(i, (-priority[i], i));
        int done = 0, active = 0, imagesPending = 0;
        long activeFiles = 0, releasedFiles = 0;
        var started = new bool[n];
        // 准入：活动项目的文件数之和不超过上限；但关键路径上的项目（优先级不低于任何尚未开工的项目）
        // 不受上限约束，否则它会排在一批低优先级项目后面，拖长整条链。
        bool Admit(int next)
        {
            if (active == 0 || activeFiles + weight[next] <= ctx.Options.MaxActiveFiles) return true;
            for (int j = 0; j < n; j++)
                if (!started[j] && j != next && priority[j] > priority[next]) return false;
            return true;
        }
        while (done < n)
        {
            while (ready.TryPeek(out int next, out _) && Admit(next))
            {
                ready.Dequeue();
                started[next] = true;
                active++;
                imagesPending++;
                activeFiles += weight[next];
                int i = next;
                MetadataReference?[] depImages = deps[i].Select(d => images[d]).ToArray();
                // 下游都拿到镜像后，调度器不再持有它（编译里的引用随下游释放）。
                foreach (int d in deps[i])
                    if (--pendingDependents[d] == 0) images[d] = null;
                Task.Run(() => RunProject(ctx, workers, priority[i], projects[i], owned[i], depImages, dependents[i].Count > 0,
                    image => { images[i] = image; events.Add((Event.ImageReady, i)); },
                    () => events.Add((Event.Done, i))));
            }
            // 绑定可能先于镜像输出完成（两者并行），所以 Done 可能早于 ImageReady 到达；只有两者都没有在途时才算卡死。
            if (ready.Count == 0 && active == 0 && imagesPending == 0 && done < n)
                throw new InvalidOperationException("project scheduler stalled");
            var (kind, project) = events.Take();
            if (kind == Event.ImageReady)
            {
                imagesPending--;
                foreach (int d in dependents[project])
                    if (--pendingDeps[d] == 0) ready.Enqueue(d, (-priority[d], d));
            }
            else
            {
                done++;
                active--;
                activeFiles -= weight[project];
                // 释放的项目数据活得够久，已晋升到第 2 代；服务器 GC 要等第 2 代预算涨满才回收，
                // 工作集会一路涨到存活量的两倍。每释放一批文件就主动发起一次后台完整 GC。
                releasedFiles += weight[project];
                if (releasedFiles >= ctx.Options.GcEveryFiles)
                {
                    releasedFiles = 0;
                    GC.Collect(2, GCCollectionMode.Forced, blocking: false);
                }
            }
        }
        ctx.Prefetch.Stop();
    }

    /// 前段（读取、解析、编译、镜像）在线程池上运行，期间临时提到高于正常的线程优先级（Boost）：它在关键路径上，决定下游何时能开工。
    /// 绑定拆成逐文件的工作项交给 BindWorkers，按项目优先级排队，最后一个文件完成时项目结束。
    static void RunProject(IndexContext ctx, BindWorkers workers, long priority, ProjectInfo proj, List<string> owned,
        MetadataReference?[] depImages, bool hasDependents, Action<MetadataReference?> imageReady, Action done)
    {
        long tStart = Stats.Now();
        bool bindingQueued = false;
        bool imagePosted = false;
        void PostImage(MetadataReference? image)
        {
            if (imagePosted) return;
            imagePosted = true;
            imageReady(image);
        }
        try
        {
            using var boost = new Boost();
            var po = ctx.ParseOptions[proj.Path];
            var parallel = new ParallelOptions { MaxDegreeOfParallelism = ctx.Options.BindConcurrency };

            // 1. 读文件 + 解析。预处理指令按所属项目的宏求值。
            var parsed = new (SyntaxTree tree, ulong hash, Utf8Coordinates coordinates)?[owned.Count];
            Parallel.For(0, owned.Count, parallel, k =>
            {
                using var parseBoost = new Boost();
                var bytes = ReadSource(ctx, owned[k]);
                if (bytes == null) { Interlocked.Increment(ref ctx.Missing); return; }
                long t = Stats.Now();
                var hash = System.IO.Hashing.XxHash64.HashToUInt64(bytes);
                var text = Microsoft.CodeAnalysis.Text.SourceText.From(new MemoryStream(bytes), throwIfBinaryDetected: false, canBeEmbedded: false);
                parsed[k] = (CSharpSyntaxTree.ParseText(text, po, path: owned[k]), hash, new Utf8Coordinates(text));
                Stats.Add(ref Stats.Parse, t);
            });
            var treeFiles = new Dictionary<SyntaxTree, (uint id, Utf8Coordinates coordinates)>(ReferenceEqualityComparer.Instance);
            // 编译中的树保持 csproj 中的顺序：partial 类型的声明位置顺序（类型参数的局部键取第一处）依赖它。
            var trees = new List<SyntaxTree>(proj.Sources.Length);
            var ownedIndex = new Dictionary<string, int>(owned.Count, StringComparer.OrdinalIgnoreCase);
            for (int k = 0; k < owned.Count; k++) ownedIndex[owned[k]] = k;
            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var s in proj.Sources)
            {
                if (!seen.Add(s)) continue;
                if (ownedIndex.TryGetValue(s, out int k))
                {
                    if (parsed[k] is { } p) { treeFiles[p.tree] = (ctx.FileIds[s], p.coordinates); trees.Add(p.tree); }
                    continue;
                }
                // 同一文件被多个项目包含时（少见），非所属项目按自己的宏重新解析；文件号仍用所属项目的。
                if (!ctx.FileOwner.ContainsKey(s)) continue;
                var bytes = ReadSource(ctx, s, prefetched: false);
                if (bytes == null) continue;
                long t = Stats.Now();
                var text = Microsoft.CodeAnalysis.Text.SourceText.From(new MemoryStream(bytes), throwIfBinaryDetected: false, canBeEmbedded: false);
                var tree = CSharpSyntaxTree.ParseText(text, po, path: s);
                Stats.Add(ref Stats.Parse, t);
                treeFiles[tree] = (ctx.FileIds[s], new Utf8Coordinates(text));
                trees.Add(tree);
            }

            // 2. 构造编译
            long tc = Stats.Now();
            var refs = new List<MetadataReference>();
            foreach (var r in proj.References)
                if (File.Exists(r) && ctx.MetadataRefs.GetOrAdd(r, LoadReference) is { } reference) refs.Add(reference);
            refs.AddRange(depImages.OfType<MetadataReference>());
            var opts = new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary,
                allowUnsafe: proj.Unsafe, concurrentBuild: true,
                metadataImportOptions: MetadataImportOptions.Internal);
            var comp = CSharpCompilation.Create(proj.AssemblyName, trees, refs, opts);
            Stats.Add(ref Stats.Compile, tc);
            SyntaxTree[] generated = [];
            if (ctx.Options.Generators && proj.Analyzers.Length > 0)
                (comp, generated) = RunGenerators(ctx, proj, comp, po);
            if (ctx.Options.DiagAssembly == proj.AssemblyName)
            {
                foreach (var d in comp.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error).GroupBy(d => d.Id).OrderByDescending(g => g.Count()).Take(8))
                    Program.Log($"diag {d.Key} x{d.Count()}: {d.First().GetMessage()} @ {d.First().Location.GetLineSpan()}");
            }

            if (ctx.Options.DiagAssembly != null)
            {
                PostImage(hasDependents ? EmitImage(comp) : null);
                return;
            }

            // 3. 绑定 + 收集：拆成逐文件的工作项排进 BindWorkers
            var genFiles = new List<(SyntaxTree tree, uint id, byte[] path, ulong hash)>();
            if (generated.Length > 0)
            {
                var prefix = "rove-generated://" + Uri.EscapeDataString(ctx.RelativePath(proj.Path)) + "/";
                var ordinals = new Dictionary<string, int>(StringComparer.Ordinal);
                foreach (var tree in generated)
                {
                    var path = prefix + Uri.EscapeDataString(tree.FilePath.Replace('\\', '/'));
                    ordinals.TryGetValue(path, out int ordinal);
                    ordinals[path] = ordinal + 1;
                    if (ordinal != 0) path += "#" + ordinal;
                    var text = tree.GetText();
                    var hash = System.IO.Hashing.XxHash64.HashToUInt64(Encoding.UTF8.GetBytes(text.ToString()));
                    uint id = (uint)Interlocked.Increment(ref ctx.NextGeneratedId);
                    treeFiles[tree] = (id, new Utf8Coordinates(text));
                    genFiles.Add((tree, id, Encoding.UTF8.GetBytes(path), hash));
                }
            }
            var binder = new ProjectBinder(ctx, comp, treeFiles);
            Interlocked.Add(ref Stats.GeneratedTrees, genFiles.Count);
            var items = new List<Action>(owned.Count + genFiles.Count);
            for (int k = 0; k < owned.Count; k++)
            {
                if (parsed[k] is not { } p) continue;
                int index = k;
                uint id = ctx.FileIds[owned[k]];
                items.Add(() =>
                {
                    ctx.Results[id] = binder.Bind(p.tree, id, Encoding.UTF8.GetBytes(ctx.RelativePath(owned[index])), p.hash);
                    ctx.FileBound();
                    parsed[index] = null;
                });
            }
            foreach (var g in genFiles) items.Add(() => ctx.Generated.Add(binder.Bind(g.tree, g.id, g.path, g.hash)));
            if (items.Count > 0)
            {
                int remaining = items.Count;
                bindingQueued = true;
                for (int k = 0; k < items.Count; k++)
                {
                    var item = items[k];
                    workers.Enqueue(() =>
                    {
                        try { item(); }
                        catch (Exception e) { Program.Log($"binding in {proj.AssemblyName} failed: {e}"); }
                        if (Interlocked.Decrement(ref remaining) == 0)
                        {
                            if (ctx.Options.Profile) Program.Log($"project {proj.AssemblyName}: done");
                            done();
                        }
                    }, (-priority, k));
                }
            }
            // 4. 元数据镜像：下游从这里开始就可以开工。绑定已在工作线程上进行，
            //    两边共享编译里按需补全的符号，镜像输出不必独自完成全部声明绑定。
            long tImage = Stats.Now();
            PostImage(hasDependents ? EmitImage(comp) : null);
            if (ctx.Options.Profile)
                Program.Log($"project {proj.AssemblyName}: files={owned.Count} start={Program.Elapsed(tStart):F1}s compiled={Program.Elapsed(tImage):F1}s image={Program.Elapsed(Stats.Now()):F1}s");
        }
        catch (Exception e)
        {
            Program.Log($"project {proj.AssemblyName} failed: {e}");
        }
        finally
        {
            PostImage(null);
            if (!bindingQueued) done();
        }
    }

    /// 先按 Roslyn 的方式打开；被其他进程以不兼容的共享方式占用时，改用可共享读取重试，仍失败则跳过并警告。
    static MetadataReference? LoadReference(string path)
    {
        try { return MetadataReference.CreateFromFile(path); }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or BadImageFormatException)
        {
            try
            {
                if (Program.ReadShared(path) is { } bytes)
                    return MetadataReference.CreateFromImage(ImmutableCollectionsMarshal.AsImmutableArray(bytes), filePath: path);
            }
            catch (BadImageFormatException) { }
            MetadataViews.Warn($"reference {path} is unreadable ({e.GetType().Name}: {e.Message}); skipped");
            return null;
        }
    }

    static byte[]? ReadSource(IndexContext ctx, string path, bool prefetched = true)
    {
        if (prefetched && ctx.Prefetch.TryTake(path, out var bytes)) return bytes;
        long t = Stats.Now();
        ctx.Io.Wait();
        try { bytes = Program.ReadShared(path); }
        finally { ctx.Io.Release(); }
        Stats.Add(ref Stats.Read, t);
        if (bytes != null) Interlocked.Add(ref Stats.ReadBytes, bytes.Length);
        return bytes;
    }

    static readonly EmitOptions ImageOptions = new(metadataOnly: true, tolerateErrors: true, includePrivateMembers: true);

    static MetadataReference EmitImage(CSharpCompilation comp)
    {
        long t = Stats.Now();
        try
        {
            if (TryEmit(comp, out var image, out var errors)) return image;
            // tolerateErrors 也挡不住某些声明错误（例如 csproj 声明 C# 9 而代码用了 C# 10 特性，Unity 的编译器更宽松）。
            // 这时下游直接引用这个编译：语义与 M0 完全相同，代价是它要活到最后一个下游绑定完。
            // （按更高语言版本重建一个编译再输出也可行，但要把整套声明绑定再做一遍，最大的项目上要多花十几秒。）
            Program.Log($"metadata image of {comp.AssemblyName} failed ({errors}); dependents reference the compilation");
        }
        catch (Exception e)
        {
            Program.Log($"metadata image of {comp.AssemblyName} failed ({e.GetType().Name} {e.Message}); dependents reference the compilation");
        }
        finally { Stats.Add(ref Stats.Emit, t); }
        Interlocked.Increment(ref Stats.ImageFailures);
        return comp.ToMetadataReference();
    }

    static bool TryEmit(CSharpCompilation comp, out MetadataReference image, out string errors)
    {
        using var ms = new MemoryStream();
        var result = comp.Emit(ms, options: ImageOptions);
        if (result.Success)
        {
            Interlocked.Increment(ref Stats.Images);
            image = MetadataReference.CreateFromImage(ImmutableCollectionsMarshal.AsImmutableArray(ms.ToArray()),
                filePath: comp.AssemblyName + ".dll");
            errors = "";
            return true;
        }
        image = null!;
        errors = string.Join("; ", result.Diagnostics.Where(d => d.Severity == DiagnosticSeverity.Error)
            .GroupBy(d => d.Id).Select(g => $"{g.Key} x{g.Count()}: {g.First().GetMessage()} @ {g.First().Location.GetLineSpan()}").Take(5));
        return false;
    }

    static (CSharpCompilation, SyntaxTree[]) RunGenerators(IndexContext ctx, ProjectInfo proj, CSharpCompilation comp, CSharpParseOptions po)
    {
        long t = Stats.Now();
        try
        {
            var gens = proj.Analyzers.Where(File.Exists)
                .SelectMany(a => ctx.AnalyzerRefs.GetOrAdd(a, x =>
                {
                    var reference = new AnalyzerFileReference(x, ctx.Loader);
                    reference.AnalyzerLoadFailed += (_, e) =>
                        Stats.GeneratorIssues.Enqueue($"load {Path.GetFileName(x)}: {e.ErrorCode} {e.Message}");
                    return reference;
                }).GetGenerators(LanguageNames.CSharp))
                .ToArray();
            if (gens.Length == 0) return (comp, []);
            GeneratorDriver driver = CSharpGeneratorDriver.Create(gens, parseOptions: po);
            driver = driver.RunGeneratorsAndUpdateCompilation(comp, out var newComp, out var diagnostics);
            foreach (var d in diagnostics.Where(d => d.Severity >= DiagnosticSeverity.Warning))
                Stats.GeneratorIssues.Enqueue($"{proj.AssemblyName}: {d.Id} {d.GetMessage()}");
            return ((CSharpCompilation)newComp, driver.GetRunResult().GeneratedTrees.ToArray());
        }
        catch (Exception e)
        {
            Program.Log($"generator failed in {proj.AssemblyName}: {e.GetType().Name} {e.Message}");
            return (comp, []);
        }
        finally { Stats.Add(ref Stats.Generators, t); }
    }

    /// Unity 项目不应有循环引用；万一有，丢掉环内的边并记录，保证调度能推进。
    static void BreakCycles(ProjectInfo[] projects, List<int>[] deps)
    {
        int n = deps.Length;
        var remaining = new HashSet<int>(Enumerable.Range(0, n));
        while (true)
        {
            bool progress = true;
            while (progress)
            {
                progress = false;
                foreach (int i in remaining.ToArray())
                    if (deps[i].All(d => !remaining.Contains(d))) { remaining.Remove(i); progress = true; }
            }
            if (remaining.Count == 0) return;
            int victim = remaining.Min();
            Program.Log($"project reference cycle: dropping references of {projects[victim].AssemblyName} inside the cycle");
            deps[victim].RemoveAll(remaining.Contains);
        }
    }

    static int[] TopoOrder(int n, List<int>[] deps, List<int>[] dependents)
    {
        var pending = deps.Select(d => d.Count).ToArray();
        var queue = new Queue<int>(Enumerable.Range(0, n).Where(i => pending[i] == 0));
        var order = new List<int>(n);
        while (queue.Count > 0)
        {
            int i = queue.Dequeue();
            order.Add(i);
            foreach (int d in dependents[i]) if (--pending[d] == 0) queue.Enqueue(d);
        }
        return order.ToArray();
    }

    /// 预读顺序：按调度器的优先级模拟一遍开工顺序。
    static IEnumerable<int> SimulatedOrder(int n, List<int>[] deps, List<int>[] dependents, long[] priority)
    {
        var pending = deps.Select(d => d.Count).ToArray();
        var queue = new PriorityQueue<int, (long, int)>();
        for (int i = 0; i < n; i++) if (pending[i] == 0) queue.Enqueue(i, (-priority[i], i));
        var order = new List<int>(n);
        while (queue.TryDequeue(out int i, out _))
        {
            order.Add(i);
            foreach (int d in dependents[i]) if (--pending[d] == 0) queue.Enqueue(d, (-priority[d], d));
        }
        return order;
    }
}

/// 前段工作期间把当前线程提到高于正常，结束时恢复（线程池线程会被复用）。
readonly struct Boost : IDisposable
{
    readonly ThreadPriority _previous;
    public Boost()
    {
        _previous = Thread.CurrentThread.Priority;
        Thread.CurrentThread.Priority = ThreadPriority.AboveNormal;
    }
    public void Dispose() => Thread.CurrentThread.Priority = _previous;
}

/// 绑定工作线程，正常优先级（不能低于正常：用户机器上同时开着 Unity 等程序时会被饿死）。
/// 关键路径上的前段改为临时提升线程优先级（见 Boost）。
/// 队列按项目优先级（关键路径长者优先）排序。
sealed class BindWorkers : IDisposable
{
    readonly PriorityQueue<Action, (long, int)> _queue = new();
    readonly Thread[] _threads;
    bool _completed;

    public BindWorkers(int count)
    {
        _threads = Enumerable.Range(0, Math.Max(1, count)).Select(i => new Thread(Work)
        {
            IsBackground = true, Name = "bind-" + i,
        }).ToArray();
        foreach (var t in _threads) t.Start();
    }

    public void Enqueue(Action item, (long, int) priority)
    {
        lock (_queue)
        {
            _queue.Enqueue(item, priority);
            Monitor.Pulse(_queue);
        }
    }

    void Work()
    {
        while (true)
        {
            Action item;
            lock (_queue)
            {
                while (_queue.Count == 0 && !_completed) Monitor.Wait(_queue);
                if (!_queue.TryDequeue(out item!, out _)) return;
            }
            item();
        }
    }

    public void Dispose()
    {
        lock (_queue)
        {
            _completed = true;
            Monitor.PulseAll(_queue);
        }
        foreach (var t in _threads) t.Join();
    }
}

/// 按调度顺序预读源文件，读取与解析/绑定重叠。缓冲有上限；读到的内容被取走后释放额度。
sealed class FilePrefetcher(SemaphoreSlim io, long budget, int threads)
{
    readonly ConcurrentDictionary<string, byte[]?> _ready = new(StringComparer.OrdinalIgnoreCase);
    readonly ConcurrentDictionary<string, byte> _taken = new(StringComparer.OrdinalIgnoreCase);
    readonly object _gate = new();
    readonly CancellationTokenSource _stop = new();
    long _bytes;
    Task[] _workers = [];

    public void Start(IEnumerable<string> order)
    {
        var queue = new ConcurrentQueue<string>(order);
        _workers = Enumerable.Range(0, Math.Max(1, threads)).Select(_ => Task.Factory.StartNew(() =>
        {
            while (!_stop.IsCancellationRequested && queue.TryDequeue(out var path))
            {
                if (_taken.ContainsKey(path)) continue;
                lock (_gate)
                    while (_bytes > budget && !_stop.IsCancellationRequested) Monitor.Wait(_gate, 100);
                if (_stop.IsCancellationRequested) return;
                long t = Stats.Now();
                byte[]? bytes;
                io.Wait();
                try
                {
                    if (_taken.ContainsKey(path)) continue;
                    bytes = Program.ReadShared(path);
                }
                finally { io.Release(); }
                Stats.Add(ref Stats.Read, t);
                if (bytes != null) Interlocked.Add(ref Stats.ReadBytes, bytes.Length);
                lock (_gate)
                {
                    if (_taken.ContainsKey(path)) continue;
                    _ready[path] = bytes;
                    _bytes += bytes?.Length ?? 0;
                }
            }
        }, TaskCreationOptions.LongRunning)).ToArray();
    }

    public bool TryTake(string path, out byte[]? bytes)
    {
        lock (_gate)
        {
            _taken[path] = 0;
            if (!_ready.TryRemove(path, out bytes)) return false;
            _bytes -= bytes?.Length ?? 0;
            Monitor.PulseAll(_gate);
            return true;
        }
    }

    public void Stop()
    {
        _stop.Cancel();
        lock (_gate) Monitor.PulseAll(_gate);
        Task.WaitAll(_workers);
    }
}

/// 每个分析器目录一个加载上下文，依赖从同目录解析；Roslyn 与运行时程序集与宿主共享。
/// M0 用默认上下文直接加载，依赖解析不到时生成器静默失败。
sealed class AnalyzerLoader : IAnalyzerAssemblyLoader
{
    readonly ConcurrentDictionary<string, DirectoryContext> _contexts = new(StringComparer.OrdinalIgnoreCase);

    public void AddDependencyLocation(string fullPath) { }

    public Assembly LoadFromPath(string fullPath)
    {
        var dir = Path.GetDirectoryName(fullPath)!;
        return _contexts.GetOrAdd(dir, d => new DirectoryContext(d)).LoadFromAssemblyPath(fullPath);
    }

    sealed class DirectoryContext(string dir) : AssemblyLoadContext("analyzers:" + dir)
    {
        protected override Assembly? Load(AssemblyName name)
        {
            var simple = name.Name ?? "";
            if (simple.StartsWith("Microsoft.CodeAnalysis", StringComparison.OrdinalIgnoreCase)
                || simple.StartsWith("System.", StringComparison.OrdinalIgnoreCase)
                || simple is "System" or "netstandard" or "mscorlib")
                return null;
            var candidate = Path.Combine(dir, simple + ".dll");
            return File.Exists(candidate) ? LoadFromAssemblyPath(candidate) : null;
        }
    }
}
