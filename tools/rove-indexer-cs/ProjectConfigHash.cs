using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Rove.Indexer;

static class ProjectConfigHash
{
    public static byte[] Compute(string root, IEnumerable<ProjectInfo> projects, bool generators, bool reconcile)
    {
        using var bytes = new MemoryStream();
        using var w = new BinaryWriter(bytes, Encoding.UTF8, leaveOpen: true);
        string Relative(string path) => Path.GetRelativePath(root, path).Replace('\\', '/');
        void WriteList(IEnumerable<string> values)
        {
            var list = values.ToArray();
            w.Write(list.Length);
            foreach (var value in list) w.Write(value);
        }

        w.Write("rove-cs-project-config-v1");
        w.Write(generators);
        w.Write(reconcile);
        var ordered = projects.OrderBy(p => Relative(p.Path), StringComparer.Ordinal).ToArray();
        w.Write(ordered.Length);
        foreach (var project in ordered)
        {
            w.Write(Relative(project.Path));
            w.Write(project.AssemblyName);
            w.Write((int)project.LangVersion);
            w.Write(project.Unsafe);
            WriteList(project.Defines.Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal));
            WriteList(project.Sources.Select(Relative).Order(StringComparer.Ordinal));
            WriteList(project.References.Select(Relative));
            WriteList(project.ProjectRefs.Select(Relative));
            WriteList(generators ? project.Analyzers.Select(Relative) : []);
        }

        var unityFiles = new[] { "Assets", "Packages" }.Select(dir => Path.Combine(root, dir))
            .Where(Directory.Exists)
            .SelectMany(dir => Directory.EnumerateFiles(dir, "*.asmdef", SearchOption.AllDirectories)
                .Concat(Directory.EnumerateFiles(dir, "*.asmref", SearchOption.AllDirectories)))
            .OrderBy(Relative, StringComparer.Ordinal).ToArray();
        w.Write(unityFiles.Length);
        foreach (var path in unityFiles)
        {
            w.Write(Relative(path));
            using var json = new MemoryStream();
            try
            {
                var raw = Program.ReadShared(path);
                if (raw != null)
                {
                    using var reader = new StreamReader(new MemoryStream(raw), Encoding.UTF8);
                    using var doc = JsonDocument.Parse(reader.ReadToEnd(),
                        new JsonDocumentOptions { CommentHandling = JsonCommentHandling.Skip, AllowTrailingCommas = true });
                    using var writer = new Utf8JsonWriter(json);
                    WriteJson(writer, doc.RootElement);
                }
            }
            catch (JsonException)
            {
                json.SetLength(0); // UnityProjectModel also ignores malformed definitions.
            }
            w.Write((int)json.Length);
            w.Write(json.GetBuffer().AsSpan(0, (int)json.Length));
        }

        return SHA256.HashData(bytes.GetBuffer().AsSpan(0, (int)bytes.Length));
    }

    static void WriteJson(Utf8JsonWriter w, JsonElement value)
    {
        switch (value.ValueKind)
        {
            case JsonValueKind.Object:
                w.WriteStartObject();
                foreach (var property in value.EnumerateObject().OrderBy(p => p.Name, StringComparer.Ordinal))
                {
                    w.WritePropertyName(property.Name);
                    WriteJson(w, property.Value);
                }
                w.WriteEndObject();
                break;
            case JsonValueKind.Array:
                w.WriteStartArray();
                foreach (var element in value.EnumerateArray()) WriteJson(w, element);
                w.WriteEndArray();
                break;
            default:
                value.WriteTo(w);
                break;
        }
    }
}
