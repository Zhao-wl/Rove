# R57 软换行

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 3 困难 | ui | 第 5 批 | R19（代码视图 Element） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

软换行在 `FEATURES.md` §3 中是**照搬**、P1，说明里只有一句「超大文件模式下禁用」。功能本身不复杂，难在它动的是 R19（代码视图 Element） 最核心的东西：**f64 行号滚动模型**。

R19（代码视图 Element） 的滚动位置是「f64 的首行号 + 行内像素偏移」，这里的「行」是**缓冲行**。软换行一旦开启，一个缓冲行会占据多个**视觉行**，于是：滚动模型、滚动条长度、行号栏、命中测试、选区绘制、光标上下移动、R55（缩略图与概览标尺） 的滑块换算、R54（粘性滚动） 的粘性区高度、R41（速览窗口） 的嵌入区域、折叠的屏幕行映射——全部要改走同一套「视觉行 ↔ 缓冲行」映射。这就是本 issue 难度为 3、而且**不能等到最后才做**的原因。

R20（代码视图） 的风险提示里已经点名：折叠带来的屏幕行映射应当是一个独立、可测试的映射结构。本 issue 的第一件事就是把软换行并进那个结构，而不是在绘制代码里另算一套。

必读：`FEATURES.md` §3（软换行、长行处理两行），`UX_SPEC.md` §3.1（超大文件禁用软换行）、§3.3（超长行与压缩 JSON），`M0_UI_SPIKE_REPORT.md` §4.4、§8（超长行是已知未验证项），`PROJECT_CHARTER.md` §6，`docs/compat/SETTINGS.md`（`editor.wordWrap`、`wordWrapColumn`、`wrappingIndent`、`wrappingStrategy`、`wrapOnEscapedLineFeeds`、`wordWrapIndicator`、`editor.wordBreak`、`editor.stopRenderingLineAfter`），`KEYBINDINGS.md`（`editor.action.toggleWordWrap`）。

## 目标

### 1. 换行点计算

- `editor.wordWrap`（默认 `off`）四个取值：`off`、`on`（按视口宽度）、`wordWrapColumn`（按 `editor.wordWrapColumn`，默认 80）、`bounded`（取视口宽度与 `wordWrapColumn` 的较小者）。
- `editor.wrappingStrategy`（默认 `simple`）：`simple` 按等宽近似切分，`advanced` 用真实字形宽度（DirectWrite 度量）切分。两者都要实现，`advanced` 更慢但对 CJK 与连字更准。
- `editor.wrappingIndent`（默认 `same`）四个取值：`none`、`same`、`indent`、`deepIndent`。
- `editor.wordBreak`（默认 `normal`）：CJK 的断行规则；`editor.wrapOnEscapedLineFeeds`（默认 false）：文本中的 `\n` 字面量是否触发换行。
- `editor.wordWrapIndicator`（默认 false）：在换行列处绘制指示标记。
- 单词边界按 `editor.wordSeparators` 与 R12（语言配置） 的 `wordPattern`（与 R19（代码视图 Element） 的双击选词同一套规则，不要另写）。

### 2. 视觉行 ↔ 缓冲行映射（本 issue 的第二个主要产出）

- 把软换行并入 R20（代码视图） 已有的屏幕行映射结构（折叠 + 嵌入区域），形成**一处**权威映射：缓冲行 ↔ 视觉行区间、视觉行 → (缓冲行, 行内列偏移)、按视觉行数求高度、按高度求视觉行。
- 映射必须是**可按行增量更新**的：编辑一行、视口宽度变化、字体 / 字号 / DPI 变化时，只重算受影响的行；并对外给出「哪些行的视觉行数变了」。
- 每个缓冲行的换行点按「行内容 + 字体 + 字号 + 可用宽度 + `wrappingIndent`」缓存，与 R19（代码视图 Element） 的整形缓存同键同失效时机（不要两套缓存各自失效）。
- 映射结构要有独立单元测试，不依赖 GPUI。

### 3. 滚动模型与界面

- 滚动位置改为「f64 的首**视觉行**号 + 行内像素偏移」；滚动条长度、`editor.scrollBeyondLastLine`、`cursorSurroundingLines` 全部按视觉行计算。水平滚动条在 `wordWrap` 非 `off` 时隐藏（照搬 VS Code）。
- 行号栏：只有缓冲行的**第一条视觉行**显示行号，续行留空（照搬）。当前行高亮覆盖该缓冲行的全部视觉行。
- 命中测试、选区绘制、折叠控件位置按视觉行重算（走 R19（代码视图 Element） 的接口，本 issue 提供映射）。
- 命令 `editor.action.toggleWordWrap`（`alt+z`，P0）：在 `off` 与 `on` 之间切换当前编辑器的软换行；照搬 VS Code 的语义——**只影响当前编辑器会话，不写回 `settings.json`**。
- 下游接入：R55（缩略图与概览标尺） 的滑块与色块、R54（粘性滚动） 的粘性区、R41（速览窗口） 的嵌入区域、R56（括号对着色与活动缩进参考线高亮） 的参考线在软换行开启时都按视觉行工作。本 issue 负责提供映射并在 PR 中逐个确认它们仍然正确。

### 4. 光标移动按视觉行

`cursorUp` / `cursorDown` 与 `cursorHome` / `cursorEnd` / `cursorPageUp` / `cursorPageDown` 及其 `*Select` 变体在软换行下按**视觉行**移动（照搬 VS Code）；`cursorLineStart` / `cursorLineEnd`（§7 附录，P0）按**缓冲行**移动。目标列的「粘性列」语义照搬：连续上下移动时记住期望列，不因续行变短而漂移。

### 5. 超长行与性能

- 与 `editor.stopRenderingLineAfter`（默认 10000）的配合：截断点之后不参与换行计算，界面表现由 R22（超大文件与超长行的界面） 负责。
- `UX_SPEC.md` §3.3 的两个场景必须验证：单行 10 万字符的文件、整个文件只有一行的压缩 JSON。R22（超大文件与超长行的界面） 的「在只读临时标签中打开这一行并开启软换行」入口（`rove.editor.openLongLine`）依赖本 issue 的换行能力。
- 预算：开启软换行后连续滚动，R19（代码视图 Element） 的帧预算不变（缓存命中每帧 CPU 平均 **< 1 ms**、帧间隔 p99 ≤ 1.3 倍刷新间隔）；**视口宽度变化（拖动窗口 / 拖动侧边栏）时重算换行点**是最坏情况，30 万行文件要给出重算耗时并保证界面不卡死（允许分帧完成，先按旧映射绘制）。

### 6. 按级别关闭

`UX_SPEC.md` §3.1：**超大文件**（≥ `rove.largeFile.threshold`）禁用软换行；大文件级别**不禁用**（表中没有列入这一级）。关闭原因登记到 R20（代码视图） 的「当前关闭了哪些功能」接口，供 R22（超大文件与超长行的界面） 的状态栏悬停提示使用。

### 7. 登记与替换

新增命令、设置项、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- f64 滚动模型、整形缓存、命中测试、行号栏的**本体**（R19（代码视图 Element））；折叠区间（R20（代码视图））。
- 超长行的截断标记与「在临时标签中查看这一行」的界面（R22（超大文件与超长行的界面））、「格式化后查看」（R66（跟随文件末尾、格式化后查看））。
- 缩略图、粘性滚动、括号参考线自身的绘制（R55（缩略图与概览标尺）、R54（粘性滚动）、R56（括号对着色与活动缩进参考线高亮））——本 issue 只提供映射并确认它们正确。
- 列选择、多光标在软换行下的行为（R51（多光标与行操作））；Markdown 预览中的换行（R29（Markdown 预览））。
- 诊断性的「异常行终止符」与 Unicode 高亮（P2）。

## 验收标准

- [ ] `editor.wordWrap` 四个取值、`wordWrapColumn`、`wrappingIndent` 四个取值、`wrappingStrategy` 两个取值、`wordBreak`、`wrapOnEscapedLineFeeds`、`wordWrapIndicator` 逐项生效（参数化测试 + 截图）
- [ ] 视觉行映射有独立单元测试（不依赖 GPUI）：缓冲行 ↔ 视觉行双向换算，含空行、Tab、CJK、emoji（ZWJ 与肤色修饰）、连字
- [ ] 软换行 + 折叠组合正确：折叠一个区间后视觉行号连续，展开后恢复（断言映射序列）
- [ ] 行号栏只在首条视觉行显示行号，当前行高亮覆盖全部视觉行
- [ ] `alt+z` 切换当前编辑器的软换行且**不写回设置**；切换后滚动位置保持在同一个缓冲行上
- [ ] `cursorUp` / `cursorDown` / `Home` / `End` / `PageUp` / `PageDown` 及 `*Select` 变体按视觉行移动，`cursorLineStart` / `cursorLineEnd` 按缓冲行移动；连续上下移动时期望列不漂移
- [ ] 单行 10 万字符的文件与单行压缩 JSON 开启软换行后不卡死，首屏 < 1 s
- [ ] 开启软换行后连续滚动满足 R19（代码视图 Element） 的帧预算（附测量数据与关闭软换行时的对比，基准接入 R04（性能基准与预算守护））
- [ ] 视口宽度变化时 30 万行文件的换行重算可分帧完成，界面不卡死（附耗时数据）
- [ ] R55（缩略图与概览标尺）、R54（粘性滚动）、R41（速览窗口） 在软换行开启时坐标正确（各一个集成用例；对应 issue 未合并时在 PR 中说明）
- [ ] 超大文件禁用软换行，关闭原因可被查询到
- [ ] 新增的命令、设置项、界面字符串都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增集成测试（开启软换行 → 断言视觉行数与某个像素点的命中行列）

## 风险与提示

- **把映射做成一处，是本 issue 成败的关键。** 折叠、软换行、速览嵌入区域、粘性区四件事都在改「屏幕上第 N 行是文档里的什么」。只要有两份实现，就一定会出现「点击落在 A 行、高亮画在 B 行」这类难查的错位。合并前先和 R20（代码视图） 的映射结构对齐接口。
- 视口宽度变化是最容易被忽略的最坏情况：拖动窗口边缘会连续触发重算。必须支持「先用旧映射绘制、后台分帧重算」，否则拖窗口会掉帧。
- `wrappingStrategy` 的 `advanced` 需要真实字形度量，开销明显高于 `simple`。两者都要实现，但默认值是 `simple`，不要为了「更准」擅自改默认值（`FEATURES.md` §13 规则 5：默认值差异必须有理由并写成 `rove.*` 附加规则）。
- `editor.action.toggleWordWrap` 不写回设置这一点与「切换主题要写回」正好相反，很容易做反。VS Code 的语义是「临时切换当前编辑器」，照搬。
- 超长行是 `M0_UI_SPIKE_REPORT.md` §8 第 10 条列出的已知未验证项。本 issue 是第一个真正要在超长行上跑换行计算的地方，建议先用 `spikes/gen_testdata.py` 造几种长行模式实测，再定实现方案。
