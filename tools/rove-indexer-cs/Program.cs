// rove-indexer-cs: M0 原型。不走 MSBuild，直接解析 Unity 生成的 .csproj 构造 Roslyn 编译，
// 生成预计算的语义索引（定义/引用/关系），供 Rove 主进程内存映射查询。
// 用法: rove-indexer-cs <unity-project-dir> <out.idx> [--generators] [--report <report.json>]

using System.Collections.Concurrent;
using System.Diagnostics;
using System.Reflection;
using System.Runtime.Loader;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.Diagnostics;
using Microsoft.CodeAnalysis.Text;

namespace Rove.Indexer;

record ProjectInfo(
    string Path, string AssemblyName, string[] Sources, string[] References,
    string[] ProjectRefs, string[] Analyzers, string[] Defines, LanguageVersion LangVersion, bool Unsafe);

enum Role : ushort { Ref = 1, Def = 2, Candidate = 4 }
enum RelKind : uint { BaseType = 1, Implements = 2, Overrides = 3 }

sealed class SymRec
{
    public int Id;
    public required string Key;
    public required string Name;
    public required string Display;
    public required byte Kind;
    public required bool InSource;
    public string? ContainerKey;
}

struct Occ { public int Line, Col, Len, Sym; public ushort Role; }

static class Program
{
    static readonly Stopwatch Total = Stopwatch.StartNew();
    static readonly Dictionary<string, double> Phases = new();

    static int Main(string[] args)
    {
        if (args.Length < 2) { Console.Error.WriteLine("usage: rove-indexer-cs <project-dir> <out.idx> [--generators] [--report file]"); return 2; }
        var root = Path.GetFullPath(args[0]);
        var outPath = Path.GetFullPath(args[1]);
        bool runGenerators = args.Contains("--generators");
        int ri = Array.IndexOf(args, "--report");
        string? reportPath = ri >= 0 && ri + 1 < args.Length ? args[ri + 1] : null;

        // 1. 解析 csproj
        var sw = Stopwatch.StartNew();
        var projects = Directory.GetFiles(root, "*.csproj").Select(ParseCsproj).Where(p => p != null).Cast<ProjectInfo>()
            .ToDictionary(p => p.Path, StringComparer.OrdinalIgnoreCase);
        Log($"projects: {projects.Count}, source entries: {projects.Values.Sum(p => p.Sources.Length)}");
        if (!args.Contains("--no-reconcile"))
        {
            var (added, removed) = UnityProjectModel.Reconcile(root, projects);
            Log($"reconcile with disk: +{added} files not in csproj, -{removed} listed but missing");
        }
        Phase("parse_csproj", sw);

        // 2. 并行解析语法树。预处理指令（#if UNITY_EDITOR 等）在解析时求值，
        //    所以必须用文件所属项目的宏来解析；文件归属第一个列出它的项目。
        var fileOwner = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var parseOpts = new Dictionary<string, CSharpParseOptions>(StringComparer.OrdinalIgnoreCase);
        foreach (var p in projects.Values.OrderBy(p => p.Path, StringComparer.OrdinalIgnoreCase))
        {
            parseOpts[p.Path] = new CSharpParseOptions(p.LangVersion, DocumentationMode.None, SourceCodeKind.Regular, p.Defines);
            foreach (var s in p.Sources) fileOwner.TryAdd(s, p.Path);
        }
        var trees = new ConcurrentDictionary<string, (SyntaxTree tree, ulong hash)>(StringComparer.OrdinalIgnoreCase);
        int missing = 0;
        Parallel.ForEach(fileOwner, kv =>
        {
            var f = kv.Key;
            var bytes = ReadShared(f);
            if (bytes == null) { Interlocked.Increment(ref missing); return; }
            var hash = System.IO.Hashing.XxHash64.HashToUInt64(bytes);
            var text = SourceText.From(new MemoryStream(bytes), throwIfBinaryDetected: false, canBeEmbedded: false);
            trees[f] = (CSharpSyntaxTree.ParseText(text, parseOpts[kv.Value], path: f), hash);
        });
        Phase("parse_syntax", sw);
        Log($"files: {trees.Count}, missing: {missing}");

        // 3. 构造编译（按项目引用拓扑，Lazy 递归）
        var metaRefs = new ConcurrentDictionary<string, MetadataReference>(StringComparer.OrdinalIgnoreCase);
        var analyzerRefs = new ConcurrentDictionary<string, AnalyzerFileReference>(StringComparer.OrdinalIgnoreCase);
        var loader = new SimpleAnalyzerLoader();
        var comps = new Dictionary<string, Lazy<CSharpCompilation>>(StringComparer.OrdinalIgnoreCase);
        var generatorStats = new ConcurrentDictionary<string, int>();
        foreach (var p in projects.Values)
        {
            var proj = p;
            comps[p.Path] = new Lazy<CSharpCompilation>(() =>
            {
                var po = parseOpts[proj.Path];
                // 同一文件被多个项目包含时（少见），非所属项目按自己的宏重新解析
                var ptrees = proj.Sources.Where(trees.ContainsKey).Select(s =>
                {
                    var t = trees[s].tree;
                    return fileOwner[s] == proj.Path ? t : CSharpSyntaxTree.ParseText(t.GetText(), po, path: s);
                }).ToList();
                var refs = new List<MetadataReference>();
                foreach (var r in proj.References)
                    if (File.Exists(r)) refs.Add(metaRefs.GetOrAdd(r, x => MetadataReference.CreateFromFile(x)));
                foreach (var pr in proj.ProjectRefs)
                    if (comps.TryGetValue(pr, out var dep)) refs.Add(dep.Value.ToMetadataReference());
                var opts = new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary,
                    allowUnsafe: proj.Unsafe, concurrentBuild: true,
                    metadataImportOptions: MetadataImportOptions.Internal);
                var comp = CSharpCompilation.Create(proj.AssemblyName, ptrees, refs, opts);
                if (runGenerators && proj.Analyzers.Length > 0)
                {
                    var gens = proj.Analyzers.Where(File.Exists)
                        .SelectMany(a => analyzerRefs.GetOrAdd(a, x => new AnalyzerFileReference(x, loader)).GetGenerators(LanguageNames.CSharp))
                        .ToArray();
                    if (gens.Length > 0)
                    {
                        try
                        {
                            var driver = CSharpGeneratorDriver.Create(gens, parseOptions: po);
                            driver.RunGeneratorsAndUpdateCompilation(comp, out var newComp, out _);
                            generatorStats.AddOrUpdate(proj.AssemblyName, newComp.SyntaxTrees.Count() - comp.SyntaxTrees.Count(), (_, v) => v);
                            comp = (CSharpCompilation)newComp;
                        }
                        catch (Exception e) { Log($"generator failed in {proj.AssemblyName}: {e.GetType().Name} {e.Message}"); }
                    }
                }
                return comp;
            }, LazyThreadSafetyMode.ExecutionAndPublication);
        }
        // 先在单线程里触发依赖链，避免并行时死锁式等待
        foreach (var c in comps.Values) _ = c.Value;
        int di = Array.IndexOf(args, "--diag");
        if (di >= 0 && di + 1 < args.Length)
        {
            var target = comps.Values.Select(c => c.Value).First(c => c.AssemblyName == args[di + 1]);
            foreach (var d in target.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error).GroupBy(d => d.Id).OrderByDescending(g => g.Count()).Take(8))
                Log($"diag {d.Key} x{d.Count()}: {d.First().GetMessage()} @ {d.First().Location.GetLineSpan()}");
            return 0;
        }
        Phase(runGenerators ? "create_compilations+generators" : "create_compilations", sw);

        // 4. 绑定 + 收集出现位置
        var symbols = new ConcurrentDictionary<string, SymRec>();
        int nextSym = -1;
        var fileOccs = new ConcurrentDictionary<string, List<Occ>>(StringComparer.OrdinalIgnoreCase);
        var rels = new ConcurrentBag<(string from, string to, RelKind kind)>();
        var unresolved = new ConcurrentDictionary<string, int>();
        var unresolvedByProject = new ConcurrentDictionary<string, int>();
        long totalNames = 0, unresolvedNames = 0, candidateNames = 0;

        int GetSym(ISymbol s, out SymRec rec)
        {
            var key = SymbolKey(s);
            rec = symbols.GetOrAdd(key, k => new SymRec
            {
                Id = Interlocked.Increment(ref nextSym), Key = k, Name = s.Name,
                Display = SafeDisplay(s), Kind = (byte)s.Kind, InSource = s.Locations.Any(l => l.IsInSource),
                ContainerKey = s.ContainingSymbol is { } c && c.Kind != SymbolKind.Namespace && c.Kind != SymbolKind.Assembly && c.Kind != SymbolKind.NetModule
                    ? SymbolKey(Norm(c)) : null,
            });
            return rec.Id;
        }

        var work = comps.Values.SelectMany(c => c.Value.SyntaxTrees
                .Where(t => fileOwner.TryGetValue(t.FilePath, out var owner) && comps[owner].Value == c.Value)
                .Select(t => (comp: c.Value, tree: t)))
            .ToArray();
        Parallel.ForEach(work, new ParallelOptions { MaxDegreeOfParallelism = Environment.ProcessorCount }, item =>
        {
            var (comp, tree) = item;
            var model = comp.GetSemanticModel(tree);
            var text = tree.GetText();
            var occs = new List<Occ>(1024);
            long names = 0, unres = 0, cand = 0;
            void Add(TextSpan span, ISymbol sym, Role role)
            {
                var id = GetSym(sym, out _);
                var lp = text.Lines.GetLinePosition(span.Start);
                occs.Add(new Occ { Line = lp.Line, Col = lp.Character, Len = span.Length, Sym = id, Role = (ushort)role });
            }
            foreach (var node in tree.GetRoot().DescendantNodes())
            {
                if (node is SimpleNameSyntax name)
                {
                    if (name.IsVar || name.IsUnmanaged || name.IsNotNull || name.IsNint || name.IsNuint
                        || name.Identifier.ValueText == "nameof" && name.Parent is InvocationExpressionSyntax) continue;
                    names++;
                    var info = model.GetSymbolInfo(name);
                    var sym = info.Symbol;
                    var role = Role.Ref;
                    if (sym == null && info.CandidateSymbols.Length > 0) { sym = info.CandidateSymbols[0]; role |= Role.Candidate; cand++; }
                    if (sym == null)
                    {
                        // 命名空间声明中的名字、未知特性目标等
                        var t2 = model.GetTypeInfo(name).Type;
                        if (t2 is { TypeKind: not TypeKind.Error }) sym = t2;
                    }
                    if (sym == null) { unres++; unresolved.AddOrUpdate(name.Identifier.ValueText, 1, (_, v) => v + 1); continue; }
                    sym = Norm(sym);
                    if (sym is IMethodSymbol { MethodKind: MethodKind.Constructor } ctor) sym = ctor.ContainingType; // new Foo() / [Foo] 计为类型引用
                    if (sym is IErrorTypeSymbol) { unres++; unresolved.AddOrUpdate(name.Identifier.ValueText, 1, (_, v) => v + 1); continue; }
                    Add(name.Identifier.Span, sym, role);
                }
                else if (DeclIdentifier(node) is { } tok && tok.Span.Length > 0)
                {
                    var ds = model.GetDeclaredSymbol(node);
                    if (ds == null) continue;
                    ds = Norm(ds);
                    Add(tok.Span, ds, Role.Def);
                    CollectRelations(ds, rels);
                }
            }
            fileOccs[tree.FilePath] = occs;
            Interlocked.Add(ref totalNames, names);
            Interlocked.Add(ref unresolvedNames, unres);
            Interlocked.Add(ref candidateNames, cand);
            if (unres > 0) unresolvedByProject.AddOrUpdate(comp.AssemblyName!, (int)unres, (_, v) => v + (int)unres);
        });
        Phase("bind_and_collect", sw);
        long peakAfterBind = Process.GetCurrentProcess().PeakWorkingSet64;

        // 5. 写索引
        var stats = IndexWriter.Write(outPath, root, trees, fileOccs, symbols, rels);
        Phase("write_index", sw);

        var proc = Process.GetCurrentProcess();
        var report = new
        {
            project = root,
            generators = runGenerators,
            projects = projects.Count,
            files = trees.Count,
            missing_files = missing,
            phases_ms = Phases,
            total_ms = Total.Elapsed.TotalMilliseconds,
            peak_working_set_mb = Math.Max(proc.PeakWorkingSet64, peakAfterBind) / 1048576.0,
            names = totalNames,
            unresolved_names = unresolvedNames,
            unresolved_ratio = totalNames == 0 ? 0 : (double)unresolvedNames / totalNames,
            candidate_names = candidateNames,
            symbols = symbols.Count,
            occurrences = stats.occurrences,
            relations = stats.relations,
            index_bytes = stats.bytes,
            generated_trees = generatorStats.Values.Sum(),
            top_unresolved = unresolved.OrderByDescending(kv => kv.Value).Take(40).ToDictionary(kv => kv.Key, kv => kv.Value),
            unresolved_by_project = unresolvedByProject.OrderByDescending(kv => kv.Value).Take(20).ToDictionary(kv => kv.Key, kv => kv.Value),
        };
        var json = JsonSerializer.Serialize(report, new JsonSerializerOptions { WriteIndented = true, Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
        if (reportPath != null) File.WriteAllText(reportPath, json);
        Console.WriteLine(json);
        return 0;
    }

    static void Phase(string name, Stopwatch sw)
    {
        Phases[name] = Math.Round(sw.Elapsed.TotalMilliseconds);
        Log($"[{name}] {sw.Elapsed.TotalSeconds:F1}s  ws={Process.GetCurrentProcess().WorkingSet64 / 1048576}MB");
        sw.Restart();
    }

    /// 以共享模式读取（Unity 编辑器、杀毒软件可能正占用文件），失败短暂重试。
    static byte[]? ReadShared(string path)
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

    static void Log(string s) => Console.Error.WriteLine($"{Total.Elapsed.TotalSeconds,7:F1}s  {s}");

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

    static ISymbol Norm(ISymbol s)
    {
        if (s is IAliasSymbol a) s = a.Target;
        if (s is IMethodSymbol m && m.ReducedFrom != null) s = m.ReducedFrom;
        return s.OriginalDefinition;
    }

    /// 符号的全局唯一键：可跨编译稳定（同一源符号被不同项目引用时得到相同键）。
    static string SymbolKey(ISymbol s)
    {
        if (s is INamespaceSymbol ns) return "N:" + ns.ToDisplayString();
        var id = s.Kind switch
        {
            SymbolKind.Local or SymbolKind.Parameter or SymbolKind.RangeVariable or SymbolKind.Label or SymbolKind.TypeParameter => null,
            SymbolKind.Method when s is IMethodSymbol { MethodKind: MethodKind.LocalFunction or MethodKind.AnonymousFunction } => null,
            _ => s.GetDocumentationCommentId(),
        };
        if (id != null) return (s.ContainingAssembly?.Name ?? "") + "|" + id;
        var loc = s.Locations.FirstOrDefault(l => l.IsInSource);
        if (loc != null) return $"@{loc.SourceTree!.FilePath}:{loc.SourceSpan.Start}:{s.Name}";
        return $"?{s.Kind}:{s.ToDisplayString()}";
    }

    static string SafeDisplay(ISymbol s)
    {
        try { return s.ToDisplayString(SymbolDisplayFormat.MinimallyQualifiedFormat); } catch { return s.Name; }
    }

    static SyntaxToken? DeclIdentifier(SyntaxNode n) => n switch
    {
        BaseTypeDeclarationSyntax t => t.Identifier,
        DelegateDeclarationSyntax d => d.Identifier,
        MethodDeclarationSyntax m => m.Identifier,
        ConstructorDeclarationSyntax c => c.Identifier,
        DestructorDeclarationSyntax d => d.Identifier,
        PropertyDeclarationSyntax p => p.Identifier,
        EventDeclarationSyntax e => e.Identifier,
        IndexerDeclarationSyntax i => i.ThisKeyword,
        VariableDeclaratorSyntax v => v.Identifier,
        ParameterSyntax p when p.Identifier.Span.Length > 0 => p.Identifier,
        TypeParameterSyntax tp => tp.Identifier,
        EnumMemberDeclarationSyntax em => em.Identifier,
        SingleVariableDesignationSyntax sv => sv.Identifier,
        ForEachStatementSyntax fe => fe.Identifier,
        CatchDeclarationSyntax cd when cd.Identifier.Span.Length > 0 => cd.Identifier,
        LocalFunctionStatementSyntax lf => lf.Identifier,
        _ => null,
    };

    static void CollectRelations(ISymbol s, ConcurrentBag<(string, string, RelKind)> rels)
    {
        switch (s)
        {
            case INamedTypeSymbol t:
                if (t.BaseType is { SpecialType: not SpecialType.System_Object and not SpecialType.System_ValueType and not SpecialType.System_Enum } bt)
                    rels.Add((SymbolKey(t), SymbolKey(Norm(bt)), RelKind.BaseType));
                foreach (var i in t.Interfaces) rels.Add((SymbolKey(t), SymbolKey(Norm(i)), RelKind.BaseType));
                foreach (var iface in t.AllInterfaces)
                    foreach (var m in iface.GetMembers())
                    {
                        if (m.IsStatic) continue;
                        var impl = t.FindImplementationForInterfaceMember(m);
                        if (impl != null && SymbolEqualityComparer.Default.Equals(impl.ContainingType, t))
                            rels.Add((SymbolKey(Norm(impl)), SymbolKey(Norm(m)), RelKind.Implements));
                    }
                break;
            case IMethodSymbol { OverriddenMethod: { } om } m:
                rels.Add((SymbolKey(m), SymbolKey(Norm(om)), RelKind.Overrides)); break;
            case IPropertySymbol { OverriddenProperty: { } op } p:
                rels.Add((SymbolKey(p), SymbolKey(Norm(op)), RelKind.Overrides)); break;
            case IEventSymbol { OverriddenEvent: { } oe } e:
                rels.Add((SymbolKey(e), SymbolKey(Norm(oe)), RelKind.Overrides)); break;
        }
    }
}

sealed class SimpleAnalyzerLoader : IAnalyzerAssemblyLoader
{
    public void AddDependencyLocation(string fullPath) { }
    public Assembly LoadFromPath(string fullPath) => AssemblyLoadContext.Default.LoadFromAssemblyPath(fullPath);
}
