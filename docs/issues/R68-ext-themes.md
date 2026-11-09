# R68 只读加载 VS Code 扩展主题、Seti 文件图标主题

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 2 中等 | ui | 第 5 批 | R10（主题引擎）、R23（资源管理器） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

因为共用设置（D4），`workbench.colorTheme` 可能指向用户在 VS Code 里装的**任何**扩展主题（One Dark Pro、GitHub Theme、Material Theme……），`workbench.iconTheme` 默认值更是直接指向 Seti（`vs-seti`）。R10（主题引擎） 已经把主题引擎做好了，但对扩展主题只做了「回退到内置主题 + 一条日志」。`FEATURES.md` §7 把这件事列为 P1：**从 `%USERPROFILE%\.vscode\extensions\*\package.json` 的 `contributes.themes` 中只读加载主题数据（不运行任何扩展代码）**。

文件图标主题是同一件事的另一半：`SYNTAX_AND_LANGUAGES.md` §7 要求内置 VS Code 的 Seti 文件图标主题（`extensions/theme-seti`，MIT；图标字体来自 seti-ui，MIT），并按 VS Code 的图标主题格式实现（文件名、扩展名、语言 ID 三种关联方式）。R23（资源管理器） 现在用的是内置 codicons 的临时图标，并把 Seti 留给了本 issue。

必读：`FEATURES.md` §7（颜色主题、文件图标主题两行）、§11（可复用资产与许可）、§13（共用设置规则）；`docs/compat/SYNTAX_AND_LANGUAGES.md` §7；`docs/compat/SETTINGS.md`（`workbench.colorTheme`、`workbench.iconTheme`、`workbench.preferredDarkColorTheme` / `preferredLightColorTheme`、`window.autoDetectColorScheme`、`workbench.editor.showIcons`）；`docs/compat/KEYBINDINGS.md` §7 附录（`workbench.action.selectIconTheme`、`workbench.action.toggleLightDarkThemes`）；`THEME_COLORS.md`。

## 目标

### 1. 扫描扩展目录（只读、不执行）

- 默认目录 `%USERPROFILE%\.vscode\extensions`。其他发行版（VSCodium、Cursor）的扩展目录与 R13（设置层） 的用户目录是两个不同的位置，所以新增启动参数 `--vscode-extensions-dir`（优先）与环境变量 `ROVE_VSCODE_EXTENSIONS_DIR`，与 `FEATURES.md` §13 规则 1 同理，**不能写在设置文件里**。这一条与 R13（设置层） 的 `--vscode-user-dir` 在开工时对齐命名。
- 只解析 `package.json` 与它引用到的 **JSON** 文件。**绝不加载、不执行任何 `.js`**，不读 `node_modules`，不看 `main` / `activationEvents`。
- 路径安全：`contributes.themes[].path`、`include` 链、图标主题里的 `iconPath` / 字体 `src` 一律解析为绝对路径后校验仍在该扩展目录内，越界即拒绝并记日志（扩展是第三方内容，`PROJECT_CHARTER.md` §2 里 Rove 不执行任何代码）。
- 同一扩展有多个版本目录（`publisher.name-1.2.3`）时取版本号最大的一个；被用户在 VS Code 中禁用的扩展无法从磁盘判断（状态在 VS Code 的数据库里），一律视为可用，并在 PR 中记录这个限制。
- 扫描**不得阻塞启动**：首帧之前只用内置主题，扫描在后台执行器上做，结果带缓存（按目录 mtime 失效），完成后按需切换。

### 2. 颜色主题的解析与接入

- 解析 `contributes.themes[]` 的 `label`、`id`、`uiTheme`（`vs` / `vs-dark` / `hc-black` / `hc-light`）、`path`，把主题 JSON 交给 R10（主题引擎） 既有的加载器（`colors`、`tokenColors`、`semanticTokenColors`、`include` 链都已实现，本 issue 不重复造）。
- `workbench.colorTheme` 的取值是**主题的 label**（VS Code 的语义），按 label 匹配，匹配不到再按 `id` 匹配；仍然找不到时沿用 R10（主题引擎） 已有的回退 + 日志。
- `workbench.action.selectTheme`（`ctrl+k ctrl+t`）的快速选择器由 R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 提供，本 issue 只往它的数据源里**追加扩展主题分组**（分组标题「来自扩展」，与 VS Code 一致），实时预览与 `Esc` 还原的行为不变。
- 系统深浅色跟随：`window.autoDetectColorScheme`（P1，默认 false）为 true 时按系统颜色模式在 `workbench.preferredDarkColorTheme` / `preferredLightColorTheme` 之间切换；`workbench.action.toggleLightDarkThemes`（P0 附录命令）在两者之间手工切换。这三项在现有 issue 中没有归属，**由本 issue 一并实现**（它是唯一碰主题选择的 M4 issue）。
- 扩展主题文件在磁盘上被改动（用户更新扩展）时热加载并通知订阅方，复用 R10（主题引擎） 的主题变更通知。

### 3. 文件图标主题

- 内置 **Seti**（`extensions/theme-seti`，MIT；`seti-ui` 字体 MIT），在 `THIRD_PARTY_NOTICES.md` 中注明两处来源与许可。`workbench.iconTheme` 的默认值 `vs-seti` 因此能开箱即用。
- 按 VS Code 的文件图标主题格式实现：`iconDefinitions`（`fontCharacter` + `fontColor` 或 `iconPath`）、`fonts`（woff / woff2，按字号与权重）、`file` / `folder` / `folderExpanded` / `rootFolder*` 默认项、`fileExtensions` / `fileNames` / `languageIds` 三种关联（优先级与 VS Code 一致：`fileNames` > `fileExtensions` > `languageIds`）、`light` 与 `highContrast` 变体、`hidesExplorerArrows`。
- `workbench.iconTheme` 为 `null` 时不显示任何文件图标（VS Code 语义）。`workbench.action.selectIconTheme`（P0 附录命令）提供选择器，列出内置 Seti 与扫描到的扩展图标主题。
- **只读加载扩展贡献的图标主题**（`contributes.iconThemes`），与颜色主题走同一条扫描与安全校验路径。⚠️ `FEATURES.md` §7 当前的措辞是「内置一套……**不支持用户安装图标主题**」，而 `SYNTAX_AND_LANGUAGES.md` §7 写的是「以后也可以只读加载……与颜色主题的处理方式相同」。本 issue 按后者实现，**需要同步修订 `FEATURES.md` §7 的那半句**（文档改动在本 PR 中一并提交）。
- 对外提供一个「文件图标服务」接口（按路径 / 语言 ID 取图标），**替换 R23（资源管理器） 中按扩展名区分的 codicons 临时实现**；标签页（`workbench.editor.showIcons`）、快速打开、面包屑等已有图标位的视图接同一个服务，没有图标位的地方不新增界面。

### 4. 登记与替换

新增命令、设置项、界面字符串全部通过 R01（【接口先行 A1】架构与接口契约） 与 R16（工作台） 的登记机制加入，不得写死；颜色键沿用 R10（主题引擎） 的登记表，本 issue 不新增颜色键。删除 R23（资源管理器） 在图标处留下的临时实现说明，并更新 `docs/ARCHITECTURE.md` 的对照表。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 与 R10（主题引擎） 的接口（需要扩展加载器时走契约变更流程单独提小 PR）。

## 不包含

- 主题 JSON 的解析、scope 匹配、颜色键默认值、用户覆盖（全部由 R10（主题引擎） 完成）。
- 安装、更新、卸载扩展；扩展市场；任何网络访问（`PROJECT_CHARTER.md` §2）。
- 产品图标主题 `workbench.productIconTheme`（`SETTINGS.md` 判定为不支持）。
- 扩展贡献的语法（`contributes.grammars`）、语言配置、代码片段、键位、设置项——**只读扩展主题不等于读扩展的其他贡献点**；语法与语言配置随二进制分发（R09（tree-sitter 集成）、R12（语言配置））。
- 语义 token 的产出（R58（C# 语义高亮与不生效预处理代码置灰））；主题选择器本体（R28（欢迎页、最近打开、启动参数打开，以及首选项入口））；文件装饰（R23（资源管理器））。

## 验收标准

- [ ] 在真实的 `%USERPROFILE%\.vscode\extensions` 下（至少装 3 个主题扩展，含一个带 `include` 链、一个带 `semanticTokenColors` 的）：`workbench.colorTheme` 指向其中任意一个都能正确加载并上屏，截图与 VS Code 同主题对照无明显差异
- [ ] `workbench.colorTheme` 按 label 匹配；写成 `id` 时也能匹配；写成不存在的名字时回退内置主题并有一条可定位的日志
- [ ] 扫描**不阻塞启动**：装有 ≥ 50 个扩展时冷启动仍 < 400 ms（附测量数据）；扫描耗时单独记录
- [ ] 扩展目录里有损坏的 `package.json`、缺失的主题文件、`path` 指向目录之外时，跳过该项并记日志，其余主题不受影响
- [ ] 断言加载过程中**没有读取任何 `.js`**、没有访问 `node_modules`、没有任何网络请求（集成测试中断言）
- [ ] `--vscode-extensions-dir` / `ROVE_VSCODE_EXTENSIONS_DIR` 指向临时目录时生效，默认目录不被读取
- [ ] Seti 图标主题：资源管理器中 `.cs`、`.json`、`.shader`、`.md`、`.meta`、文件夹与展开态的图标都正确，与 VS Code 截图对照
- [ ] 三种关联方式的优先级有测试（同一个文件同时命中 `fileNames` 与 `fileExtensions` 时取前者）；`light` 变体在浅色主题下生效
- [ ] `workbench.iconTheme` 设为 `null` 时不显示文件图标；设为某个扩展图标主题时生效；`workbench.action.selectIconTheme` 能列出并切换
- [ ] 标签页在 `workbench.editor.showIcons=true` 时显示与资源管理器一致的图标
- [ ] `window.autoDetectColorScheme=true` 时跟随系统深浅色切换，`workbench.preferredDark/LightColorTheme` 生效；`workbench.action.toggleLightDarkThemes` 可用
- [ ] 扩展主题文件在运行中被改动后热加载生效，不需要重启
- [ ] `THIRD_PARTY_NOTICES.md` 中已注明 `theme-seti`（MIT）与 seti-ui 字体（MIT）
- [ ] `FEATURES.md` §7 关于图标主题的措辞已同步修订
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增「从扩展目录加载主题 → 解析某个颜色键」与「按路径取图标」的集成测试
- [ ] `docs/ARCHITECTURE.md` 的对照表已更新

## 风险与提示

- **扩展目录是第三方内容**，是整个 Rove 里少数几处「读外部不可信数据」的地方。路径穿越、超大 JSON、深层 `include` 循环、指向网络的字体 `src` 都要当成恶意输入处理，解析器要有大小与深度上限。
- 用户装了几百个扩展是常态，其中绝大多数不贡献主题。先只读 `package.json` 的 `contributes` 字段再决定要不要往下读，不要一上来就读每个扩展的全部文件。
- 图标字体是 woff/woff2，字形通过 `fontCharacter` 取。GPUI 的字体栈能否直接加载内存中的 woff 要**先验证**（M0 报告没有覆盖这一项）；不行的话退回把需要的字形预转成 SVG / 位图，并在 PR 中说明。
- `workbench.iconTheme` 在 `SETTINGS.md` 里标的是 P0、默认值 `vs-seti`，而文件图标主题功能本身是 P1（M4）。这意味着 M1–M3 期间这个键是「已登记但不生效」——本 issue 合并后它才真正生效，验收时注意不要把它当成回归。
- 不要顺手去支持 `contributes.grammars`。那是另一条完全不同的路（tree-sitter vs TextMate），而且 D1 已经决定不走 TextMate 引擎。
