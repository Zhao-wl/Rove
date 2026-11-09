## 背景

[[codeview-hl]] 已经实现了**括号匹配**（光标两侧的一对括号加框）与**缩进参考线**，但明确把两件事留给了本 issue：按嵌套深度给括号上色（`FEATURES.md` §3「括号对着色」，判定照搬、P1），以及活动缩进参考线的高亮（`editor.guides.highlightActiveIndentation`）。

这两件事之所以同属一个 issue，是因为它们依赖同一份数据：**一棵平衡的括号对树**。VS Code 的实现不是「正则扫一遍」，而是维护一棵可增量更新的括号对 AST（`src/vs/editor/common/model/bracketPairsTextModelPart/`），这样编辑一个字符时只重算受影响的子树。Rove 必须移植这个做法，或者在 PR 中论证一个等价的替代方案——因为括号对着色默认开启（`editor.bracketPairColorization.enabled` 默认 `true`），一旦做成「每帧或每次编辑全文重扫」，[[codeview]] 的 1 ms 每帧预算立刻失守。

必读：`FEATURES.md` §3（括号对着色一行），`UX_SPEC.md` §3.1，`PROJECT_CHARTER.md` §6，`docs/compat/SYNTAX_AND_LANGUAGES.md` §6（`colorizedBracketPairs` 字段：C#、Markdown、XML 定义了，其余回落到 `brackets`），`docs/compat/SETTINGS.md`（`editor.bracketPairColorization.*`、`editor.guides.*`、`editor.language.brackets`、`editor.language.colorizedBracketPairs`），`THEME_COLORS.md`（P1 的 `editorBracketHighlight.*`、`editorBracketPairGuide.*`，P0 的 `editorIndentGuide.activeBackground1`–`6`）。

## 目标

### 1. 括号对的增量计算（本 issue 的地基）

- 括号定义来自 [[langcfg]] 的 `colorizedBracketPairs`，未定义时回落到 `brackets`；用户可用 `editor.language.colorizedBracketPairs` / `editor.language.brackets` 覆盖（语言级设置，经 [[settings]] 读取）。
- **忽略注释与字符串中的括号**：是否在注释 / 字符串内由 [[syntax]] 的 token（scope 列表）判定，不要自己扫引号与注释起止。
- 数据结构：移植 VS Code 的括号对 AST（平衡的括号对树，节点带长度而非绝对偏移，便于编辑后只重建受影响的子树）。接收 [[syntax]] 的编辑与变化行集合后增量更新，并对外给出「哪些行的括号着色变了」。
- 未配对的括号单独标记，用 `editorBracketHighlight.unexpectedBracket.foreground`。
- 如果不采用 AST 方案，PR 中必须给出替代方案的复杂度分析与实测数据，说明为什么它也能满足下面的预算。

### 2. 括号对着色

- `editor.bracketPairColorization.enabled`（默认 true）；颜色按嵌套深度循环取 `editorBracketHighlight.foreground1`–`foreground6`。
- `editor.bracketPairColorization.independentColorPoolPerBracketType`（默认 false）：为 true 时每种括号类型（`()`、`[]`、`{}`、`<>`）各自从 1 开始计数。
- 着色以**行内样式区间**的形式提交给 [[codeview]]（与 [[codeview-hl]] 的 token 样式同一条通道，优先级高于 token 前景色），可缓存、可按行失效。
- 与 `editor.tokenColorCustomizations`、`workbench.colorCustomizations` 的交互照搬 VS Code（着色颜色通过 `workbench.colorCustomizations` 覆盖，见设置项说明）。

### 3. 括号对参考线

- `editor.guides.bracketPairs`（默认 `false`，取值 `true` / `false` / `"active"`）：在括号对之间画竖直参考线，颜色 `editorBracketPairGuide.background1`–`background6`，当前所在的括号对用 `activeBackground1`–`activeBackground6`。
- `editor.guides.bracketPairsHorizontal`（默认 `"active"`，取值 `true` / `false` / `"active"`）：在括号对的首尾行画水平连接线。
- 参考线的深度配色与第 2 节的着色共用同一套深度计数，保证颜色一致。

### 4. 活动缩进参考线高亮与活动括号对

- `editor.guides.highlightActiveIndentation`（默认 true，取值 `true` / `false` / `"always"`）：高亮光标所在层级的缩进参考线，颜色 `editorIndentGuide.activeBackground1`–`activeBackground6`。缩进参考线的**绘制**在 [[codeview-hl]]，本 issue 只提供「哪一条是活动的」并接入高亮颜色。
- `editor.guides.highlightActiveBracketPair`（默认 true）：判定光标所在的括号对并交给 [[codeview-hl]] 用 `editorBracketMatch.*` 绘制。判定改用本 issue 的括号对树（比 [[codeview-hl]] 的就近扫描更准，尤其在嵌套与未配对的情况下），接入后 [[codeview-hl]] 的 `editor.matchBrackets` 行为不变。
- 概览标尺上的 `editorOverviewRuler.bracketMatchForeground` 由 [[minimap]] 绘制，本 issue 只提供位置。

### 5. 按级别关闭与性能预算

- `UX_SPEC.md` §3.1：**大文件**（≥ 20 MB 或 ≥ 30 万行）与**超大文件**都关闭括号对着色（超大文件连语法高亮都没有）。关闭原因登记到 [[codeview-hl]] 的「当前关闭了哪些功能」接口。缩进参考线本身在大文件级别保留，活动高亮随之保留。
- 预算：5000 行 C# 文件中做一次单字符编辑，括号对树增量更新 **< 1 ms**（与 [[syntax]] 的 2 ms 重解析预算分开计）；连续滚动时括号着色带来的每帧额外 CPU **< 100 µs**。
- 括号对极多的文件（压缩 JSON、生成代码）要给出最坏情况数据，并遵循与 `editor.foldingMaximumRegions` 同量级的上限策略，超过上限时**整体关闭着色而不是画错**。

### 6. 登记与替换

新增设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。样式区间一律经 [[codeview]] 的装饰接口提交。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 括号匹配的绘制与 `editor.matchBrackets`、缩进参考线的绘制、`editor.guides.indentation`（[[codeview-hl]]）。
- `language-configuration.json` 的加载与 `colorizedBracketPairs` 字段解析（[[langcfg]]）。
- `editor.action.jumpToBracket`、`editor.action.selectToBracket`、`editor.action.removeBrackets`、`editor.doubleClickSelectsBlock`（[[smartselect]]）。
- 自动闭合与包围括号（[[autoedit]]）；缩略图与概览标尺的绘制（[[minimap]]）。
- 括号对参考线在软换行下的走向：[[wrap]] 合并后按它的视觉行映射自动生效，本 issue 不单独处理。

## 验收标准

- [ ] C#、JSON、XML、Markdown 四种语言各有一份括号对着色的快照测试（断言每个括号的深度与颜色键）
- [ ] 注释与字符串中的括号**不参与**配对与着色，各有用例（含 C# 的逐字字符串与 XML 的 CDATA）
- [ ] 未配对的括号用 `editorBracketHighlight.unexpectedBracket.foreground`，多余的右括号与缺失的右括号各一例
- [ ] `editor.bracketPairColorization.enabled` 为 false 时完全关闭；`independentColorPoolPerBracketType` 为 true 时每种括号独立计数（断言颜色序号）
- [ ] `editor.guides.bracketPairs` 的三个取值与 `bracketPairsHorizontal` 的三个取值表现正确，各附一张截图
- [ ] `editor.guides.highlightActiveIndentation` 的三个取值正确；光标在不同缩进层级时活动参考线切换
- [ ] `editor.guides.highlightActiveBracketPair` 为 false 时不高亮；为 true 时与 [[codeview-hl]] 的 `editorBracketMatch.*` 绘制一致，嵌套与未配对的情况下优于就近扫描（给出对比用例）
- [ ] `colorizedBracketPairs` 未定义的语言（如 YAML）回落到 `brackets`，有用例
- [ ] 5000 行 C# 文件单字符编辑后括号对树增量更新 < 1 ms；连续滚动每帧额外 CPU < 100 µs（附测量数据，基准接入 [[perf]]）
- [ ] 括号对极多的文件（单行压缩 JSON）有最坏情况数据；超过上限时整体关闭着色并在状态栏悬停提示中列出
- [ ] 20 MB 文件与超大文件打开后括号对着色已关闭，关闭原因可被查询到
- [ ] 新增的设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（打开 `.cs` → 断言某一行各括号的着色深度）

## 风险与提示

- 不要低估「增量」这两个字。括号对着色默认开启，而且是**全文**范围的计算：做成每次编辑全文重扫，在 5000 行文件上大概还能忍，在 30 万行文件上会直接卡住。先读 VS Code 的 `bracketPairsTextModelPart`，把「节点存长度、父节点可局部重建」这个核心性质照搬过来。
- 依赖 token 判定注释 / 字符串意味着**括号对树的正确性依赖 [[syntax]] 的解析进度**。解析未完成时按「先显示已有内容」的原则沿用旧的着色结果，不要闪烁（与 [[codeview-hl]] 同一原则）。
- 深度颜色一共只有 6 个，第 7 层回到第 1 层。循环的起点与 VS Code 一致（第一层用 `foreground1`），这是用户一眼能看出来的细节。
- 活动括号对的判定从 [[codeview-hl]] 的就近扫描换成括号对树，属于**行为更正**而不是重构：切换时要保证 `editor.matchBrackets` 的三个取值（`always` / `near` / `never`）语义不变，并补上回归测试。
