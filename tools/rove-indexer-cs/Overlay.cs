// #35 覆盖层：一次重绑（一批文件）写成一个独立的段，追加到与基础索引配套的日志文件里，不重写基础索引。
// 布局见 docs/CS_INDEX_FORMAT.md「覆盖层（#35）」；读取与合并在 crates/rove-lang-cs/src/overlay.rs。

using System.Text;

namespace Rove.Indexer;

/// 覆盖层里的符号键：全局键（程序集|文档注释 ID 等）或局部键（项目相对路径 + 声明的 UTF-8 字节偏移）。
readonly record struct OvKey(string? Global, string? Path, uint Offset)
{
    public static OvKey Of(string global) => new(global, null, 0);
    public static OvKey Local(string path, uint offset) => new(null, path, offset);
    public override string ToString() => Global ?? $"{Path}@{Offset}";
}

sealed class OvSymbol
{
    public required OvKey Key;
    public required string Name;
    public required string Display;
    public byte Kind;
    public bool InSource, External;
    public OvKey? Container;
}

sealed class OvFile
{
    public required string Path;
    public bool Deleted;
    public ulong Hash;
    /// Sym 是本段符号表的下标。
    public Occ[] Occs = [];
    public (uint start, uint end)[] Inactive = [];
    /// 本文件声明的类型与成员（本段符号下标）及其声明指纹。
    public (int symbol, ulong fingerprint)[] Declarations = [];
}

/// 关系的归属：Self = 符号自己的基类型 / 重写关系；Members = 类型的成员实现接口成员的关系。
enum OwnerPart : byte { Self = 1, Members = 2 }

sealed class OvSegment
{
    public ulong Sequence;
    public readonly List<OvSymbol> Symbols = new();
    public readonly List<OvFile> Files = new();
    public readonly List<(int symbol, OwnerPart part)> Owners = new();
    public readonly List<(int from, int to, RelKind kind)> Relations = new();
    /// 写入时：键 → 下标。
    readonly Dictionary<OvKey, int> _index = new();

    public int Symbol(OvKey key, Func<OvSymbol> create)
    {
        if (_index.TryGetValue(key, out int i)) return i;
        i = Symbols.Count;
        Symbols.Add(create());
        _index[key] = i;
        return i;
    }

    public int? Find(OvKey key) => _index.TryGetValue(key, out int i) ? i : null;

    static readonly byte[] Magic = "RVCSOVL\0"u8.ToArray();
    public const uint Version = 1;
    const int HeaderLength = 48;

    public byte[] Encode(byte[] baseIdentity)
    {
        using var ms = new MemoryStream();
        using var w = new BinaryWriter(ms, Encoding.UTF8);
        w.Write(Magic);
        w.Write(Version);
        w.Write(0u);
        w.Write(0UL); // 长度，最后回填
        w.Write(baseIdentity);
        w.Write(Sequence);
        void Str(string s)
        {
            var bytes = Encoding.UTF8.GetBytes(s);
            w.Write(bytes.Length);
            w.Write(bytes);
        }
        void Key(OvKey? key)
        {
            switch (key)
            {
                case null: w.Write((byte)0); break;
                case { Global: { } g }: w.Write((byte)1); Str(g); break;
                case { } k: w.Write((byte)2); Str(k.Path!); w.Write(k.Offset); break;
            }
        }
        w.Write(Symbols.Count);
        foreach (var s in Symbols)
        {
            Key(s.Key);
            Str(s.Name);
            Str(s.Display);
            w.Write(s.Kind);
            w.Write((byte)((s.InSource ? 1 : 0) | (s.External ? 2 : 0)));
            Key(s.Container);
        }
        w.Write(Files.Count);
        foreach (var f in Files)
        {
            Str(f.Path);
            w.Write((byte)(f.Deleted ? 1 : 0));
            w.Write(f.Hash);
            w.Write(f.Occs.Length);
            foreach (var o in f.Occs)
            {
                w.Write(o.Line); w.Write(o.Col); w.Write(o.Len); w.Write((uint)o.Sym); w.Write((byte)o.Role);
            }
            w.Write(f.Inactive.Length);
            foreach (var (start, end) in f.Inactive) { w.Write(start); w.Write(end); }
            w.Write(f.Declarations.Length);
            foreach (var (symbol, fp) in f.Declarations) { w.Write((uint)symbol); w.Write(fp); }
        }
        w.Write(Owners.Count);
        foreach (var (symbol, part) in Owners) { w.Write((uint)symbol); w.Write((byte)part); }
        w.Write(Relations.Count);
        foreach (var (from, to, kind) in Relations) { w.Write((uint)from); w.Write((uint)to); w.Write((byte)kind); }
        while ((ms.Length + 8) % 8 != 0) w.Write((byte)0);
        w.Flush();
        long total = ms.Length + 8;
        ms.Position = 16;
        w.Write((ulong)total);
        w.Flush();
        var bytes = ms.ToArray();
        ulong checksum = Fnv(bytes);
        var result = new byte[total];
        bytes.CopyTo(result, 0);
        BitConverter.GetBytes(checksum).CopyTo(result, bytes.Length);
        return result;
    }

    static ulong Fnv(ReadOnlySpan<byte> bytes)
    {
        ulong h = 14695981039346656037UL;
        foreach (byte b in bytes) h = unchecked((h ^ b) * 1099511628211UL);
        return h;
    }

    /// 解出一个段；格式不对、校验失败或与基础索引不配套时返回 null。
    public static OvSegment? Decode(ReadOnlySpan<byte> data, byte[] baseIdentity, out int length)
    {
        length = 0;
        if (data.Length < HeaderLength + 8 || !data[..8].SequenceEqual(Magic) || BitConverter.ToUInt32(data[8..]) != Version) return null;
        ulong total = BitConverter.ToUInt64(data[16..]);
        if (total % 8 != 0 || total < HeaderLength + 8 || total > (ulong)data.Length) return null;
        var bytes = data[..(int)total];
        if (Fnv(bytes[..^8]) != BitConverter.ToUInt64(bytes[^8..]) || !bytes[24..40].SequenceEqual(baseIdentity)) return null;
        length = (int)total;
        try
        {
            var r = new BinaryReader(new MemoryStream(bytes[..^8].ToArray()), Encoding.UTF8);
            r.BaseStream.Position = 40;
            var seg = new OvSegment { Sequence = r.ReadUInt64() };
            string Str() => Encoding.UTF8.GetString(r.ReadBytes(r.ReadInt32()));
            OvKey? Key() => r.ReadByte() switch
            {
                0 => null,
                1 => OvKey.Of(Str()),
                2 => OvKey.Local(Str(), r.ReadUInt32()),
                _ => throw new InvalidDataException("bad key tag"),
            };
            int symbols = r.ReadInt32();
            for (int i = 0; i < symbols; i++)
            {
                var key = Key() ?? throw new InvalidDataException("missing key");
                var s = new OvSymbol { Key = key, Name = Str(), Display = Str(), Kind = r.ReadByte() };
                byte flags = r.ReadByte();
                s.InSource = (flags & 1) != 0;
                s.External = (flags & 2) != 0;
                s.Container = Key();
                seg._index[key] = seg.Symbols.Count;
                seg.Symbols.Add(s);
            }
            int files = r.ReadInt32();
            for (int i = 0; i < files; i++)
            {
                var f = new OvFile { Path = Str(), Deleted = r.ReadByte() != 0, Hash = r.ReadUInt64() };
                f.Occs = new Occ[r.ReadInt32()];
                for (int k = 0; k < f.Occs.Length; k++)
                    f.Occs[k] = new Occ { Line = r.ReadUInt32(), Col = r.ReadUInt32(), Len = r.ReadUInt32(), Sym = (int)r.ReadUInt32(), Role = r.ReadByte() };
                f.Inactive = new (uint, uint)[r.ReadInt32()];
                for (int k = 0; k < f.Inactive.Length; k++) f.Inactive[k] = (r.ReadUInt32(), r.ReadUInt32());
                f.Declarations = new (int, ulong)[r.ReadInt32()];
                for (int k = 0; k < f.Declarations.Length; k++) f.Declarations[k] = ((int)r.ReadUInt32(), r.ReadUInt64());
                seg.Files.Add(f);
            }
            int owners = r.ReadInt32();
            for (int i = 0; i < owners; i++) seg.Owners.Add(((int)r.ReadUInt32(), (OwnerPart)r.ReadByte()));
            int relations = r.ReadInt32();
            for (int i = 0; i < relations; i++) seg.Relations.Add(((int)r.ReadUInt32(), (int)r.ReadUInt32(), (RelKind)r.ReadByte()));
            return seg;
        }
        catch (Exception e) when (e is EndOfStreamException or InvalidDataException or ArgumentException) { length = 0; return null; }
    }
}

/// 覆盖层日志：与基础索引配套、只追加。每段写完即 Flush 到系统缓存（Rove 随即读取），空闲修剪与退出前刷盘。
/// 打开时校验已有的段，丢弃写到一半的尾部；属于其他基础索引的日志整体清空。
sealed class Journal : IDisposable
{
    readonly FileStream _stream;
    readonly byte[] _identity;
    public ulong LastSequence { get; private set; }
    public long Length => _stream.Length;
    public int Segments { get; private set; }

    public Journal(string path, byte[] identity, out List<OvSegment> existing)
    {
        _identity = identity;
        _stream = new FileStream(path, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.Read | FileShare.Delete);
        existing = new List<OvSegment>();
        var bytes = new byte[_stream.Length];
        _stream.ReadExactly(bytes);
        int at = 0;
        while (at < bytes.Length && OvSegment.Decode(bytes.AsSpan(at), identity, out int length) is { } seg && seg.Sequence > LastSequence)
        {
            existing.Add(seg);
            LastSequence = seg.Sequence;
            at += length;
        }
        if (at != bytes.Length)
        {
            Program.Log($"overlay journal: keeping {existing.Count} segments, dropping {bytes.Length - at} trailing bytes");
            _stream.SetLength(at);
        }
        Segments = existing.Count;
        _stream.Position = at;
    }

    /// 追加一段，返回它在日志里的（偏移, 长度）。
    public (long offset, int length) Append(OvSegment segment)
    {
        segment.Sequence = ++LastSequence;
        var bytes = segment.Encode(_identity);
        long offset = _stream.Position;
        _stream.Write(bytes);
        _stream.Flush(flushToDisk: false);
        Segments++;
        return (offset, bytes.Length);
    }

    public void Sync() => _stream.Flush(flushToDisk: true);

    public void Dispose()
    {
        try { Sync(); } catch (IOException) { }
        _stream.Dispose();
    }
}
