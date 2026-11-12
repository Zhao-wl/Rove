// rove-indexer-cs：不走 MSBuild，直接解析 Unity 生成的 .csproj 构造 Roslyn 编译，
// 生成预计算的语义索引（定义/引用/关系），供 Rove 主进程内存映射查询。
// 用法: rove-indexer-cs <unity-project-dir> <out.idx> [--generators] [--report <report.json>]
//       [--io-concurrency N] [--max-active-files N] [--prefetch-mb N] [--bind-concurrency N] [--gc-every-files N]
//       [--soft-memory-mb N]
//       [--no-reconcile] [--diag <AssemblyName>] [--profile]

using System.Diagnostics;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace Rove.Indexer;

static class Program
{
    static readonly Stopwatch Total = Stopwatch.StartNew();
    static readonly Dictionary<string, double> Phases = new();

    const string Usage = "usage: rove-indexer-cs <project-dir> <out.idx> [--generators] [--report file] [--io-concurrency N] "
        + "[--max-active-files N] [--prefetch-mb N] [--bind-concurrency N] [--gc-every-files N] [--soft-memory-mb N] [--no-reconcile] [--diag asm] [--profile]";

    static int Main(string[] args)
    {
        if (args.Length < 2) { Console.Error.WriteLine(Usage); return 2; }
        var root = Path.GetFullPath(args[0]);
        var outPath = Path.GetFullPath(args[1]);
        string? Value(string name)
        {
            int i = Array.IndexOf(args, name);
            return i >= 0 && i + 1 < args.Length ? args[i + 1] : null;
        }
        int? Int(string name)
        {
            if (Value(name) is not { } v) return null;
            if (int.TryParse(v, out int n) && n > 0) return n;
            throw new ArgumentException($"{name} expects a positive integer, got '{v}'");
        }
        var options = new IndexOptions
        {
            Generators = args.Contains("--generators"),
            DiagAssembly = Value("--diag"),
            Profile = args.Contains("--profile"),
        };
        options.IoConcurrency = Int("--io-concurrency") ?? options.IoConcurrency;
        options.MaxActiveFiles = Int("--max-active-files") ?? options.MaxActiveFiles;
        options.PrefetchBytes = (Int("--prefetch-mb") ?? (int)(options.PrefetchBytes >> 20)) * (1L << 20);
        options.BindConcurrency = Int("--bind-concurrency") ?? options.BindConcurrency;
        options.GcEveryFiles = Int("--gc-every-files") ?? options.GcEveryFiles;
        string? reportPath = Value("--report");
        bool reconcile = !args.Contains("--no-reconcile");
        using var sampler = new MemorySampler(options.Profile, (Int("--soft-memory-mb") ?? 2200) * (1L << 20));

        // 1. 解析 csproj
        var sw = Stopwatch.StartNew();
        var projects = Directory.GetFiles(root, "*.csproj").Select(ParseCsproj).Where(p => p != null).Cast<ProjectInfo>()
            .ToDictionary(p => p.Path, StringComparer.OrdinalIgnoreCase);
        Log($"projects: {projects.Count}, source entries: {projects.Values.Sum(p => p.Sources.Length)}");
        if (reconcile)
        {
            var (added, removed) = UnityProjectModel.Reconcile(root, projects);
            Log($"reconcile with disk: +{added} files not in csproj, -{removed} listed but missing");
        }
        var configHash = ProjectConfigHash.Compute(root, projects.Values, options.Generators, reconcile);
        Phase("parse_csproj", sw);

        // 文件归属第一个列出它的项目（按项目路径序）；磁盘文件按最终的路径字节序预先编号。
        var fileOwner = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var parseOpts = new Dictionary<string, CSharpParseOptions>(StringComparer.OrdinalIgnoreCase);
        foreach (var p in projects.Values.OrderBy(p => p.Path, StringComparer.OrdinalIgnoreCase))
        {
            parseOpts[p.Path] = new CSharpParseOptions(p.LangVersion, DocumentationMode.None, SourceCodeKind.Regular, p.Defines);
            foreach (var s in p.Sources) fileOwner.TryAdd(s, p.Path);
        }
        string RelativePath(string fullPath) => Path.GetRelativePath(root, fullPath).Replace('\\', '/');
        var ordered = fileOwner.Keys.Select(f => (f, path: Encoding.UTF8.GetBytes(RelativePath(f)))).ToArray();
        Array.Sort(ordered, (a, b) => a.path.AsSpan().SequenceCompareTo(b.path));
        var fileIds = new Dictionary<string, uint>(ordered.Length, StringComparer.OrdinalIgnoreCase);
        for (int i = 0; i < ordered.Length; i++) fileIds[ordered[i].f] = (uint)i;

        var io = new SemaphoreSlim(options.IoConcurrency);
        var ctx = new IndexContext
        {
            Root = root,
            Options = options,
            Projects = projects,
            FileOwner = fileOwner,
            ParseOptions = parseOpts,
            FileIds = fileIds,
            Results = new FileResult?[ordered.Length],
            ProjectAssemblies = new HashSet<string>(projects.Values.Select(p => p.AssemblyName), StringComparer.Ordinal),
            Io = io,
            Prefetch = new FilePrefetcher(io, options.PrefetchBytes, options.IoConcurrency),
            NextGeneratedId = ordered.Length - 1,
        };

        // 2. 按依赖拓扑的流水线：读取、解析、编译、元数据镜像、绑定，项目完成即释放
        Pipeline.Run(ctx);
        Phase("index_projects", sw);
        if (options.Profile)
        {
            GC.Collect(2, GCCollectionMode.Forced, blocking: true, compacting: true);
            Log($"PROFILE retained after pipeline: {GC.GetTotalMemory(false) >> 20}MB (symbols {ctx.Symbols.Map.Count}, "
                + $"occurrences {ctx.Results.Sum(r => (long)(r?.Occs.Length ?? 0))})");
            sw.Restart();
        }
        Log($"files: {ctx.Results.Count(r => r != null)}, missing: {ctx.Missing}, symbols: {ctx.Symbols.Map.Count}, "
            + $"images: {Stats.Images} (+{Stats.ImageFailures} fallback)");
        foreach (var issue in Stats.GeneratorIssues.Distinct().Take(40)) Log($"generator: {issue}");
        if (options.DiagAssembly != null) return 0;

        // 3. 元数据视图（#34）：流水线已释放各项目的编译，这里用缓存的 DLL 引用统一生成被引用到的外部类型。
        var views = MetadataViews.Generate(ctx.Metadata,
            ctx.MetadataRefs.OrderBy(kv => kv.Key, StringComparer.OrdinalIgnoreCase).Select(kv => kv.Value).OfType<MetadataReference>(),
            ctx.Symbols, options.BindConcurrency);
        Log($"metadata views: {views.Count} types from {ctx.Metadata.AssemblyCount} assemblies, "
            + $"{views.Sum(v => (long)v.Text.Length) >> 10} K chars, warnings {MetadataViews.Warnings}");
        Phase("metadata_views", sw);

        // 4. 写索引。先把流水线留下的空闲堆还给系统，写入阶段的临时数组不再叠在上面。
        GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);
        if (options.Profile) Log($"write: gc {sw.Elapsed.TotalSeconds:F1}s");
        IndexWriter.Verbose = options.Profile;
        var files = ctx.Results.OfType<FileResult>().Concat(ctx.Generated).ToArray();
        Array.Sort(files, (a, b) => a.Path.AsSpan().SequenceCompareTo(b.Path));
        long namesTotal = Stats.Names;
        var metadata = new MetadataStats();
        var stats = IndexWriter.Write(outPath, configHash, files, ctx.Symbols, ctx.Relations, views, metadata);
        views = null!;
        Phase("write_index", sw);
        sampler.Sample();

        var proc = Process.GetCurrentProcess();
        var gc = GC.GetGCMemoryInfo();
        var report = new
        {
            project = root,
            generators = options.Generators,
            projects = projects.Count,
            files = files.Length,
            missing_files = ctx.Missing,
            phases_ms = Phases,
            total_ms = Total.Elapsed.TotalMilliseconds,
            cpu_seconds = Math.Round(proc.TotalProcessorTime.TotalSeconds, 1),
            peak_working_set_mb = Math.Max(proc.PeakWorkingSet64, sampler.PeakWorkingSet) / 1048576.0,
            peak_gc_heap_mb = sampler.PeakHeap / 1048576.0,
            peak_gc_committed_mb = sampler.PeakCommitted / 1048576.0,
            thread_seconds = Stats.ThreadSeconds(),
            gc_collections = new[] { GC.CollectionCount(0), GC.CollectionCount(1), GC.CollectionCount(2) },
            gc_pause_percent = gc.PauseTimePercentage,
            soft_limit_collections = sampler.SoftLimitCollections,
            options = new
            {
                io_concurrency = options.IoConcurrency,
                max_active_files = options.MaxActiveFiles,
                prefetch_mb = options.PrefetchBytes >> 20,
                bind_concurrency = options.BindConcurrency,
                gc_every_files = options.GcEveryFiles,
            },
            read_bytes = Stats.ReadBytes,
            metadata_images = Stats.Images,
            metadata_image_fallbacks = Stats.ImageFailures,
            names = namesTotal,
            unresolved_names = Stats.UnresolvedNames,
            unresolved_ratio = namesTotal == 0 ? 0 : (double)Stats.UnresolvedNames / namesTotal,
            candidate_names = Stats.CandidateNames,
            symbols = stats.symbols,
            occurrences = stats.occurrences,
            relations = stats.relations,
            index_bytes = stats.bytes,
            generated_trees = Stats.GeneratedTrees,
            generator_issues = Stats.GeneratorIssues.Distinct().Take(40).ToArray(),
            top_unresolved = Stats.Unresolved.OrderByDescending(kv => kv.Value).ThenBy(kv => kv.Key, StringComparer.Ordinal).Take(40).ToDictionary(kv => kv.Key, kv => kv.Value),
            metadata = metadata.Report(),
            unresolved_by_project = Stats.UnresolvedByProject.OrderByDescending(kv => kv.Value).ThenBy(kv => kv.Key, StringComparer.Ordinal).Take(20).ToDictionary(kv => kv.Key, kv => kv.Value),
        };
        var json = JsonSerializer.Serialize(report, new JsonSerializerOptions { WriteIndented = true, Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
        if (reportPath != null) File.WriteAllText(reportPath, json);
        Console.WriteLine(json);
        return 0;
    }

    static void Phase(string name, Stopwatch sw)
    {
        Phases[name] = Math.Round(sw.Elapsed.TotalMilliseconds);
        Log($"[{name}] {sw.Elapsed.TotalSeconds:F1}s  ws={Environment.WorkingSet / 1048576}MB");
        sw.Restart();
    }

    /// 以共享模式读取（Unity 编辑器、杀毒软件可能正占用文件），失败短暂重试。
    internal static byte[]? ReadShared(string path)
    {
        for (int attempt = 0; attempt < 5; attempt++)
        {
            try
            {
                using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
                var buf = new byte[fs.Length];
                fs.ReadExactly(buf);
                return buf;
            }
            catch (FileNotFoundException) { return null; }
            catch (DirectoryNotFoundException) { return null; }
            catch (IOException) { Thread.Sleep(20 << attempt); }
        }
        Log($"unreadable (locked): {path}");
        return null;
    }

    static readonly long StartTimestamp = Stopwatch.GetTimestamp();
    internal static double Elapsed(long timestamp) => (double)(timestamp - StartTimestamp) / Stopwatch.Frequency;
    internal static void Log(string s) => Console.Error.WriteLine($"{Total.Elapsed.TotalSeconds,7:F1}s  {s}");

    static ProjectInfo? ParseCsproj(string path)
    {
        try
        {
            var dir = Path.GetDirectoryName(path)!;
            var doc = XDocument.Load(path);
            IEnumerable<XElement> E(string n) => doc.Descendants().Where(e => e.Name.LocalName == n);
            string Full(string p) => Path.GetFullPath(Path.IsPathRooted(p) ? p : Path.Combine(dir, p));

            string? defines = E("DefineConstants")
                .FirstOrDefault(e => (e.Parent?.Attribute("Condition")?.Value ?? "").Contains("Debug"))?.Value
                ?? E("DefineConstants").FirstOrDefault()?.Value;
            var langStr = E("LangVersion").FirstOrDefault()?.Value ?? "latest";
            if (!LanguageVersionFacts.TryParse(langStr, out var lang)) lang = LanguageVersion.Latest;
            var unsafeStr = E("AllowUnsafeBlocks").FirstOrDefault()?.Value;
            return new ProjectInfo(
                Path.GetFullPath(path),
                E("AssemblyName").FirstOrDefault()?.Value ?? Path.GetFileNameWithoutExtension(path),
                E("Compile").Select(e => e.Attribute("Include")?.Value).OfType<string>().Where(s => s.EndsWith(".cs", StringComparison.OrdinalIgnoreCase)).Select(Full).ToArray(),
                E("Reference").Select(e => e.Element(e.Name.Namespace + "HintPath")?.Value).OfType<string>().Select(Full).ToArray(),
                E("ProjectReference").Select(e => e.Attribute("Include")?.Value).OfType<string>().Select(Full).ToArray(),
                E("Analyzer").Select(e => e.Attribute("Include")?.Value).OfType<string>().Select(Full).ToArray(),
                (defines ?? "").Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries),
                lang,
                string.Equals(unsafeStr, "true", StringComparison.OrdinalIgnoreCase));
        }
        catch (Exception e) { Log($"bad csproj {path}: {e.Message}"); return null; }
    }
}
