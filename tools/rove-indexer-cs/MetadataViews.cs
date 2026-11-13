// 外部符号的元数据视图（#34）：为索引中被引用到的外部类型生成只有签名与 XML 文档注释的 C# 文本。
//
// 绑定阶段只登记"哪个程序集的哪个最外层类型被引用到了"（MetadataRequests.Note），开销是每个项目每个类型一次字典操作。
// 流水线结束、各项目的编译都已释放后，再用缓存里的 DLL 引用建一个只含元数据的编译，统一生成视图（MetadataViews.Generate）。
// 这时符号表已经完整，只为表中存在的成员写位置记录，体积只随"实际被引用到的类型"增长。
// 同一程序集标识（含版本）+ 同一类型只生成一次；不同版本的同名程序集各自生成。

using System.Collections.Concurrent;
using System.Collections.Immutable;
using System.Text;
using System.Xml;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace Rove.Indexer;

/// 一个最外层外部类型的视图。Text 含文件头；成员位置相对于 Text，列与长度为 UTF-8 字节。
sealed class MetadataView
{
    public required string TypeKey;
    public required string Identity;
    public required string Text;
    public required ViewMember[] Members;
    /// 被引用到的成员（含类型自身）的签名，供段 11。XML 文档只在视图正文里存一份（成员上方的 /// 行）。
    public required (string key, string signature)[] Texts;
    public int Declared, Documented;
    public bool SyntaxErrors;
}

readonly record struct ViewMember(string Key, uint Line, uint Col, uint Len);

/// 渲染出的签名里一个参数名的位置（字符计，相对于 Rendered.Text）。
record struct ParameterSpan(IParameterSymbol Symbol, int Start, int Length);

/// 一行声明的渲染结果：文本、名字片段的位置，以及签名里各参数名的位置（#126）。
/// 文本被改写（加修饰符、去掉外层类型限定、替换类型参数的变体写法）时，这些位置跟着移动。
sealed class Rendered(string text, int nameStart, int nameLength)
{
    public string Text { get; private set; } = text;
    public int NameStart { get; private set; } = nameStart;
    public int NameLength { get; } = nameLength;
    public readonly List<ParameterSpan> Parameters = new();

    /// 在文本开头插入（修饰符等）。
    public void Prefix(string s)
    {
        if (s.Length == 0) return;
        Text = s + Text;
        Shift(0, s.Length);
    }

    /// 在末尾追加（基类型列表、约束等），不影响既有位置。
    public void Append(string s) => Text += s;

    public void Remove(int at, int count)
    {
        Text = Text.Remove(at, count);
        Shift(at, -count);
    }

    public void Replace(int at, int count, string s)
    {
        Text = Text[..at] + s + Text[(at + count)..];
        Shift(at + count, s.Length - count);
    }

    void Shift(int from, int delta)
    {
        if (NameStart >= from) NameStart += delta;
        for (int i = 0; i < Parameters.Count; i++)
            if (Parameters[i].Start >= from) Parameters[i] = Parameters[i] with { Start = Parameters[i].Start + delta };
    }
}

/// 绑定阶段登记的外部类型。键为程序集标识的显示名（含版本），值为该程序集的引用与被引用到的类型。
sealed class MetadataRequests
{
    sealed class Entry(PortableExecutableReference reference, string assemblyName)
    {
        public PortableExecutableReference Reference = reference;
        public readonly string AssemblyName = assemblyName;
        public readonly ConcurrentDictionary<string, byte> Types = new(StringComparer.Ordinal);
    }

    readonly ConcurrentDictionary<string, Entry> _entries = new(StringComparer.Ordinal);

    public int AssemblyCount => _entries.Count;

    /// 同一标识被多个项目从不同路径引用时，取路径的 Ordinal 最小者，保证结果与项目处理顺序无关。
    public void Note(IAssemblySymbol assembly, PortableExecutableReference reference, string metadataName)
    {
        var identity = assembly.Identity.GetDisplayName();
        var entry = _entries.GetOrAdd(identity, _ => new Entry(reference, assembly.Name));
        if (!ReferenceEquals(entry.Reference, reference))
            lock (entry)
                if (string.CompareOrdinal(reference.FilePath, entry.Reference.FilePath) < 0) entry.Reference = reference;
        entry.Types.TryAdd(metadataName, 0);
    }

    public IEnumerable<(string identity, string assemblyName, PortableExecutableReference reference, string[] types)> Snapshot()
        => _entries.OrderBy(kv => kv.Key, StringComparer.Ordinal).Select(kv => (kv.Key, kv.Value.AssemblyName, kv.Value.Reference,
            kv.Value.Types.Keys.OrderBy(t => t, StringComparer.Ordinal).ToArray()));
}

static class MetadataViews
{
    public static long Warnings;
    public static readonly ConcurrentQueue<string> WarningLog = new();

    public static void Warn(string message)
    {
        Interlocked.Increment(ref Warnings);
        if (WarningLog.Count < 200) WarningLog.Enqueue(message);
        Program.Log("warning: " + message);
    }

    /// 导入全部成员：源码经 InternalsVisibleTo 引用到的 internal 成员也要能在视图里找到。成员只在生成视图的类型上按需加载。
    static readonly CSharpCompilationOptions ViewCompilation = new(OutputKind.DynamicallyLinkedLibrary,
        metadataImportOptions: MetadataImportOptions.All);

    /// 生成全部视图。references 是流水线缓存的全部 DLL 引用（用于解析签名里出现的其他程序集的类型）。
    public static List<MetadataView> Generate(MetadataRequests requests, IEnumerable<MetadataReference> references,
        SymbolTable symbols, int concurrency)
    {
        var all = references.Distinct().ToArray();
        var main = CSharpCompilation.Create("rove-metadata-views", references: all,
            options: ViewCompilation);
        var snapshot = requests.Snapshot().ToArray();
        // 文档文件只保留被请求类型（及其成员、嵌套类型）的条目；BCL 的 netstandard.xml 有 16 MB，全读进来白占几百 MB。
        var docs = new XmlDocs(snapshot.SelectMany(r => r.types).ToHashSet(StringComparer.Ordinal));
        var work = new List<(INamedTypeSymbol type, string identity, string? path, Func<string, string?> doc)>();
        foreach (var (identity, assemblyName, reference, types) in snapshot)
        {
            var assembly = Resolve(main, all, identity, reference);
            if (assembly == null)
            {
                Warn($"metadata of {identity} could not be loaded ({reference.FilePath}); its symbols have no metadata view");
                continue;
            }
            var doc = docs.For(reference.FilePath, assemblyName);
            foreach (var name in types)
            {
                if (assembly.GetTypeByMetadataName(name) is { } type) work.Add((type, identity, reference.FilePath, doc));
                else Warn($"type {name} not found in {identity}");
            }
        }
        var views = new MetadataView?[work.Count];
        Parallel.For(0, work.Count, new ParallelOptions { MaxDegreeOfParallelism = concurrency }, i =>
        {
            var (type, identity, path, doc) = work[i];
            try { views[i] = new ViewBuilder(type, identity, path, doc, symbols).Build(); }
            catch (Exception e) { Warn($"metadata view of {type.ToDisplayString()} in {identity} failed: {e.GetType().Name} {e.Message}"); }
        });
        foreach (var w in docs.MissingWarnings()) Warn(w);
        return views.OfType<MetadataView>().ToList();
    }

    /// 在共用编译中找到该标识的程序集；同名不同版本的程序集与其他版本冲突时，单独建一个只换掉同名引用的编译。
    static IAssemblySymbol? Resolve(CSharpCompilation main, MetadataReference[] all, string identity, PortableExecutableReference reference)
    {
        if (main.GetAssemblyOrModuleSymbol(reference) is IAssemblySymbol a && a.Identity.GetDisplayName() == identity) return a;
        var simple = Path.GetFileNameWithoutExtension(reference.FilePath);
        var refs = all.Where(r => r is not PortableExecutableReference p
            || !string.Equals(Path.GetFileNameWithoutExtension(p.FilePath), simple, StringComparison.OrdinalIgnoreCase)).Append(reference);
        var comp = CSharpCompilation.Create("rove-metadata-views-" + simple, references: refs,
            options: ViewCompilation);
        return comp.GetAssemblyOrModuleSymbol(reference) as IAssemblySymbol;
    }
}

/// 生成一个最外层类型的视图文本。签名一律交给 SymbolDisplayFormat 渲染，不自己拼类型名。
sealed class ViewBuilder(INamedTypeSymbol root, string identity, string? path, Func<string, string?> docs, SymbolTable symbols)
{
    const string Indent = "    ";
    // 文件头固定 4 行：#region、DLL 路径、#endregion、空行。
    const int HeaderLines = 4;

    const SymbolDisplayGenericsOptions Generics = SymbolDisplayGenericsOptions.IncludeTypeParameters
        | SymbolDisplayGenericsOptions.IncludeTypeConstraints;
    const SymbolDisplayMemberOptions MemberOptions = SymbolDisplayMemberOptions.IncludeAccessibility
        | SymbolDisplayMemberOptions.IncludeModifiers | SymbolDisplayMemberOptions.IncludeType | SymbolDisplayMemberOptions.IncludeParameters
        | SymbolDisplayMemberOptions.IncludeConstantValue | SymbolDisplayMemberOptions.IncludeRef
        | SymbolDisplayMemberOptions.IncludeExplicitInterface;
    const SymbolDisplayKindOptions Kinds = SymbolDisplayKindOptions.IncludeTypeKeyword | SymbolDisplayKindOptions.IncludeMemberKeyword;

    /// 签名中出现的类型只写类型名（含外层类型），命名空间由文件开头的 using 提供。
    static readonly SymbolDisplayFormat MemberFormat = Format(SymbolDisplayTypeQualificationStyle.NameAndContainingTypes, Generics, MemberOptions, Kinds);
    /// 类型声明行：不含约束的部分与完整渲染，约束取两者之差，基类型列表插在中间。
    /// 声明自身不用 int、string 这类关键字别名，也不写成元组语法。
    static readonly SymbolDisplayFormat TypeHeadFormat = Format(SymbolDisplayTypeQualificationStyle.NameOnly,
        Generics & ~SymbolDisplayGenericsOptions.IncludeTypeConstraints, MemberOptions, Kinds)
        .RemoveMiscellaneousOptions(SymbolDisplayMiscellaneousOptions.UseSpecialTypes)
        .AddMiscellaneousOptions(SymbolDisplayMiscellaneousOptions.ExpandValueTuple);
    static readonly SymbolDisplayFormat TypeFullFormat = Format(SymbolDisplayTypeQualificationStyle.NameOnly, Generics, MemberOptions, Kinds)
        .RemoveMiscellaneousOptions(SymbolDisplayMiscellaneousOptions.UseSpecialTypes)
        .AddMiscellaneousOptions(SymbolDisplayMiscellaneousOptions.ExpandValueTuple);
    /// 声明名与类型参数：协变/逆变只写在声明处。IncludeVariance 会把 out T 也带进签名里对 T 的每次引用，所以单独渲染后替换。
    static readonly SymbolDisplayFormat PlainNameFormat = Format(SymbolDisplayTypeQualificationStyle.NameOnly,
        SymbolDisplayGenericsOptions.IncludeTypeParameters, SymbolDisplayMemberOptions.None, SymbolDisplayKindOptions.None,
        SymbolDisplayDelegateStyle.NameOnly)
        .RemoveMiscellaneousOptions(SymbolDisplayMiscellaneousOptions.UseSpecialTypes)
        .AddMiscellaneousOptions(SymbolDisplayMiscellaneousOptions.ExpandValueTuple);
    static readonly SymbolDisplayFormat VarianceNameFormat = PlainNameFormat.AddGenericsOptions(SymbolDisplayGenericsOptions.IncludeVariance);
    /// 基类型列表、枚举成员名等引用。
    static readonly SymbolDisplayFormat TypeRefFormat = Format(SymbolDisplayTypeQualificationStyle.NameAndContainingTypes,
        SymbolDisplayGenericsOptions.IncludeTypeParameters, SymbolDisplayMemberOptions.None, SymbolDisplayKindOptions.None);

    static SymbolDisplayFormat Format(SymbolDisplayTypeQualificationStyle qualification, SymbolDisplayGenericsOptions generics,
        SymbolDisplayMemberOptions members, SymbolDisplayKindOptions kinds,
        SymbolDisplayDelegateStyle delegates = SymbolDisplayDelegateStyle.NameAndSignature) => new(
        globalNamespaceStyle: SymbolDisplayGlobalNamespaceStyle.Omitted,
        typeQualificationStyle: qualification,
        genericsOptions: generics,
        memberOptions: members,
        delegateStyle: delegates,
        extensionMethodStyle: SymbolDisplayExtensionMethodStyle.StaticMethod,
        parameterOptions: SymbolDisplayParameterOptions.IncludeExtensionThis | SymbolDisplayParameterOptions.IncludeParamsRefOut
            | SymbolDisplayParameterOptions.IncludeType | SymbolDisplayParameterOptions.IncludeName
            | SymbolDisplayParameterOptions.IncludeDefaultValue,
        propertyStyle: SymbolDisplayPropertyStyle.ShowReadWriteDescriptor,
        kindOptions: kinds,
        miscellaneousOptions: SymbolDisplayMiscellaneousOptions.UseSpecialTypes | SymbolDisplayMiscellaneousOptions.EscapeKeywordIdentifiers
            | SymbolDisplayMiscellaneousOptions.IncludeNullableReferenceTypeModifier);

    readonly List<string> _lines = new();
    readonly List<ViewMember> _members = new();
    readonly List<(string key, int line)> _referenced = new();
    readonly SortedSet<string> _usings = new(StringComparer.Ordinal);
    readonly string _prefix = root.ContainingAssembly.Name + "|";
    string _namespace = "";
    int _declared, _documented;

    public MetadataView Build()
    {
        _namespace = root.ContainingNamespace is { IsGlobalNamespace: false } ns ? ns.ToDisplayString() : "";
        // 文件范围的命名空间：比块状命名空间每行少一级缩进，体积约省 5%。
        if (_namespace.Length > 0)
        {
            _lines.Add("namespace " + _namespace + ";");
            _lines.Add("");
        }
        WriteType(root, "");

        var head = new List<string>
        {
            "#region 程序集 " + identity,
            "// " + (path ?? "(unknown path)"),
            "#endregion",
            "",
        };
        // using 放在文件头与正文之间，所以正文的行号整体后移。
        foreach (var u in _usings) if (u != _namespace && !_namespace.StartsWith(u + ".", StringComparison.Ordinal)) head.Add("using " + u + ";");
        if (head.Count > HeaderLines) head.Add("");
        int shift = head.Count;
        head.AddRange(_lines);
        var text = string.Join("\n", head) + "\n";

        var members = _members.Select(m => m with { Line = m.Line + (uint)shift }).ToArray();
        var texts = _referenced.Select(r => (r.key, _lines[r.line].Trim())).ToArray();
        var tree = CSharpSyntaxTree.ParseText(text, new CSharpParseOptions(LanguageVersion.Preview, DocumentationMode.Parse));
        bool errors = tree.GetDiagnostics().Any(d => d.Severity == DiagnosticSeverity.Error);
        if (errors)
        {
            var first = tree.GetDiagnostics().First(d => d.Severity == DiagnosticSeverity.Error);
            var line = first.Location.GetLineSpan().StartLinePosition.Line;
            MetadataViews.Warn($"metadata view of {root.ToDisplayString()} has syntax errors: {first.GetMessage()} @ \"{head[line].Trim()}\"");
        }
        return new MetadataView
        {
            TypeKey = _prefix + root.GetDocumentationCommentId(),
            Identity = identity,
            Text = text,
            Members = members,
            Texts = texts,
            Declared = _declared,
            Documented = _documented,
            SyntaxErrors = errors,
        };
    }

    void WriteType(INamedTypeSymbol type, string indent)
    {
        if (type.TypeKind == TypeKind.Delegate)
        {
            var d = Variance(type, Render(type, MemberFormat));
            // 嵌套委托的名字会带上外层类型（Outer.Callback），声明处去掉。
            if (type.ContainingType is { } outer && Render(outer, TypeRefFormat).Text + "." is var qualifier
                && d.NameStart >= qualifier.Length && string.CompareOrdinal(d.Text, d.NameStart - qualifier.Length, qualifier, 0, qualifier.Length) == 0)
                d.Remove(d.NameStart - qualifier.Length, qualifier.Length);
            d.Prefix(TypeModifiers(type, d.Text));
            DeclareLine(type, indent, d, ";");
            // 调用委托（callback()）绑定到 Invoke；视图里没有单独的 Invoke 声明，落到委托名上。
            if (type.DelegateInvokeMethod is { } invoke) Alias(invoke, _members.Count > 0 ? _members[^1] : null, type);
            return;
        }
        var head = Variance(type, Render(type, TypeHeadFormat));
        var full = Variance(type, Render(type, TypeFullFormat));
        // 基类型列表要插在类型参数约束之前：约束部分取完整渲染比不含约束的渲染多出来的后缀。
        var constraints = full.Text.StartsWith(head.Text, StringComparison.Ordinal) ? full.Text[head.Text.Length..] : "";
        var bases = new List<string>();
        if (type.TypeKind == TypeKind.Enum)
        {
            if (type.EnumUnderlyingType is { SpecialType: not SpecialType.System_Int32 } u) bases.Add(TypeRef(u));
        }
        else
        {
            if (type.TypeKind == TypeKind.Class && type.BaseType is { SpecialType: not SpecialType.System_Object } bt)
                bases.Add(TypeRef(bt));
            foreach (var i in type.Interfaces) if (IsVisible(i)) bases.Add(TypeRef(i));
        }
        head.Prefix(TypeModifiers(type, head.Text));
        head.Append((bases.Count > 0 ? " : " + string.Join(", ", bases) : "") + constraints);
        DeclareLine(type, indent, head);
        _lines.Add(indent + "{");
        var inner = indent + Indent;
        if (type.TypeKind == TypeKind.Enum)
        {
            foreach (var f in type.GetMembers().OfType<IFieldSymbol>().Where(f => f.IsConst && IsVisible(f)))
            {
                var name = SyntaxFacts.GetKeywordKind(f.Name) != SyntaxKind.None ? "@" + f.Name : f.Name;
                var value = f.ConstantValue is { } v ? " = " + SymbolDisplay.FormatPrimitive(v, quoteStrings: true, useHexadecimalNumbers: false) : "";
                DeclareLine(f, inner, name + value, 0, name.Length, ",");
            }
        }
        else
        {
            foreach (var m in Members(type))
            {
                if (m is INamedTypeSymbol nested) WriteType(nested, inner);
                else
                {
                    var r = Render(m, MemberFormat);
                    // decimal 常量在元数据里是带 DecimalConstant 特性的 static readonly 字段，Roslyn 同时报告 const 与 readonly。
                    if (m is IFieldSymbol { IsConst: true, IsReadOnly: true } && r.Text.IndexOf("const readonly ", StringComparison.Ordinal) is var at and >= 0
                        && at < r.NameStart)
                        r.Remove(at + 5, 9);
                    // decimal 常量的值没有后缀，超出 long 范围的字面量会被当作整数常量报错。
                    if (m is IFieldSymbol { IsConst: true, Type.SpecialType: SpecialType.System_Decimal } && r.Text.Length > 0 && char.IsDigit(r.Text[^1]))
                        r.Append("M");
                    DeclareLine(m, inner, r, m is IPropertySymbol ? "" : ";");
                }
            }
        }
        _lines.Add(indent + "}");
    }

    /// 类型的显示片段不含可访问性与 static/abstract/sealed，按符号属性补上（Roslyn 已给出的关键字不重复）。
    static string TypeModifiers(INamedTypeSymbol type, string head)
    {
        var words = new List<string> { SyntaxFacts.GetText(type.DeclaredAccessibility) };
        if (type.TypeKind == TypeKind.Class)
        {
            if (type.IsStatic) words.Add("static");
            else if (type.IsAbstract) words.Add("abstract");
            else if (type.IsSealed) words.Add("sealed");
        }
        else if (type.TypeKind == TypeKind.Struct)
        {
            if (type.IsReadOnly) words.Add("readonly");
            if (type.IsRefLikeType) words.Add("ref");
        }
        var present = head.Split(' ', StringSplitOptions.RemoveEmptyEntries).TakeWhile(IsWord).ToHashSet();
        var prefix = string.Join(" ", words.Where(w => w.Length > 0 && !present.Contains(w)));
        return prefix.Length == 0 ? "" : prefix + " ";
        static bool IsWord(string w) => w is "public" or "protected" or "internal" or "static" or "abstract" or "sealed" or "readonly" or "ref";
    }

    /// 成员顺序：字段、构造函数、属性、索引器、事件、方法、运算符、嵌套类型；同类内保持元数据中的声明顺序。
    IEnumerable<ISymbol> Members(INamedTypeSymbol type) => type.GetMembers().Where(IsDeclared)
        .Select((m, i) => (m, i)).OrderBy(x => Order(x.m)).ThenBy(x => x.i).Select(x => x.m)
        .Concat(type.GetTypeMembers().Where(t => (IsVisible(t) || IsReferenced(t)) && SyntaxFacts.IsValidIdentifier(t.Name)));

    /// 符号表中有这个符号：源码里引用到了（含经 InternalsVisibleTo 访问的 internal 成员）。
    bool IsReferenced(ISymbol s) => s.GetDocumentationCommentId() is { } id && symbols.Map.ContainsKey(SymbolKey.Global(_prefix + id));

    /// 与 target 同一位置的另一个符号键（委托的 Invoke）。
    void Alias(ISymbol alias, ViewMember? target, ISymbol owner)
    {
        if (target is not { } t || t.Key != _prefix + owner.GetDocumentationCommentId()) return;
        if (alias.GetDocumentationCommentId() is not { } id || !symbols.Map.ContainsKey(SymbolKey.Global(_prefix + id))) return;
        _members.Add(t with { Key = _prefix + id });
    }

    static int Order(ISymbol m) => m switch
    {
        IFieldSymbol { IsConst: true } => 0,
        IFieldSymbol => 1,
        IMethodSymbol { MethodKind: MethodKind.Constructor } => 2,
        IMethodSymbol { MethodKind: MethodKind.Destructor } => 3,
        IPropertySymbol { IsIndexer: false } => 4,
        IPropertySymbol => 5,
        IEventSymbol => 6,
        IMethodSymbol { MethodKind: MethodKind.UserDefinedOperator or MethodKind.Conversion } => 8,
        _ => 7,
    };

    static bool IsVisible(ISymbol s) => s.DeclaredAccessibility is Accessibility.Public or Accessibility.Protected or Accessibility.ProtectedOrInternal;

    /// 公共与受保护成员；源码里实际引用到的 internal / private 成员（InternalsVisibleTo）也写出，否则跳转会落空。
    bool IsDeclared(ISymbol m)
    {
        if (m.IsImplicitlyDeclared || m is INamedTypeSymbol || !IsVisible(m) && !IsReferenced(m)) return false;
        return m switch
        {
            IMethodSymbol method => method.MethodKind switch
            {
                MethodKind.Ordinary or MethodKind.UserDefinedOperator or MethodKind.Conversion => SyntaxFacts.IsValidIdentifier(method.Name)
                    || method.MethodKind != MethodKind.Ordinary,
                MethodKind.Constructor or MethodKind.Destructor => true,
                _ => false,
            },
            // 带参数但不是索引器的属性（VB 风格）在 C# 中无法声明。
            IPropertySymbol p => p.IsIndexer || p.Parameters.Length == 0 && SyntaxFacts.IsValidIdentifier(p.Name),
            IFieldSymbol or IEventSymbol => SyntaxFacts.IsValidIdentifier(m.Name),
            _ => false,
        };
    }

    /// 把声明名处不带变体的 Name<T> 替换为带变体的 Name<out T>。
    Rendered Variance(INamedTypeSymbol type, Rendered r)
    {
        if (type.TypeParameters.All(p => p.Variance == VarianceKind.None)) return r;
        var plain = type.ToDisplayString(PlainNameFormat);
        var varied = type.ToDisplayString(VarianceNameFormat);
        if (string.CompareOrdinal(r.Text, r.NameStart, plain, 0, plain.Length) != 0) return r;
        r.Replace(r.NameStart, plain.Length, varied);
        return r;
    }

    string TypeRef(ITypeSymbol t) => Render(t, TypeRefFormat).Text;

    void DeclareLine(ISymbol symbol, string indent, string text, int nameStart, int nameLength, string suffix = "") =>
        DeclareLine(symbol, indent, new Rendered(text, nameStart, nameLength), suffix);

    void DeclareLine(ISymbol symbol, string indent, Rendered r, string suffix = "")
    {
        _declared++;
        var id = symbol.GetDocumentationCommentId();
        if (id != null && docs(id) is { } xml)
        {
            _documented++;
            foreach (var l in DocLines(xml)) _lines.Add(indent + (l.Length == 0 ? "///" : "/// " + l));
        }
        int line = _lines.Count;
        var text = r.Text;
        _lines.Add(indent + text + suffix);
        uint indentBytes = (uint)Encoding.UTF8.GetByteCount(indent);
        uint ColOf(int at) => indentBytes + (uint)Encoding.UTF8.GetByteCount(text.AsSpan(0, at));
        // 命名实参引用的外部参数：位置落在本行签名里的参数名上（#126）。参数不进段 11，签名取所属成员的。
        foreach (var p in r.Parameters)
        {
            if (ParameterKey(p.Symbol) is not { } pkey || !symbols.Map.ContainsKey(SymbolKey.Global(pkey))) continue;
            _members.Add(new ViewMember(pkey, (uint)line, ColOf(p.Start),
                (uint)Encoding.UTF8.GetByteCount(text.AsSpan(p.Start, p.Length))));
        }
        if (id == null) return;
        var key = _prefix + id;
        // 只为符号表中存在的符号（被引用到的成员与类型链）写位置记录。
        if (!symbols.Map.ContainsKey(SymbolKey.Global(key))) return;
        uint len = (uint)Encoding.UTF8.GetByteCount(text.AsSpan(r.NameStart, r.NameLength));
        _members.Add(new ViewMember(key, (uint)line, ColOf(r.NameStart), len));
        _referenced.Add((key, line));
    }

    /// 参数的全局键，与 ProjectBinder 里命名实参引用用的键一致。
    static string? ParameterKey(IParameterSymbol p) => ProjectBinder.OwnerOf(p) is { } owner
        ? ProjectBinder.ParameterKey(owner.assembly, owner.ownerId, owner.ordinal) : null;

    /// 渲染并定位成员名：取显示片段中指向该符号本身的名字片段；构造函数、析构函数取类型名，索引器取 this，运算符取 operator。
    /// 同时记下签名里各参数名的位置。
    Rendered Render(ISymbol symbol, SymbolDisplayFormat format)
    {
        var parts = symbol.ToDisplayParts(format);
        var sb = new StringBuilder();
        var parameters = new List<ParameterSpan>();
        int nameStart = -1, nameLength = 0;
        foreach (var part in parts)
        {
            var s = part.ToString();
            if (nameStart < 0 && IsName(symbol, part, s))
            {
                nameStart = sb.Length;
                nameLength = s.Length;
            }
            if (part.Kind == SymbolDisplayPartKind.ParameterName && part.Symbol is IParameterSymbol parameter)
                parameters.Add(new ParameterSpan(parameter, sb.Length, s.Length));
            if (part.Symbol is INamedTypeSymbol { ContainingNamespace: { IsGlobalNamespace: false } ns } named
                && !SymbolEqualityComparer.Default.Equals(named, symbol) && named.TypeKind != TypeKind.Error)
                _usings.Add(ns.ToDisplayString());
            sb.Append(s);
        }
        var text = sb.ToString();
        if (nameStart < 0)
        {
            nameStart = Math.Max(0, text.IndexOf('('));
            nameLength = 0;
        }
        var result = new Rendered(text, nameStart, nameLength);
        result.Parameters.AddRange(parameters);
        return result;
    }

    static bool IsName(ISymbol symbol, SymbolDisplayPart part, string text) => symbol switch
    {
        IPropertySymbol { IsIndexer: true } => part.Kind == SymbolDisplayPartKind.Keyword && text == "this",
        IMethodSymbol { MethodKind: MethodKind.UserDefinedOperator or MethodKind.Conversion } =>
            part.Kind == SymbolDisplayPartKind.Keyword && text == "operator",
        IMethodSymbol { MethodKind: MethodKind.Constructor or MethodKind.Destructor } m =>
            text == m.ContainingType.Name && part.Kind is not SymbolDisplayPartKind.Keyword and not SymbolDisplayPartKind.Punctuation,
        _ => SymbolEqualityComparer.Default.Equals(part.Symbol, symbol)
            && part.Kind is not SymbolDisplayPartKind.Keyword and not SymbolDisplayPartKind.Punctuation and not SymbolDisplayPartKind.Space,
    };

    /// XML 文档原样保留（含 see cref、paramref 等标签），只去掉公共缩进与首尾空行。
    static IEnumerable<string> DocLines(string xml)
    {
        var lines = xml.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n').Select(l => l.TrimEnd()).ToList();
        while (lines.Count > 0 && lines[0].Length == 0) lines.RemoveAt(0);
        while (lines.Count > 0 && lines[^1].Length == 0) lines.RemoveAt(lines.Count - 1);
        // 以首行的缩进为准：BCL 的文档里续行常常顶格，取最小值会让整段保留首行的缩进。
        int indent = lines.Count == 0 ? 0 : lines[0].Length - lines[0].TrimStart().Length;
        return lines.Select(l => l[Math.Min(indent, l.Length - l.TrimStart().Length)..]);
    }
}

/// XML 文档文件：DLL 旁的同名 .xml；BCL 程序集另查本机 .NET SDK 的参考程序集包。只读本地文件，不联网，不解析 DTD。
sealed class XmlDocs(HashSet<string> types)
{
    readonly HashSet<string>.AlternateLookup<ReadOnlySpan<char>> _types = types.GetAlternateLookup<ReadOnlySpan<char>>();

    readonly ConcurrentDictionary<string, Lazy<Dictionary<string, string>?>> _files = new(StringComparer.OrdinalIgnoreCase);
    readonly ConcurrentQueue<string> _missing = new();
    static readonly string? PacksDir = FindPacks();

    public IEnumerable<string> MissingWarnings() => _missing;

    public Func<string, string?> For(string? dllPath, string assemblyName)
    {
        var candidates = new List<string>();
        if (dllPath != null) candidates.Add(Path.ChangeExtension(dllPath, ".xml"));
        if (PacksDir != null && IsFramework(assemblyName))
        {
            candidates.Add(Path.Combine(PacksDir, "NETStandard.Library.Ref", "2.1.0", "ref", "netstandard2.1", "netstandard.xml"));
            if (LatestRefPack() is { } refDir) candidates.Add(Path.Combine(refDir, assemblyName + ".xml"));
        }
        var files = candidates.Where(File.Exists).Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
        if (files.Length == 0)
        {
            _missing.Enqueue($"no XML documentation for {assemblyName} ({dllPath}); its metadata views have signatures only");
            return _ => null;
        }
        return id =>
        {
            foreach (var f in files)
                if (_files.GetOrAdd(f, x => new Lazy<Dictionary<string, string>?>(() => Load(x))).Value is { } map
                    && map.TryGetValue(id, out var doc)) return doc;
            return null;
        };
    }

    /// 文档注释 ID（如 M:Ns.Outer.Inner.Method(System.Int32)）属于某个被请求的最外层类型：
    /// 冒号之后、参数括号之前的每个以点分隔的前缀，任何一个是被请求的类型名即可。
    bool Wanted(string id)
    {
        if (id.Length < 3 || id[1] != ':') return false;
        var body = id.AsSpan(2);
        int paren = body.IndexOf('(');
        if (paren >= 0) body = body[..paren];
        for (int i = 0; i <= body.Length; i++)
            if ((i == body.Length || body[i] == '.') && _types.Contains(body[..i])) return true;
        return false;
    }

    static bool IsFramework(string name) => name is "mscorlib" or "netstandard" or "System"
        || name.StartsWith("System.", StringComparison.Ordinal) || name.StartsWith("Microsoft.Win32.", StringComparison.Ordinal);

    static string? FindPacks()
    {
        // 运行时目录形如 <dotnet>/shared/Microsoft.NETCore.App/9.0.x/
        var runtime = Path.GetDirectoryName(typeof(object).Assembly.Location);
        var root = runtime == null ? null : Path.GetFullPath(Path.Combine(runtime, "..", "..", ".."));
        var packs = root == null ? null : Path.Combine(root, "packs");
        return packs != null && Directory.Exists(packs) ? packs : null;
    }

    static string? LatestRefPack()
    {
        var dir = Path.Combine(PacksDir!, "Microsoft.NETCore.App.Ref");
        if (!Directory.Exists(dir)) return null;
        var version = Directory.GetDirectories(dir).Select(Path.GetFileName)
            .Select(n => (n, v: Version.TryParse(n?.Split('-')[0], out var v) ? v : null)).Where(x => x.v != null)
            .OrderByDescending(x => x.v).FirstOrDefault().n;
        if (version == null) return null;
        var refDir = Path.Combine(dir, version, "ref");
        return Directory.Exists(refDir) ? Directory.GetDirectories(refDir).OrderByDescending(d => d, StringComparer.Ordinal).FirstOrDefault() : null;
    }

    Dictionary<string, string>? Load(string path)
    {
        var bytes = Program.ReadShared(path);
        if (bytes == null)
        {
            _missing.Enqueue($"XML documentation {path} is unreadable");
            return null;
        }
        try
        {
            var map = new Dictionary<string, string>(StringComparer.Ordinal);
            var settings = new XmlReaderSettings { DtdProcessing = DtdProcessing.Ignore, XmlResolver = null, IgnoreComments = true };
            using var reader = XmlReader.Create(new MemoryStream(bytes), settings);
            while (!reader.EOF)
            {
                if (reader.NodeType == XmlNodeType.Element && reader.Name == "member" && reader.GetAttribute("name") is { } name)
                {
                    if (!Wanted(name))
                    {
                        reader.Skip();
                        continue;
                    }
                    var inner = reader.ReadInnerXml();
                    map.TryAdd(name, inner);
                }
                else reader.Read();
            }
            return map;
        }
        catch (XmlException e)
        {
            _missing.Enqueue($"XML documentation {path} is malformed: {e.Message}");
            return null;
        }
    }
}

/// 写索引时统计的元数据视图数据，进 --report。
sealed class MetadataStats
{
    public int Views, Members, SyntaxErrorViews;
    public long TextBytes, Declared, Documented;
    public long ExternalSymbols, ExternalSymbolsMissing, ExternalRefs, ExternalRefsMissing;
    readonly Dictionary<string, long> _missingByKind = new(StringComparer.Ordinal);
    readonly List<(string key, long refs)> _missing = new();
    readonly List<(string type, string identity, long bytes)> _sizes = new();

    public void Add(MetadataView view, int type, long bytes, int members)
    {
        Views++;
        Members += members;
        TextBytes += bytes;
        Declared += view.Declared;
        Documented += view.Documented;
        if (view.SyntaxErrors) SyntaxErrorViews++;
        _sizes.Add((view.TypeKey, view.Identity, bytes));
    }

    public void External(SymRec s, int refs, bool hit)
    {
        ExternalSymbols++;
        ExternalRefs += refs;
        if (hit) return;
        ExternalSymbolsMissing++;
        ExternalRefsMissing += refs;
        var kind = ((SymbolKind)s.Kind).ToString();
        _missingByKind[kind] = _missingByKind.GetValueOrDefault(kind) + refs;
        if (refs > 0) _missing.Add((s.Key.Text ?? s.Display, refs));
    }

    public object Report() => new
    {
        views = Views,
        view_members = Members,
        view_text_bytes = TextBytes,
        declared_members = Declared,
        documented_members = Documented,
        documented_ratio = Declared == 0 ? 0 : Math.Round((double)Documented / Declared, 4),
        views_with_syntax_errors = SyntaxErrorViews,
        external_symbols = ExternalSymbols,
        external_symbols_without_target = ExternalSymbolsMissing,
        external_references = ExternalRefs,
        external_references_without_target = ExternalRefsMissing,
        external_miss_ratio = ExternalRefs == 0 ? 0 : Math.Round((double)ExternalRefsMissing / ExternalRefs, 4),
        missing_by_kind = _missingByKind.OrderByDescending(kv => kv.Value).ThenBy(kv => kv.Key, StringComparer.Ordinal)
            .ToDictionary(kv => kv.Key, kv => kv.Value),
        top_missing = _missing.OrderByDescending(m => m.refs).ThenBy(m => m.key, StringComparer.Ordinal).Take(40)
            .ToDictionary(m => m.key, m => m.refs),
        largest_views = _sizes.OrderByDescending(s => s.bytes).ThenBy(s => s.type, StringComparer.Ordinal).Take(20)
            .Select(s => new { type = s.type, assembly = s.identity, bytes = s.bytes }).ToArray(),
        warnings = MetadataViews.Warnings,
        warning_samples = MetadataViews.WarningLog.Take(40).ToArray(),
    };
}
