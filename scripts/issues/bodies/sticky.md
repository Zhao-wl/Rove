## 背景

粘性滚动是 `FEATURES.md` §3 中少数几个「为阅读而生」的功能：滚到方法体中间时，把所在的命名空间、类型、方法签名钉在编辑器顶部，判定是**照搬**、优先级 P1。

它本身几乎没有新数据：行来自 [[outline]] 产出的符号树（类型由 [[contracts]] 定义，是折叠、大纲、面包屑、粘性滚动、上下文篮子共用的那一份），绘制在 [[codeview]] / [[codeview-hl]] 的代码视图内部。本 issue 的难点只有两个：**不要在滚动的每一帧重新遍历符号树**，以及把「粘性行占了顶部若干行」这件事与折叠、软换行共用的那套「屏幕行 ↔ 文档行」映射对齐。

必读：`FEATURES.md` §3（粘性滚动一行）、§12（D1），`UX_SPEC.md` §3.1（大小分级），`PROJECT_CHARTER.md` §6（每帧 < 1 ms），`docs/compat/SETTINGS.md`（`editor.stickyScroll.*`），`KEYBINDINGS.md`（阅读一节的 3 条 `stickyScrollFocused` 键位、§7 附录「P1 · 阅读」的 2 条、`editor.action.selectEditor`），`THEME_COLORS.md`（Editor widget colors 中的 `editorStickyScroll*`）。

## 目标

### 1. 粘性行的计算

按 `editor.stickyScroll.defaultModel`（默认 `outlineModel`）照搬 VS Code 的三档回退：**大纲模型 → 折叠提供方模型 → 缩进模型**，三档的顺序在任何情况下都保持一致。

- 大纲模型：取 [[outline]] 的符号树，按视口首行所在的符号路径，自顶向下取各级符号的**声明行**。
- 折叠提供方模型：取 [[codeview-hl]] 的折叠区间（语法、`#region`、缩进），用包含首行的区间链的起始行。
- 缩进模型：按缩进层级回溯。
- 遵循 `editor.stickyScroll.maxLineCount`（默认 5）：路径超过上限时**保留最内层**的若干级。
- 计算必须是「按位置二分查找 + 缓存上一次路径」，视口首行仍落在同一个符号内时直接复用，不重新遍历树（与 [[outline]] 的面包屑同一做法）。

### 2. 绘制与交互

- 作为 [[codeview]] 的顶部覆盖区域绘制，复用 [[codeview-hl]] 的高亮与整形缓存，**不要为粘性行另写一套简化渲染**（否则同一行在正文与粘性区的颜色不一致）。
- 颜色键：`editorStickyScroll.background`、`editorStickyScroll.border`、`editorStickyScroll.shadow`、`editorStickyScrollGutter.background`、`editorStickyScrollHover.background`。
- `editor.stickyScroll.enabled`（默认 true）、`editor.stickyScroll.scrollWithEditor`（默认 true，水平滚动时粘性行一起横向滚动）。
- 行号栏与字形边距对齐正文；点击某一行跳转到该行并定位（经 [[codeview]] 的滚动定位接口）；`Ctrl+点击`与悬停行为与正文一致。
- 粘性区占据的高度要计入 [[codeview]] 的视口换算，否则「滚动到某行居中」会被粘性区遮住——用 [[codeview]] 已有的映射结构，不要另算坐标。

### 3. 命令、键位与上下文键

全部沿用 VS Code 的 ID 与默认键（`KEYBINDINGS.md`）：

| 命令 | 默认键 | 说明 |
|---|---|---|
| `editor.action.toggleStickyScroll` | — （§7 附录，P1 · 阅读） | 切换开关，写回 `editor.stickyScroll.enabled` |
| `editor.action.focusStickyScroll` | — （同上） | 焦点进入粘性区，置 `stickyScrollFocused` |
| `editor.action.selectNextStickyScrollLine` | `down`（when `stickyScrollFocused`） | |
| `editor.action.selectPreviousStickyScrollLine` | `up`（when `stickyScrollFocused`） | |
| `editor.action.goToFocusedStickyScrollLine` | `enter`（when `stickyScrollFocused`） | |
| `editor.action.selectEditor` | `escape`（when `stickyScrollFocused`，P2） | 焦点退回编辑器 |

`stickyScrollFocused` 上下文键由本 issue 维护。注意它与 [[lists]] 的 `treestickyScrollFocused` 是**两个不同的键**，不要混用。

### 4. 速览窗口中的粘性滚动

[[peek]] 已经登记了 `peekViewEditorStickyScroll.background` 与 `peekViewEditorStickyScrollGutter.background` 两个颜色键但没有接入。本 issue 把粘性滚动接进速览左侧预览，并使用这两个键；接入方式是向 [[peek]] 的嵌入预览复用同一套计算与绘制代码，不在 [[peek]] 里复制一份。

### 5. 按级别关闭与性能预算

- `UX_SPEC.md` §3.1：**大文件**（≥ 20 MB 或 ≥ 30 万行）与**超大文件**（≥ `rove.largeFile.threshold`）都关闭粘性滚动。级别由 [[groups]] 的打开文档服务给出；关闭原因要能被 [[largefile-ui]] 的状态栏悬停提示查询到（用 [[codeview-hl]] 已有的「当前关闭了哪些功能」接口登记）。
- 预算：连续滚动时粘性滚动带来的每帧额外 CPU **< 100 µs**（`PROJECT_CHARTER.md` §6 的 1 ms 每帧预算里只能占很小一块），并且粘性行变化时不得让正文行失效重排。

### 6. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。装饰、视口事件、滚动定位一律经 [[codeview]] 的接口。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 符号树与折叠区间的产出（[[outline]]、[[codeview-hl]]）；面包屑（[[outline]]）。
- 列表与树的粘性滚动（`workbench.tree.enableStickyScroll`、`list.stickyScroll*`、`treestickyScrollFocused`）：[[lists]]。
- 面板与侧边栏的粘性滚动颜色（`panelStickyScroll.*`、`sideBarStickyScroll.*`）：[[shell]]。
- 缩略图与概览标尺（[[minimap]]）、软换行（[[wrap]]）、括号对着色（[[brackets]]）。
- 速览窗口本身（[[peek]]）；元数据视图中的粘性滚动按同一套实现自动生效，不单独做（[[meta-ui]]）。

## 验收标准

- [ ] 在 5000 行 C# 文件中滚动，粘性区依次显示命名空间 → 类型 → 方法，与 VS Code 的截图逐级一致
- [ ] 三种模型各有用例：C#（大纲）、无符号但有 `#region` 的文件（折叠）、YAML（缩进）；`editor.stickyScroll.defaultModel` 的三个取值都生效
- [ ] `editor.stickyScroll.maxLineCount` 为 1 / 3 / 5 时分别保留最内层的 1 / 3 / 5 级
- [ ] `editor.stickyScroll.scrollWithEditor` 为 true / false 时横向滚动行为正确
- [ ] 点击粘性行跳转并定位；粘性区高度已计入视口换算（跳转后目标行不被粘性区遮挡）
- [ ] 键盘全套：`focusStickyScroll` → `↑` / `↓` → `Enter` → `Esc`，`stickyScrollFocused` 取值正确且不与 `treestickyScrollFocused` 冲突
- [ ] 速览左侧预览中粘性滚动生效，使用 `peekViewEditorStickyScroll*` 两个颜色键
- [ ] 连续滚动时每帧额外 CPU < 100 µs，[[codeview]] 的帧预算仍然满足（附测量数据，基准接入 [[perf]]）
- [ ] 20 MB 文件与超大文件打开后粘性滚动已关闭，且关闭原因可被查询到
- [ ] 深色与浅色主题各一张截图；`editorStickyScroll*` 五个颜色键都取自登记过的键
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（滚动到指定行 → 断言粘性行的行号集合）

## 风险与提示

- 最容易踩的坑是「视口首行一变就重算整条符号路径」。滚动时首行每帧都在变，但符号路径大多数帧是不变的：先用二分查到路径，再缓存「这条路径在哪个行区间内有效」，只有越过边界时才重算。
- 粘性区会改变「屏幕行 ↔ 文档行」的映射，与折叠（[[codeview-hl]]）、软换行（[[wrap]]）、速览嵌入区域（[[peek]]）共用同一套映射。接入时务必复用 [[codeview]] 的映射结构，否则跳转定位、命中测试、缩略图滑块会出现互相矛盾的坐标。
- 粘性行在折叠状态下的语义容易写错：被折叠掉的父级仍然要出现在粘性区（VS Code 如此）。实现前先在 VS Code 里实际试一遍折叠 + 滚动的组合。
- `editor.action.toggleStickyScroll` 要写回设置（经 [[settings]] 的保留格式写入），不要只改内存状态，否则与 VS Code 共用设置文件的语义不一致。
