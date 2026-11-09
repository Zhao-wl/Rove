# R20 代码视图：高亮与主题、括号匹配、文本同名高亮、折叠（语法、标记、缩进）、缩进参考线、空白字符、只读状态界面

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 3 困难 | ui | 第 5 批 | R19（代码视图 Element）、R09（tree-sitter 集成）、R10（主题引擎）、R12（语言配置）、R15（工作区） | R31（【集成验收】M1）、R40（C# 导航交互）、R53（智能扩选、括号跳转与选择）、R54（粘性滚动）、R55（缩略图与概览标尺）、R56（括号对着色与活动缩进参考线高亮） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

R19（代码视图 Element） 画出了文本、行号、选区与滚动，但没有颜色。本 issue 把 R09（tree-sitter 集成） 的 token、R10（主题引擎） 的 scope 样式、R12（语言配置） 的语言配置、R15（工作区） 的只读规则接进代码视图，让它真正成为“代码阅读视图”：语法着色、括号匹配、同名高亮、折叠、缩进参考线、空白字符，以及只读状态的界面表现。

同时，这里是 `UX_SPEC.md` §3.1 文件大小分级的落点：大文件级别要关掉一部分功能，超大文件级别要关得更多。

必读：`FEATURES.md` §3、§12（D1），`UX_SPEC.md` §3.1、§3.4，`docs/compat/KEYBINDINGS.md`（阅读一节的折叠命令），`SETTINGS.md`（`editor.folding*`、`editor.guides.*`、`editor.renderWhitespace`、`editor.matchBrackets`、`editor.occurrencesHighlight`、`editor.maxTokenizationLineLength`、`files.readonly*`），`THEME_COLORS.md`（Editor colors）。

## 目标

### 1. 语法着色与主题

- 把 R09（tree-sitter 集成） 的按行 token（字节区间 + TextMate scope 列表）经 R10（主题引擎） 的 scope 匹配解析为样式，作为 R19（代码视图 Element） 的行样式区间提交；**样式包含 `fontStyle`**（粗体、斜体、下划线、删除线），不只是前景色。
- 样式参与整形缓存键（粗体 / 斜体会改变字形与宽度），与 R19（代码视图 Element） 的缓存协议对齐。
- 增量更新：解析结果变化时只让“发生变化的行”失效（R09（tree-sitter 集成） 给出变化行集合）。
- **结果过期时沿用旧样式，不闪烁**；首次解析未完成时按 §3.1 的规则先显示纯文本，完成后一次性替换。
- 遵循 `editor.tokenColorCustomizations`、`editor.maxTokenizationLineLength`（默认 20000，`[csharp]` / `[javascript]` 为 2500），主题切换时整体重算。语义高亮是 R58（C# 语义高亮与不生效预处理代码置灰）（M4），本 issue 不做。

### 2. 括号匹配

`editor.matchBrackets`（默认 `always`）：按 R12（语言配置） 的括号定义，高亮光标两侧匹配的括号对；颜色 `editorBracketMatch.background` / `border` / `foreground`。括号对着色与活动缩进参考线高亮属于 R56（括号对着色与活动缩进参考线高亮）（M4）；`editor.action.jumpToBracket`（P1）属于 R53（智能扩选、括号跳转与选择）。

### 3. 文本同名高亮

- 按文本匹配高亮当前单词的其他出现（`editor.occurrencesHighlight` 默认 `singleFile`、`occurrencesHighlightDelay`），以及选区相似匹配高亮（`editor.selectionHighlight`、`selectedTextMatchMode`、`selectionHighlightMaxLength`、`selectionHighlightMultiline`）。
- 颜色 `editor.wordHighlightBackground` / `Border`、`wordHighlightStrong*`、`wordHighlightText*`、`editor.selectionHighlightBackground` / `Border`，并设置 `hasWordHighlights` 上下文键。
- **语义级同名高亮**（只高亮同一个符号）由 R40（C# 导航交互） 接入，并按 `UX_SPEC.md` §2.2 在近似结果时退化为文本高亮且不加提示；`editor.action.wordHighlight.next` / `prev`（`f7` / `shift+f7`，P1）随 R40（C# 导航交互） 实现。

### 4. 折叠界面

- 折叠区间来源：R09（tree-sitter 集成） 的语法区间、`#region` 标记区间、缩进区间（`editor.foldingStrategy`、`editor.folding`、`editor.foldingMaximumRegions`）。
- 折叠控件与绘制：`editor.showFoldingControls`、`editor.foldingHighlight`、`editor.unfoldOnClickAfterEndOfLine`、`editor.foldBackground`、`editor.foldPlaceholderForeground`、`editorGutter.foldingControlForeground`；折叠行的占位标记。
- 命令全套（KEYBINDINGS.md 阅读一节）：`editor.fold`、`editor.unfold`、`editor.toggleFold`、`editor.foldRecursively`、`editor.unfoldRecursively`、`editor.toggleFoldRecursively`、`editor.foldAll`、`editor.unfoldAll`、`editor.foldLevel1`–`foldLevel7`、`editor.foldAllBlockComments`、`editor.foldAllMarkerRegions`、`editor.unfoldAllMarkerRegions`、`editor.foldAllExcept`、`editor.unfoldAllExcept`、`editor.createFoldingRangeFromSelection`、`editor.removeManualFoldingRanges`，以及 `foldingEnabled` 上下文键。
- 折叠状态随标签保存与恢复（通过 R17（编辑器组与标签） 的标签视图状态）。

### 5. 缩进参考线与空白字符

`editor.guides.indentation`（`editorIndentGuide.background1`–`background6`）、`editor.renderWhitespace`（`editorWhitespace.foreground`）、`editor.renderControlCharacters`（P2）。活动缩进参考线高亮（`editor.guides.highlightActiveIndentation`）属于 R56（括号对着色与活动缩进参考线高亮）。

### 6. 只读状态界面（UX_SPEC §3.4）

- 只读来源：超大文件、元数据视图、`files.readonlyFromPermissions`、`files.readonlyInclude` / `readonlyExclude`，以及 Rove 的附加规则 `rove.files.extraReadonlyInclude`（默认 `Library/PackageCache/**`）。判定由 R15（工作区） 提供，本 issue 只负责表现。
- 表现：标签与编辑器标题显示锁形图标（与 R17（编辑器组与标签） 协调绘制位置）；尝试输入时在**光标处**弹出提示“无法在只读编辑器中编辑”，并说明原因（如“Unity 包缓存目录，修改会被覆盖”）——这是 `UX_SPEC.md` §6.1 的第 2 级反馈，用 R16（工作台） 的接口，不自己造浮层。
- 命令“在此会话中切换只读”（沿用 VS Code 的 `workbench.action.files.toggleActiveEditorReadonlyInSession`，无默认键位）：可临时取消只读，**超大文件除外**；设置 `editorReadonly` 上下文键。

### 7. 按级别关闭功能（UX_SPEC §3.1）

文档级别由 R17（编辑器组与标签） 的打开文档服务给出，本 issue 据此开关：

| 级别 | 本 issue 关闭的功能 |
|---|---|
| 大文件（≥ 20 MB 或 ≥ 30 万行，`editor.largeFileOptimizations` 为 false 时不启用这一级） | 同名高亮、基于语法的折叠（**保留基于缩进的折叠**）；语法高亮改为后台完成，完成前显示纯文本。缩略图、粘性滚动、括号对着色、语义高亮由各自的 issue 按同一级别关闭 |
| 超大文件（≥ `rove.largeFile.threshold`，默认 64 MB） | 语法高亮、**全部折叠**；只读（软换行与替换由 R57（软换行）、R47（当前文件替换） 关闭） |

被关闭的功能要能在状态栏悬停提示中列出（文案与条目由 R22（超大文件与超长行的界面） 组装，本 issue 提供“当前关闭了哪些功能”的查询接口）。

### 8. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。替换并删除 R02（【接口先行 A2】行走骨架） 中代码视图高亮接入处的 `SKELETON(codeview-hl)` 标记（以及骨架里硬编码主题 / 高亮的相关标记中属于本 issue 的部分），更新 `docs/ARCHITECTURE.md` 对照表。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- tree-sitter 查询与增量解析（R09（tree-sitter 集成））、主题加载与 scope 匹配算法（R10（主题引擎））、语言配置加载（R12（语言配置））、着色黄金测试（R11（着色黄金测试））。
- 括号对着色与活动缩进参考线高亮（R56（括号对着色与活动缩进参考线高亮））、粘性滚动（R54（粘性滚动））、缩略图与概览标尺（R55（缩略图与概览标尺））、语义高亮（R58（C# 语义高亮与不生效预处理代码置灰））、语义级同名高亮与悬停（R40（C# 导航交互））。
- 超大文件的提示条、状态栏条目、行索引进度、超长行截断标记（R22（超大文件与超长行的界面））。
- 只读的判定规则本身（R15（工作区））、保存与只读属性错误处理（R48（保存与外部修改））。

## 验收标准

- [ ] 打开 5000 行的 C# 文件，**首屏就带高亮**，不出现先白后彩的闪烁（用首帧截图 + 样式断言）
- [ ] 带高亮连续滚动时，R19（代码视图 Element） 的帧预算仍然满足：缓存命中每帧 CPU 平均 < 1 ms、帧间隔 p99 ≤ 1.3 倍刷新间隔；全部唯一行时 p99 ≤ 1.5 倍（附测量数据，并与 R19（代码视图 Element） 合并前的数字对比）
- [ ] 深色与浅色两套主题各附一张截图；切换主题后颜色与 `fontStyle` 立即更新且缓存不泄漏
- [ ] 编辑或重新解析后只有变化的行失效（断言失效行集合）；解析未完成时沿用旧样式
- [ ] 括号匹配在 C#、JSON、XML 三种语言上正确，包括光标在括号左右两侧的两种情况
- [ ] 同名高亮：文本匹配正确、`editor.occurrencesHighlight` 设为 `off` 时关闭、`hasWordHighlights` 上下文键取值正确
- [ ] 折叠：语法区间、`#region`、缩进三种来源各有用例；KEYBINDINGS.md 阅读一节的每个折叠命令都有测试；折叠状态随标签切换恢复
- [ ] 缩进参考线与 `editor.renderWhitespace` 的四种取值（`none` / `boundary` / `selection` / `all`）表现正确
- [ ] 只读：`Library/PackageCache/**` 下的文件打开即只读并显示锁形图标；尝试输入时光标处提示带原因；“在此会话中切换只读”对普通只读文件有效、对超大文件无效
- [ ] 大文件级别：20 MB 文件打开后同名高亮与语法折叠已关闭、缩进折叠仍可用、语法高亮在后台完成；超大文件级别：高亮与折叠全部关闭
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过（它断言第一行关键字的 scope 与颜色）；新增高亮、折叠、只读三类集成测试
- [ ] `SKELETON(codeview-hl)` 标记已删除，`docs/ARCHITECTURE.md` 对照表与代码一致

## 风险与提示

- 最容易踩的坑是把 token 应用做成每帧重算。token → 样式区间的转换结果要按行缓存，并且只在解析结果或主题变化时失效，否则 R19（代码视图 Element） 的 1 ms 预算立刻失守。
- `fontStyle` 中的粗体 / 斜体会改变字形宽度，必须进入整形缓存键，否则会出现“颜色对了但列对不上”的错位。
- 折叠会让“屏幕行”与“文档行”不再一一对应。这个映射是 R57（软换行）（软换行）、R54（粘性滚动）、R55（缩略图与概览标尺） 的共同依赖，建议在本 issue 里就把它做成一个独立、可测试的映射结构，而不是散落在绘制代码里。
- 只读提示的措辞按 `UX_SPEC.md` §6 的风格：先说发生了什么，再说可以怎么做，不要加“错误：”前缀。
