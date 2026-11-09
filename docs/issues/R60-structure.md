# R60 JSON/YAML/XML：结构树、语法校验、问题面板与转到下一个问题、格式化与压缩、复制路径

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 3 困难 | docview | 第 5 批 | R26（大纲）、R09（tree-sitter 集成）、R16（工作台）、R45（编辑核心模型） | R66（跟随文件末尾、格式化后查看）、R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

`FEATURES.md` §9 把「JSON/YAML/XML 结构树视图」列为 Rove **新增**的能力（P1），`UX_SPEC.md` §4.2 把它落到了一张表上。关键的定位决定是：**结构树不是一个新视图，而是资源管理器「大纲」小节在这三种文件上的加强版**——VS Code 本来就在大纲里显示 JSON 的键，Rove 把同样的做法扩展到 YAML 和 XML，并补上值预览。

R26（大纲） 已经做好了大纲小节的框架、符号树类型（R01（【接口先行 A1】架构与接口契约） 定义）与「键名 + 类型图标」的最简显示，并明确把值预览、复制路径 / 值、折叠同级、语法校验留给本 issue。R16（工作台） 已经做好了面板容器与「问题」视图的空状态（「未在工作区检测到问题。」），内容由本 issue 填充。

这是一个**纵向条目**：结构数据模型、大纲中的呈现、语法校验与问题面板、格式化与压缩，四件事共用同一棵带源码区间的结构树，分开做会让接口在多个 PR 之间来回改。

必读：`UX_SPEC.md` §4.2（整张表 + Unity YAML 段）、§1.4（面板）、§3.1、§3.3（压缩 JSON）、§6.1–§6.4，`FEATURES.md` §4（格式化只做 JSON、XML）、§5（问题面板只显示 JSON/YAML/XML 语法错误，不做 C# 编译诊断）、§9，`docs/compat/SETTINGS.md`（`json.format.*`、`json.maxItemsComputed`、`problems.*`、`outline.problems.*`、`editor.formatOnSave*`），`KEYBINDINGS.md`（问题一节、格式化一节、§7 附录「P1 · 问题」与 Rove 专有命令 `rove.json.minify` / `rove.structure.copyPath` / `rove.structure.copyValue`），`THEME_COLORS.md`（`editorError/Warning/Info.*`、`problemsErrorIcon/WarningIcon/InfoIcon.foreground`、`editorMarkerNavigation*`）。

## 目标

### 1. 结构数据模型（核心层，不依赖 gpui）

在 R09（tree-sitter 集成） 的 tree-sitter 语法树之上，为 JSON / JSONC、YAML、XML 产出一棵结构树：

- 每个节点带**源码区间**（完整区间 + 选择区间，用 R01（【接口先行 A1】架构与接口契约） 的符号树类型，不新造），以及种类（对象、数组、字符串、数字、布尔、null、元素、属性）。
- **路径双向映射**：位置 → 路径、路径 → 位置。JSON / YAML 用 `$.items[3].name`，XML 用 XPath `/root/item[3]`（`UX_SPEC.md` §4.2 的右键菜单一行）。
- **惰性展开**：50 MB 的 JSON 不得一次性建完整棵树，子节点在展开时才构造；遵循 `json.maxItemsComputed`（默认 5000）对大纲符号与折叠区域的数量上限。
- YAML 多文档（`---`）各成一棵子树（与 R26（大纲） 已有的规则一致）。
- 增量：文件重新解析后只重建受影响的子树，并给出「哪些节点变了」。

### 2. 大纲中的结构树呈现（`UX_SPEC.md` §4.2）

在 R26（大纲） 的大纲小节里，对这三种文件把节点显示升级为：

- **值预览**：字符串、数字等显示截断后的值；对象和数组显示子项数量（`items [24]`）。XML 节点显示元素名 + 关键属性（`id`、`name`）+ 子元素数量。
- **跟随光标**沿用 R26（大纲） 已有的「大纲视图菜单中的开关」（**不是设置项**，§4.2 已明确），本 issue 不新增设置项；**过滤**复用 R26（大纲） / R18（通用列表、树与快速输入组件） 的实现。
- **右键菜单新增三项**：复制路径（`rove.structure.copyPath`）、复制值（`rove.structure.copyValue`）、折叠同级节点。前两条是 Rove 专有命令，无默认键位。
- 当前位置的路径只在**面包屑**显示，**不在状态栏另显示路径**（§4.2 明确「避免信息重复」）。

### 3. 语法校验、问题面板与转到下一个问题

- 校验：JSON、YAML、XML 的语法错误及其精确位置（行、UTF-8 列、一句话说明）。**不做 JSON Schema 校验**（§4.2、`FEATURES.md` §9），相关设置项（`json.schemas`、`json.validate.enable` 等）在 `SETTINGS.md` 中已判为不支持。波浪线装饰经 R19（代码视图 Element） 的装饰接口提交，颜色 `editorError.foreground` / `border` / `background`、`editorWarning.*`、`editorInfo.*`；概览标尺与缩略图上的标记由 R55（缩略图与概览标尺） 绘制，本 issue 只提供位置集合。
- **问题面板**：填充 R16（工作台） 已登记的「问题」视图，`problemsErrorIcon` / `WarningIcon` / `InfoIcon.foreground`；遵循 `problems.autoReveal`、`problems.decorations.enabled`、`problems.defaultViewMode`（`tree` / `table`）、`problems.showCurrentInStatus`、`problems.sortOrder`、`problems.visibility`。命令：`workbench.actions.view.problems`（`ctrl+shift+m`，含「人工补充的键位」那一条 `!workbench.panel.markers.view.active`）、`workbench.actions.view.toggleProblems`、`problems.action.clearFilterText` / `copy` / `focusFilter` / `focusProblemsFromFilter` / `open` / `openToSide`，以及 §7 附录「P1 · 问题」的 11 条（`showQuickFixes` 只登记，行为为空——不做快速修复）。上下文键 `problemFocus`、`problemsFilterFocus`、`problemsVisibility`。
- **转到下一个问题**：`editor.action.marker.next`（`alt+f8`）、`marker.nextInFiles`（`f8`）、`marker.prev`（`shift+alt+f8`）、`marker.prevInFiles`（`shift+f8`），以及它们打开的**问题速览**（`editorMarkerNavigation.background`、`editorMarkerNavigationError/Warning/Info.background` 与三个 `headerBackground`，`markersNavigationVisible` 上下文键）。速览形态复用 R41（速览窗口） 的「行下方嵌入区域」，不另造浮层。
- **问题数据的对外出口**：本 issue 提供一个「问题提供方」登记点与查询接口，供三个已存在的消费方接入——R16（工作台） 的状态栏问题计数条目、R23（资源管理器） 的文件 / 文件夹错误警告计数装饰、以及大纲上的问题徽章（`outline.problems.badges` / `colors` / `enabled`，R26（大纲） 当时因为没有数据而留空，本 issue 补上）。R13（设置层） 的「设置文件有语法错误」（`UX_SPEC.md` §6.4）也通过这个登记点把错误位置报进问题面板。
- 空状态照搬 R16（工作台） 已有的「未在工作区检测到问题。」（`UX_SPEC.md` §6.3）。

### 4. 格式化与压缩

- `editor.action.formatDocument`（`shift+alt+f`，when `editorHasDocumentFormattingProvider && editorTextFocus && !editorReadonly && !inCompositeEditor`）、`editor.action.formatDocument.none`（无提供方时的提示）、`editor.action.formatSelection`（`ctrl+k ctrl+f`）、以及 §7 附录的 `formatDocument.multiple` / `formatSelection.multiple`（只登记，Rove 只有一个内置提供方）。
- 提供方：**JSON 与 XML**（`FEATURES.md` §4 判定改造、P1；**C# 不做格式化**，YAML 也不做）。上下文键 `editorHasDocumentFormattingProvider` / `editorHasDocumentSelectionFormattingProvider` 按当前文档语言设置。
- JSON 遵循 `json.format.enable`（true）、`json.format.keepLines`（false）；缩进取 `editor.tabSize` / `insertSpaces`（含语言级覆盖）。
- **格式化必须幂等**（连续两次与一次结果相同），**保持原有编码与换行符**（含混合换行符按 `UX_SPEC.md` §3.6 保持原样），BOM 保留。改动通过 R45（编辑核心模型） 的编辑事务一次性提交，**整次格式化可以一次撤销**；`rove.json.minify`（Rove 专有命令，无默认键位）压缩 JSON，同样是一次事务。
- `editor.formatOnSave` / `formatOnSaveMode` / `formatOnPaste`（P1）对这两种语言生效，保存路径走 R48（保存与外部修改）。
- 耗时操作可取消（`UX_SPEC.md` §6.2 最后一行：超过 1 秒的操作必须可取消）。

### 5. 按级别降级（`UX_SPEC.md` §4.2 最后一行 + §3.1）

**大文件**（≥ 20 MB 或 ≥ 30 万行）：结构树在后台构建、节点展开时才加载，校验也在后台完成。**超大文件**（≥ `rove.largeFile.threshold`）：不提供结构树，大纲中显示原因；不校验、不格式化（文件只读）。

`UX_SPEC.md` §3.3 的「压缩过的 JSON（整个文件只有一行）」场景要能走通：结构树正常可用，并与 R66（跟随文件末尾、格式化后查看） 的「格式化后查看」（`rove.editor.viewFormatted`）共用本 issue 的格式化实现。

### 6. 登记与替换

新增命令、设置项、颜色键、界面字符串、视图内容通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。装饰经 R19（代码视图 Element） 的接口，视图经 R16（工作台） 的贡献点，编辑经 R45（编辑核心模型） 的事务。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- **Unity YAML 资产增强**（`.prefab` / `.unity` / `.asset` 按类型 ID 显示 GameObject / Transform / MonoBehaviour、显示 `m_Name`、按 `fileID` 跳转）：`UX_SPEC.md` §4.2 与 `FEATURES.md` §9 都标为 **P2**，本 issue 不做。这三种扩展名按普通 YAML 处理（R09（tree-sitter 集成） 已有关联），多文档分子树的能力已经覆盖基本可读性。
- JSON Schema 校验与补全、C# 编译诊断、快速修复（`FEATURES.md` §5、§9 明确不做）。
- 大纲小节的框架、树与键盘导航、面包屑（R26（大纲）、R18（通用列表、树与快速输入组件））；面板容器与空状态（R16（工作台））。
- tree-sitter 语法与增量解析（R09（tree-sitter 集成））；折叠区间（R20（代码视图））。
- 编辑事务与撤销重做（R45（编辑核心模型））、保存（R48（保存与外部修改））；概览标尺与缩略图的绘制（R55（缩略图与概览标尺））；速览窗口框架（R41（速览窗口））。
- Markdown 的大纲与链接（R26（大纲）、R29（Markdown 预览））；`linkedEditing`（XML 标签同步重命名，P2）。

## 验收标准

- [ ] JSON、JSONC、YAML（含多文档）、XML 各有结构树快照测试（含嵌套、空文件、语法错误的文件）
- [ ] 路径双向映射有测试：`$.items[3].name` 与 `/root/item[3]` 各一组（位置 → 路径、路径 → 位置）
- [ ] 值预览：字符串 / 数字截断、`items [24]` 形式的计数、XML 的元素名 + `id` / `name` + 子元素数量，各有用例
- [ ] 右键菜单三项可用：复制路径、复制值、折叠同级（`rove.structure.copyPath` / `copyValue` 无默认键位）；路径只出现在面包屑，状态栏**没有**路径条目
- [ ] 三种语言的语法错误都产生波浪线 + 问题面板条目，位置（行、UTF-8 列）准确；**没有** JSON Schema 相关的校验
- [ ] `problems.*` 六个设置项逐项生效；`Ctrl+Shift+M` 在面板隐藏与问题视图已激活两种状态下都能打开
- [ ] `alt+f8` / `f8` / `shift+alt+f8` / `shift+f8` 四个命令可用，问题速览使用 `editorMarkerNavigation*` 颜色键，`markersNavigationVisible` 取值正确
- [ ] 问题数据的三个消费方都拿到数据：状态栏问题计数、资源管理器错误警告装饰、大纲问题徽章（`outline.problems.*` 三个设置项生效）
- [ ] JSON 与 XML 的格式化：**幂等**（两次 = 一次）、保持编码与换行符（含混合换行符与 BOM）、`json.format.keepLines` 两个取值都生效；格式化与 `rove.json.minify` 各自**可一次撤销**，超过 1 秒时可取消
- [ ] 「格式化选中内容」（`ctrl+k ctrl+f`）可用；C# 与 YAML 文件上 `shift+alt+f` 走 `formatDocument.none` 的提示分支
- [ ] **50 MB 的 JSON 结构树首屏 < 300 ms**（惰性展开，附测量数据，基准接入 R04（性能基准与预算守护））；单行压缩 JSON 场景走通
- [ ] 大文件级别结构树在后台构建、展开时加载；超大文件不提供结构树且大纲中显示原因
- [ ] 新增的命令、设置项、颜色键、界面字符串、视图内容都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增集成测试（打开 `.json` → 断言结构树节点与值预览；造一个语法错误 → 断言波浪线与问题面板条目；`Shift+Alt+F` → 断言格式化结果与一次撤销）

## 风险与提示

- 50 MB JSON 的 300 ms 首屏只有靠**真正的惰性**才能达到：解析可以是增量的，但结构树节点一定要按需构造。先把「根节点 + 第一层」做出来，剩下的在展开事件里补。`json.maxItemsComputed` 是 VS Code 为同一个问题设的闸门，照搬它的默认值。
- 本 issue 是「问题」这条数据链路的**唯一源头**，而 R16（工作台）、R23（资源管理器）、R26（大纲）、R13（设置层） 四个已合并的 issue 都在等它。把「问题提供方」做成一个登记点并自带空实现，这样它们不用改代码就能接上；PR 中要逐个确认它们确实拿到了数据。
- 格式化依赖 R45（编辑核心模型） 的编辑事务，但本 issue 登记的前置依赖只有 R26（大纲）、R09（tree-sitter 集成）、R16（工作台）。M4 在 M3 之后，实际开工时 R45（编辑核心模型） 必定已合并；如果排期调整导致它还没合并，先把格式化做成整体文本替换，并在 PR 中写明这是临时形态。
- 幂等不是「看起来一样」。要写一个属性测试：随机生成 JSON / XML，格式化两次比对字节级结果，并且**编码与换行符不变**。混合换行符的文件是最容易出事的一类（`UX_SPEC.md` §3.6 要求保持原样，不许静默统一）。
- XPath 的下标是从 1 开始的（`/root/item[3]` 是第 3 个），JSON 路径的下标是从 0 开始的（`$.items[3]` 是第 4 个）。这两套下标在同一个右键菜单里，很容易写成同一套，务必各写一组测试。
- 问题面板的措辞按 `UX_SPEC.md` §6 的风格：先说发生了什么，再说可以怎么做，不加「错误：」前缀。
