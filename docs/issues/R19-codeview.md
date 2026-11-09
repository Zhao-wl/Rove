# R19 代码视图 Element：虚拟化、f64 滚动、整形缓存与预取、行号、只读选区、命中测试、滚动条

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 4 专家 | ui | 第 4 批 | R02（【接口先行 A2】行走骨架）、R07（文本缓冲） | R20（代码视图）、R21（当前文件查找条）、R22（超大文件与超长行的界面）、R30（高 DPI 与多显示器验证与修复）、R41（速览窗口）、R46（代码视图的编辑交互与输入法）、R57（软换行）、R65（上下文篮子）、R71（链接检测与打开） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

代码视图是 Rove 性能最关键的组件，M0 已经给出三条硬结论（`M0_UI_SPIKE_REPORT.md` §4.4、§6.1、§7 约束 3）：

1. **不能用 `uniform_list`**。GPUI 的 `Pixels` 是 f32：1429 万行 × 20 px ≈ 2.86 亿 px，f32 在该量级的分辨率约 32 px，按像素偏移的内建控件会抖动。必须自己写 `Element`，并用 **f64 的行号偏移**作为滚动模型。
2. **整形必须离开绘制关键路径**。可见行全部需要重新整形时，M0 两条路线的帧间隔 p99 都是约 16.8 ms（120 Hz 下每 100 帧丢 1 帧）。需要按“行内容 + 字体”缓存整形结果，并沿滚动方向预取。
3. **每帧 CPU 目标 < 1 ms**。M0 原型实测 823 µs（缓存命中）/ 4279 µs（全部未命中），后者还没叠加语法高亮。

本 issue 把 R02（【接口先行 A2】行走骨架） 的最简代码视图替换为正式实现，并对外提供一套干净的接口，供 R20（代码视图）、R21（当前文件查找条）、R22（超大文件与超长行的界面）、R46（代码视图的编辑交互与输入法）、R65（上下文篮子）、R40（C# 导航交互）、R41（速览窗口）、R57（软换行）、R55（缩略图与概览标尺） 接入。可以参考原型代码 `spikes/ui-gpui/src/`。

必读：`M0_UI_SPIKE_REPORT.md`（尤其 §4.4、§4.5、§6.1、§7），`FEATURES.md` §3，`PROJECT_CHARTER.md` §6，`SETTINGS.md`（`editor.*`），`THEME_COLORS.md`（Editor colors）。

## 目标

### 1. 自定义 Element 与虚拟化

面向 R01（【接口先行 A1】架构与接口契约） 的 `TextSource` trait（由 R07（文本缓冲） / R08（文件大小分级与超大文件） 实现），只布局与绘制可见行加上下各预留若干行。绝不复制全文，绝不按行数做 O(n) 的布局。

### 2. f64 滚动模型

- 滚动位置 = `f64` 首行号 + 行内像素偏移；垂直滚动、水平滚动、平滑滚动（`editor.smoothScrolling`）、滚轮灵敏度（`editor.mouseWheelScrollSensitivity`、`fastScrollSensitivity`、`scrollPredominantAxis`、`inertialScroll`）、`editor.scrollBeyondLastLine`、`scrollBeyondLastColumn`、`cursorSurroundingLines` / `cursorSurroundingLinesStyle`。
- 滚动条：`editor.scrollbar.vertical` / `horizontal`、`verticalScrollbarSize` / `horizontalScrollbarSize`、`scrollbar.scrollByPage`、`ignoreHorizontalScrollbarInContentHeight`；拖动、点击跳转；颜色 `scrollbar.shadow`、`scrollbarSlider.background` / `hoverBackground` / `activeBackground`。概览标尺上的标记留给 R55（缩略图与概览标尺），但滑块与标记的坐标换算接口在本 issue 提供。
- 命令：`scrollLineUp` / `scrollLineDown`（`ctrl+up` / `ctrl+down`）、`scrollPageUp` / `scrollPageDown`（`alt+pageup` / `alt+pagedown`）；跳转到任意行的接口（`workbench.action.gotoLine` 的界面在 R24（快速打开与命令面板），超大文件的等待行为在 R22（超大文件与超长行的界面））。

### 3. 整形缓存与预取

- 缓存键：**行内容 + 字体 + 字号**（含字重、连字设置），LRU 淘汰，容量可配置并计入内存预算。
- 沿滚动方向预取：在后台或上一帧的空闲时间预整形即将进入视口的行。
- **永不阻塞当前帧**：缓存未命中时先用等宽近似排布占位渲染，下一帧替换为正式结果，且替换时不得跳动行高。
- 字体与度量变化（主题切换、字号缩放、DPI 变化）时整体失效；DPI 相关的清空时机与 R30（高 DPI 与多显示器验证与修复） 协调。

### 4. 行号、当前行、只读选区、插入符

- 行号栏（`editor.lineNumbers`、`renderFinalNewline`、`editorLineNumber.foreground` / `activeForeground` / `dimmedForeground`）、字形边距（`editor.glyphMargin`、`editorGutter.background`）、上下内边距（`editor.padding.top` / `bottom`）。
- 当前行高亮：`editor.renderLineHighlight`、`renderLineHighlightOnlyWhenFocus`、`editor.lineHighlightBackground` / `lineHighlightBorder`、`inactiveLineHighlightBackground`。
- **只读选区**：拖选、双击选词、三击选行、`Shift+点击`扩展、`ctrl+a`（`editor.action.selectAll`）、`ctrl+l`（`expandLineSelection`）、`escape`（`cancelSelection`）、插入符移动与扩选（`cursorUp` / `Down` / `Left` / `Right` / `Home` / `End` / `Top` / `Bottom` / `PageUp` / `PageDown` 及其 `*Select` 变体）。双击选词先按 `editor.wordSeparators` 判定，语言级 `wordPattern` 由 R12（语言配置） 提供后在 R20（代码视图） 接入；`editor.doubleClickSelectsBlock` 依赖括号信息，同样留到 R20（代码视图）。
- 插入符：`editor.cursorStyle`、`cursorWidth`、`cursorHeight`、`cursorBlinking`、`cursorSmoothCaretAnimation`、`editorCursor.foreground` / `background`；选区颜色 `editor.selectionBackground` / `selectionForeground` / `inactiveSelectionBackground`、`editor.roundedSelection`。

### 5. 命中测试

像素坐标 ↔ 行列（UTF-8 列）的双向换算，必须正确处理 **CJK 全角、Tab 制表位、emoji（含 ZWJ 序列与肤色修饰）、代理对与组合字符**。M0 已验证 DirectWrite 会把 Tab 当空白处理、基线一致（§4.5），但命中测试仍须自己实现并测试。

### 6. 复制与字体缩放

- `editor.action.clipboardCopyAction`（`ctrl+c` / `ctrl+insert`），遵循 `editor.emptySelectionClipboard`（无选区时复制当前行）；带语法高亮复制是 P2。
- 字体缩放：登记 `editor.action.fontZoomIn` / `fontZoomOut` / `fontZoomReset`（KEYBINDINGS.md 中这三个命令没有默认键位，**不要自行分配**），以及 `editor.mouseWheelZoom`（`Ctrl+滚轮`）。
- 字体设置：`editor.fontFamily`、`fontSize`、`fontWeight`、`fontLigatures`、`fontVariations`、`lineHeight`、`letterSpacing`、`fullwidthCharacterWidth`、`wordBreak`。

### 7. 对外接口（本 issue 的第二个主要产出）

为后续 issue 提供：设置 / 更换文本源；**按行的样式区间（装饰）**与整行级样式（背景、边框、字形边距图标），支持多个提供方叠加并定义优先级（R20（代码视图） 的 token、R21（当前文件查找条） 的匹配、R40（C# 导航交互） 的同名高亮、R25（全局搜索） 的范围高亮）；选区读写与事件；滚动定位（行、行列、居中策略）；视口变化事件（首行、末行、宽高）；行内覆盖物与“行下方嵌入区域”（供 R41（速览窗口） 的速览窗口与 R22（超大文件与超长行的界面） 的截断标记）；单行渲染上限的挂钩（超长行不得拖垮布局，M0 原型用 2048 字节截断，正式实现由 R22（超大文件与超长行的界面） 按 `editor.stopRenderingLineAfter` 接入）。

### 8. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。替换并删除 R02（【接口先行 A2】行走骨架） 的 `SKELETON(codeview)` 标记（高亮接入处的标记由 R20（代码视图） 删除），更新 `docs/ARCHITECTURE.md` 对照表。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 语法高亮与主题 token、括号匹配、折叠、缩进参考线、空白字符、只读状态界面（R20（代码视图））。
- 查找条（R21（当前文件查找条））、超大文件与超长行界面（R22（超大文件与超长行的界面））、编辑与输入法（R46（代码视图的编辑交互与输入法））、软换行（R57（软换行））、缩略图与概览标尺（R55（缩略图与概览标尺））、粘性滚动（R54（粘性滚动））、悬停与跳转（R40（C# 导航交互））、链接检测（R71（链接检测与打开））。
- 多光标（R51（多光标与行操作））、`editor.action.selectAllMatches`（需要多选区，随 R51（多光标与行操作） 实现）。
- 高 DPI 与多显示器的专项验证与修复（R30（高 DPI 与多显示器验证与修复））。

## 验收标准

性能项都在 R04（性能基准与预算守护） 的测试数据与基准框架上测量，口径同 `M0_UI_SPIKE_REPORT.md` §2.5，并把原始数据附在 PR 中：

- [ ] **连续滚动（缓存命中）：每帧 CPU 平均 < 1 ms，帧间隔 p99 ≤ 1.3 倍刷新间隔**
- [ ] **全部唯一行的文件（缓存全未命中）连续滚动：帧间隔 p99 ≤ 1.5 倍刷新间隔**，不再出现 M0 的 16.8 ms
- [ ] 1 GB / 1429 万行文件（配合 R08（文件大小分级与超大文件））从头拖到尾没有精度问题，跳转到任意行准确落位；滚动条位置与行号一致
- [ ] 缓存未命中时有占位渲染，替换为正式结果时行高不跳动（用连续帧截图或断言行高序列）
- [ ] CJK、emoji（含 ZWJ 家庭与肤色修饰）、Tab、混排长行的点击命中准确；用 `spikes/gen_testdata.py` 的 7 种模式逐一断言行列换算
- [ ] 只读选区的四种手势（拖选、双击选词、三击选行、`Shift+点击`）与 `ctrl+a`、`ctrl+l`、`escape` 行为与 VS Code 一致
- [ ] 复制内容与选区完全一致（含 CRLF 保持、`emptySelectionClipboard` 行为）
- [ ] 字体缩放三个命令与 `Ctrl+滚轮` 生效，缩放后整形缓存正确失效且不泄漏
- [ ] 目标清单中的 `editor.*` 设置项逐项生效（参数化测试）
- [ ] 对外接口有文档注释，并为装饰、选区、滚动定位、视口事件四类各写一个“消费方示例测试”
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增代码视图入口的集成测试；性能用例进入 CI（R04（性能基准与预算守护）、R03（工程规范与 CI））
- [ ] `SKELETON(codeview)` 标记已删除，`docs/ARCHITECTURE.md` 对照表与代码一致

## 风险与提示

- 这是 UI 的关键路径，建议最早开工。装饰与视口接口不够用会让 R20（代码视图） 之后的所有 issue 返工。
- M0 的 823 µs 是**原型级用法**的数字（逐行 `shape_line`、每帧把 `ShapedLine` 回写进 view），不是 GPUI 的上限。先按正确的缓存结构实现，再看数据，不要先照抄原型再优化。
- 预算要留给高亮：`M0_UI_SPIKE_REPORT.md` §4.4 指出参考设备（8 核 / 集显）上耗时可能翻倍，而 R20（代码视图） 还要叠加 token 应用成本。本 issue 的 1 ms 预算里要为此留出余量，并在 PR 中说明留了多少。
- 不要把样式区间做成“每帧由提供方重算”，否则高亮一接入就吃掉全部预算。装饰应当是可缓存、可按行失效的。
- 超长行是已知未验证项（`M0_UI_SPIKE_REPORT.md` §8 第 10 条）：本 issue 至少要保证单行 10 万字符时不卡死，界面表现留给 R22（超大文件与超长行的界面）。
