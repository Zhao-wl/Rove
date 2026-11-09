# R51 多光标与行操作

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 2 中等 | ui | 第 6 批 | R46（代码视图的编辑交互与输入法）、R48（保存与外部修改）、R21（当前文件查找条） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

多光标与行操作是 VS Code 用户最肌肉记忆的一组操作（`FEATURES.md` §4：两行都是“照搬 / P1”）。它们之所以能放到 M4，是因为 R45（编辑核心模型） 从第一天起就把编辑事务做成了**多区间**的、选区模型做成了**多选区**的——本 issue 不需要改模型，只负责把手势、命令与界面接上来。

两个需要特别对齐的点：

1. `editor.multiCursorModifier`（默认 `alt`）会反过来影响**转到定义的鼠标手势**。VS Code 的规则是“多光标修饰键与跳转修饰键互换”：默认 `Alt+点击`加光标、`Ctrl+点击`跳转；设为 `ctrlCmd` 时 `Ctrl+点击`加光标、`Alt+点击`跳转。R40（C# 导航交互） 已经实现了 `Ctrl+点击`跳转，本 issue 必须与它协商同一个修饰键来源，不能各读各的设置。
2. **列选择是一种模式，不是一种手势**。`editor.columnSelection` 为 true 时，普通的 `shift+方向键` 就变成列选择（见 KEYBINDINGS「编辑基础」中那批 `when` 为 `editorColumnSelection && textInputFocus` 的条目）。

必读：`FEATURES.md` §4，`docs/compat/KEYBINDINGS.md`（多光标 8 条、行操作 6 条、编辑基础中的 `cursorColumnSelect*` 与 `removeSecondaryCursors`，以及 §7 附录中“P1 · 多光标”“P1 · 行操作”），`SETTINGS.md`（`editor.multiCursor*`、`editor.columnSelection`），`THEME_COLORS.md`（`editorMultiCursor.primary.*` / `secondary.*`）。

## 目标

### 1. 添加与移除光标

- 鼠标：`Alt+点击`添加光标（修饰键取自 `editor.multiCursorModifier`，与 R40（C# 导航交互） 共用同一个判定）；按住修饰键拖动添加一段选区。
- 键盘：`editor.action.insertCursorAbove`（`ctrl+alt+up`）、`insertCursorBelow`（`ctrl+alt+down`）、`editor.action.insertCursorAtEndOfEachLineSelected`（`shift+alt+i`）。
- 移除：`removeSecondaryCursors`（`escape` / `shift+escape`，when `editorHasMultipleSelections && textInputFocus`）——这条在 KEYBINDINGS 里属于「编辑基础」且是 P0，但只有存在次光标时才有意义，因此归本 issue；同时维护 `editorHasMultipleSelections` 上下文键。
- 绘制：主光标用 `editorMultiCursor.primary.foreground` / `background`，次光标用 `editorMultiCursor.secondary.*`，走 R19（代码视图 Element） 的装饰接口。
- 遵循 `editor.multiCursorLimit`（默认 10000）与 `editor.multiCursorMergeOverlapping`（默认 true），语义实现在 R45（编辑核心模型），本 issue 只接设置与界面表现。

### 2. 按匹配添加选区

- `editor.action.addSelectionToNextFindMatch`（`ctrl+d`）、`addSelectionToPreviousFindMatch`、`moveSelectionToNextFindMatch`（`ctrl+k ctrl+d`）、`moveSelectionToPreviousFindMatch`。
- `editor.action.selectHighlights`（`ctrl+shift+l`）、`editor.action.changeAll`（`ctrl+f2`，当前文件内所有同名出现一起改）。
- `editor.action.selectAllMatches`（`alt+enter`，when `editorFocus && findWidgetVisible`）——R21（当前文件查找条） 与 R47（当前文件替换） 都明确把它留给本 issue。
- `addCursorsAtSearchResults`（`ctrl+shift+l`，when `fileMatchOrMatchFocus && searchViewletVisible`）：从 R25（全局搜索） 的结果列表把光标放到对应位置。
- 匹配查找一律复用 R21（当前文件查找条） 的查找模型与 `editor.wordSeparators`，不另写一套。

### 3. 列选择

- 手势：`Shift+Alt+拖动`；命令 `cursorColumnSelectUp` / `Down` / `Left` / `Right` / `PageUp` / `PageDown`（`ctrl+shift+alt+方向键`）。
- 模式：`editor.columnSelection` 为 true 时 `shift+方向键` 直接进入列选择（对应 KEYBINDINGS 中 `when` 带 `editorColumnSelection` 的那批条目），并维护 `editorColumnSelection` 上下文键。
- 列选择在 Tab、CJK 全角、超长行上的列对齐按 R19（代码视图 Element） 的命中测试换算，不按字节数。

### 4. 行操作

- 有默认键位：`editor.action.copyLinesUpAction` / `copyLinesDownAction`（`shift+alt+up` / `down`）、`editor.action.moveLinesUpAction` / `moveLinesDownAction`（`alt+up` / `down`）、`editor.action.deleteLines`（`ctrl+shift+k`）、`editor.action.trimTrailingWhitespace`（`ctrl+k ctrl+x`，**变换算法复用 R48（保存与外部修改） 的保存时变换**，不重写）。
- 无默认键位（KEYBINDINGS §7 附录“P1 · 行操作”）：`editor.action.duplicateSelection`、`joinLines`、`sortLinesAscending` / `sortLinesDescending`、`transpose`、`transposeLetters`、`transformToUppercase` / `ToLowercase` / `ToTitlecase` / `ToCamelcase` / `ToPascalcase` / `ToSnakecase` / `ToKebabcase`。**不要自行分配键位。**
- 所有行操作都走 R45（编辑核心模型） 的多区间事务：多光标在多行上执行同一个行操作时合并为**一次**撤销；移动行要保持该行原有的换行符（混合换行符文件不被统一）。

### 5. 多光标下的输入与剪贴板

每个光标独立插入 / 删除，合并为一次事务与一次撤销；粘贴按 `editor.multiCursorPaste`（默认 `spread`：剪贴板行数与光标数相同时按行分配，否则每个光标粘贴全文）；复制多选区产出分段内容（语义在 R45（编辑核心模型），系统剪贴板读写在 R46（代码视图的编辑交互与输入法））。

### 6. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。R02（【接口先行 A2】行走骨架） 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 与 R45（编辑核心模型） 的接口（需要扩展时走契约变更流程单独提 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 多区间事务、多选区模型、合并与上限语义、单词边界（R45（编辑核心模型））。
- 基础输入、输入法、撤销重做命令、剪贴板系统调用、`editor.action.insertLineBefore` / `insertLineAfter`（后两条在 KEYBINDINGS 中属于「编辑基础」P0，已由 R46（代码视图的编辑交互与输入法） 在 M3 实现）。
- **改写输入模式**（`editor.action.toggleOvertypeInsertMode`）：虽然在 KEYBINDINGS 中与多光标同为 P1，但它是 `InputHandler` 与插入符样式的事，已归 R46（代码视图的编辑交互与输入法）。
- 注释切换、自动缩进、自动闭合与包围（R52（注释切换、自动缩进、自动闭合、包围、回车规则））；智能扩选与括号跳转（R53（智能扩选、括号跳转与选择））。
- 查找条与查找模型（R21（当前文件查找条））、当前文件替换（R47（当前文件替换））、全局搜索与全局替换（R25（全局搜索）、R61（全局替换））。
- `Ctrl+点击`转到定义本身（R40（C# 导航交互）），本 issue 只与它协商修饰键来源。
- 编辑器内拖放文本（P2）。

## 验收标准

- [ ] `Alt+点击`添加光标、`Ctrl+Alt+↑↓`、`Shift+Alt+I`、`Escape` 移除次光标，四种操作各有用例；`editorHasMultipleSelections` 上下文键取值正确
- [ ] `editor.multiCursorModifier` 设为 `ctrlCmd` 后，加光标与 R40（C# 导航交互） 的跳转手势**同时**互换（两边行为各断言一次）
- [ ] `Ctrl+D` 连续添加、到达末尾后的行为、`Ctrl+K Ctrl+D` 跳过当前匹配、`Ctrl+Shift+L` 全选同名、`Ctrl+F2` 改全部，与 VS Code 一致
- [ ] `Alt+Enter`（查找条可见时全选匹配）与搜索视图中的 `Ctrl+Shift+L` 各有用例
- [ ] 列选择：拖动与 `ctrl+shift+alt+方向键` 在含 Tab、CJK 全角、长度不一的行上列对齐正确；`editor.columnSelection` 为 true 时 `shift+方向键` 进入列选择
- [ ] 行操作逐条有测试（含在文件首行上移、末行下移、选区跨多行、多光标同时执行）；每次行操作只产生一次撤销
- [ ] 混合换行符文件上执行移动行 / 复制行后，未改动的行尾**一个都没变**（字节级比对）
- [ ] 多光标输入：10 个光标同时输入一个字符后一次撤销全部回退；`editor.multiCursorPaste` 两种取值各有用例
- [ ] `editor.multiCursorLimit` 生效（超过上限时按 VS Code 截断，不卡死）；**1000 个光标**同时输入时 R19（代码视图 Element） 的每帧 CPU 预算不退化（附数据）
- [ ] 只读文档上全部命令被拒绝且无修改
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增多光标与行操作入口的集成测试

## 风险与提示

- 次光标的绘制很容易成为性能陷阱：1000 个光标如果每帧各自提交一次装饰，R19（代码视图 Element） 的 1 ms 预算立刻失守。按“一批光标 = 一组可缓存的装饰”提交，并按行失效。
- `Ctrl+D` 的细节比看起来多：没有选区时先选中当前单词、已有选区时按选区内容找下一处、是否区分大小写取决于查找条的当前状态。建议直接对照 VS Code 的行为逐条写测试。
- 行操作与折叠、软换行的交互留意：移动行时折叠区域应当整体移动（折叠在 R20（代码视图），软换行在 R57（软换行））。本 issue 至少要保证不产生错乱的折叠状态，遇到冲突记录并开 issue。
- `editor.action.trimTrailingWhitespace` 与 `files.trimTrailingWhitespace` 是同一个变换的两个入口。复用 R48（保存与外部修改） 的实现，不要写第二份——两份实现会在“字符串里的尾随空格”上分叉。
- 列选择在超长行（`editor.stopRenderingLineAfter` 之后）上没有渲染结果可依据。按 VS Code 的做法限制在已渲染范围内，并在 PR 中说明取舍。
