using System.Collections.Concurrent;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace Rove.Indexer;

record ProjectInfo(
    string Path, string AssemblyName, string[] Sources, string[] References,
    string[] ProjectRefs, string[] Analyzers, string[] Defines, LanguageVersion LangVersion, bool Unsafe);

enum Role : ushort { Ref = 1, Def = 2, Candidate = 4 }
enum RelKind : uint { BaseType = 1, Implements = 2, Overrides = 3 }

struct Occ { public uint Line, Col, Len; public int Sym; public ushort Role; }

/// 文件号在索引过程中是"临时号"：磁盘文件按最终路径序预先编号，生成树按出现顺序追加。
/// 写索引时统一按路径排序并重映射，所以生成树插进中间也不影响局部符号键。
readonly record struct SymbolKey(string? Text, uint File, uint Offset)
{
    public static SymbolKey Global(string text) => new(text, 0, 0);
    public static SymbolKey Local(uint file, uint offset) => new(null, file, offset);
}

/// 一个文件的全部产出。绑定完就不再需要语法树、语义模型和编译。
sealed class FileResult
{
    public required uint TempId;
    public required byte[] Path;
    public required ulong Hash;
    public Occ[] Occs = [];
    public (uint start, uint end)[] Inactive = [];
}

sealed class SymRec
{
    public int Id;
    /// 写索引时的最终序号。
    public int Final = -1;
    public required SymbolKey Key;
    public required string Name;
    public required string Display;
    public required byte Kind;
    public bool InSource;
    /// 来自外部 DLL（不是本次索引的任何项目）。写索引时与 InSource 一起判定：有源码版本的不算外部。
    public bool External;
    public SymbolKey? ContainerKey;
    /// 来自本项目其他程序集元数据镜像的参数：解析不到源码声明时使用的后备键。
    public string? ParameterFallback;
}

/// 全局符号表。键与 M0 相同，可跨编译稳定；同一键被不同符号对象命中时，
/// 名称、显示名等字段取确定的最小值，保证同样的输入产出同样的索引。
sealed class SymbolTable
{
    public readonly ConcurrentDictionary<SymbolKey, SymRec> Map = new();
    /// ("程序集|成员文档 ID", 序号) → 源码参数的局部键。供依赖项目里经元数据看到的参数回指源码声明。
    public readonly ConcurrentDictionary<(string owner, int ordinal), SymbolKey> SourceParameters = new();
    int _next = -1;
    public int MaxId => Volatile.Read(ref _next);

    /// 同一键第一次出现可能来自下游项目看到的元数据镜像，也可能来自源码；源码版本优先，
    /// 同为源码或同为元数据时（匿名类型成员、后备键等真正撞键的情形）取各字段的最小值。
    public SymRec GetOrAdd(SymbolKey key, Func<SymbolKey, SymRec> create, bool collisionProne, bool inSource)
    {
        if (Map.TryGetValue(key, out var rec))
        {
            if (inSource && !rec.InSource || collisionProne) Merge(rec, create(key));
            return rec;
        }
        var created = create(key);
        created.Id = Interlocked.Increment(ref _next);
        rec = Map.GetOrAdd(key, created);
        if (!ReferenceEquals(rec, created) && (inSource && !rec.InSource || collisionProne)) Merge(rec, created);
        return rec;
    }

    static void Merge(SymRec rec, SymRec other)
    {
        lock (rec)
        {
            int c = rec.InSource.CompareTo(other.InSource);
            if (c == 0) c = string.CompareOrdinal(other.Display, rec.Display);
            if (c == 0) c = string.CompareOrdinal(other.Name, rec.Name);
            if (c == 0) c = other.Kind.CompareTo(rec.Kind);
            if (c == 0) c = CompareKeys(other.ContainerKey, rec.ContainerKey);
            if (c >= 0) return;
            rec.InSource |= other.InSource;
            rec.Display = other.Display;
            rec.Name = other.Name;
            rec.Kind = other.Kind;
            rec.ContainerKey = other.ContainerKey;
        }
    }

    /// 若表中已有以该文本为键的符号，返回那条记录里的同一个字符串实例，避免成员各存一份容器键文本。
    public string Canonical(string keyText) => Map.TryGetValue(SymbolKey.Global(keyText), out var rec) ? rec.Key.Text! : keyText;

    public SymbolKey Canonical(SymbolKey key) => key.Text is { } text ? SymbolKey.Global(Canonical(text)) : key;

    static int CompareKeys(SymbolKey? a, SymbolKey? b)
    {
        if (a is not { } x) return b is null ? 0 : -1;
        if (b is not { } y) return 1;
        int c = string.CompareOrdinal(x.Text, y.Text);
        if (c == 0) c = x.File.CompareTo(y.File);
        return c != 0 ? c : x.Offset.CompareTo(y.Offset);
    }

    public void RegisterSourceParameter(string owner, int ordinal, SymbolKey local)
    {
        SourceParameters.AddOrUpdate((Canonical(owner), ordinal), local, (_, old) =>
            (local.File, local.Offset).CompareTo((old.File, old.Offset)) < 0 ? local : old);
    }
}
