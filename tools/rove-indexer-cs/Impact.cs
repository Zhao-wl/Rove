// #35 影响分析：一个文件重绑后，哪些声明的“签名或可见性”变了、哪些名字是新出现的。
//
// 规则（判定只在 Impact.Diff 一处，可单独测试，见 SelfTest.cs）：
//   - 声明指纹 = 带可访问性、修饰符、返回类型、参数（含名字）、类型参数约束与基类型列表的完整显示串的 xxHash64；
//     方法体、字段初始值、常量的值不参与，所以只改实现不会波及其他文件。
//   - 旧声明里有、新声明里没有（删除、改名、参数类型变了导致文档注释 ID 变了）→ 变化；
//     两边都有但指纹不同（可见性、修饰符、返回类型、参数名、基类型列表、约束）→ 变化；
//   - 新声明里才有的键 → 它的简单名字是“新出现的名字”：之前绑定到别处或解析不到的同名标识符可能改绑。
// 引擎再把它们换成文件：变化的键按引用表反查（含指向它的实现/重写关系的声明文件），
// 新名字在能看到该项目的项目范围内按标识符做文本查找。两者都进后台队列，可取消、限速。

using System.Text;
using Microsoft.CodeAnalysis;

namespace Rove.Indexer;

/// 一条声明：指纹（0 表示未知）、弱指纹（显示名 + 种类，基础索引里能还原）与简单名字。
readonly record struct Decl(ulong Fingerprint, ulong Weak, string Name);

static class Fingerprints
{
    static readonly SymbolDisplayFormat Full = new(
        globalNamespaceStyle: SymbolDisplayGlobalNamespaceStyle.Included,
        typeQualificationStyle: SymbolDisplayTypeQualificationStyle.NameAndContainingTypesAndNamespaces,
        genericsOptions: SymbolDisplayGenericsOptions.IncludeTypeParameters | SymbolDisplayGenericsOptions.IncludeTypeConstraints
            | SymbolDisplayGenericsOptions.IncludeVariance,
        memberOptions: SymbolDisplayMemberOptions.IncludeParameters | SymbolDisplayMemberOptions.IncludeType
            | SymbolDisplayMemberOptions.IncludeRef | SymbolDisplayMemberOptions.IncludeContainingType
            | SymbolDisplayMemberOptions.IncludeModifiers | SymbolDisplayMemberOptions.IncludeAccessibility
            | SymbolDisplayMemberOptions.IncludeExplicitInterface,
        kindOptions: SymbolDisplayKindOptions.IncludeMemberKeyword | SymbolDisplayKindOptions.IncludeTypeKeyword,
        propertyStyle: SymbolDisplayPropertyStyle.ShowReadWriteDescriptor,
        parameterOptions: SymbolDisplayParameterOptions.IncludeName | SymbolDisplayParameterOptions.IncludeType
            | SymbolDisplayParameterOptions.IncludeParamsRefOut | SymbolDisplayParameterOptions.IncludeDefaultValue
            | SymbolDisplayParameterOptions.IncludeExtensionThis | SymbolDisplayParameterOptions.IncludeOptionalBrackets,
        miscellaneousOptions: SymbolDisplayMiscellaneousOptions.UseSpecialTypes
            | SymbolDisplayMiscellaneousOptions.IncludeNullableReferenceTypeModifier);

    /// 参与影响分析的声明：类型与成员（有全局键的那些）。局部变量、参数、类型参数是文件内的局部键。
    public static bool IsSurface(ISymbol s) => s.Kind is SymbolKind.NamedType or SymbolKind.Method or SymbolKind.Property
        or SymbolKind.Field or SymbolKind.Event
        && s is not IMethodSymbol { MethodKind: MethodKind.LocalFunction or MethodKind.AnonymousFunction };

    public static ulong Of(ISymbol s)
    {
        var text = new StringBuilder();
        text.Append(s.Kind).Append('|').Append(s.DeclaredAccessibility).Append('|');
        text.Append(s.IsStatic ? 's' : '-').Append(s.IsAbstract ? 'a' : '-').Append(s.IsVirtual ? 'v' : '-')
            .Append(s.IsOverride ? 'o' : '-').Append(s.IsSealed ? 'x' : '-').Append(s.IsExtern ? 'e' : '-').Append('|');
        try { text.Append(s.ToDisplayString(Full)); } catch { text.Append(s.Name); }
        switch (s)
        {
            case INamedTypeSymbol t:
                text.Append('|').Append(t.TypeKind).Append('|').Append(t.IsReadOnly ? 'r' : '-').Append(t.IsRefLikeType ? 'R' : '-');
                if (t.BaseType is { } b) text.Append("|:").Append(Safe(b));
                foreach (var i in t.Interfaces) text.Append("|+").Append(Safe(i));
                break;
            case IMethodSymbol m:
                text.Append('|').Append(m.MethodKind).Append(m.IsExtensionMethod ? 'X' : '-').Append(m.IsReadOnly ? 'r' : '-');
                break;
            case IFieldSymbol f:
                text.Append('|').Append(f.IsConst ? 'c' : '-').Append(f.IsReadOnly ? 'r' : '-').Append(f.IsVolatile ? 'v' : '-');
                break;
            case IPropertySymbol p:
                text.Append('|').Append(p.GetMethod?.DeclaredAccessibility).Append('/').Append(p.SetMethod?.DeclaredAccessibility)
                    .Append(p.SetMethod?.IsInitOnly == true ? 'i' : '-');
                break;
        }
        return Hash(text.ToString()) | 1; // 0 留给“未知”
    }

    /// 基础索引里能还原的弱指纹：显示名 + 符号种类。可见性、修饰符变化看不出来。
    public static ulong Weak(string display, byte kind) => Hash(display + "|" + kind) | 1;

    public static ulong Hash(string text) => System.IO.Hashing.XxHash64.HashToUInt64(Encoding.UTF8.GetBytes(text));

    static string Safe(ISymbol s)
    {
        try { return s.ToDisplayString(Full); } catch { return s.Name; }
    }

    /// 影响分析用的名字：构造函数用类型名（`new Foo()` 处写的是类型名），其他不是标识符的成员名（运算符、索引器、析构）返回 null。
    public static string? SearchName(ISymbol s) => s switch
    {
        IMethodSymbol { MethodKind: MethodKind.Constructor or MethodKind.StaticConstructor } => s.ContainingType?.Name,
        IMethodSymbol { MethodKind: MethodKind.UserDefinedOperator or MethodKind.Conversion or MethodKind.Destructor } => null,
        IPropertySymbol { IsIndexer: true } => null,
        _ => IsIdentifier(s.Name) ? s.Name : null,
    };

    public static bool IsIdentifier(string name) => name.Length > 0 && (char.IsLetter(name[0]) || name[0] == '_')
        && name.All(c => char.IsLetterOrDigit(c) || c == '_');
}

static class Impact
{
    /// 一个文件重绑前后的声明比较。返回签名或可见性变化（含删除）的键，与新出现的名字（按 Ordinal 排序、去重）。
    /// 旧声明的指纹未知（0）时退回比较弱指纹。
    public static (List<string> changed, List<string> addedNames) Diff(
        IReadOnlyDictionary<string, Decl> before, IReadOnlyDictionary<string, Decl> after)
    {
        var changed = new List<string>();
        foreach (var (key, old) in before)
        {
            if (!after.TryGetValue(key, out var now)) { changed.Add(key); continue; }
            bool same = old.Fingerprint != 0 && now.Fingerprint != 0 ? old.Fingerprint == now.Fingerprint : old.Weak == now.Weak;
            if (!same) changed.Add(key);
        }
        var added = new SortedSet<string>(StringComparer.Ordinal);
        foreach (var (key, now) in after)
            if (!before.ContainsKey(key) && now.Name.Length > 0) added.Add(now.Name);
        changed.Sort(StringComparer.Ordinal);
        return (changed, added.ToList());
    }

    /// 文本里是否出现名为 name 的标识符（前后都不是标识符字符；允许 `@` 前缀）。UTF-8 字节上查找。
    public static bool ContainsIdentifier(ReadOnlySpan<byte> text, ReadOnlySpan<byte> name)
    {
        int from = 0;
        while (from <= text.Length - name.Length)
        {
            int i = text[from..].IndexOf(name);
            if (i < 0) return false;
            i += from;
            int end = i + name.Length;
            bool left = i == 0 || !IsIdentByte(text[i - 1]);
            bool right = end >= text.Length || !IsIdentByte(text[end]);
            if (left && right) return true;
            from = i + 1;
        }
        return false;
    }

    static bool IsIdentByte(byte b) => b is >= (byte)'a' and <= (byte)'z' or >= (byte)'A' and <= (byte)'Z' or >= (byte)'0' and <= (byte)'9'
        or (byte)'_' or >= 0x80;
}

/// 全量索引的旁路文件 `<索引>.decls`：源码声明的键哈希 → 声明指纹，按键哈希排序。
/// 不属于索引格式（读取端不需要它）；只给常驻模式还原“旧声明的指纹”。缺失时退回弱指纹。
static class DeclarationTable
{
    static readonly byte[] Magic = "RVCSDCL\0"u8.ToArray();
    const uint Version = 1;

    public static void Write(string indexPath, IReadOnlyDictionary<string, ulong> declarations)
    {
        var entries = declarations.Select(kv => (key: Fingerprints.Hash(kv.Key), fp: kv.Value)).ToArray();
        Array.Sort(entries);
        string path = indexPath + ".decls";
        string temp = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            using (var fs = new FileStream(temp, FileMode.CreateNew, FileAccess.Write))
            using (var w = new BinaryWriter(fs))
            {
                w.Write(Magic);
                w.Write(Version);
                w.Write(entries.Length);
                w.Write(BaseIndex.ReadIdentity(indexPath));
                foreach (var (key, fp) in entries) { w.Write(key); w.Write(fp); }
            }
            File.Move(temp, path, overwrite: true);
        }
        catch (Exception e) { Program.Log($"warning: cannot write declaration table: {e.Message}"); }
        finally { File.Delete(temp); }
    }

    /// 读取与给定基础索引配套的表；不存在、损坏或不配套时返回 null。
    public static Dictionary<ulong, ulong>? Read(string indexPath, byte[] identity)
    {
        try
        {
            var bytes = File.ReadAllBytes(indexPath + ".decls");
            if (bytes.Length < 32 || !bytes.AsSpan(0, 8).SequenceEqual(Magic) || BitConverter.ToUInt32(bytes, 8) != Version
                || !bytes.AsSpan(16, 16).SequenceEqual(identity)) return null;
            int count = BitConverter.ToInt32(bytes, 12);
            if (count < 0 || 32 + (long)count * 16 != bytes.Length) return null;
            var map = new Dictionary<ulong, ulong>(count);
            for (int i = 0; i < count; i++) map[BitConverter.ToUInt64(bytes, 32 + i * 16)] = BitConverter.ToUInt64(bytes, 40 + i * 16);
            return map;
        }
        catch (IOException) { return null; }
        catch (UnauthorizedAccessException) { return null; }
    }
}
