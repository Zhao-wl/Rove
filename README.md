# Rove

Rove 是一款轻量、高性能的代码阅读编辑器。它以 VS Code 的使用体验为基线，保留阅读与轻编辑所需的能力，去掉插件、终端、调试、构建等重度功能，并用原生渲染重新实现。

## 核心能力

- **高效阅读**：任意大小的文件秒开（GB 级日志以只读方式打开），快速打开、全局搜索、大纲、面包屑。
- **C# 导航**：基于预先计算的语义索引，提供转到定义、查找引用（默认以速览窗口展示）、转到实现、悬停与元数据视图，响应在毫秒级以内，不需要常驻语言服务。
- **文档阅读**：JSON / YAML / XML 的结构树与校验；Markdown 默认以预览打开，完整支持内嵌 HTML、Mermaid 与数学公式。
- **局部编辑**：输入、中文输入法、撤销重做、查找替换；保存时保留原文件的编码和换行格式。
- **与 VS Code 共用配置**：直接读写 VS Code 的 `settings.json` 和 `keybindings.json`，并能加载 VS Code 主题。
- **上下文打包**：把多段代码和备注整理成 Markdown 或文件引用，复制给外部 AI 工具；Rove 自身不联网。

以后再评估的能力：AI 改动审阅（工作区检查点、跟踪外部改动、逐段标记已审），见 `docs/PROJECT_CHARTER.md`。

## 安装（Windows 10/11 x64）

从 [Releases](https://github.com/Zhao-wl/Rove/releases) 下载，两种形态任选：

- **安装包** `Rove-<版本>-win-x64-setup.exe`：**用户级安装**，装到 `%LOCALAPPDATA%\Programs\Rove`，
  不需要管理员权限，不写 `HKLM`。文件关联（`.cs`、`.json`、`.yaml`、`.xml`、`.md`、`.log` 等）与
  资源管理器右键菜单「用 Rove 打开」是安装时的**可选项，默认不勾选**；勾上后 Rove 只会出现在
  「打开方式」里，**不会抢走默认程序**。Rove 不往 `PATH` 里加东西。
- **便携版** `Rove-<版本>-win-x64.zip`：解压即用，不写注册表。

每个 Release 都附 `SHA256SUMS.txt`，建议下载后核对。

**首次运行会看到 SmartScreen 提示。** 当前版本**没有做代码签名**（见 `docs/FEATURES.md` D11），
Windows 会提示「Windows 已保护你的电脑 / 未知发布者」。放行：点「更多信息」→「仍要运行」。
介意的话先用上面的 SHA-256 核对文件。

**Markdown 预览需要 WebView2 运行时。** Windows 11 自带，部分 Windows 10 没有；
这时 Markdown 会以源码打开并提示一次。**安装器不捆绑、也不会联网下载 WebView2**，
需要时请自行从微软官方页面安装。

**C# 语义导航需要可选的索引组件。** `rove-indexer-cs-<版本>-win-x64.zip` 单独下载，
解压到 `%LOCALAPPDATA%\Rove\components\indexer-cs\`（压缩包的顶层目录就是版本号），
或在安装器里勾选「安装 C# 索引组件」。组件自带 .NET 运行时，**不需要另外安装 .NET**；
没有它时 C# 导航自动降级为声明索引。**Rove 运行期不会替你下载任何东西。**

**卸载**：从「应用和功能」卸载。程序目录、快捷方式与注册表项都会清除；用户数据
（`%LOCALAPPDATA%\Rove`：索引缓存、设置备份、工作区状态）**默认保留**，卸载时可以勾选一并删除。

发布流程、安装器选型与干净机器验收清单见 [`docs/RELEASE.md`](docs/RELEASE.md)。

## 项目状态

- 首发平台：Windows 10/11 x64；以后扩展 macOS。
- 技术路线：Rust + GPUI（M0 已验证），C# 语义索引由独立的 Roslyn 索引进程生成。
- 当前阶段：规格已确认，正在按 issue 推进。文档入口：`docs/PROJECT_CHARTER.md`（立项）、`docs/FEATURES.md`（功能清单）、`docs/UX_SPEC.md`（交互规格）、`docs/PLAN.md`（计划）、`docs/issues/README.md`（issue 总览）。
