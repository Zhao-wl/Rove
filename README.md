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

## 项目状态

- 首发平台：Windows 10/11 x64；以后扩展 macOS。
- 技术路线：Rust + GPUI（M0 已验证），C# 语义索引由独立的 Roslyn 索引进程生成。
- 当前阶段：规格已确认，正在按 issue 推进。文档入口：`docs/PROJECT_CHARTER.md`（立项）、`docs/FEATURES.md`（功能清单）、`docs/UX_SPEC.md`（交互规格）、`docs/PLAN.md`（计划）、`docs/issues/README.md`（issue 总览）。
