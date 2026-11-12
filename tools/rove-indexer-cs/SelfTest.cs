// `rove-indexer-cs --self-test`：不依赖项目的纯函数自检（影响分析的判定规则、标识符查找、覆盖层编解码）。
// 由 crates/rove-lang-cs/tests/incremental.rs 在索引进程已构建时调用；失败时退出码非零并打印原因。

using System.Text;

namespace Rove.Indexer;

static class SelfTest
{
    public static int Run()
    {
        int failures = 0;
        void Check(bool ok, string what)
        {
            if (ok) return;
            failures++;
            Console.Error.WriteLine($"self-test failed: {what}");
        }
        static Dictionary<string, Decl> D(params (string key, ulong fp, string name)[] items) =>
            items.ToDictionary(x => x.key, x => new Decl(x.fp, x.fp ^ 0x55, x.name), StringComparer.Ordinal);

        // 1. 只改方法体：声明指纹不变 → 没有影响
        var before = D(("A|T:Foo", 11, "Foo"), ("A|M:Foo.Run", 21, "Run"));
        var (changed, added) = Impact.Diff(before, D(("A|T:Foo", 11, "Foo"), ("A|M:Foo.Run", 21, "Run")));
        Check(changed.Count == 0 && added.Count == 0, "body-only edit has no impact");

        // 2. 可见性 / 返回类型变化：键不变、指纹变 → 变化
        (changed, added) = Impact.Diff(before, D(("A|T:Foo", 11, "Foo"), ("A|M:Foo.Run", 22, "Run")));
        Check(changed.SequenceEqual(["A|M:Foo.Run"]) && added.Count == 0, "fingerprint change is a signature change");

        // 3. 参数类型变化：文档注释 ID 变了 → 旧键变化，名字不是新名字（同名重载）
        (changed, added) = Impact.Diff(before, D(("A|T:Foo", 11, "Foo"), ("A|M:Foo.Run(System.Int32)", 23, "Run")));
        Check(changed.SequenceEqual(["A|M:Foo.Run"]) && added.SequenceEqual(["Run"]), "parameter change removes old key, adds name");

        // 4. 改名：旧键变化 + 新名字
        (changed, added) = Impact.Diff(before, D(("A|T:Foo", 11, "Foo"), ("A|M:Foo.Go", 21, "Go")));
        Check(changed.SequenceEqual(["A|M:Foo.Run"]) && added.SequenceEqual(["Go"]), "rename");

        // 5. 旧指纹未知（没有声明表）：退回弱指纹
        var unknown = new Dictionary<string, Decl> { ["A|M:Foo.Run"] = new(0, 7, "Run") };
        (changed, _) = Impact.Diff(unknown, new Dictionary<string, Decl> { ["A|M:Foo.Run"] = new(99, 7, "Run") });
        Check(changed.Count == 0, "unknown fingerprint falls back to weak comparison (same)");
        (changed, _) = Impact.Diff(unknown, new Dictionary<string, Decl> { ["A|M:Foo.Run"] = new(99, 8, "Run") });
        Check(changed.Count == 1, "unknown fingerprint falls back to weak comparison (different)");

        // 6. 删除文件：全部旧声明变化
        (changed, added) = Impact.Diff(before, new Dictionary<string, Decl>());
        Check(changed.Count == 2 && added.Count == 0, "deleted file");

        // 7. 标识符查找的边界
        var text = Encoding.UTF8.GetBytes("var runner = Run(); @Go(); x.RunAll(); 中Run");
        Check(Impact.ContainsIdentifier(text, "Run"u8), "finds Run(");
        Check(Impact.ContainsIdentifier(text, "Go"u8), "finds @Go");
        Check(!Impact.ContainsIdentifier(Encoding.UTF8.GetBytes("runner RunAll 中Run"), "Run"u8), "word boundaries (incl. non-ASCII)");

        // 8. 覆盖层段编解码往返，篡改后校验失败
        var id = new byte[16];
        id[3] = 7;
        var seg = new OvSegment { Sequence = 5 };
        int t = seg.Symbol(OvKey.Of("A|T:Foo"), () => new OvSymbol { Key = OvKey.Of("A|T:Foo"), Name = "Foo", Display = "Foo", Kind = 11, InSource = true });
        int l = seg.Symbol(OvKey.Local("Assets/中.cs", 12), () => new OvSymbol { Key = OvKey.Local("Assets/中.cs", 12), Name = "x", Display = "int x", Kind = 8, Container = OvKey.Of("A|T:Foo") });
        seg.Files.Add(new OvFile { Path = "Assets/中.cs", Hash = 42, Occs = [new Occ { Line = 1, Col = 2, Len = 3, Sym = t, Role = 2 }], Inactive = [(3, 4)], Declarations = [(t, 9)] });
        seg.Files.Add(new OvFile { Path = "Assets/Gone.cs", Deleted = true });
        seg.Owners.Add((t, OwnerPart.Self));
        seg.Relations.Add((t, l, RelKind.BaseType));
        var bytes = seg.Encode(id);
        var back = OvSegment.Decode(bytes, id, out int length);
        Check(back != null && length == bytes.Length && bytes.Length % 8 == 0, "segment round trip");
        Check(back?.Files[0].Path == "Assets/中.cs" && back.Files[0].Occs[0].Col == 2 && back.Files[1].Deleted
            && back.Symbols[1].Key == OvKey.Local("Assets/中.cs", 12) && back.Symbols[1].Container == OvKey.Of("A|T:Foo")
            && back.Relations[0] == (t, l, RelKind.BaseType) && back.Sequence == 5, "segment content");
        bytes[60] ^= 1;
        Check(OvSegment.Decode(bytes, id, out _) == null, "corrupted segment rejected");
        Check(OvSegment.Decode(seg.Encode(id), new byte[16], out _) == null, "segment of another base rejected");

        Console.Error.WriteLine(failures == 0 ? "self-test passed" : $"self-test: {failures} failures");
        return failures == 0 ? 0 : 1;
    }
}
