using System.Collections.Concurrent;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;
using Microsoft.CodeAnalysis.Text;

namespace Rove.Indexer;

/// 一个项目里语法树到文件号与坐标的映射，以及这个编译内的 ISymbol → 符号记录缓存。
/// 缓存只在项目存活期间有效：项目绑定完连同编译一起释放。
sealed class ProjectBinder(IndexContext ctx, CSharpCompilation comp,
    Dictionary<SyntaxTree, (uint id, Utf8Coordinates coordinates)> treeFiles)
{
    readonly ConcurrentDictionary<ISymbol, SymRec> _cache = new(ReferenceEqualityComparer.Instance);
    readonly string _assembly = comp.AssemblyName ?? "";

    public FileResult Bind(SyntaxTree tree, uint id, byte[] path, ulong hash)
    {
        long start = Stats.Now();
        var model = comp.GetSemanticModel(tree);
        var text = tree.GetText();
        var coordinates = treeFiles[tree].coordinates;
        var occs = new List<Occ>();
        long names = 0, unres = 0, cand = 0;
        void Add(TextSpan span, ISymbol sym, Role role)
        {
            var symId = GetSym(sym);
            var line = text.Lines.GetLineFromPosition(span.Start);
            uint begin = coordinates.ByteOffset(span.Start);
            occs.Add(new Occ
            {
                Line = (uint)line.LineNumber,
                Col = begin - coordinates.ByteOffset(line.Start),
                Len = coordinates.ByteOffset(span.End) - begin,
                Sym = symId,
                Role = (ushort)role,
            });
        }
        void Unresolved(SimpleNameSyntax name)
        {
            unres++;
            Stats.Unresolved.AddOrUpdate(name.Identifier.ValueText, 1, (_, v) => v + 1);
        }

        var root = tree.GetRoot();
        foreach (var node in root.DescendantNodes())
        {
            if (node is SimpleNameSyntax name)
            {
                if (name.IsVar || name.IsUnmanaged || name.IsNotNull || name.IsNint || name.IsNuint
                    || name.Identifier.ValueText == "nameof" && name.Parent is InvocationExpressionSyntax) continue;
                names++;
                long t = Stats.Now();
                var info = model.GetSymbolInfo(name);
                Stats.Add(ref Stats.SymbolInfo, t);
                var sym = info.Symbol;
                var role = Role.Ref;
                if (sym == null && info.CandidateSymbols.Length > 0) { sym = info.CandidateSymbols[0]; role |= Role.Candidate; cand++; }
                if (sym == null)
                {
                    // 命名空间声明中的名字、未知特性目标等
                    var t2 = model.GetTypeInfo(name).Type;
                    if (t2 is { TypeKind: not TypeKind.Error }) sym = t2;
                }
                if (sym == null) { Unresolved(name); continue; }
                sym = Symbols.Norm(sym);
                if (sym is IMethodSymbol { MethodKind: MethodKind.Constructor } ctor) sym = ctor.ContainingType; // new Foo() / [Foo] 计为类型引用
                if (sym is IErrorTypeSymbol) { Unresolved(name); continue; }
                Add(name.Identifier.Span, sym, role);
            }
            else if (Symbols.DeclIdentifier(node) is { } tok && tok.Span.Length > 0)
            {
                long t = Stats.Now();
                var ds = model.GetDeclaredSymbol(node);
                Stats.Add(ref Stats.DeclaredSymbol, t);
                if (ds == null) continue;
                ds = Symbols.Norm(ds);
                Add(tok.Span, ds, Role.Def);
                if (ds is IParameterSymbol p && p.ContainingSymbol?.GetDocumentationCommentId() is { } owner
                    && Key(p) is { Text: null } local)
                    ctx.Symbols.RegisterSourceParameter(_assembly + "|" + owner, p.Ordinal, local);
                t = Stats.Now();
                Symbols.CollectRelations(ds, ctx.Relations, s => ctx.Symbols.Canonical(Key(s)));
                Stats.Add(ref Stats.Relations, t);
            }
        }

        Interlocked.Add(ref Stats.Names, names);
        Interlocked.Add(ref Stats.UnresolvedNames, unres);
        Interlocked.Add(ref Stats.CandidateNames, cand);
        if (unres > 0) Stats.UnresolvedByProject.AddOrUpdate(_assembly, (int)unres, (_, v) => v + (int)unres);
        var result = new FileResult
        {
            TempId = id, Path = path, Hash = hash, Occs = occs.ToArray(), Inactive = InactiveLines(root, text),
        };
        Stats.Add(ref Stats.BindTotal, start);
        return result;
    }

    int GetSym(ISymbol s)
    {
        if (_cache.TryGetValue(s, out var cached)) return cached.Id;
        long t = Stats.Now();
        var key = Key(s, out var parameterFallback);
        bool inSource = s.Locations.Any(l => l.IsInSource);
        bool collisionProne = key.Text is { } text && (text[0] == '?' || s is INamedTypeSymbol { IsAnonymousType: true }
            || s.ContainingType is { IsAnonymousType: true } or { IsTupleType: true });
        SymRec Create(SymbolKey k) => new()
        {
            Key = k, Name = s.Name, Display = Symbols.SafeDisplay(s), Kind = (byte)s.Kind, InSource = inSource,
            ContainerKey = s.ContainingSymbol is { } c && c.Kind != SymbolKind.Namespace && c.Kind != SymbolKind.Assembly
                && c.Kind != SymbolKind.NetModule ? ctx.Symbols.Canonical(Key(Symbols.Norm(c))) : null,
            ParameterFallback = parameterFallback,
        };
        var rec = ctx.Symbols.GetOrAdd(key, Create, collisionProne, inSource);
        _cache.TryAdd(s, rec);
        Stats.Add(ref Stats.SymbolKey, t);
        return rec.Id;
    }

    SymbolKey Key(ISymbol s) => Key(s, out _);

    /// 符号的全局唯一键：可跨编译稳定（同一源符号被不同项目引用时得到相同键）。
    SymbolKey Key(ISymbol s, out string? parameterFallback)
    {
        parameterFallback = null;
        if (s is INamespaceSymbol ns) return SymbolKey.Global("N:" + ns.ToDisplayString());
        var id = s.Kind switch
        {
            SymbolKind.Local or SymbolKind.Parameter or SymbolKind.RangeVariable or SymbolKind.Label or SymbolKind.TypeParameter => null,
            SymbolKind.Method when s is IMethodSymbol { MethodKind: MethodKind.LocalFunction or MethodKind.AnonymousFunction } => null,
            _ => s.GetDocumentationCommentId(),
        };
        if (id != null) return SymbolKey.Global((s.ContainingAssembly?.Name ?? "") + "|" + id);

        var loc = s.Locations.FirstOrDefault(l => l.IsInSource);
        if (loc?.SourceTree is { } tree && treeFiles.TryGetValue(tree, out var file))
        {
            // 单参数 lambda 的起点也是参数名的起点。用箭头作为 lambda 自己的声明位置。
            int offset = s is IMethodSymbol { MethodKind: MethodKind.AnonymousFunction }
                ? s.DeclaringSyntaxReferences.FirstOrDefault()?.GetSyntax() switch
                {
                    SimpleLambdaExpressionSyntax lambda => lambda.ArrowToken.SpanStart,
                    ParenthesizedLambdaExpressionSyntax lambda => lambda.ArrowToken.SpanStart,
                    AnonymousMethodExpressionSyntax method => method.DelegateKeyword.SpanStart,
                    _ => loc.SourceSpan.Start,
                }
                : loc.SourceSpan.Start;
            return SymbolKey.Local(file.id, file.coordinates.ByteOffset(offset));
        }

        var fallback = $"?{s.Kind}:{s.ToDisplayString()}";
        // 依赖项目以元数据镜像提供，其中参数没有源码位置。按"所属成员 + 序号"记下，写索引时回指源码声明。
        if (s is IParameterSymbol p && p.ContainingSymbol is { } owner)
        {
            int ordinal = p.Ordinal;
            // 扩展方法以实例形式调用时，参数属于约简后的方法，序号不含 this
            if (owner is IMethodSymbol { ReducedFrom: { } reducedFrom }) { owner = reducedFrom; ordinal++; }
            owner = owner.OriginalDefinition;
            if (owner.ContainingAssembly?.Name is { } asm && ctx.ProjectAssemblies.Contains(asm)
                && owner.GetDocumentationCommentId() is { } ownerId)
            {
                parameterFallback = fallback;
                return SymbolKey.Global("\u0001" + asm + "|" + ownerId + "#" + ordinal);
            }
        }
        return SymbolKey.Global(fallback);
    }

    /// 不生效的预处理区域。DisabledTextTrivia 只出现在预处理指令所在的前导 trivia 里，
    /// 所以沿指令链找，不必遍历整棵树的 trivia。
    static (uint start, uint end)[] InactiveLines(SyntaxNode root, SourceText text)
    {
        if (!root.ContainsDirectives) return [];
        var result = new List<(uint start, uint end)>();
        var lines = text.Lines;
        SyntaxToken last = default;
        for (var d = root.GetFirstDirective(); d != null; d = d.GetNextDirective())
        {
            var token = d.ParentTrivia.Token;
            if (token == last) continue;
            last = token;
            foreach (var trivia in token.LeadingTrivia)
            {
                if (trivia.RawKind != (int)SyntaxKind.DisabledTextTrivia || trivia.Span.IsEmpty) continue;
                uint start = (uint)lines.IndexOf(trivia.SpanStart);
                uint end = (uint)lines.IndexOf(trivia.Span.End - 1) + 1;
                if (result.Count > 0 && start <= result[^1].end)
                    result[^1] = (result[^1].start, Math.Max(result[^1].end, end));
                else
                    result.Add((start, end));
            }
        }
        return result.ToArray();
    }
}
