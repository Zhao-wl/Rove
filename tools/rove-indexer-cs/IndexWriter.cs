// RVCSIDX v2. See docs/CS_INDEX_FORMAT.md.
//
// 输入是绑定阶段留下的紧凑结果（每文件的出现位置数组 + 符号表），不再需要语法树。
// 符号按键排序编号、文件内出现位置按 (line, col, symbol, role, len) 排序，同样的输入产出逐字节相同的索引。

using System.Collections;
using System.Runtime.InteropServices;
using System.Text;

namespace Rove.Indexer;

static class IndexWriter
{
    public const uint Version = 2;
    const uint None = uint.MaxValue;
    const uint LocalBit = 1u << 31;
    const int TableOffset = 96;
    public static bool Verbose;

    enum Section
    {
        StringOffsets, Files, Paths, Symbols, LocalKeys, Occurrences, WideOccurrences,
        References, RelationsFrom, RelationsTo, Inactive, Signatures, XmlDocs, MetadataViews, Strings, Count,
    }

    public static (long occurrences, long relations, long bytes, long symbols) Write(
        string outPath, byte[] configHash, FileResult[] files, SymbolTable symbols,
        IEnumerable<(SymbolKey from, SymbolKey to, RelKind kind)> rels)
    {
        if (!BitConverter.IsLittleEndian) throw new PlatformNotSupportedException("RVCSIDX is little-endian");
        string tempPath = outPath + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            var stats = WriteFile(tempPath, configHash, files, symbols, rels);
            File.Move(tempPath, outPath, overwrite: true);
            return stats;
        }
        finally
        {
            File.Delete(tempPath);
        }
    }

    /// files 已按路径的 UTF-8 字节序排好，下标即最终文件号。
    static (long occurrences, long relations, long bytes, long symbols) WriteFile(
        string path, byte[] configHash, FileResult[] files, SymbolTable table,
        IEnumerable<(SymbolKey from, SymbolKey to, RelKind kind)> rels)
    {
        var stepWatch = System.Diagnostics.Stopwatch.StartNew();
        void Step(string name)
        {
            if (Verbose) Program.Log($"write: {name} {stepWatch.Elapsed.TotalSeconds:F1}s");
            stepWatch.Restart();
        }
        uint maxTemp = files.Length == 0 ? 0 : files.Max(f => f.TempId);
        var finalFile = new uint[maxTemp + 1];
        for (int i = 0; i < files.Length; i++) finalFile[files[i].TempId] = (uint)i;

        // 1. 符号：解析延迟参数键、合并别名、按键排序编号
        var resolved = new SymRec?[table.MaxId + 1];
        var byKey = new Dictionary<SymbolKey, SymRec>(table.Map.Count);
        var deferred = new List<SymRec>();
        foreach (var rec in table.Map.Values)
        {
            if (rec.Key.Text is ['\u0001', ..]) deferred.Add(rec);
            else { byKey.Add(rec.Key, rec); resolved[rec.Id] = rec; }
        }
        foreach (var rec in deferred.OrderBy(r => r.Key.Text, StringComparer.Ordinal))
        {
            SymRec? target = null;
            var text = rec.Key.Text!;
            int hash = text.LastIndexOf('#');
            if (table.SourceParameters.TryGetValue((text[1..hash], int.Parse(text.AsSpan(hash + 1))), out var local))
                byKey.TryGetValue(local, out target);
            if (target == null)
            {
                var fallback = SymbolKey.Global(rec.ParameterFallback ?? text);
                if (!byKey.TryGetValue(fallback, out target))
                {
                    target = new SymRec
                    {
                        Id = -1, Key = fallback, Name = rec.Name, Display = rec.Display, Kind = rec.Kind,
                        InSource = rec.InSource, ContainerKey = rec.ContainerKey,
                    };
                    byKey.Add(fallback, target);
                }
            }
            target.InSource |= rec.InSource;
            resolved[rec.Id] = target;
        }
        // 字符串键在前（Ordinal），局部键在后（最终文件号、偏移）
        var globals = byKey.Values.Where(r => r.Key.Text != null).AsParallel()
            .OrderBy(r => r.Key.Text, StringComparer.Ordinal).ToArray();
        var locals = byKey.Values.Where(r => r.Key.Text == null).ToArray();
        var localOrder = locals.Select(r => ((ulong)finalFile[r.Key.File] << 32) | r.Key.Offset).ToArray();
        Array.Sort(localOrder, locals);
        var symArr = new SymRec[globals.Length + locals.Length];
        globals.CopyTo(symArr, 0);
        locals.CopyTo(symArr, globals.Length);
        for (int i = 0; i < symArr.Length; i++) symArr[i].Final = i;
        int Final(int id) => resolved[id]!.Final;
        int? FinalOf(SymbolKey key) => table.Map.TryGetValue(key, out var r) && resolved[r.Id] is { } t ? t.Final : null;

        Step("symbols");
        // 2. 出现位置：文件内排序、打包，同时按符号计数
        long totalOccs = files.Sum(f => (long)f.Occs.Length);
        var occurrences = new ulong[totalOccs];
        var occSym = new int[totalOccs];
        var occDef = new BitArray(checked((int)totalOccs));
        var wide = new List<(uint file, Occ occ)>();
        var fileRecs = new FileRec[files.Length];
        var inactive = new List<Pair>();
        var counts = new int[symArr.Length];
        var defCounts = new int[symArr.Length];
        uint pathOffset = 0;
        int next = 0;
        Parallel.ForEach(files, file =>
        {
            var list = file.Occs;
            for (int k = 0; k < list.Length; k++) list[k].Sym = Final(list[k].Sym);
            Array.Sort(list, static (a, b) =>
            {
                int c = a.Line.CompareTo(b.Line);
                if (c == 0) c = a.Col.CompareTo(b.Col);
                if (c == 0) c = a.Sym.CompareTo(b.Sym);
                if (c == 0) c = a.Role.CompareTo(b.Role);
                if (c == 0) c = a.Len.CompareTo(b.Len);
                return c;
            });
        });
        for (int fi = 0; fi < files.Length; fi++)
        {
            var file = files[fi];
            var list = file.Occs;
            fileRecs[fi] = new FileRec(pathOffset, (uint)next, (uint)list.Length, (uint)inactive.Count, file.Hash);
            pathOffset = checked(pathOffset + (uint)file.Path.Length);
            foreach (var (start, end) in file.Inactive) inactive.Add(new Pair(start, end));
            foreach (var occ in list)
            {
                bool def = (occ.Role & (ushort)Role.Def) != 0;
                counts[occ.Sym]++;
                if (def) defCounts[occ.Sym]++;
                occSym[next] = occ.Sym;
                occDef[next] = def;
                occurrences[next++] = Pack((uint)fi, occ, wide);
            }
            file.Occs = [];
        }

        Step("occurrences");
        // 按符号的出现位置索引：计数排序，定义在前、引用在后，两组内保持出现位置序号升序
        var refStarts = new uint[symArr.Length];
        var defCursor = new uint[symArr.Length];
        var refCursor = new uint[symArr.Length];
        uint acc = 0;
        for (int s = 0; s < symArr.Length; s++)
        {
            refStarts[s] = acc;
            defCursor[s] = acc;
            refCursor[s] = acc + (uint)defCounts[s];
            acc += (uint)counts[s];
        }
        var refs = new uint[totalOccs];
        for (int k = 0; k < totalOccs; k++)
        {
            int s = occSym[k];
            refs[occDef[k] ? defCursor[s]++ : refCursor[s]++] = (uint)k;
        }
        occSym = null!;

        Step("references");
        var strings = new StringTable();
        var localKeys = new List<Pair>();
        var symRecs = new SymOut[symArr.Length];
        for (int i = 0; i < symArr.Length; i++)
        {
            var s = symArr[i];
            uint key;
            if (s.Key.Text is { } text)
            {
                key = strings.Add(text);
                if ((key & LocalBit) != 0) throw new InvalidDataException("too many strings");
            }
            else
            {
                key = LocalBit | (uint)localKeys.Count;
                localKeys.Add(new Pair(finalFile[s.Key.File], s.Key.Offset));
            }
            uint container = s.ContainerKey is { } parent && FinalOf(parent) is { } id ? (uint)id : None;
            symRecs[i] = new SymOut(key, strings.Add(s.Name), strings.Add(s.Display), container, refStarts[i], s.Kind,
                (byte)(s.InSource ? 1 : 0), (ushort)Math.Min(defCounts[i], ushort.MaxValue));
        }

        var relArr = rels.Select(r => (from: FinalOf(r.from), to: FinalOf(r.to), kind: (uint)r.kind))
            .Where(r => r.from != null && r.to != null)
            .Select(r => new Rel((uint)r.from!.Value, (uint)r.to!.Value, r.kind))
            .Distinct().ToArray();
        var relsByFrom = relArr.OrderBy(r => r.From).ThenBy(r => r.To).ThenBy(r => r.Kind).ToArray();
        var relsByTo = relArr.OrderBy(r => r.To).ThenBy(r => r.From).ThenBy(r => r.Kind).ToArray();

        Step("strings+relations");
        using var fs = new FileStream(path, FileMode.CreateNew, FileAccess.ReadWrite, FileShare.None, 1 << 20);
        using var w = new BinaryWriter(fs);
        int headerLength = TableOffset + (int)Section.Count * 16;
        w.Write(new byte[headerLength]);
        var sections = new (long offset, long count)[(int)Section.Count];

        void Align()
        {
            while (fs.Position % 8 != 0) w.Write((byte)0);
        }
        void Begin(Section section, long count)
        {
            Align();
            sections[(int)section] = (fs.Position, count);
        }

        // 段内记录都是按格式布局声明的定长结构，整段按字节写出（格式为小端，见 Write 开头的检查）
        void Bulk<T>(ReadOnlySpan<T> items) where T : struct
        {
            w.Flush();
            fs.Write(MemoryMarshal.AsBytes(items));
        }

        Begin(Section.StringOffsets, strings.Offsets.Count);
        Bulk<uint>(CollectionsMarshal.AsSpan(strings.Offsets));

        Begin(Section.Files, fileRecs.Length);
        Bulk<FileRec>(fileRecs);
        Begin(Section.Paths, pathOffset);
        foreach (var f in files) w.Write(f.Path);

        Begin(Section.Symbols, symRecs.Length);
        Bulk<SymOut>(symRecs);
        Begin(Section.LocalKeys, localKeys.Count);
        Bulk<Pair>(CollectionsMarshal.AsSpan(localKeys));

        Begin(Section.Occurrences, occurrences.Length);
        Bulk<ulong>(occurrences);
        Begin(Section.WideOccurrences, wide.Count);
        foreach (var (file, occ) in wide)
        {
            w.Write(file);
            w.Write(occ.Line);
            w.Write(occ.Col);
            w.Write((uint)occ.Sym);
            w.Write(occ.Len);
            w.Write((uint)occ.Role);
        }

        bool compactRefs = occurrences.Length <= 1 << 24;
        Begin(Section.References, (long)occurrences.Length * (compactRefs ? 3 : 4));
        if (compactRefs)
        {
            var buffer = new byte[refs.Length * 3];
            for (int k = 0; k < refs.Length; k++)
            {
                uint id = refs[k];
                buffer[3 * k] = (byte)id;
                buffer[3 * k + 1] = (byte)(id >> 8);
                buffer[3 * k + 2] = (byte)(id >> 16);
            }
            w.Write(buffer);
        }
        else foreach (uint id in refs) w.Write(id);

        Begin(Section.RelationsFrom, relsByFrom.Length);
        Bulk<Rel>(relsByFrom);
        Begin(Section.RelationsTo, relsByTo.Length);
        Bulk<Rel>(relsByTo);
        Begin(Section.Inactive, inactive.Count);
        Bulk<Pair>(CollectionsMarshal.AsSpan(inactive));

        Begin(Section.Signatures, 0);
        Begin(Section.XmlDocs, 0);
        Begin(Section.MetadataViews, 0);
        Begin(Section.Strings, strings.Bytes.Length);
        strings.Bytes.WriteTo(fs);
        Align();

        long total = fs.Position;
        using var header = new MemoryStream(headerLength);
        using var h = new BinaryWriter(header);
        h.Write(Encoding.ASCII.GetBytes("RVCSIDX\0"));
        h.Write(Version);
        h.Write((uint)Section.Count);
        h.Write(total);
        var version = typeof(IndexWriter).Assembly.GetName().Version!;
        h.Write(checked((ushort)version.Major));
        h.Write(checked((ushort)version.Minor));
        h.Write(checked((ushort)version.Build));
        h.Write(checked((ushort)version.Revision));
        h.Write(configHash);
        h.Write(new byte[24]); // flags + base generation
        h.Write(0UL); // header checksum, excluded from its own input
        foreach (var (offset, count) in sections) { h.Write(offset); h.Write(count); }

        ulong checksum = 14695981039346656037UL;
        var headerBytes = header.GetBuffer().AsSpan(0, headerLength);
        foreach (byte b in headerBytes[..88]) checksum = unchecked((checksum ^ b) * 1099511628211UL);
        foreach (byte b in headerBytes[TableOffset..]) checksum = unchecked((checksum ^ b) * 1099511628211UL);
        System.Buffers.Binary.BinaryPrimitives.WriteUInt64LittleEndian(headerBytes[88..], checksum);

        Step("write");
        fs.Position = 0;
        w.Write(headerBytes);
        w.Flush();
        fs.Flush(flushToDisk: true);
        Step("flush");
        return (occurrences.Length, relArr.Length, total, symArr.Length);
    }

    // 与 CS_INDEX_FORMAT.md 段表一致的定长记录（小端，顺序布局，无填充）
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    readonly record struct FileRec(uint Path, uint Start, uint Count, uint Inactive, ulong Hash);
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    readonly record struct SymOut(uint Key, uint Name, uint Display, uint Container, uint RefStart, byte Kind, byte Flags, ushort Definitions);
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    readonly record struct Pair(uint A, uint B);
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    readonly record struct Rel(uint From, uint To, uint Kind);

    static ulong Pack(uint file, Occ occ, List<(uint file, Occ occ)> wide)
    {
        if (occ.Role is 0 or > 7)
            throw new InvalidDataException($"Invalid occurrence role: {occ.Role}");
        if (file < 1 << 15 && occ.Sym < 1 << 21 && occ.Line < 1 << 12 && occ.Col < 1 << 8 && occ.Len < 1 << 5)
            return occ.Role | ((ulong)occ.Len << 3) | ((ulong)occ.Col << 8) | ((ulong)occ.Line << 16)
                | ((ulong)occ.Sym << 28) | ((ulong)file << 49);
        ulong index = (ulong)wide.Count;
        wide.Add((file, occ));
        return index << 3;
    }

    sealed class StringTable
    {
        readonly Dictionary<string, uint> _ids = new();
        public readonly List<uint> Offsets = new() { 0 };
        public readonly MemoryStream Bytes = new();

        byte[] _buffer = new byte[1024];

        public uint Add(string s)
        {
            if (_ids.TryGetValue(s, out var id)) return id;
            int max = Encoding.UTF8.GetMaxByteCount(s.Length);
            if (max > _buffer.Length) _buffer = new byte[max];
            Bytes.Write(_buffer, 0, Encoding.UTF8.GetBytes(s, _buffer));
            id = (uint)(Offsets.Count - 1);
            Offsets.Add(checked((uint)Bytes.Length));
            _ids[s] = id;
            return id;
        }
    }
}
