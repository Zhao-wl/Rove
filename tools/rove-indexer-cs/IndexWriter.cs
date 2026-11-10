// RVCSIDX v2. See docs/CS_INDEX_FORMAT.md.

using System.Collections.Concurrent;
using System.Reflection;
using System.Text;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Text;

namespace Rove.Indexer;

static class IndexWriter
{
    public const uint Version = 2;
    const uint None = uint.MaxValue;
    const uint LocalBit = 1u << 31;
    const int TableOffset = 96;

    enum Section
    {
        StringOffsets, Files, Paths, Symbols, LocalKeys, Occurrences, WideOccurrences,
        References, RelationsFrom, RelationsTo, Inactive, Signatures, XmlDocs, MetadataViews, Strings, Count,
    }

    public static (long occurrences, long relations, long bytes) Write(
        string outPath, byte[] configHash, IndexedFile[] files,
        ConcurrentDictionary<string, List<Occ>> fileOccs,
        ConcurrentDictionary<SymbolKey, SymRec> symbols,
        ConcurrentBag<(SymbolKey from, SymbolKey to, RelKind kind)> rels)
    {
        string tempPath = outPath + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try
        {
            var stats = WriteFile(tempPath, configHash, files, fileOccs, symbols, rels);
            File.Move(tempPath, outPath, overwrite: true);
            return stats;
        }
        finally
        {
            File.Delete(tempPath);
        }
    }

    static (long occurrences, long relations, long bytes) WriteFile(
        string path, byte[] configHash, IndexedFile[] files,
        ConcurrentDictionary<string, List<Occ>> fileOccs,
        ConcurrentDictionary<SymbolKey, SymRec> symbols,
        ConcurrentBag<(SymbolKey from, SymbolKey to, RelKind kind)> rels)
    {
        var strings = new StringTable();
        var symArr = symbols.Values.OrderBy(s => s.Id).ToArray();
        var remap = new int[symArr.Length == 0 ? 0 : symArr[^1].Id + 1];
        for (int i = 0; i < symArr.Length; i++) remap[symArr[i].Id] = i;
        var keyToId = symArr.Select((s, i) => (s.Key, i)).ToDictionary(x => x.Key, x => x.i);

        var occurrences = new List<ulong>();
        var wide = new List<(uint file, Occ occ)>();
        var fileRecs = new List<(uint path, uint start, uint count, uint inactive, ulong hash)>();
        var inactive = new List<(uint start, uint end)>();
        var bySym = new List<uint>[symArr.Length];
        var definitionCounts = new ushort[symArr.Length];
        uint pathOffset = 0;
        foreach (var file in files)
        {
            fileOccs.TryGetValue(file.FullPath, out var list);
            fileRecs.Add((pathOffset, (uint)occurrences.Count, (uint)(list?.Count ?? 0), (uint)inactive.Count, file.Hash));
            pathOffset = checked(pathOffset + (uint)file.Path.Length);
            AddInactive(file, inactive);
            if (list == null) continue;

            list.Sort((a, b) => a.Line != b.Line ? a.Line.CompareTo(b.Line) : a.Col.CompareTo(b.Col));
            foreach (var source in list)
            {
                var occ = source;
                occ.Sym = remap[occ.Sym];
                (bySym[occ.Sym] ??= []).Add((uint)occurrences.Count);
                if ((occ.Role & (ushort)Role.Def) != 0 && definitionCounts[occ.Sym] < ushort.MaxValue)
                    definitionCounts[occ.Sym]++;
                occurrences.Add(Pack(file.Id, occ, wide));
            }
        }

        bool IsDefinition(uint i)
        {
            ulong occ = occurrences[(int)i];
            var role = (occ & 7) == 0 ? wide[(int)(occ >> 3)].occ.Role : (ushort)(occ & 7);
            return (role & (ushort)Role.Def) != 0;
        }

        var localKeys = new List<(uint file, uint offset)>();
        var symRecs = new (uint key, uint name, uint display, uint container, byte kind, byte flags)[symArr.Length];
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
                localKeys.Add((s.Key.File, s.Key.Offset));
            }
            uint container = s.ContainerKey is { } parent && keyToId.TryGetValue(parent, out var id) ? (uint)id : None;
            symRecs[i] = (key, strings.Add(s.Name), strings.Add(s.Display), container, s.Kind, (byte)(s.InSource ? 1 : 0));
        }

        var relArr = rels.Where(r => keyToId.ContainsKey(r.from) && keyToId.ContainsKey(r.to))
            .Select(r => (from: (uint)keyToId[r.from], to: (uint)keyToId[r.to], kind: (uint)r.kind))
            .Distinct().ToArray();
        var relsByFrom = relArr.OrderBy(r => r.from).ThenBy(r => r.to).ToArray();
        var relsByTo = relArr.OrderBy(r => r.to).ThenBy(r => r.from).ToArray();

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

        Begin(Section.StringOffsets, strings.Offsets.Count);
        foreach (uint offset in strings.Offsets) w.Write(offset);

        Begin(Section.Files, fileRecs.Count);
        foreach (var f in fileRecs)
        {
            w.Write(f.path);
            w.Write(f.start);
            w.Write(f.count);
            w.Write(f.inactive);
            w.Write(f.hash);
        }
        Begin(Section.Paths, pathOffset);
        foreach (var f in files) w.Write(f.Path);

        Begin(Section.Symbols, symRecs.Length);
        uint refStart = 0;
        for (int i = 0; i < symRecs.Length; i++)
        {
            var s = symRecs[i];
            uint refCount = (uint)(bySym[i]?.Count ?? 0);
            w.Write(s.key);
            w.Write(s.name);
            w.Write(s.display);
            w.Write(s.container);
            w.Write(refStart);
            w.Write(s.kind);
            w.Write(s.flags);
            w.Write(definitionCounts[i]);
            refStart += refCount;
        }
        Begin(Section.LocalKeys, localKeys.Count);
        foreach (var key in localKeys) { w.Write(key.file); w.Write(key.offset); }

        Begin(Section.Occurrences, occurrences.Count);
        foreach (ulong occ in occurrences) w.Write(occ);
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

        bool compactRefs = occurrences.Count <= 1 << 24;
        Begin(Section.References, (long)occurrences.Count * (compactRefs ? 3 : 4));
        void WriteRef(uint id)
        {
            if (compactRefs)
            {
                w.Write((ushort)id);
                w.Write((byte)(id >> 16));
            }
            else w.Write(id);
        }
        foreach (var list in bySym)
        {
            if (list == null) continue;
            foreach (uint id in list)
                if (IsDefinition(id)) WriteRef(id);
            foreach (uint id in list)
                if (!IsDefinition(id)) WriteRef(id);
        }

        Begin(Section.RelationsFrom, relsByFrom.Length);
        foreach (var r in relsByFrom) { w.Write(r.from); w.Write(r.to); w.Write(r.kind); }
        Begin(Section.RelationsTo, relsByTo.Length);
        foreach (var r in relsByTo) { w.Write(r.from); w.Write(r.to); w.Write(r.kind); }
        Begin(Section.Inactive, inactive.Count);
        foreach (var r in inactive) { w.Write(r.start); w.Write(r.end); }

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

        fs.Position = 0;
        w.Write(headerBytes);
        w.Flush();
        fs.Flush(flushToDisk: true);
        return (occurrences.Count, relArr.Length, total);
    }

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

    static void AddInactive(IndexedFile file, List<(uint start, uint end)> inactive)
    {
        int first = inactive.Count;
        var lines = file.Tree.GetText().Lines;
        foreach (var trivia in file.Tree.GetRoot().DescendantTrivia())
        {
            if (trivia.RawKind != (int)SyntaxKind.DisabledTextTrivia || trivia.Span.IsEmpty) continue;
            uint start = (uint)lines.IndexOf(trivia.SpanStart);
            uint end = (uint)lines.IndexOf(trivia.Span.End - 1) + 1;
            if (inactive.Count > first && start <= inactive[^1].end)
                inactive[^1] = (inactive[^1].start, Math.Max(inactive[^1].end, end));
            else
                inactive.Add((start, end));
        }
    }

    sealed class StringTable
    {
        readonly Dictionary<string, uint> _ids = new();
        public readonly List<uint> Offsets = new() { 0 };
        public readonly MemoryStream Bytes = new();

        public uint Add(string s)
        {
            if (_ids.TryGetValue(s, out var id)) return id;
            Bytes.Write(Encoding.UTF8.GetBytes(s));
            id = (uint)(Offsets.Count - 1);
            Offsets.Add(checked((uint)Bytes.Length));
            _ids[s] = id;
            return id;
        }
    }
}
