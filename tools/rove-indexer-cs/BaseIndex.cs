// #35 常驻模式读取基础索引（RVCSIDX v2，格式见 docs/CS_INDEX_FORMAT.md）：影响分析的反查只需要
// 文件表、符号表（按键二分）、按符号的出现位置与“按 to 排序的关系”。整个文件只读映射，不拷贝。

using System.IO.MemoryMappedFiles;
using System.Text;

namespace Rove.Indexer;

sealed unsafe class BaseIndex : IDisposable
{
    const int TableOffset = 96;
    const uint LocalBit = 1u << 31;
    enum S { StringOffsets, Files, Paths, Symbols, LocalKeys, Occurrences, WideOccurrences, References, RelationsFrom, RelationsTo, Inactive, Signatures, XmlDocs, MetadataViews, Strings, Count }

    readonly MemoryMappedFile _file;
    readonly MemoryMappedViewAccessor _view;
    readonly byte* _base;
    readonly long _length;
    readonly (long offset, long count)[] _sections = new (long, long)[(int)S.Count];
    readonly int _refWidth;
    /// 全局键在前、局部键在后；这是第一个局部键符号的序号。
    readonly int _firstLocal;
    public readonly byte[] Identity;
    public readonly string Path;

    public BaseIndex(string path)
    {
        Path = path;
        using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
        _length = fs.Length;
        _file = MemoryMappedFile.CreateFromFile(fs, null, 0, MemoryMappedFileAccess.Read, HandleInheritability.None, leaveOpen: false);
        _view = _file.CreateViewAccessor(0, 0, MemoryMappedFileAccess.Read);
        byte* p = null;
        _view.SafeMemoryMappedViewHandle.AcquirePointer(ref p);
        _base = p + _view.PointerOffset;
        if (_length < TableOffset + 16 * (int)S.Count || !new ReadOnlySpan<byte>(_base, 8).SequenceEqual("RVCSIDX\0"u8)
            || U32(8) != 2 || U32(12) != (uint)S.Count || (long)U64(16) != _length)
            throw new InvalidDataException($"not a RVCSIDX v2 index: {path}");
        for (int i = 0; i < (int)S.Count; i++)
            _sections[i] = ((long)U64(TableOffset + i * 16), (long)U64(TableOffset + i * 16 + 8));
        _refWidth = OccurrenceCount <= 1 << 24 ? 3 : 4;
        Identity = IdentityOf(new ReadOnlySpan<byte>(_base, 96));
        int lo = 0, hi = SymbolCount;
        while (lo < hi)
        {
            int mid = (lo + hi) >>> 1;
            if ((SymField(mid, 0) & LocalBit) == 0) lo = mid + 1; else hi = mid;
        }
        _firstLocal = lo;
    }

    /// 覆盖层与基础索引的配套标识：头部校验和（偏移 88）+ 文件总长（偏移 16），共 16 字节。
    static byte[] IdentityOf(ReadOnlySpan<byte> header)
    {
        var id = new byte[16];
        header[88..96].CopyTo(id);
        header[16..24].CopyTo(id.AsSpan(8));
        return id;
    }

    public static byte[] ReadIdentity(string path)
    {
        using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
        var header = new byte[96];
        fs.ReadExactly(header);
        return IdentityOf(header);
    }

    uint U32(long at) => *(uint*)(_base + at);
    ulong U64(long at) => *(ulong*)(_base + at);
    long Section(S s) => _sections[(int)s].offset;
    long CountOf(S s) => _sections[(int)s].count;

    public int FileCount => (int)CountOf(S.Files);
    public int SymbolCount => (int)CountOf(S.Symbols);
    public long OccurrenceCount => CountOf(S.Occurrences);

    uint FileField(int file, int field) => U32(Section(S.Files) + file * 24L + field * 4);
    public ulong FileHash(int file) => U64(Section(S.Files) + file * 24L + 16);

    ReadOnlySpan<byte> PathBytes(int file)
    {
        long start = FileField(file, 0);
        long end = file + 1 < FileCount ? FileField(file + 1, 0) : CountOf(S.Paths);
        return new ReadOnlySpan<byte>(_base + Section(S.Paths) + start, (int)(end - start));
    }

    public string FilePath(int file) => Encoding.UTF8.GetString(PathBytes(file));

    /// 路径的文件序号（精确字节比较 + 二分）；没有时返回 -1。
    public int FileId(string path)
    {
        var needle = Encoding.UTF8.GetBytes(path);
        int lo = 0, hi = FileCount - 1;
        while (lo <= hi)
        {
            int mid = (lo + hi) >>> 1;
            int c = PathBytes(mid).SequenceCompareTo(needle);
            if (c == 0) return mid;
            if (c < 0) lo = mid + 1; else hi = mid - 1;
        }
        return -1;
    }

    public string Str(uint id)
    {
        long offsets = Section(S.StringOffsets);
        uint start = U32(offsets + id * 4L), end = U32(offsets + (id + 1) * 4L);
        return Encoding.UTF8.GetString(_base + Section(S.Strings) + start, (int)(end - start));
    }

    uint SymField(int symbol, int field) => U32(Section(S.Symbols) + symbol * 24L + field * 4);
    public string Name(int symbol) => Str(SymField(symbol, 1));
    public string Display(int symbol) => Str(SymField(symbol, 2));
    public byte Kind(int symbol) => _base[Section(S.Symbols) + symbol * 24L + 20];
    public bool InSource(int symbol) => (_base[Section(S.Symbols) + symbol * 24L + 21] & 1) != 0;
    public int Container(int symbol) => SymField(symbol, 3) is var c && c < SymbolCount ? (int)c : -1;

    /// 全局键文本；局部键返回 null。
    public string? GlobalKey(int symbol) => SymField(symbol, 0) is var k && (k & LocalBit) == 0 ? Str(k) : null;

    public (int file, uint offset)? LocalKey(int symbol)
    {
        uint k = SymField(symbol, 0);
        if ((k & LocalBit) == 0) return null;
        long at = Section(S.LocalKeys) + (k & ~LocalBit) * 8L;
        return ((int)U32(at), U32(at + 4));
    }

    /// 全局键的符号序号（键按 UTF-16 Ordinal 排序，与写入端一致）；没有时返回 -1。
    public int FindGlobal(string key)
    {
        int lo = 0, hi = _firstLocal - 1;
        while (lo <= hi)
        {
            int mid = (lo + hi) >>> 1;
            int c = string.CompareOrdinal(Str(SymField(mid, 0)), key);
            if (c == 0) return mid;
            if (c < 0) lo = mid + 1; else hi = mid - 1;
        }
        return -1;
    }

    public int FindLocal(int file, uint offset)
    {
        int lo = _firstLocal, hi = SymbolCount - 1;
        ulong needle = ((ulong)(uint)file << 32) | offset;
        while (lo <= hi)
        {
            int mid = (lo + hi) >>> 1;
            var (f, o) = LocalKey(mid)!.Value;
            ulong v = ((ulong)(uint)f << 32) | o;
            if (v == needle) return mid;
            if (v < needle) lo = mid + 1; else hi = mid - 1;
        }
        return -1;
    }

    /// 在某个文件里声明的局部键符号（局部键按（文件, 偏移）排序，一段连续区间）。
    public IEnumerable<int> LocalsDeclaredIn(int file)
    {
        int lo = _firstLocal, hi = SymbolCount;
        while (lo < hi)
        {
            int mid = (lo + hi) >>> 1;
            if (LocalKey(mid)!.Value.file < file) lo = mid + 1; else hi = mid;
        }
        for (int s = lo; s < SymbolCount && LocalKey(s)!.Value.file == file; s++) yield return s;
    }

    public readonly record struct Occ(int File, uint Line, uint Col, uint Len, int Symbol, ushort Role);

    public Occ Occurrence(long index)
    {
        ulong packed = U64(Section(S.Occurrences) + index * 8);
        if ((packed & 7) != 0)
            return new Occ((int)(packed >> 49), (uint)((packed >> 16) & 0xfff), (uint)((packed >> 8) & 0xff), (uint)((packed >> 3) & 0x1f),
                (int)((packed >> 28) & 0x1f_ffff), (ushort)(packed & 7));
        long at = Section(S.WideOccurrences) + (long)(packed >> 3) * 24;
        return new Occ((int)U32(at), U32(at + 4), U32(at + 8), U32(at + 16), (int)U32(at + 12), (ushort)U32(at + 20));
    }

    /// 符号的全部出现位置（定义在前）。
    public IEnumerable<Occ> OccurrencesOf(int symbol)
    {
        long start = SymField(symbol, 4);
        long end = symbol + 1 < SymbolCount ? SymField(symbol + 1, 4) : OccurrenceCount;
        for (long k = start; k < end; k++) yield return Occurrence(RefId(k));
    }

    uint RefId(long k)
    {
        byte* p = _base + Section(S.References) + k * _refWidth;
        return (uint)(p[0] | p[1] << 8 | p[2] << 16 | (_refWidth == 4 ? p[3] << 24 : 0));
    }

    /// 某个文件的全部出现位置（文件内按行列排序）。
    public IEnumerable<Occ> OccurrencesIn(int file)
    {
        long start = FileField(file, 1), count = FileField(file, 2);
        for (long k = start; k < start + count; k++) yield return Occurrence(k);
    }

    /// 以 symbol 为 to 的关系的 from 符号（实现、重写、派生）。
    public IEnumerable<int> RelationSources(int symbol)
    {
        long at = Section(S.RelationsTo), n = CountOf(S.RelationsTo);
        long lo = 0, hi = n;
        while (lo < hi)
        {
            long mid = (lo + hi) >>> 1;
            if (U32(at + mid * 12 + 4) < (uint)symbol) lo = mid + 1; else hi = mid;
        }
        for (long k = lo; k < n && U32(at + k * 12 + 4) == (uint)symbol; k++) yield return (int)U32(at + k * 12);
    }

    public void Dispose()
    {
        _view.SafeMemoryMappedViewHandle.ReleasePointer();
        _view.Dispose();
        _file.Dispose();
    }
}
