// Unity 生成的 .csproj 只在编辑器刷新时更新；AI/外部工具新增或删除的 .cs 文件会让它过期。
// 这里按 Unity 自身的程序集归属规则把磁盘上的实际文件与 csproj 对齐：
//   1. 文件归属于最近祖先目录中的 .asmdef（或 .asmref 指向的 asmdef）
//   2. 否则：路径含 /Editor/ → Assembly-CSharp-Editor；否则 Assembly-CSharp
//      （Plugins/Standard Assets 下归 *-firstpass）

using System.Text.Json;

namespace Rove.Indexer;

static class UnityProjectModel
{
    public static (int added, int removed) Reconcile(string root, Dictionary<string, ProjectInfo> projects)
    {
        var byAsm = projects.Values.GroupBy(p => p.AssemblyName, StringComparer.OrdinalIgnoreCase)
            .ToDictionary(g => g.Key, g => g.First().Path, StringComparer.OrdinalIgnoreCase);
        var listed = new HashSet<string>(projects.Values.SelectMany(p => p.Sources), StringComparer.OrdinalIgnoreCase);
        var sources = projects.Values.ToDictionary(p => p.Path, p => p.Sources.Where(File.Exists).ToList(), StringComparer.OrdinalIgnoreCase);
        int removed = projects.Values.Sum(p => p.Sources.Length) - sources.Values.Sum(l => l.Count);

        // 目录 → 程序集名
        var dirAsm = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var scanRoots = new[] { "Assets", "Packages" }.Select(d => Path.Combine(root, d)).Where(Directory.Exists).ToArray();
        foreach (var sr in scanRoots)
        {
            foreach (var f in Directory.EnumerateFiles(sr, "*.asmdef", SearchOption.AllDirectories))
                if (ReadJsonString(f, "name") is { } n) dirAsm[Path.GetDirectoryName(f)!] = n;
            foreach (var f in Directory.EnumerateFiles(sr, "*.asmref", SearchOption.AllDirectories))
                if (ReadJsonString(f, "reference") is { } r && !r.StartsWith("GUID:", StringComparison.OrdinalIgnoreCase))
                    dirAsm[Path.GetDirectoryName(f)!] = r;
        }

        int added = 0;
        foreach (var sr in scanRoots)
            foreach (var f in Directory.EnumerateFiles(sr, "*.cs", SearchOption.AllDirectories))
            {
                var full = Path.GetFullPath(f);
                if (listed.Contains(full)) continue;
                var asm = AssemblyFor(full, root, dirAsm);
                if (asm != null && byAsm.TryGetValue(asm, out var projPath)) { sources[projPath].Add(full); added++; }
            }

        foreach (var key in projects.Keys.ToList())
            projects[key] = projects[key] with { Sources = sources[key].ToArray() };
        return (added, removed);
    }

    static string? AssemblyFor(string file, string root, Dictionary<string, string> dirAsm)
    {
        for (var d = Path.GetDirectoryName(file); d != null && d.Length >= root.Length; d = Path.GetDirectoryName(d))
            if (dirAsm.TryGetValue(d, out var a)) return a;
        var rel = Path.GetRelativePath(root, file).Replace('\\', '/');
        if (!rel.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)) return null;
        bool editor = rel.Contains("/Editor/", StringComparison.OrdinalIgnoreCase);
        bool firstpass = rel.StartsWith("Assets/Plugins/", StringComparison.OrdinalIgnoreCase)
                      || rel.StartsWith("Assets/Standard Assets/", StringComparison.OrdinalIgnoreCase);
        return (firstpass ? "Assembly-CSharp-firstpass" : "Assembly-CSharp") + (editor ? "-Editor" : "");
    }

    static string? ReadJsonString(string path, string prop)
    {
        try
        {
            using var doc = JsonDocument.Parse(File.ReadAllText(path), new JsonDocumentOptions { CommentHandling = JsonCommentHandling.Skip, AllowTrailingCommas = true });
            return doc.RootElement.TryGetProperty(prop, out var v) ? v.GetString() : null;
        }
        catch { return null; }
    }
}
