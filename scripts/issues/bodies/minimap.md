## 背景

`FEATURES.md` §3 对缩略图的判定是**改造**（P1）：**只渲染色块，不渲染字符**，以控制性能；概览标尺（滚动条上的标记）是**照搬**（P1），显示搜索结果、选区、同名高亮、问题的位置。

这两件事放在一个 issue 里，是因为它们共用同一套「文档位置 → 右侧窄条像素」的换算，也共用同一批标记来源（[[findwidget]] 的匹配、[[search]] 的范围、[[codeview-hl]] / [[nav-ui]] 的同名高亮、[[structure]] 的问题）。本 issue 是这些标记的**唯一绘制方**：各来源只提供位置集合，不自己画。

缩略图是整个阅读界面里最容易吃掉帧预算的组件（它要为**全文**而不是视口生成图像），所以实现方式比功能清单更重要：按块离屏生成、按行失效、滚动时只做纹理搬运。

必读：`FEATURES.md` §3（缩略图、概览标尺两行），`UX_SPEC.md` §3.1（大小分级）、§6.2，`PROJECT_CHARTER.md` §6，`M0_UI_SPIKE_REPORT.md` §4.4，`docs/compat/SETTINGS.md`（`editor.minimap.*`、`editor.overviewRulerBorder`、`editor.hideCursorInOverviewRuler`），`KEYBINDINGS.md`（§7 附录「P0 · 阅读」中的 `editor.action.toggleMinimap`），`THEME_COLORS.md`（Minimap 14 个键、Editor colors 中的 `editorOverviewRuler.*`）。

## 目标

### 1. 色块渲染

- 每一行按 [[codeview-hl]] 的样式区间生成若干**色块**：同一段连续同色 token 合成一个块，块的宽度按字符数折算，高度按 `editor.minimap.scale`（1 / 2 / 3）。**不渲染字形**。
- `editor.minimap.renderCharacters`：Rove 按 `FEATURES.md` §3 始终只渲染色块，这个设置项在 `SETTINGS.md` 中已判为不支持（静默忽略）。
- 生成按**块**（例如 256 行一块）离屏完成，结果缓存；只有样式变化的行所在的块才重新生成（[[codeview-hl]] 给出变化行集合）。滚动时只搬运已生成的块，不重新生成。
- `editor.minimap.maxColumn`（默认 120）截断超长行；`minimap.foregroundOpacity` 控制整体不透明度；底色 `minimap.background`。

### 2. 布局与开关

`editor.minimap.enabled`（true）、`side`（`right`）、`size`（`proportional` / `fill` / `fit`）、`autohide`（`none` / `mouseover` / `scroll`）、`showSlider`（`mouseover`）。命令 `editor.action.toggleMinimap`（无默认键位，不要自行分配）写回 `editor.minimap.enabled`。

### 3. 滑块与拖动

- 滑块表示当前视口，颜色 `minimapSlider.background` / `hoverBackground` / `activeBackground`。
- 点击缩略图跳转、拖动滑块连续滚动、在缩略图上滚轮滚动：全部经 [[codeview]] 的 **f64 行号滚动模型**与它提供的滑块坐标换算接口完成，**不要自己用像素推算行号**（1429 万行时 f32 像素会失真，见 `M0_UI_SPIKE_REPORT.md` §4.4）。

### 4. 分节标题（sectionHeaders）

- `// MARK:` 风格：按 `editor.minimap.markSectionHeaderRegex`（默认 `\bMARK:\s*(?<separator>-?)\s*(?<label>.*)$`，含命名捕获组 `label` 与 `separator`）在**注释 token 内**匹配；`separator` 命中时在该处画一条分隔线。正则是 JavaScript 语法，用 [[langcfg]] 同一套 `fancy-regex` 方案编译。
- `#region` 风格：命名区域的标题，区域来自 [[codeview-hl]] 的标记折叠区间（`folding.markers`，[[langcfg]] 提供）。
- 开关与排版：`editor.minimap.showMarkSectionHeaders`（true）、`showRegionSectionHeaders`（true）、`sectionHeaderFontSize`（9）、`sectionHeaderLetterSpacing`（1）。分节标题是缩略图中**唯一绘制文字**的地方。

### 5. 缩略图上的标记

| 来源 | 颜色键 |
|---|---|
| 查找匹配（[[findwidget]]） | `minimap.findMatchHighlight` |
| 选区（[[codeview]]） | `minimap.selectionHighlight` |
| 选区相似匹配与同名高亮（[[codeview-hl]] / [[nav-ui]]） | `minimap.selectionOccurrenceHighlight` |
| 问题（[[structure]]） | `minimap.errorHighlight`、`minimap.warningHighlight`、`minimap.infoHighlight` |

### 6. 概览标尺

- 窄条本体：`editor.overviewRulerBorder`（true）、`editorOverviewRuler.background`、`editorOverviewRuler.border`；光标标记遵循 `editor.hideCursorInOverviewRuler`（false）。
- 标记与来源：`findMatchForeground`（[[findwidget]]）、`rangeHighlightForeground`（[[search]] 的范围高亮）、`selectionHighlightForeground`、`wordHighlightForeground` / `wordHighlightStrongForeground` / `wordHighlightTextForeground`（[[codeview-hl]] 与 [[nav-ui]] 提供位置集合）、`errorForeground` / `warningForeground` / `infoForeground`（[[structure]]）、`bracketMatchForeground`（[[codeview-hl]] 的匹配括号）。
- **Git 相关的标记不做**（`FEATURES.md` §10 不做版本控制）：`minimapGutter.addedBackground` / `modifiedBackground` / `deletedBackground` 在 `THEME_COLORS.md` 中已判为不支持，不登记、不绘制。
- 标记要能叠加：同一行有多种标记时按 VS Code 的通道分配（左 / 中 / 右三栏）与优先级绘制。

### 7. 按级别关闭与性能预算

- `UX_SPEC.md` §3.1：**大文件**（≥ 20 MB 或 ≥ 30 万行）与**超大文件**都关闭缩略图；关闭原因登记到 [[codeview-hl]] 的「当前关闭了哪些功能」接口，供 [[largefile-ui]] 的状态栏悬停提示使用。概览标尺在大文件级别**保留**（它只画标记，成本与标记数相关而非行数）。
- 预算：连续滚动时缩略图带来的每帧额外 CPU **< 200 µs**（缓存命中），并给出块生成的离线耗时；30 万行文件首次生成全部块的总耗时与内存占用要在 PR 中列出，块缓存计入 [[codeview]] 的内存预算。

### 8. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。装饰、视口事件、滚动定位与滑块换算一律经 [[codeview]] 的接口。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 标记位置的**计算**：查找匹配（[[findwidget]]）、搜索范围（[[search]]）、同名高亮（[[codeview-hl]]、[[nav-ui]]）、问题（[[structure]]）、括号匹配（[[codeview-hl]]）。本 issue 只负责绘制。
- 滚动条本体与 f64 滚动模型（[[codeview]]）；粘性滚动（[[sticky]]）；软换行（[[wrap]]）——软换行开启后缩略图按**视觉行**绘制，映射由 [[wrap]] 提供。
- Git 装饰、内联聊天相关的缩略图颜色键（`minimap.chatEditHighlight`、`editorMinimap.inlineChatInserted`，`THEME_COLORS.md` 判定为不支持）。
- 问题面板、波浪线与 `editor.action.marker.*`（[[structure]]）。

## 验收标准

- [ ] 打开 5000 行 C# 文件，缩略图显示色块且**不含字形**；深色与浅色主题各一张截图，颜色与 [[codeview-hl]] 的 token 颜色一致
- [ ] `editor.minimap.enabled` / `side` / `size`（三个取值）/ `scale`（1、2、3）/ `maxColumn` / `autohide`（三个取值）/ `showSlider`（两个取值）逐项生效（参数化测试）
- [ ] `editor.action.toggleMinimap` 生效并写回设置；无默认键位
- [ ] 滑块：拖动连续滚动、点击跳转、滚轮滚动；在 1429 万行文件上从头拖到尾落位准确且与行号一致
- [ ] 分节标题：`// MARK: - Foo` 与 `#region Foo` 各有用例；`markSectionHeaderRegex` 改为自定义正则后生效；两个开关为 false 时不显示
- [ ] 缩略图标记四类（查找、选区、相似匹配、问题）各有用例，颜色取自对应颜色键
- [ ] 概览标尺：三栏通道分配与叠加正确；`editor.overviewRulerBorder`、`editor.hideCursorInOverviewRuler` 生效；Git 三个键登记但不绘制（PR 中说明）
- [ ] 连续滚动时缩略图每帧额外 CPU < 200 µs（缓存命中），[[codeview]] 的帧预算仍满足；30 万行文件的块生成总耗时与内存占用已记录（附测量数据，基准接入 [[perf]]）
- [ ] 样式变化时只重新生成受影响的块（断言重建的块索引集合）；主题切换后全部块重建且缓存不泄漏
- [ ] 20 MB 文件与超大文件打开后缩略图已关闭、概览标尺仍可用，关闭原因可被查询到
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（打开文件 → 断言块数量与某一行的色块颜色、拖动滑块后首行号）

## 风险与提示

- **最大的风险是把缩略图做成「每帧为全文重画」**。它必须是「按块离屏生成 + 缓存 + 按行失效」，滚动帧里只做纹理搬运。先把这个结构搭对，再看数字；`M0_UI_SPIKE_REPORT.md` §4.4 提醒参考设备（8 核 / 集显）上耗时可能翻倍，而本 issue 的预算只有 200 µs。
- 不要把缩略图实现成渲染字符：这直接违反 `FEATURES.md` §3 的改造判定，也会破坏每帧 CPU 预算。
- 滑块坐标千万不要自己用像素算。`M0_UI_SPIKE_REPORT.md` §4.4 的结论是 GPUI 的 `Pixels` 是 f32，在 1429 万行量级上分辨率约 32 px——必须走 [[codeview]] 的 f64 换算接口。
- 概览标尺的标记来源会随后续 issue 陆续接入（[[structure]] 在 M4、[[nav-ui]] 在 M2）。把「标记提供方」做成一个登记点并自带空实现，这样来源没合并时标尺照常工作，合并后不改本 issue 的代码。
- `markSectionHeaderRegex` 必须只在**注释 token 内**匹配，否则字符串里的 `MARK:` 会变成分节标题。token 信息来自 [[codeview-hl]]，不要自己扫文本。
