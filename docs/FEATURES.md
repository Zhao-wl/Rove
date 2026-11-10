# Rove 功能清单（以 VS Code 为基线）

> 版本：v0.1（草案，待确认） · 日期：2026-11-09
> 方法：逐个盘点 VS Code（microsoft/vscode，MIT 许可）的 `workbench/contrib`、`editor/contrib`、`workbench/services`、`workbench/browser/parts` 以及内置扩展。对每一项，用第 1 节的设计哲学判断它应该“照搬”“改造”还是“不做”。

## 1. 移植原则（设计哲学）

Rove = **VS Code 的阅读与轻编辑体验，去掉一切重度能力，并用原生渲染重写**。对 VS Code 的每一项功能，按顺序回答下面 5 个问题：

1. **是否服务于“阅读与轻编辑”？** 是：考虑移植。否：例如调试、终端、构建、SCM 写操作、插件、内置 AI、远程开发，不做。
2. **是否威胁性能预算？** 判断标准是立项文档 §6：冷启动 < 400 ms、空闲内存 < 60 MB、每帧 CPU < 1 ms。会威胁的功能，要么改造为按需或惰性加载，要么不做。
3. **是否需要“实时语言服务”？** 例如补全、实时诊断、重构、参数提示。这类功能不做。能由预计算索引回答的功能（跳转、引用、实现、层级、语义高亮），可以做。
4. **是否依赖网络或账号？** 依赖就不做。
5. **是否能保持 VS Code 的使用习惯？** 尽量保持：键位、设置项名称、主题格式、交互细节都与 VS Code 一致，让用户零成本迁移。只有当性能或定位要求不一样时，才偏离 VS Code。

**移植的含义**：VS Code 是 TypeScript 加 DOM 实现的，我们无法直接复用它的代码。我们移植的是三类东西：
- **行为与规格**：交互细节、快捷键、设置语义；
- **数据资产**：主题、语言配置、图标；
- **算法**：模糊匹配打分、单词边界规则、缩进规则、查找模型等，翻译成 Rust。

移植时保留 MIT 许可声明（见第 9 节）。

**判定标记**：
- **照搬**：行为与 VS Code 保持一致
- **改造**：保留核心体验，按 Rove 的定位做删减或替换实现
- **不做**：明确不实现

**优先级**：
- **P0**：首个可用版本必须有
- **P1**：首版应该有，可以在 P0 之后紧接着完成
- **P2**：以后再做

## 2. 工作台布局（workbench/browser/parts）

| VS Code 部件 | 判定 | 优先级 | 说明 |
|---|---|---|---|
| 标题栏（自定义标题栏 + 命令中心） | 照搬 | P0 | 自绘标题栏，中间是命令中心（点击后打开快速打开）；菜单栏折叠为一个汉堡菜单 |
| 活动栏 | 改造 | P0 | 固定几个视图：资源管理器、搜索（大纲是资源管理器中的一个小节，见 UX_SPEC §1.3）；P1 加上下文篮子。不能由插件扩展 |
| 主侧边栏 | 照搬 | P0 | 可以隐藏（`Ctrl+B`），可以切换到左侧或右侧 |
| 辅助侧边栏（右侧） | 照搬 | P2 | |
| 编辑区（编辑器组网格） | 照搬 | P0 | 支持任意方向的分屏网格、拖动标签分屏、编辑器组的锁定与最大化 |
| 面板（底部） | 改造 | P0 | 只有：引用结果、搜索结果（可选，默认在侧边栏）、问题。没有终端、输出、调试控制台 |
| 状态栏 | 照搬 | P0 | 内容见第 8 节 |
| 通知 | 照搬 | P0 | 右下角的通知提示 |
| 视图容器（可拖动、重排） | 改造 | P2 | 首版视图位置固定 |
| 全屏 / Zen 模式 / 居中布局 | 照搬 | P1 | 阅读场景中有用 |

## 3. 编辑器：阅读相关（editor/contrib + 编辑器核心）

| VS Code 功能 | 判定 | 优先级 | 说明 |
|---|---|---|---|
| 语法高亮（tokenization） | 改造 | P0 | 用 tree-sitter 代替 TextMate 正则引擎，性能更好；把 tree-sitter 的捕获名映射为 TextMate scope，从而**直接加载 VS Code 的主题**（见第 7 节） |
| 语义高亮（semanticTokens） | 改造 | P1 | 由 C# 预计算索引提供（区分字段、局部变量、参数、类型等），不需要实时语言服务 |
| 行号、当前行高亮、空白字符显示、缩进参考线 | 照搬 | P0 | |
| 括号匹配（bracketMatching） | 照搬 | P0 | |
| 括号对着色（bracket pair colorization） | 照搬 | P1 | |
| 代码折叠（folding） | 改造 | P0 | 折叠区间来自 tree-sitter 和 `#region`；支持折叠全部、按级别折叠等命令 |
| 粘性滚动（stickyScroll） | 照搬 | P1 | 滚动时把当前所在的类、方法签名固定在顶部，对阅读很有价值 |
| 面包屑 | 照搬 | P0 | |
| 缩略图（minimap） | 改造 | P1 | 只渲染色块，不渲染字符，以控制性能；支持 `// MARK:` 分节标题（sectionHeaders） |
| 滚动条上的标记（概览标尺） | 照搬 | P1 | 显示搜索结果、选区、同名高亮、问题的位置 |
| 同名高亮（wordHighlighter） | 改造 | P0 | C# 文件使用索引做语义级高亮（只高亮同一个符号）；其他语言按文本匹配 |
| 悬停（hover） | 改造 | P0 | 显示签名、文档注释和来源层级（精确或近似）；不显示诊断与快速修复 |
| 链接检测（links） | 照搬 | P1 | URL、文件路径可以 `Ctrl+点击` 打开；打开外部链接前需要确认 |
| 软换行（word wrap） | 照搬 | P1 | 超大文件模式下禁用 |
| 缩放 | 照搬 | P0 | 与 VS Code 相同的两套机制：**窗口缩放** `workbench.action.zoomIn/zoomOut/zoomReset`（默认键位 `Ctrl+=` / `Ctrl+-` / `Ctrl+小键盘0`）；**编辑器字体缩放** `editor.action.fontZoomIn/fontZoomOut/fontZoomReset`（VS Code 没有默认键位，另有 `editor.mouseWheelZoom`） |
| 中键滚动（middleScroll） | 照搬 | P2 | |
| 长行处理（longLinesHelper） | 改造 | P0 | 超长行截断显示，避免卡顿 |
| Unicode 高亮（不可见字符、易混淆字符） | 照搬 | P2 | |
| 异常行终止符提示 | 照搬 | P2 | |
| 只读提示（readOnlyMessage） | 照搬 | P0 | 用于元数据视图、超大文件 |
| 渲染引擎（含 VS Code 的 GPU 渲染器） | 不做 | — | 我们自己用 GPUI 实现，不移植 |

## 4. 编辑器：编辑相关

| VS Code 功能 | 判定 | 优先级 | 说明 |
|---|---|---|---|
| 基本输入、IME、剪贴板 | 照搬 | P0 | |
| 撤销 / 重做、光标撤销（cursorUndo） | 照搬 | P0 | |
| 多光标（multicursor） | 照搬 | P1 | `Alt+点击`、`Ctrl+Alt+↑↓`、`Ctrl+D`、`Ctrl+Shift+L` |
| 行操作（linesOperations） | 照搬 | P1 | 复制、移动、删除、合并行、排序行 |
| 单词操作（wordOperations、wordPartOperations） | 照搬 | P0 | 按单词、驼峰子词移动和删除，移植 VS Code 的单词边界规则 |
| 智能扩选（smartSelect） | 改造 | P1 | 基于 tree-sitter 语法树 |
| 注释切换（comment） | 照搬 | P1 | 使用各语言的 `language-configuration.json` |
| 自动缩进、缩进转换（indentation） | 照搬 | P1 | 移植 VS Code 的缩进规则引擎；自动检测缩进风格 |
| 自动闭合括号与引号 | 照搬 | P1 | 来自 `language-configuration.json` |
| 末尾自动加换行、去除行尾空格 | 照搬 | P1 | 默认关闭，与 VS Code 一致 |
| 查找 / 替换（find） | 照搬 | P0 | 浮动查找条，支持正则、大小写、全词、在选区内查找、保留大小写 |
| 编辑器内拖放文本（dnd） | 照搬 | P2 | |
| 格式化（format） | 改造 | P1 | 只做 JSON、XML 的格式化；**C# 不做格式化** |
| 补全（suggest）、参数提示（parameterHints）、代码片段（snippet） | 不做 | — | 需要实时语言服务，或者超出“局部编辑”的范围 |
| 重命名重构（rename）、代码动作（codeAction）、快速修复 | 不做 | — | 同上 |
| 联动编辑（linkedEditing，XML 标签同步重命名） | 照搬 | P2 | 不需要语言服务 |
| 内联补全（inlineCompletions）、行内对话（inlineChat） | 不做 | — | 内置 AI，不做 |

## 5. 导航与语言智能

| VS Code 功能 | 判定 | 优先级 | 说明 |
|---|---|---|---|
| 转到定义 / 声明 / 类型定义（gotoSymbol） | 改造 | P0 | 数据来自 C# 预计算索引；外部符号跳转到元数据视图 |
| 查找所有引用 | 改造 | P0 | **默认以速览窗口内嵌展开**（决策 D3）：`Shift+F12` 在当前位置下方展开速览，左侧代码预览、右侧按文件分组的结果列表。“引用”视图（侧边树形分组）作为备选，P1 |
| 速览（peekView：速览定义、速览引用、速览实现） | 照搬 | **P0** | 查找引用的默认展示方式（D3），因此提升为 P0；速览内部可以继续跳转，按 `Esc` 关闭，双击结果在编辑器中打开 |
| 转到实现 | 改造 | P1 | 使用索引中的关系表 |
| 调用层级（callHierarchy） | 改造 | P2 | 可以由索引推导：引用中属于“调用”的那部分 |
| 类型层级（typeHierarchy） | 改造 | P2 | 使用索引中的继承关系 |
| 引用数量提示（codelens 中的 “N 个引用”） | 改造 | P2 | 由索引直接给出，不需要实时计算 |
| 转到文件中的符号（`Ctrl+Shift+O`） | 照搬 | P0 | 数据来自大纲 |
| 转到工作区中的符号（`Ctrl+T`） | 改造 | P1 | 使用 L1 快速索引 |
| 转到行（`Ctrl+G`） | 照搬 | P0 | |
| 导航历史（前进、后退、上次编辑位置） | 照搬 | P0 | `Alt+←/→` |
| 大纲视图（outline） | 照搬 | P0 | |
| 问题面板（markers）、转到下一个问题（gotoError） | 改造 | P1 | 只显示 JSON/YAML/XML 的语法错误；**不做 C# 编译诊断** |
| 不生效的预处理代码置灰（`#if` 中按当前宏不成立的代码） | 照搬（VS Code 的 C# 扩展） | P1 | 数据来自预计算索引（D9） |
| 内嵌提示（inlayHints） | 不做 | P2 待议 | 参数名、推断类型可以由索引提供，但收益与成本待评估 |
| 语言检测（languageDetection） | 改造 | P1 | VS Code 用机器学习模型；我们只做基于扩展名和内容的启发式判断 |
| 语言状态（languageStatus） | 照搬 | P0 | 状态栏显示语言，以及 C# 索引的状态和进度 |

## 6. 文件、搜索与快速访问

| VS Code 功能 | 判定 | 优先级 | 说明 |
|---|---|---|---|
| 资源管理器（files） | 照搬 | P0 | 文件树、键盘导航、“在资源管理器中定位当前文件”、折叠全部、按名称过滤 |
| 资源管理器中的文件操作 | 照搬 | P1 | 新建、重命名、删除（移到回收站）、复制路径、在系统资源管理器中打开 |
| 文件排除（`files.exclude`） | 改造 | P0 | 默认排除 Unity 的大目录；**不读取 `.gitignore`** |
| 文件监听与自动重新加载 | 照搬 | P0 | |
| 已打开的编辑器列表 | 照搬 | P2 | |
| 快速打开（quickaccess：`Ctrl+P`） | 照搬 | P0 | 移植 VS Code 的模糊匹配打分算法；支持 `:行号`、`@符号`、`#工作区符号`、`>命令` 前缀 |
| 命令面板（`Ctrl+Shift+P`） | 照搬 | P0 | |
| 全局搜索（search） | 照搬 | P0 | 侧边栏视图：正则、大小写、全词、包含和排除过滤、按文件分组、匹配高亮 |
| 全局替换 | 照搬 | P1 | 带预览，可以一次撤销 |
| 搜索编辑器（searchEditor） | 照搬 | P2 | 把搜索结果作为可编辑的文档 |
| 本地历史（localHistory） | 改造 | P2 | 保存时留存历史版本，作为外部工具改坏文件时的安全网 |
| 比较文件 / 与已保存版本比较（diff 编辑器） | 改造 | P2 | 只读对比视图 |
| 图片预览（mediaPreview） | 照搬 | P1 | png、jpg、gif、svg、tga；只读 |
| 跟随文件末尾（类似 `tail -f`） | 新增（VS Code 没有） | P1 | 查看 Unity `Editor.log` 等日志；见 UX_SPEC §3.2 |
| 欢迎页 / 最近打开 | 改造 | P0 | 只保留“最近打开的文件夹和文件”与“打开文件夹”，没有引导流程 |
| 多根工作区（workspaces） | 不做 | — | 一个窗口一个文件夹；可以开多个窗口 |
| 工作区信任 | 不做 | — | Rove 不执行任何代码 |
| 热退出（关闭时保留未保存内容，下次恢复） | 照搬 | P1 | |

## 7. 外观、设置与键位

| VS Code 功能 | 判定 | 优先级 | 说明 |
|---|---|---|---|
| 颜色主题（themes） | 照搬 | P0 | **直接加载 VS Code 的主题 JSON**（`colors` 和 `tokenColors`）；内置 VS Code 当前的默认主题 Dark 2026、Light 2026（VS Code 1.145 的 `workbench.colorTheme` 默认值是 `Dark 2026`），以及 Dark Modern、Light Modern、Dark+、Light+。由于共用设置（D4），`workbench.colorTheme` 可能指向用户在 VS Code 中安装的扩展主题：P1 支持从 `%USERPROFILE%\.vscode\extensions\*\package.json` 的 `contributes.themes` 中**只读加载主题数据**（不运行任何扩展代码）；找不到时回退到内置主题 |
| 文件图标主题 | 改造 | P1 | 内置 VS Code 的 Seti；与颜色主题相同，可以只读加载用户在 VS Code 中安装的图标主题（不运行任何扩展代码），扩展目录通过启动参数 `--vscode-extensions-dir` 或环境变量 `ROVE_VSCODE_EXTENSIONS_DIR` 覆盖 |
| 产品图标（codicons） | 照搬 | P0 | codicons 字体为 CC-BY-4.0 许可，需注明来源 |
| 设置（`settings.json`） | 改造 | P0 | **与 VS Code 共用同一份设置文件**（D4），规则见第 13 节 |
| 设置界面（preferences） | 改造 | P1 | 首版只提供 JSON 编辑（打开 `settings.json`），图形化的设置界面以后再做 |
| 键位（keybindings） | 照搬 | P0 | 默认键位与 VS Code（Windows）一致；**与 VS Code 共用 `keybindings.json`**（D4），支持 `when` 子句的一个子集。为此，Rove 的 action ID 直接采用 VS Code 的命令 ID（如 `editor.action.revealDefinition`），用户的自定义键位才能原样生效 |
| 键位编辑器 | 改造 | P2 | 首版只提供 JSON 编辑 |
| 界面语言（localization） | 改造 | P1 | 内置中文和英文，不使用语言包扩展 |
| 设置同步、配置文件（profiles） | 不做 | — | 需要账号或网络 |

## 8. 状态栏

照搬 VS Code 的布局，只保留以下内容：
- 光标位置、选区大小
- 缩进（点击可以切换）、编码（点击可以用其他编码重新打开或保存）、换行符（点击可以切换）
- 语言模式（点击可以切换）
- C# 索引状态和进度
- 只读或超大文件标记
- 通知铃铛

## 9. 文档类型

| 功能 | VS Code 中的来源 | 判定 | 优先级 | 说明 |
|---|---|---|---|---|
| Markdown 预览（侧边预览、滚动同步） | markdown-language-features | **照搬** | P0 | **完整支持**（D5）：用系统自带的 WebView2 渲染，效果与 VS Code 一致，包括内嵌 HTML、Mermaid、数学公式，样式使用 VS Code 的 `markdown.css`；**`.md` 文件默认以预览模式打开**（D6） |
| Markdown 大纲、链接跳转 | markdown-language-features | 照搬 | P1 | |
| JSON：高亮、折叠、校验、格式化 | json、json-language-features | 改造 | P0（格式化 P1） | 不做基于 JSON Schema 的校验与补全 |
| JSON/YAML/XML 结构树视图 | （VS Code 没有内置） | 新增 | P1 | Rove 的增强 |
| Unity YAML 资产增强（`.prefab`、`.unity`、`.asset`） | （VS Code 没有） | 新增 | P2 | 结构树按 Unity 类型 ID 显示可读名称（GameObject、Transform、MonoBehaviour……）和 `m_Name`；以后支持按 `fileID` 跳转；见 UX_SPEC §4.2 |
| 上下文篮子：选中内容打包交给外部 AI | （VS Code 没有内置） | 新增 | P1 | 来自立项时的原始需求：把多段选区（可以跨文件）加入篮子，导出为结构化 Markdown 或 `@文件#L行` 引用，复制到剪贴板；不联网。活动栏中有一个入口 |
| YAML、XML：高亮、折叠、校验 | yaml、xml | 改造 | P0 | |
| 首批编程语言：C#（完整的语言智能） | csharp | 改造 | P0 | |
| 其他语言只做高亮和折叠（D2） | 内置语言扩展 | 改造 | P1 | ShaderLab（`.shader`）、HLSL（`.hlsl`、`.cginc`、`.compute`）、Lua、Protobuf；`.asmdef`/`.asmref` 按 JSON、`.meta` 按 YAML 处理（P0）。没有跳转和引用。ShaderLab 没有成熟的 tree-sitter 语法，需要自己编写一个简单语法或寻找替代方案，在规格阶段确认 |

## 10. 明确不做（对应 VS Code 的模块）

| 类别 | VS Code 模块 |
|---|---|
| 插件 | extensions、扩展市场、webview、webviewPanel、webviewView、customEditor（第三方）|
| 调试与运行 | debug、tasks、testing、externalTerminal、terminal、output、replNotebook、notebook、interactive |
| 版本控制 | scm、git、github、timeline、mergeEditor（以后可能提供只读的对比视图）|
| 内置 AI | chat、inlineChat、chatWindow、inlineCompletions、mcp、remoteCodingAgents、agentsVoice、sessions、speech |
| 远程与账号 | remote、remoteTunnel、authentication、userDataSync、editSessions、share、userDataProfile |
| 遥测与运营 | telemetry、surveys、issue、update（自动更新以后再评估）、emergencyAlert、meteredConnection、onboarding、welcomeWalkthrough |
| 实时语言服务 | suggest、parameterHints、codeAction、rename、snippets、emmet、colorPicker、inlayHints（待议）|

## 11. 可以直接复用的 VS Code 资产

| 资产 | 位置 | 许可 | 用途 |
|---|---|---|---|
| 默认主题 | `extensions/theme-defaults/themes/*.json` | MIT | 内置主题；同时也是主题格式兼容性的测试样本 |
| 语言配置 | `extensions/{csharp,json,markdown-basics,xml,yaml}/language-configuration.json` | MIT | 括号、注释、自动闭合、缩进规则、单词模式 |
| 默认键位表 | 散布在源码各处的 `KeybindingsRegistry` 注册中 | MIT | 整理出一份 Windows 默认键位表 |
| 设置项 schema | 各模块的 `configurationRegistry` 注册 | MIT | 设置名称、默认值、说明文字 |
| codicons 图标字体 | microsoft/vscode-codicons | CC-BY-4.0 | 界面图标 |
| Seti 文件图标 | `extensions/theme-seti` | MIT | 文件类型图标 |
| Markdown 预览样式 | `extensions/markdown-language-features/media/markdown.css` | MIT | 原生渲染时参照的排版参数 |
| 模糊匹配、单词边界、缩进规则等算法 | `src/vs/base/common/fuzzyScorer.ts` 等 | MIT | 翻译成 Rust，保证行为与 VS Code 一致 |

所有移植内容都在 `THIRD_PARTY_NOTICES.md` 中注明来源与许可证。

## 12. 决策记录

| # | 问题 | 决策（2026-11-09） |
|---|---|---|
| D1 | 语法高亮引擎 | tree-sitter，捕获名映射为 TextMate scope，兼容 VS Code 主题 |
| D2 | 其他语言 | Unity 项目常见语言只提供高亮和折叠，不支持跳转 |
| D3 | 查找引用的默认展示方式 | 在代码中内嵌展开的速览窗口 |
| D4 | 设置迁移方式 | 与 VS Code 共用同一份设置文件和键位文件 |
| D5 | Markdown 渲染方式 | 完整支持（内嵌 HTML、Mermaid、数学公式）；**仅 Markdown 预览使用 WebView2**，其余界面仍然原生渲染 |
| D6 | `.md` 默认打开方式 | 默认以预览模式打开（VS Code 默认打开源码） |
| D7 | 所见即所得 Markdown 编辑器（VS Code 1.145 新增的 `vscode.markdown.editor`） | 不做，保持“预览 + 源码”两种模式 |
| D8 | 热退出的里程碑 | 从 M4 提前到 M3，避免 M3 交付后意外退出丢失未保存的修改 |
| D9 | 不生效的预处理代码置灰 | 新增，P1，与 VS Code 的 C# 扩展一致，归 M4 的语义高亮 |
| D10 | 文件图标主题 | 内置 Seti，并像颜色主题一样只读加载用户在 VS Code 中安装的图标主题（不运行扩展代码） |

## 13. 与 VS Code 共用设置文件的规则（D4 的落地）

1. **文件位置**：默认读取 `%APPDATA%\Code\User\settings.json` 和 `keybindings.json`。可以通过环境变量 `ROVE_VSCODE_USER_DIR` 或启动参数 `--vscode-user-dir` 改为其他发行版的目录（例如 VSCodium、Cursor）——这个位置不能写在设置文件里，否则会出现“先要知道文件在哪，才能读到文件在哪”的问题。如果本机没有安装 VS Code，就在同样的位置创建文件。
2. **读取**：解析 JSONC（允许注释和尾逗号）。支持语言级覆盖（如 `"[csharp]": {...}`）。Rove 不支持的设置项静默忽略，不报错，不提示（因为同一个文件里本来就有大量 VS Code 专有的设置项）。
3. **写入**：只有当用户在 Rove 中主动修改设置时才写入（例如切换主题、修改字号）。写入时**保留原有的注释、格式和键的顺序**，只修改目标键。写入前先备份到 `%LOCALAPPDATA%\Rove\settings-backup\`。
4. **Rove 专有设置**：放在 `rove.*` 命名空间下（如 `rove.index.cacheDir`、`rove.largeFile.threshold`）。VS Code 会把它们当作未知设置并忽略，两边互不干扰。
5. **默认值的差异**：如果 Rove 需要与 VS Code 不同的默认值（例如默认排除 Unity 的大目录），**只在用户没有设置该项时生效**，并且写成 `rove.*` 中的附加规则（如 `rove.files.extraExclude`），而不是改写用户的 VS Code 设置项。 **明确例外**（已确认，见 `compat/SETTINGS.md` §3）：`files.autoGuessEncoding`、`files.candidateGuessEncodings`、`workbench.editorAssociations`、`markdown.preview.doubleClickToSwitchToEditor` 可以直接使用 Rove 的缺省值，但只在用户未设置时生效，绝不自动写回共用文件；其他差异仍通过 `rove.*` 附加规则表达。
6. **热加载**：监听这两个文件。VS Code 修改了设置时，Rove 实时生效，反之亦然。
7. **键位**：`keybindings.json` 中的命令 ID 如果在 Rove 中不存在，就忽略；`when` 子句中 Rove 不认识的上下文键，一律视为 false。
