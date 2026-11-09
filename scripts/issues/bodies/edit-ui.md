## 背景

这是 Rove 第一次让用户真正**打字**。两个事实决定了本 issue 的难度评级：

1. **输入法只验证了主路径**。`M0_UI_SPIKE_REPORT.md` §5 用微软拼音实机驱动验证了“组合态内嵌渲染 → 候选框跟随插入符 → 提交正确”，但 §8 第 6 条把深水区如实列为未验证：**整句长串输入、组合中途点击别处、组合中途切换输入法、候选框在屏幕边缘翻转、全角半角与 Shift 临时英文、第三方输入法（搜狗 / 微信）**。GPUI 在 Windows 上只在 `WM_IME_STARTCOMPOSITION` 定位一次候选框的缺陷由 [[gpui]] 修复，本 issue 在**真实代码视图**里（滚动过的长文档、CJK 混排行、靠近窗口边缘的插入符）复验同一组场景——[[gpui]] 验证的是框架，本 issue 验证的是产品。
2. **按键到上屏 < 8 ms**（`PROJECT_CHARTER.md` §6）。这是全项目最紧的延迟预算，而输入还要触发 [[syntax]] 的增量重解析与 [[codeview-hl]] 的高亮刷新，两者都不能进入这一帧的关键路径。

本 issue 在 [[codeview]] 的 Element 上实现 GPUI 的 `InputHandler`，把 [[edit-core]] 的事务接到命令与键位上。可以参考被取代的草稿 `out/issues-superseded/05-edit-docview.md` 的「代码视图编辑交互与输入法」一节（其中的多光标、行操作、注释切换在当前拆分中已划给 [[multicursor]]、[[autoedit]]）。

必读：`M0_UI_SPIKE_REPORT.md` §5、§8，`PROJECT_CHARTER.md` §6，`FEATURES.md` §4，`UX_SPEC.md` §3.4（只读）、§6.1，`docs/compat/KEYBINDINGS.md`（编辑基础 82 条 + §7 附录中“P0 · 编辑基础”31 条），`SETTINGS.md`（`editor.overtypeCursorStyle` / `overtypeOnPaste` / `useTabStops` / `stickyTabStops` / `trimAutoWhitespace` / `cursorSurroundingLines`），`THEME_COLORS.md`（`editor.compositionBorder`、`editorCursor.*`）。

## 目标

### 1. `InputHandler` 与输入法

- 在 [[codeview]] 的 Element 上实现 GPUI 的 `EntityInputHandler`：`replace_and_mark_text_in_range`（组合态）、`replace_text_in_range`（提交）、`marked_text_range`、`bounds_for_range`、`selected_text_range`、`character_index_for_point`、`text_for_range`、`unmark_text`。**偏移量单位是 UTF-16**（`M0_UI_SPIKE_REPORT.md` §5.4 的 API 对比表），转换走 [[contracts]] 的位置类型，不在这里手写。
- 组合态**内嵌渲染**在正文中：带下划线，组合串内的光标按 `GCS_CURSORPOS` 定位，边框用 `editor.compositionBorder`。组合期间不产生编辑事务，不触发 [[syntax]] 重解析，不进入撤销栈。
- 每次插入符区域变化都调用 `update_ime_position`（依赖 [[gpui]] 的修复），让候选框跟随；提交时作为**一次**编辑事务写入。
- 取消（`Esc`、失焦、点击别处、切换输入法）必须清干净组合串，不留残片、不重复提交。

### 2. 键盘输入与编辑基础命令（KEYBINDINGS「编辑基础」）

登记并实现下列命令，行为取自 [[edit-core]]，本 issue 只做绑定与反馈：

- 字符输入 `default:type`、`lineBreakInsert`、`tab` / `outdent`、`editor.action.indentLines` / `outdentLines`（`ctrl+oem_6` / `ctrl+oem_4`），遵循 `editor.useTabStops`、`editor.stickyTabStops`、`editor.trimAutoWhitespace`。
- 删除：`deleteLeft`、`deleteRight`、`deleteWordLeft` / `deleteWordRight`、`deleteWordStart*` / `deleteWordEnd*` / `deleteWordPart*`、`deleteAllLeft` / `deleteAllRight`。
- 单词导航：`cursorWordLeft` / `cursorWordRight` / `cursorWordStart*` / `cursorWordEnd*` / `cursorWordPart*` 及其 `*Select` 变体，以及 `cursorLineStart` / `cursorLineEnd` 及其 `*Select` 变体。**基础光标移动与选区（`cursorUp` 等）已由 [[codeview]] 实现，本 issue 不重复登记**；带 `*Select` 的单词变体属于本 issue。
- 撤销重做：`undo`（`ctrl+z`）、`redo`（`ctrl+y` / `ctrl+shift+z`）、`cursorUndo`（`ctrl+u`）、`cursorRedo`。
- 选区锚点：`editor.action.setSelectionAnchor`（`ctrl+k ctrl+b`）、`selectFromAnchorToCursor`（`ctrl+k ctrl+k`）、`goToSelectionAnchor`、`cancelSelectionAnchor`（`escape`），以及 `selectionAnchorSet` 上下文键。
- 插入行：`editor.action.insertLineBefore`（`ctrl+shift+enter`）、`insertLineAfter`（`ctrl+enter`）——KEYBINDINGS 把这两条列在「编辑基础」且是 P0，因此归本 issue，不随 [[multicursor]] 的行操作延后到 M4。
- 上下文键：`editorTextFocus`、`textInputFocus`、`editorReadonly`、`editorHasSelection`。

### 3. 剪贴板

`editor.action.clipboardCutAction`（`ctrl+x` / `shift+delete`）、`clipboardPasteAction`（`ctrl+v` / `shift+insert`）接系统剪贴板；复制（`ctrl+c`）已由 [[codeview]] 实现，本 issue 只补剪切与粘贴。语义（`editor.emptySelectionClipboard` 的整行行为）来自 [[edit-core]]；粘贴遵循 `editor.overtypeOnPaste`；粘贴 CRLF 内容到 LF 文档时按文档主换行符归一（混合文档保持混合）。

### 4. 改写输入模式（overtype）

`editor.action.toggleOvertypeInsertMode`（`insert`）：命令在 KEYBINDINGS 中是 P1，但它的两个设置项 `editor.overtypeCursorStyle`（默认 `block`）与 `editor.overtypeOnPaste`（默认 `true`）都是 **P0**，而且它完全是 `InputHandler` + 插入符样式的事——与多光标、行操作无关。**因此归本 issue，不归 [[multicursor]]**：改写模式与输入法提交的交互（组合串提交时按整串覆盖，而不是逐字符覆盖）只有在这里才能测。

### 5. 只读执行与错误反馈

只读文档（超大文件、元数据视图、`files.readonly*`、`rove.files.extraReadonlyInclude`）上的一切编辑命令直接拒绝，并复用 [[codeview-hl]] 已实现的“光标处提示 + 原因”（`UX_SPEC.md` §3.4、§6.1 第 2 级），**不自己造浮层、不弹通知**。输入法在只读文档上不进入组合态。

### 6. 编辑后的联动

一次事务提交后：把 [[edit-core]] 的 `InputEdit` 交给 [[syntax]] 做增量重解析，并按返回的“变更行区间”让 [[codeview-hl]] 的高亮装饰按行失效。**重解析与高亮刷新不得阻塞当前帧**：先用旧 token 画出新文本，下一帧或解析完成后替换，替换时不得闪烁、不得改变行高。

### 7. 插入符与自动滚动

插入符闪烁（`editor.cursorBlinking`，组合期间不闪烁）、输入时自动把插入符滚进视口（遵循 `editor.cursorSurroundingLines` / `cursorSurroundingLinesStyle`，走 [[codeview]] 的滚动定位接口）、窗口失焦时按 VS Code 停止闪烁。

### 8. 状态栏的缩进转换写入动作

[[statusbar]]（M1）把“将缩进转换为空格 / 制表符”推迟到了 [[edit-core]] 合并之后（见该 issue 第 24 行）。本 issue 登记 `editor.action.indentationToSpaces` / `editor.action.indentationToTabs`，并把它们补进 `changeEditorIndentation` 选择器，算法来自 [[edit-core]] 第 6 节。换行符统一（“统一为 LF / CRLF”）由 [[save]] 负责登记。

### 9. 性能预算与测量方法

**按键到上屏 < 8 ms**（p99）。PR 中必须写明测量方法：建议注入按键（沿用 `spikes/ime_test.ps1` 的前台窗口校验思路）并记录“按键事件进入 → 包含新字符的帧提交”的时间戳，在 10 万行 C# 文件、插入符位于文件中部、高亮已启用的条件下取样 ≥ 300 次。基准接入 [[perf]]，预算写进 `perf/budgets.toml`。

### 10. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。[[skeleton]] 中没有本 issue 对应的 `SKELETON(...)` 标记，本 issue 只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 多光标手势、列选择、行操作、`removeSecondaryCursors`、`editor.action.selectAllMatches`（[[multicursor]]，M4）。
- 注释切换、自动缩进与 `onEnterRules`、自动闭合与包围（[[autoedit]]）；智能扩选与括号跳转（[[smartselect]]）。
- 替换界面（[[replace]]）、保存与外部修改（[[save]]）、热退出（[[hotexit]]）。
- 只读判定规则（[[workspace]]）与只读界面表现（[[codeview-hl]]）；软换行（[[wrap]]）。
- GPUI 自身的 `update_ime_position` 修复与框架级输入法验证（[[gpui]]）；高 DPI 专项验证（[[dpi]]）。
- 格式化（`editor.action.formatDocument`，JSON/XML 的实现在 [[structure]]）。

## 验收标准

- [ ] **输入法场景逐项通过并附截图**：① 短词（`ni'hao`）；② **≥ 20 字整句**，候选框全程跟随插入符；③ 组合中途用鼠标点到另一行（提交或取消，无残留组合串）；④ 组合中途切换输入法；⑤ 插入符靠近屏幕右 / 下边缘时候选框翻转不被裁切；⑥ 全角 / 半角符号与 Shift 临时英文；⑦ 微软拼音必测，搜狗或微信输入法条件允许时测，**未测就在 PR 中写未测**
- [ ] 组合态内嵌渲染带下划线，组合期间不进撤销栈、不触发重解析；提交后一次 `ctrl+z` 撤销整串
- [ ] **按键到上屏 p99 < 8 ms**（附测量方法、采样数与原始数据）；输入过程中 [[codeview]] 的每帧 CPU 预算不退化（复跑其滚动基准并对比）
- [ ] 目标清单中的「编辑基础」P0 命令逐条有测试（参数化），`when` 条件与 KEYBINDINGS 一致
- [ ] 单词导航与删除在 CJK、驼峰、下划线、emoji（含 ZWJ）混排行上与 VS Code 一致（复用 [[edit-core]] 的对拍用例，在界面层再验一次光标落点）
- [ ] 改写模式：`insert` 切换后插入符变为 `editor.overtypeCursorStyle` 指定的样式，输入覆盖后一个字符，行尾不吞换行；粘贴行为遵循 `editor.overtypeOnPaste`；输入法提交在改写模式下按整串覆盖
- [ ] 剪切 / 粘贴与选区一致；跨编码、跨换行符粘贴后混合换行符文档仍为混合（字节级比对）
- [ ] 只读文档（`Library/PackageCache/**` 下的文件）上输入不产生任何修改，提示来自 [[codeview-hl]] 的光标处提示且带原因；输入法不进入组合态
- [ ] 编辑后高亮在视觉上无闪烁、行高不跳动；[[syntax]] 收到的 `InputEdit` 与实际改动一致（断言）
- [ ] 状态栏“将缩进转换为空格 / 制表符”可用，转换结果可一次撤销
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（输入一个字符 → 断言缓冲内容、变更通知、插入符位置、可撤销）

## 风险与提示

- 输入法测试**必须实机驱动**，不要用合成事件冒充。沿用 `spikes/ime_test.ps1` 的安全阀（注入按键前校验前台窗口句柄，不相等立即中止），并在结束时把用户原本的输入法模式还原（`M0_UI_SPIKE_REPORT.md` §5.1）。
- 8 ms 预算里，[[codeview]] 的绘制已经占掉约 1 ms，剩下的要分给事务、重解析调度与高亮失效。**千万不要在输入的同一帧里同步调用 tree-sitter**——哪怕增量解析只要几百微秒，长行或深层嵌套时也会突刺。
- 组合态与撤销栈的边界是经典 bug 源：组合中途按 `ctrl+z` 应当由输入法消费或取消组合，而不是撤销上一次提交。明确这条行为并写测试。
- 单词导航命令在 M1 的只读阅读器里**并不存在**（[[codeview]] 只实现了基础光标移动），所以 M3 是用户第一次能用 `Ctrl+←/→`。验收时别只在编辑场景测，只读文档上也要能用（这些命令的 `when` 是 `textInputFocus`，不要求 `!editorReadonly`）。
- 措辞按 `UX_SPEC.md` §6 的风格：先说发生了什么，再说可以怎么做，不要“错误：”前缀。
