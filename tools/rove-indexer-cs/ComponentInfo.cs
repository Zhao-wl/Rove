using System.Text.Json;
using System.Text.Json.Nodes;

namespace Rove.Indexer;

/// 组件自述（`--version` 的输出，也是安装目录里 `component.json` 的内容）。
/// Rove 靠它不启动进程就能判断组件是否兼容：规则见 docs/CS_INDEX_FORMAT.md「组件、协议与索引格式的版本」。
static class ComponentInfo
{
    public const string Name = "indexer-cs";

    /// 发布时是否开启了 trim（`Rove.Trimmed` 由 csproj 在 PublishTrimmed 时写进 runtimeconfig）。
    public static bool Trimmed => AppContext.TryGetSwitch("Rove.Trimmed", out var on) && on;

    public static string Version => typeof(ComponentInfo).Assembly.GetName().Version!.ToString();

    public static string Json() => new JsonObject
    {
        ["component"] = Name,
        ["version"] = Version,
        ["protocol"] = Server.Protocol,
        ["protocol_min"] = Server.ProtocolMin,
        ["index_format"] = (int)IndexWriter.Version,
        ["trimmed"] = Trimmed,
    }.ToJsonString(new JsonSerializerOptions { WriteIndented = false });
}
