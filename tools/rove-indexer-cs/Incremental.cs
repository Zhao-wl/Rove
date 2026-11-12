// #35 增量索引引擎（常驻模式）：保留项目模型与按需“预热”的项目编译，文件变化时用 Roslyn 的增量编译
// （ReplaceSyntaxTree / AddSyntaxTrees / RemoveSyntaxTrees）只重绑变化的文件，结果写成覆盖层段。
//
// - 预处理宏：每个文件按所属项目的 DefineConstants 重新解析（与全量索引同一份 CSharpParseOptions）。
// - 读文件：共享模式读取并重试（Program.ReadShared）；仍被独占时放进重试列表，稍后再读，不中断进程。
// - 依赖项目以元数据镜像引用（与全量流水线相同）；声明变化的项目在它的下游重绑前重新输出镜像。
// - 影响分析见 Impact.cs：变化的键按“基础索引 + 覆盖层”的引用表反查，新名字在可见项目内文本查找，
//   受影响的文件进后台队列（可取消：前台请求优先、关闭即停；限速：按占空比休眠）。
// - 项目结构文件（.csproj / .asmdef / .asmref）变化不做增量，回复 rebuild_required，由 Rove 全量重建。

using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.Text;

namespace Rove.Indexer;

sealed class EngineOptions
{
    /// 同时保持预热的项目的文件数之和上限（超出时按最近使用淘汰）。大型项目（约 2.2 万个源文件）全部预热
    /// 也在上限内；空闲修剪时整体丢掉。
    public int MaxWarmFiles = 30000;
    /// 后台重绑的占空比：每批之后休眠（批耗时 × (1/占空比 − 1)），不把 CPU 吃满。
    public double BackgroundDuty = 0.5;
    /// 后台一批最多重绑的文件数（批之间检查前台请求与取消）。
    public int BackgroundBatch = 16;
}

/// 覆盖层里一个文件的“当前知识”：影响分析需要它的旧声明、旧局部键与引用过的键。
sealed class FileKnowledge
{
    public ulong Hash;
    public bool Deleted;
    public Dictionary<string, Decl> Decls = new(StringComparer.Ordinal);
    /// 声明的位置（命名实参回指参数声明时用）。
    public Dictionary<string, (uint line, uint col)> DeclPositions = new(StringComparer.Ordinal);
    public HashSet<OvKey> Refs = new();
    public List<OvKey> LocalDecls = new();
    public HashSet<OvKey> RelationTargets = new();
}

sealed class RebindResult
{
    public List<string> Files = new();
    public (ulong sequence, long offset, int length)? Segment;
    public List<string> Changed = new();
    public List<string> AddedNames = new();
    public int Queued;
    public List<string> Locked = new();
    public bool RebuildRequired;
    public double ElapsedMs;
}

sealed class Engine : IDisposable
{
    readonly string _root;
    readonly EngineOptions _options;
    BaseIndex _base;
    Journal _journal;
    Dictionary<ulong, ulong>? _baseFingerprints;

    // 项目模型
    readonly Dictionary<string, ProjectInfo> _projects;
    readonly ProjectInfo[] _order;
    readonly Dictionary<string, int> _projectIndex = new(StringComparer.OrdinalIgnoreCase);
    readonly List<int>[] _deps;
    readonly List<int>[] _dependents;
    readonly Dictionary<string, string> _fileOwner;
    readonly Dictionary<string, CSharpParseOptions> _parseOptions;
    readonly HashSet<string> _projectAssemblies;
    readonly ConcurrentDictionary<string, MetadataReference?> _metadataRefs = new(StringComparer.OrdinalIgnoreCase);

    // 预热状态
    sealed class Warm
    {
        public required CSharpCompilation Comp;
        public required Dictionary<string, SyntaxTree> Trees;
        public required Dictionary<SyntaxTree, (uint id, Utf8Coordinates coordinates)> TreeFiles;
        public required Dictionary<int, MetadataReference> DepImages;
        public long LastUsed;
        public int Files => Trees.Count;
    }
    readonly Dictionary<int, Warm> _warm = new();
    readonly Dictionary<int, MetadataReference> _images = new();
    readonly HashSet<int> _dirtyImages = new();
    long _clock;

    // 文件临时号（局部键用）：全路径 → 号，号 → 项目相对路径
    readonly Dictionary<string, uint> _tempIds = new(StringComparer.OrdinalIgnoreCase);
    readonly List<string> _tempPaths = new();

    // 覆盖层知识
    readonly Dictionary<string, FileKnowledge> _overlay = new(StringComparer.Ordinal);
    readonly Dictionary<OvKey, HashSet<string>> _overlayRefs = new();
    readonly Dictionary<OvKey, HashSet<string>> _overlayRelationTargets = new();
    readonly Dictionary<string, string> _overlayDeclFile = new(StringComparer.Ordinal);

    // 后台队列（项目相对路径，保持入队顺序）与读不到的文件
    readonly LinkedList<string> _queue = new();
    readonly HashSet<string> _queued = new(StringComparer.Ordinal);
    readonly Dictionary<string, (int attempts, long due)> _locked = new(StringComparer.Ordinal);

    public int Queued => _queue.Count;
    public int LockedCount => _locked.Count;
    public int OverlayFiles => _overlay.Count;
    public int Segments => _journal.Segments;
    public long JournalBytes => _journal.Length;
    public IEnumerable<string> WarmProjects => _warm.Keys.Select(i => _order[i].AssemblyName);
    public string BasePath => _base.Path;

    public Engine(string root, string basePath, string journalPath, EngineOptions options)
    {
        _root = Path.GetFullPath(root);
        _options = options;
        _base = new BaseIndex(basePath);
        _baseFingerprints = DeclarationTable.Read(basePath, _base.Identity);
        if (_baseFingerprints == null) Program.Log("declaration table missing: signature checks fall back to display names");
        _journal = new Journal(journalPath, _base.Identity, out var existing);
        foreach (var seg in existing) Learn(seg);
        Program.Log($"overlay journal: {existing.Count} segments, {_overlay.Count} files");

        _projects = Program.LoadProjects(_root, reconcile: true);
        (_fileOwner, _parseOptions) = Program.Ownership(_projects);
        _order = _projects.Values.OrderBy(p => p.Path, StringComparer.OrdinalIgnoreCase).ToArray();
        for (int i = 0; i < _order.Length; i++) _projectIndex[_order[i].Path] = i;
        _deps = _order.Select((p, i) => p.ProjectRefs.Where(_projectIndex.ContainsKey).Select(r => _projectIndex[r]).Where(d => d != i)
            .Distinct().ToList()).ToArray();
        _dependents = _order.Select(_ => new List<int>()).ToArray();
        for (int i = 0; i < _order.Length; i++) foreach (int d in _deps[i]) _dependents[d].Add(i);
        _projectAssemblies = new HashSet<string>(_order.Select(p => p.AssemblyName), StringComparer.Ordinal);
    }

    /// 切换到新的基础索引（全量重建完成后）：覆盖层知识清空，预热的编译保留（它们反映的就是磁盘）。
    public void Rebase(string basePath, string journalPath)
    {
        _journal.Dispose();
        _base.Dispose();
        _base = new BaseIndex(basePath);
        _baseFingerprints = DeclarationTable.Read(basePath, _base.Identity);
        _overlay.Clear();
        _overlayRefs.Clear();
        _overlayRelationTargets.Clear();
        _overlayDeclFile.Clear();
        _journal = new Journal(journalPath, _base.Identity, out var existing);
        foreach (var seg in existing) Learn(seg);
    }

    public string Rel(string full) => Path.GetRelativePath(_root, full).Replace('\\', '/');
    string Full(string rel) => Path.GetFullPath(Path.Combine(_root, rel));

    public static bool IsConfigPath(string rel)
    {
        var ext = Path.GetExtension(rel);
        return ext.Equals(".csproj", StringComparison.OrdinalIgnoreCase) || ext.Equals(".asmdef", StringComparison.OrdinalIgnoreCase)
            || ext.Equals(".asmref", StringComparison.OrdinalIgnoreCase);
    }

    uint TempId(string full)
    {
        if (_tempIds.TryGetValue(full, out uint id)) return id;
        id = (uint)_tempPaths.Count;
        _tempIds[full] = id;
        _tempPaths.Add(Rel(full));
        return id;
    }

    // ───────────── 覆盖层知识 ─────────────

    void Learn(OvSegment seg)
    {
        foreach (var f in seg.Files)
        {
            var k = new FileKnowledge { Hash = f.Hash, Deleted = f.Deleted };
            foreach (var (symbol, fp) in f.Declarations)
            {
                var s = seg.Symbols[symbol];
                if (s.Key.Global is not { } key) continue;
                k.Decls[key] = new Decl(fp, Fingerprints.Weak(s.Display, s.Kind), SearchName(s.Name, s.Container is { Global: { } c } ? Tail(c) : null));
            }
            foreach (var o in f.Occs)
            {
                var key = seg.Symbols[o.Sym].Key;
                k.Refs.Add(key);
                if ((o.Role & (ushort)Role.Def) == 0) continue;
                if (key.Path != null) k.LocalDecls.Add(key);
                else if (key.Global is { } g && k.Decls.ContainsKey(g)) k.DeclPositions.TryAdd(g, (o.Line, o.Col));
            }
            Replace(f.Path, k);
        }
        LearnRelations(seg);
    }

    static string Tail(string key)
    {
        int i = key.LastIndexOf('.');
        var tail = i >= 0 ? key[(i + 1)..] : key;
        int g = tail.IndexOf('`');
        return g >= 0 ? tail[..g] : tail;
    }

    static string SearchName(string name, string? containerName) =>
        name is ".ctor" or ".cctor" ? containerName ?? "" : Fingerprints.IsIdentifier(name) ? name : "";

    static void Index(Dictionary<OvKey, HashSet<string>> map, OvKey key, string path)
    {
        if (!map.TryGetValue(key, out var set)) map[key] = set = new(StringComparer.Ordinal);
        set.Add(path);
    }

    void Replace(string path, FileKnowledge k)
    {
        if (_overlay.TryGetValue(path, out var old))
        {
            foreach (var key in old.Refs) if (_overlayRefs.TryGetValue(key, out var set)) set.Remove(path);
            foreach (var key in old.RelationTargets) if (_overlayRelationTargets.TryGetValue(key, out var set)) set.Remove(path);
            foreach (var key in old.Decls.Keys) if (_overlayDeclFile.GetValueOrDefault(key) == path) _overlayDeclFile.Remove(key);
        }
        _overlay[path] = k;
        foreach (var key in k.Refs) Index(_overlayRefs, key, path);
        foreach (var key in k.Decls.Keys) _overlayDeclFile[key] = path;
    }

    /// 文件当前已知的内容哈希（覆盖层优先，其次基础索引）；都没有返回 null。
    ulong? KnownHash(string rel)
    {
        if (_overlay.TryGetValue(rel, out var k)) return k.Deleted ? null : k.Hash;
        int id = _base.FileId(rel);
        return id >= 0 ? _base.FileHash(id) : null;
    }

    /// 文件重绑前的声明（覆盖层，或基础索引 + 声明指纹表）。
    Dictionary<string, Decl> OldDecls(string rel)
    {
        if (_overlay.TryGetValue(rel, out var k)) return k.Decls;
        var result = new Dictionary<string, Decl>(StringComparer.Ordinal);
        int file = _base.FileId(rel);
        if (file < 0) return result;
        foreach (var o in _base.OccurrencesIn(file))
        {
            if ((o.Role & (ushort)Role.Def) == 0 || !_base.InSource(o.Symbol)) continue;
            byte kind = _base.Kind(o.Symbol);
            if (kind is not ((byte)SymbolKind.NamedType or (byte)SymbolKind.Method or (byte)SymbolKind.Property
                or (byte)SymbolKind.Field or (byte)SymbolKind.Event)) continue;
            if (_base.GlobalKey(o.Symbol) is not { } key || key.StartsWith('?') || result.ContainsKey(key)) continue;
            ulong fp = _baseFingerprints?.GetValueOrDefault(Fingerprints.Hash(key)) ?? 0;
            int container = _base.Container(o.Symbol);
            result[key] = new Decl(fp, Fingerprints.Weak(_base.Display(o.Symbol), kind),
                SearchName(_base.Name(o.Symbol), container >= 0 ? _base.Name(container) : null));
        }
        return result;
    }

    List<OvKey> OldLocals(string rel)
    {
        if (_overlay.TryGetValue(rel, out var k)) return k.LocalDecls;
        int file = _base.FileId(rel);
        if (file < 0) return [];
        return _base.LocalsDeclaredIn(file).Select(s => OvKey.Local(rel, _base.LocalKey(s)!.Value.offset)).ToList();
    }

    int BaseSymbol(OvKey key)
    {
        if (key.Global is { } g) return _base.FindGlobal(g);
        int file = _base.FileId(key.Path!);
        return file < 0 ? -1 : _base.FindLocal(file, key.Offset);
    }

    /// 引用了 key 的文件（含声明处），基础索引中被覆盖层取代的文件不算。
    void FilesReferencing(OvKey key, HashSet<string> into)
    {
        int s = BaseSymbol(key);
        if (s >= 0)
        {
            int last = -1;
            foreach (var o in _base.OccurrencesOf(s))
            {
                if (o.File == last) continue;
                last = o.File;
                var path = _base.FilePath(o.File);
                if (!_overlay.ContainsKey(path)) into.Add(path);
            }
            // 指向它的实现、重写与派生：这些声明所在的文件
            foreach (int from in _base.RelationSources(s))
                foreach (var o in _base.OccurrencesOf(from))
                {
                    if ((o.Role & (ushort)Role.Def) == 0) break;
                    var path = _base.FilePath(o.File);
                    if (!_overlay.ContainsKey(path)) into.Add(path);
                }
        }
        if (_overlayRefs.TryGetValue(key, out var files)) into.UnionWith(files);
        if (_overlayRelationTargets.TryGetValue(key, out files)) into.UnionWith(files);
    }

    // ───────────── 项目与编译 ─────────────

    int? OwnerOf(string full, bool exists)
    {
        if (_fileOwner.TryGetValue(full, out var owner)) return _projectIndex[owner];
        if (!exists) return null;
        // 新文件：按 Unity 的程序集归属规则（与全量索引的 Reconcile 一致）
        var asm = UnityProjectModel.AssemblyFor(full, _root);
        var project = _order.Select((p, i) => (p, i)).FirstOrDefault(x => string.Equals(x.p.AssemblyName, asm, StringComparison.OrdinalIgnoreCase));
        if (asm == null || project.p == null) return null;
        int index = project.i;
        _fileOwner[full] = project.p.Path;
        var info = _order[index] with { Sources = _order[index].Sources.Append(full).ToArray() };
        _order[index] = info;
        _projects[info.Path] = info;
        return index;
    }

    static SourceText Text(byte[] bytes) =>
        SourceText.From(new MemoryStream(bytes), throwIfBinaryDetected: false, canBeEmbedded: false);

    static readonly CSharpCompilationOptions BaseOptions = new(OutputKind.DynamicallyLinkedLibrary, concurrentBuild: true,
        metadataImportOptions: MetadataImportOptions.Internal);

    Warm EnsureWarm(int p, HashSet<string>? stale = null)
    {
        if (_warm.TryGetValue(p, out var warm)) { warm.LastUsed = ++_clock; return warm; }
        var watch = Stopwatch.StartNew();
        var proj = _order[p];
        PrepareImages(_deps[p]);
        var depImages = new Dictionary<int, MetadataReference>();
        foreach (int d in _deps[p]) if (EnsureImage(d) is { } image) depImages[d] = image;
        var po = _parseOptions[proj.Path];
        var sources = proj.Sources.Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
        var parsed = new (SyntaxTree tree, Utf8Coordinates coordinates, ulong hash)?[sources.Length];
        Parallel.For(0, sources.Length, k =>
        {
            var bytes = Program.ReadShared(sources[k]);
            if (bytes == null) return;
            var text = Text(bytes);
            parsed[k] = (CSharpSyntaxTree.ParseText(text, po, path: sources[k]), new Utf8Coordinates(text),
                System.IO.Hashing.XxHash64.HashToUInt64(bytes));
        });
        var trees = new Dictionary<string, SyntaxTree>(StringComparer.OrdinalIgnoreCase);
        var treeFiles = new Dictionary<SyntaxTree, (uint, Utf8Coordinates)>(ReferenceEqualityComparer.Instance);
        var list = new List<SyntaxTree>();
        for (int k = 0; k < sources.Length; k++)
        {
            if (parsed[k] is not { } x) continue;
            trees[sources[k]] = x.tree;
            treeFiles[x.tree] = (TempId(sources[k]), x.coordinates);
            list.Add(x.tree);
            // 预热时发现磁盘内容与索引不同（进程不在时改过）：一并重绑
            if (stale != null && _fileOwner.GetValueOrDefault(sources[k]) == proj.Path && KnownHash(Rel(sources[k])) != x.hash)
                stale.Add(Rel(sources[k]));
        }
        var refs = new List<MetadataReference>();
        foreach (var r in proj.References)
            if (File.Exists(r) && _metadataRefs.GetOrAdd(r, Pipeline.LoadReference) is { } reference) refs.Add(reference);
        refs.AddRange(depImages.Values);
        var comp = CSharpCompilation.Create(proj.AssemblyName, list, refs, BaseOptions.WithAllowUnsafe(proj.Unsafe));
        warm = new Warm { Comp = comp, Trees = trees, TreeFiles = treeFiles, DepImages = depImages, LastUsed = ++_clock };
        _warm[p] = warm;
        Program.Log($"warm {proj.AssemblyName}: {trees.Count} files in {watch.ElapsedMilliseconds} ms");
        Evict(keep: p);
        return warm;
    }

    void Evict(int keep)
    {
        while (_warm.Values.Sum(w => w.Files) > _options.MaxWarmFiles && _warm.Count > 1)
        {
            var victim = _warm.Where(kv => kv.Key != keep).OrderBy(kv => kv.Value.LastUsed).First().Key;
            _warm.Remove(victim);
        }
    }

    /// 冷启动时上游项目还没有镜像：按依赖层次并行地“读取 → 解析 → 编译 → 输出镜像”（不保留编译，
    /// 也不碰引擎的共享状态），否则第一次预热大项目要把几百个上游项目串行编译一遍。
    void PrepareImages(IEnumerable<int> roots)
    {
        var need = new HashSet<int>();
        var warm = new List<int>();
        void Collect(int d)
        {
            if (_images.ContainsKey(d) && !_dirtyImages.Contains(d)) return;
            // 已预热的项目直接从它的编译输出（最新的树）；否则从磁盘重新编译
            if (_warm.ContainsKey(d)) { if (!warm.Contains(d)) warm.Add(d); return; }
            if (!need.Add(d)) return;
            foreach (int x in _deps[d]) Collect(x);
        }
        foreach (int d in roots) Collect(d);
        foreach (int d in warm) EnsureImage(d);
        if (need.Count == 0) return;
        var watch = Stopwatch.StartNew();
        int total = need.Count;
        while (need.Count > 0)
        {
            var wave = need.Where(d => _deps[d].All(x => !need.Contains(x))).ToArray();
            if (wave.Length == 0) wave = [need.Min()]; // 环（不应出现）：按序号硬推进
            var built = new MetadataReference?[wave.Length];
            Parallel.For(0, wave.Length, new ParallelOptions { MaxDegreeOfParallelism = Math.Max(1, Environment.ProcessorCount / 2) }, k =>
            {
                int d = wave[k];
                var proj = _order[d];
                var po = _parseOptions[proj.Path];
                var trees = proj.Sources.Distinct(StringComparer.OrdinalIgnoreCase).AsParallel().AsOrdered()
                    .Select(f => Program.ReadShared(f) is { } bytes ? CSharpSyntaxTree.ParseText(Text(bytes), po, path: f) : null)
                    .OfType<SyntaxTree>().ToList();
                var refs = new List<MetadataReference>();
                foreach (var r in proj.References)
                    if (File.Exists(r) && _metadataRefs.GetOrAdd(r, Pipeline.LoadReference) is { } reference) refs.Add(reference);
                foreach (int x in _deps[d]) if (_images.TryGetValue(x, out var image)) refs.Add(image);
                var comp = CSharpCompilation.Create(proj.AssemblyName, trees, refs, BaseOptions.WithAllowUnsafe(proj.Unsafe));
                built[k] = Pipeline.EmitImage(comp);
            });
            for (int k = 0; k < wave.Length; k++)
            {
                if (built[k] is { } image) _images[wave[k]] = image;
                _dirtyImages.Remove(wave[k]);
                need.Remove(wave[k]);
            }
        }
        Program.Log($"images: {total} upstream projects in {watch.ElapsedMilliseconds} ms");
    }

    /// 依赖项目的元数据镜像（与全量流水线一样输出 metadata-only，失败时退回引用编译本身）。
    MetadataReference? EnsureImage(int d)
    {
        if (_images.TryGetValue(d, out var image) && !_dirtyImages.Contains(d)) return image;
        bool wasWarm = _warm.ContainsKey(d);
        var warm = EnsureWarm(d);
        var fresh = Pipeline.EmitImage(warm.Comp);
        _images[d] = fresh;
        _dirtyImages.Remove(d);
        if (!wasWarm) _warm.Remove(d);
        // 已预热的下游换上新镜像
        foreach (var (p, w) in _warm)
            if (w.DepImages.TryGetValue(d, out var old) && !ReferenceEquals(old, fresh))
            {
                w.Comp = w.Comp.ReplaceReference(old, fresh);
                w.DepImages[d] = fresh;
            }
        return fresh;
    }

    public void Trim()
    {
        _warm.Clear();
        _journal.Sync();
        GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);
    }

    public void Sync() => _journal.Sync();

    // ───────────── 重绑 ─────────────

    /// 重绑一批文件（项目相对路径）。前台请求与后台队列共用。
    public RebindResult Rebind(IReadOnlyCollection<string> rels, bool background)
    {
        var watch = Stopwatch.StartNew();
        var result = new RebindResult();
        var segment = new OvSegment();
        var byProject = new Dictionary<int, List<(string rel, string full, byte[]? bytes)>>();
        var orphans = new List<string>();
        foreach (var rel in rels.Distinct(StringComparer.Ordinal))
        {
            if (IsConfigPath(rel)) { result.RebuildRequired = true; continue; }
            if (!rel.EndsWith(".cs", StringComparison.OrdinalIgnoreCase)) continue;
            Dequeue(rel);
            var full = Full(rel);
            var bytes = Program.ReadShared(full);
            bool exists = bytes != null || File.Exists(full);
            if (bytes == null && exists)
            {
                // 被独占：稍后重试，不当成删除
                var (attempts, _) = _locked.GetValueOrDefault(rel);
                _locked[rel] = (attempts + 1, Stopwatch.GetTimestamp() + Stopwatch.Frequency * Math.Min(30, 1 << Math.Min(attempts, 5)) / 2);
                result.Locked.Add(rel);
                continue;
            }
            _locked.Remove(rel);
            var owner = OwnerOf(full, exists);
            if (owner is not { } p)
            {
                // 不属于任何项目（或删掉的文件本就不在项目里）：之前有索引数据就写“已清空”
                if (KnownHash(rel) != null) orphans.Add(rel);
                continue;
            }
            if (!byProject.TryGetValue(p, out var list)) byProject[p] = list = new();
            list.Add((rel, full, bytes));
        }
        if (result.RebuildRequired) return result;

        var impacted = new HashSet<string>(StringComparer.Ordinal);
        var batchFiles = new HashSet<string>(StringComparer.Ordinal);
        var stale = new HashSet<string>(StringComparer.Ordinal);
        var plans = new List<(string rel, Dictionary<string, Decl> before, List<OvKey> locals, FileKnowledge after, int project)>();
        var bound = new List<(OvFile file, FileKnowledge knowledge)>();

        foreach (var rel in orphans)
        {
            plans.Add((rel, OldDecls(rel), OldLocals(rel), new FileKnowledge { Deleted = true }, -1));
            var file = new OvFile { Path = rel, Deleted = true };
            segment.Files.Add(file);
            bound.Add((file, plans[^1].after));
            batchFiles.Add(rel);
        }

        // 按依赖拓扑序处理：先上游，下游绑定前刷新声明变化了的镜像
        foreach (int p in TopoOrder(byProject.Keys))
        {
            var files = byProject[p];
            var warm = EnsureWarm(p, stale);
            foreach (int d in _deps[p]) EnsureImage(d);
            var po = _parseOptions[_order[p].Path];
            var comp = warm.Comp;
            var fresh = new List<(string rel, string full, SyntaxTree tree, ulong hash)>();
            foreach (var (rel, full, bytes) in files)
            {
                warm.Trees.TryGetValue(full, out var oldTree);
                if (bytes == null)
                {
                    if (oldTree != null) { comp = comp.RemoveSyntaxTrees(oldTree); warm.Trees.Remove(full); warm.TreeFiles.Remove(oldTree); }
                    _fileOwner.Remove(full);
                    var info = _order[p] with { Sources = _order[p].Sources.Where(s => !s.Equals(full, StringComparison.OrdinalIgnoreCase)).ToArray() };
                    _order[p] = info;
                    _projects[info.Path] = info;
                    continue;
                }
                var text = Text(bytes);
                var tree = CSharpSyntaxTree.ParseText(text, po, path: full);
                if (oldTree != null) { comp = comp.ReplaceSyntaxTree(oldTree, tree); warm.TreeFiles.Remove(oldTree); }
                else comp = comp.AddSyntaxTrees(tree);
                warm.Trees[full] = tree;
                warm.TreeFiles[tree] = (TempId(full), new Utf8Coordinates(text));
                fresh.Add((rel, full, tree, System.IO.Hashing.XxHash64.HashToUInt64(bytes)));
            }
            warm.Comp = comp;

            foreach (var (rel, _, bytes) in files.Where(f => f.bytes == null))
            {
                var knowledge = new FileKnowledge { Deleted = true };
                plans.Add((rel, OldDecls(rel), OldLocals(rel), knowledge, p));
                var file = new OvFile { Path = rel, Deleted = true };
                segment.Files.Add(file);
                bound.Add((file, knowledge));
                batchFiles.Add(rel);
            }
            if (fresh.Count == 0) continue;
            var ctx = MiniContext();
            var binder = new ProjectBinder(ctx, comp, warm.TreeFiles);
            foreach (var (rel, full, tree, hash) in fresh)
            {
                var declared = new List<(ISymbol symbol, SymRec rec)>();
                binder.Declared = (s, r) => declared.Add((s, r));
                var bindResult = binder.Bind(tree, warm.TreeFiles[tree].id, Encoding.UTF8.GetBytes(rel), hash);
                binder.Declared = null;
                var (file, knowledge) = Emit(segment, ctx, binder, rel, bindResult, declared);
                plans.Add((rel, OldDecls(rel), OldLocals(rel), knowledge, p));
                bound.Add((file, knowledge));
                batchFiles.Add(rel);
            }
            // 声明变了：下游（包括本批里排在后面的项目）绑定前重新输出这个项目的镜像
            if (_dependents[p].Count > 0 && plans.Any(x => x.project == p && Impact.Diff(x.before, x.after.Decls) is var (c, a) && (c.Count > 0 || a.Count > 0)))
                _dirtyImages.Add(p);
        }

        if (segment.Files.Count > 0)
        {
            var (offset, length) = _journal.Append(segment);
            result.Segment = (segment.Sequence, offset, length);
        }

        // 影响分析（用重绑前的知识）→ 更新知识 → 入队
        var changedAll = new SortedSet<string>(StringComparer.Ordinal);
        var addedAll = new SortedSet<string>(StringComparer.Ordinal);
        var addedByProject = new Dictionary<int, HashSet<string>>();
        foreach (var (rel, before, locals, after, p) in plans)
        {
            var (changed, added) = Impact.Diff(before, after.Decls);
            foreach (var key in changed) FilesReferencing(OvKey.Of(key), impacted);
            // 本文件里声明、被其他文件引用的局部键（命名实参、partial 类型的类型参数）：偏移可能变了
            foreach (var local in locals) FilesReferencing(local, impacted);
            changedAll.UnionWith(changed);
            addedAll.UnionWith(added);
            if (added.Count > 0)
            {
                int key = Math.Max(p, -1);
                if (!addedByProject.TryGetValue(key, out var set)) addedByProject[key] = set = new(StringComparer.Ordinal);
                set.UnionWith(added);
            }
        }
        foreach (var (file, knowledge) in bound) Replace(file.Path, knowledge);
        LearnRelations(segment);
        foreach (var (p, names) in addedByProject) FilesNaming(p, names, impacted);
        impacted.UnionWith(stale);
        impacted.ExceptWith(batchFiles);
        foreach (var rel in impacted.Order(StringComparer.Ordinal)) Enqueue(rel);

        result.Files = batchFiles.Order(StringComparer.Ordinal).ToList();
        result.Changed = changedAll.ToList();
        result.AddedNames = addedAll.ToList();
        result.Queued = _queue.Count;
        result.ElapsedMs = Math.Round(watch.Elapsed.TotalMilliseconds, 1);
        Program.Log($"{(background ? "background" : "rebind")}: {result.Files.Count} files in {result.ElapsedMs} ms, "
            + $"changed {result.Changed.Count}, new names {result.AddedNames.Count}, queued {result.Queued}");
        return result;
    }

    void LearnRelations(OvSegment seg)
    {
        var ownerFiles = new Dictionary<OvKey, List<string>>();
        foreach (var f in seg.Files)
            foreach (var (symbol, _) in f.Declarations)
            {
                var key = seg.Symbols[symbol].Key;
                if (!ownerFiles.TryGetValue(key, out var list)) ownerFiles[key] = list = new();
                list.Add(f.Path);
            }
        foreach (var (from, to, _) in seg.Relations)
        {
            var s = seg.Symbols[from];
            var files = ownerFiles.GetValueOrDefault(s.Key) ?? (s.Container is { } c ? ownerFiles.GetValueOrDefault(c) : null);
            if (files == null) continue;
            foreach (var path in files)
                if (_overlay.TryGetValue(path, out var k) && k.RelationTargets.Add(seg.Symbols[to].Key))
                    Index(_overlayRelationTargets, seg.Symbols[to].Key, path);
        }
    }

    /// 能看到项目 p 的项目（p 与传递下游）里，出现了这些名字之一的文件。p &lt; 0 表示全部项目。
    void FilesNaming(int p, HashSet<string> names, HashSet<string> into)
    {
        var visible = new HashSet<int>();
        if (p < 0) visible.UnionWith(Enumerable.Range(0, _order.Length));
        else
        {
            var stack = new Stack<int>([p]);
            while (stack.Count > 0) if (visible.Add(stack.Peek())) foreach (int d in _dependents[stack.Pop()]) stack.Push(d); else stack.Pop();
        }
        var needles = names.Select(n => Encoding.UTF8.GetBytes(n)).ToArray();
        var files = _fileOwner.Where(kv => visible.Contains(_projectIndex[kv.Value])).Select(kv => kv.Key).ToArray();
        var hits = new ConcurrentBag<string>();
        Parallel.ForEach(files, new ParallelOptions { MaxDegreeOfParallelism = Math.Max(1, Environment.ProcessorCount / 2) }, full =>
        {
            var bytes = Program.ReadShared(full);
            if (bytes == null) return;
            foreach (var needle in needles)
                if (Impact.ContainsIdentifier(bytes, needle)) { hits.Add(Rel(full)); return; }
        });
        into.UnionWith(hits);
    }

    IEnumerable<int> TopoOrder(IEnumerable<int> projects)
    {
        var want = new HashSet<int>(projects);
        var done = new HashSet<int>();
        var order = new List<int>();
        void Visit(int p, int depth)
        {
            if (depth > _order.Length || !done.Add(p)) return;
            foreach (int d in _deps[p]) Visit(d, depth + 1);
            if (want.Contains(p)) order.Add(p);
        }
        foreach (int p in want.Order()) Visit(p, 0);
        return order;
    }

    IndexContext MiniContext()
    {
        var io = new SemaphoreSlim(1);
        return new IndexContext
        {
            Root = _root,
            Options = new IndexOptions(),
            Projects = _projects,
            FileOwner = _fileOwner,
            ParseOptions = _parseOptions,
            FileIds = new(),
            Results = [],
            ProjectAssemblies = _projectAssemblies,
            Io = io,
            Prefetch = new FilePrefetcher(io, 0, 1),
        };
    }

    /// 一个文件的绑定结果写进段：符号（按键去重）、出现位置、声明指纹、关系归属。
    (OvFile file, FileKnowledge knowledge) Emit(OvSegment segment, IndexContext ctx, ProjectBinder binder, string rel,
        FileResult bound, List<(ISymbol symbol, SymRec rec)> declared)
    {
        var byId = new Dictionary<int, SymRec>();
        foreach (var rec in ctx.Symbols.Map.Values) byId[rec.Id] = rec;
        var deferredCache = new Dictionary<string, OvKey>(StringComparer.Ordinal);
        OvKey KeyOf(SymRec rec) => Convert(rec.Key, rec, ctx, deferredCache);
        int Sym(SymRec rec)
        {
            var key = KeyOf(rec);
            return segment.Symbol(key, () => new OvSymbol
            {
                Key = key, Name = rec.Name, Display = rec.Display, Kind = rec.Kind, InSource = rec.InSource, External = rec.External,
                Container = rec.ContainerKey is { } c ? Convert(c, null, ctx, deferredCache) : null,
            });
        }
        var knowledge = new FileKnowledge { Hash = bound.Hash };
        var occs = bound.Occs.Select(o => o with { Sym = Sym(byId[o.Sym]) }).ToArray();
        Array.Sort(occs, static (a, b) =>
        {
            int c = a.Line.CompareTo(b.Line);
            if (c == 0) c = a.Col.CompareTo(b.Col);
            if (c == 0) c = a.Sym.CompareTo(b.Sym);
            if (c == 0) c = a.Role.CompareTo(b.Role);
            return c != 0 ? c : a.Len.CompareTo(b.Len);
        });
        foreach (var o in occs)
        {
            var key = segment.Symbols[o.Sym].Key;
            knowledge.Refs.Add(key);
            if ((o.Role & (ushort)Role.Def) != 0 && key.Path != null) knowledge.LocalDecls.Add(key);
        }
        var decls = new List<(int, ulong)>();
        var seen = new HashSet<ISymbol>(SymbolEqualityComparer.Default);
        foreach (var (symbol, rec) in declared)
        {
            if (!seen.Add(symbol) || !Fingerprints.IsSurface(symbol) || rec.Key.Text is not { } text || text.StartsWith('?')) continue;
            ulong fp = Fingerprints.Of(symbol);
            int index = Sym(rec);
            decls.Add((index, fp));
            knowledge.Decls[text] = new Decl(fp, Fingerprints.Weak(rec.Display, rec.Kind), Fingerprints.SearchName(symbol) ?? "");
            var first = occs.FirstOrDefault(o => o.Sym == index && (o.Role & (ushort)Role.Def) != 0);
            knowledge.DeclPositions.TryAdd(text, (first.Line, first.Col));
            // 关系归属：符号自己的基类型 / 重写；类型还拥有“成员实现接口成员”这一组
            segment.Owners.Add((index, OwnerPart.Self));
            // 与全量索引一致：只保留两端都在符号表里的关系（全量写索引时丢掉端点没有记录的关系，
            // 例如属性访问器、只被重写过而从未被引用的外部方法）。“在表里”= 本次绑定见过，或基础索引 / 覆盖层里有。
            SymRec? Known(ISymbol x)
            {
                var key = binder.KeyOf(x);
                if (ctx.Symbols.Map.TryGetValue(key, out var r)) return r;
                return key.Text is { } t && (_base.FindGlobal(t) >= 0 || _overlayRefs.ContainsKey(OvKey.Of(t))) ? binder.Record(x) : null;
            }
            void Rel(ISymbol from, ISymbol to, RelKind kind)
            {
                if (Known(from) is { } f && Known(to) is { } t) segment.Relations.Add((Sym(f), Sym(t), kind));
            }
            switch (symbol)
            {
                case INamedTypeSymbol t:
                    segment.Owners.Add((index, OwnerPart.Members));
                    if (t.BaseType is { SpecialType: not SpecialType.System_Object and not SpecialType.System_ValueType and not SpecialType.System_Enum } bt)
                        Rel(t, bt, RelKind.BaseType);
                    foreach (var i in t.Interfaces) Rel(t, i, RelKind.BaseType);
                    foreach (var iface in t.AllInterfaces)
                        foreach (var m in iface.GetMembers())
                        {
                            if (m.IsStatic) continue;
                            var impl = t.FindImplementationForInterfaceMember(m);
                            if (impl != null && SymbolEqualityComparer.Default.Equals(impl.ContainingType, t)) Rel(impl, m, RelKind.Implements);
                        }
                    break;
                case IMethodSymbol { OverriddenMethod: { } om } m: Rel(m, om, RelKind.Overrides); break;
                case IPropertySymbol { OverriddenProperty: { } op } p: Rel(p, op, RelKind.Overrides); break;
                case IEventSymbol { OverriddenEvent: { } oe } e: Rel(e, oe, RelKind.Overrides); break;
            }
        }
        var file = new OvFile { Path = rel, Hash = bound.Hash, Occs = occs, Inactive = bound.Inactive, Declarations = decls.ToArray() };
        segment.Files.Add(file);
        return (file, knowledge);
    }

    /// 绑定期的键 → 覆盖层的键：局部键的临时文件号换成路径；“经元数据看到的源码参数”回指参数声明。
    OvKey Convert(SymbolKey key, SymRec? rec, IndexContext ctx, Dictionary<string, OvKey> cache)
    {
        if (key.Text is null) return OvKey.Local(_tempPaths[(int)key.File], key.Offset);
        var text = key.Text;
        if (text is not ['\u0001', ..]) return OvKey.Of(text);
        if (cache.TryGetValue(text, out var done)) return done;
        int hash = text.LastIndexOf('#');
        var owner = text[1..hash];
        int ordinal = int.Parse(text.AsSpan(hash + 1));
        OvKey result;
        if (ctx.Symbols.SourceParameters.TryGetValue((owner, ordinal), out var local) && local.Text == null)
            result = OvKey.Local(_tempPaths[(int)local.File], local.Offset);
        else result = ParameterDeclaration(owner, ordinal) ?? OvKey.Of(rec?.ParameterFallback ?? text);
        cache[text] = result;
        return result;
    }

    /// 方法（或索引器、委托、构造函数）owner 的第 ordinal 个参数的声明位置：从索引找到方法名所在位置，
    /// 只做语法解析取参数标识符。找不到时返回 null（退回后备键，与全量索引一致）。
    OvKey? ParameterDeclaration(string owner, int ordinal)
    {
        string? path = null;
        (uint line, uint col) at = default;
        if (_overlayDeclFile.TryGetValue(owner, out var file) && _overlay[file].DeclPositions.TryGetValue(owner, out at)) path = file;
        else if (_base.FindGlobal(owner) is var s and >= 0)
            foreach (var o in _base.OccurrencesOf(s))
            {
                if ((o.Role & (ushort)Role.Def) == 0) break;
                var p = _base.FilePath(o.File);
                if (_overlay.ContainsKey(p)) continue;
                path = p;
                at = (o.Line, o.Col);
                break;
            }
        if (path == null) return null;
        var full = Full(path);
        var bytes = Program.ReadShared(full);
        if (bytes == null) return null;
        var po = _fileOwner.TryGetValue(full, out var proj) ? _parseOptions[proj] : CSharpParseOptions.Default;
        var text = Text(bytes);
        var tree = CSharpSyntaxTree.ParseText(text, po, path: full);
        if (at.line >= text.Lines.Count) return null;
        var line = text.Lines[(int)at.line];
        var coordinates = new Utf8Coordinates(text);
        uint lineStart = coordinates.ByteOffset(line.Start);
        // UTF-8 列 → UTF-16 位置
        int pos = line.Start;
        while (pos < line.End && coordinates.ByteOffset(pos) - lineStart < at.col) pos++;
        var token = tree.GetRoot().FindToken(pos);
        SeparatedSyntaxList<ParameterSyntax>? parameters = token.Parent switch
        {
            BaseMethodDeclarationSyntax m => m.ParameterList.Parameters,
            DelegateDeclarationSyntax d => d.ParameterList.Parameters,
            IndexerDeclarationSyntax i => i.ParameterList.Parameters,
            TypeDeclarationSyntax { ParameterList: { } primary } => primary.Parameters,
            _ => null,
        };
        if (parameters is not { } list || ordinal >= list.Count || list[ordinal].Identifier.Span.IsEmpty) return null;
        return OvKey.Local(path, coordinates.ByteOffset(list[ordinal].Identifier.SpanStart));
    }

    // ───────────── 后台队列 ─────────────

    void Enqueue(string rel)
    {
        if (_queued.Add(rel)) _queue.AddLast(rel);
    }

    void Dequeue(string rel)
    {
        if (_queued.Remove(rel)) _queue.Remove(rel);
    }

    /// 取出下一批后台工作：到期的被独占文件优先，然后按入队顺序，同一项目的放在一批。
    public List<string> NextBatch()
    {
        var batch = new List<string>();
        long now = Stopwatch.GetTimestamp();
        foreach (var (rel, (_, due)) in _locked.ToArray())
            if (due <= now && batch.Count < _options.BackgroundBatch) batch.Add(rel);
        if (_queue.Count == 0) return batch;
        string? project = null;
        for (var node = _queue.First; node != null && batch.Count < _options.BackgroundBatch;)
        {
            var next = node.Next;
            var owner = _fileOwner.GetValueOrDefault(Full(node.Value)) ?? "";
            project ??= owner;
            if (owner == project)
            {
                batch.Add(node.Value);
                _queued.Remove(node.Value);
                _queue.Remove(node);
            }
            node = next;
        }
        return batch;
    }

    /// 被独占的文件里最早到期的还要等多久。
    public TimeSpan? NextLockedRetry()
    {
        if (_locked.Count == 0) return null;
        long due = _locked.Values.Min(v => v.due);
        return TimeSpan.FromSeconds(Math.Max(0, (double)(due - Stopwatch.GetTimestamp()) / Stopwatch.Frequency));
    }

    public double BackgroundDuty => _options.BackgroundDuty;

    public void CancelBackground()
    {
        _queue.Clear();
        _queued.Clear();
    }

    public void Dispose()
    {
        _journal.Dispose();
        _base.Dispose();
    }
}
