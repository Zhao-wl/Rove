// 索引文件格式 RVCSIDX v1（小端，各段 8 字节对齐，便于 Rust 侧 mmap 零拷贝读取）。
// 详见 docs/CS_INDEX_FORMAT.md。

using System.Collections.Concurrent;
using System.Text;
using Microsoft.CodeAnalysis;

namespace Rove.Indexer;

static class IndexWriter
{
    public const uint Version = 1;
    const uint None = uint.MaxValue;

    public static (long occurrences, long relations, long bytes) Write(
        string outPath, string root,
        ConcurrentDictionary<string, (SyntaxTree tree, ulong hash)> trees,
        ConcurrentDictionary<string, List<Occ>> fileOccs,
        ConcurrentDictionary<string, SymRec> symbols,
        ConcurrentBag<(string from, string to, RelKind kind)> rels)
    {
        var strings = new StringTable();
        // ConcurrentDictionary.GetOrAdd 竞争时会丢弃部分已分配的 Id，这里重排为连续编号
        var symArr = symbols.Values.OrderBy(s => s.Id).ToArray();
        var remap = new int[symArr.Length == 0 ? 0 : symArr[^1].Id + 1];
        for (int i = 0; i < symArr.Length; i++) remap[symArr[i].Id] = i;
        var keyToId = symArr.Select((s, i) => (s.Key, i)).ToDictionary(x => x.Key, x => x.i);
        foreach (var list in fileOccs.Values)
            for (int i = 0; i < list.Count; i++) { var o = list[i]; o.Sym = remap[o.Sym]; list[i] = o; }

        // 文件按相对路径排序；出现位置按 (file, line, col) 排序
        var files = trees.Keys.OrderBy(f => f, StringComparer.Ordinal).ToArray();
        var occAll = new List<(uint file, Occ o)>();
        var fileRecs = new List<(uint path, uint start, uint count, ulong hash)>();
        for (uint fi = 0; fi < files.Length; fi++)
        {
            var f = files[fi];
            var list = fileOccs.TryGetValue(f, out var l) ? l : new List<Occ>();
            list.Sort((a, b) => a.Line != b.Line ? a.Line.CompareTo(b.Line) : a.Col.CompareTo(b.Col));
            fileRecs.Add((strings.Add(Path.GetRelativePath(root, f).Replace('\\', '/')), (uint)occAll.Count, (uint)list.Count, trees[f].hash));
            foreach (var o in list) occAll.Add((fi, o));
        }

        // 每个符号的出现位置索引（定义在前）
        var bySym = new List<uint>[symArr.Length];
        for (int i = 0; i < occAll.Count; i++)
            (bySym[occAll[i].o.Sym] ??= new List<uint>()).Add((uint)i);
        var refIndex = new List<uint>(occAll.Count);
        var symRefRange = new (uint start, uint count)[symArr.Length];
        for (int s = 0; s < symArr.Length; s++)
        {
            var l = bySym[s];
            if (l == null) { symRefRange[s] = ((uint)refIndex.Count, 0); continue; }
            l.Sort((a, b) => ((occAll[(int)b].o.Role & 2) != 0).CompareTo((occAll[(int)a].o.Role & 2) != 0) is var c && c != 0 ? c : a.CompareTo(b));
            symRefRange[s] = ((uint)refIndex.Count, (uint)l.Count);
            refIndex.AddRange(l);
        }

        var relArr = rels.Where(r => keyToId.ContainsKey(r.from) && keyToId.ContainsKey(r.to))
            .Select(r => (from: (uint)keyToId[r.from], to: (uint)keyToId[r.to], kind: (uint)r.kind))
            .Distinct().ToArray();
        var relsByFrom = relArr.OrderBy(r => r.from).ThenBy(r => r.to).ToArray();
        var relsByTo = relArr.OrderBy(r => r.to).ThenBy(r => r.from).ToArray();

        var symRecs = symArr.Select(s => (
            key: strings.Add(s.Key), name: strings.Add(s.Name), display: strings.Add(s.Display),
            container: s.ContainerKey != null && keyToId.TryGetValue(s.ContainerKey, out var cid) ? (uint)cid : None,
            kind: s.Kind, flags: (byte)(s.InSource ? 1 : 0))).ToArray();

        using var fs = new FileStream(outPath, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 20);
        using var w = new BinaryWriter(fs);
        // 头部：magic(8) version(4) reserved(4) + 9 段 × (offset u64, count u64)
        const int SectionCount = 9;
        w.Write(Encoding.ASCII.GetBytes("RVCSIDX\0")); w.Write(Version); w.Write(0u);
        long tableAt = fs.Position;
        w.Write(new byte[SectionCount * 16]);
        var sections = new (long off, long count)[SectionCount];

        void Align() { while (fs.Position % 8 != 0) w.Write((byte)0); }
        void Begin(int i, long count) { Align(); sections[i] = (fs.Position, count); }

        Begin(0, strings.Offsets.Count); foreach (var o in strings.Offsets) w.Write(o);                    // u32 string offsets (n+1)
        Begin(1, strings.Bytes.Length); w.Write(strings.Bytes.ToArray());                                   // utf8 blob
        Begin(2, fileRecs.Count);                                                                          // files: 24B
        foreach (var f in fileRecs) { w.Write(f.path); w.Write(f.start); w.Write(f.count); w.Write(0u); w.Write(f.hash); }
        Begin(3, symRecs.Length);                                                                          // symbols: 28B
        for (int i = 0; i < symRecs.Length; i++)
        {
            var s = symRecs[i];
            w.Write(s.key); w.Write(s.name); w.Write(s.display); w.Write(s.container);
            w.Write(symRefRange[i].start); w.Write(symRefRange[i].count);
            w.Write(s.kind); w.Write(s.flags); w.Write((ushort)0);
        }
        Begin(4, occAll.Count);                                                                            // occurrences: 20B
        foreach (var (file, o) in occAll)
        { w.Write(file); w.Write((uint)o.Line); w.Write((uint)o.Col); w.Write((uint)o.Sym); w.Write((ushort)o.Len); w.Write(o.Role); }
        Begin(5, refIndex.Count); foreach (var r in refIndex) w.Write(r);                                  // u32 occurrence idx by symbol
        Begin(6, relsByFrom.Length); foreach (var r in relsByFrom) { w.Write(r.from); w.Write(r.to); w.Write(r.kind); }
        Begin(7, relsByTo.Length); foreach (var r in relsByTo) { w.Write(r.from); w.Write(r.to); w.Write(r.kind); }
        Begin(8, 0);                                                                                       // reserved: doc comments
        Align();
        long total = fs.Position;
        fs.Position = tableAt;
        foreach (var (off, count) in sections) { w.Write(off); w.Write(count); }
        return (occAll.Count, relArr.Length, total);
    }

    sealed class StringTable
    {
        readonly Dictionary<string, uint> _ids = new();
        public readonly List<uint> Offsets = new() { 0 };
        public readonly MemoryStream Bytes = new();
        public uint Add(string s)
        {
            if (_ids.TryGetValue(s, out var id)) return id;
            var b = Encoding.UTF8.GetBytes(s);
            Bytes.Write(b);
            id = (uint)(Offsets.Count - 1);
            Offsets.Add((uint)Bytes.Length);
            _ids[s] = id;
            return id;
        }
    }
}
