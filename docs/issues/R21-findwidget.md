# R21 当前文件查找条（正则、大小写、全词、在选区内查找；替换留到 M3）

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 2 中等 | ui | 第 5 批 | R19（代码视图 Element） | R22（超大文件与超长行的界面）、R47（当前文件替换）、R51（多光标与行操作） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

当前文件查找是阅读场景中使用频率最高的功能之一（`FEATURES.md` §4 判定为照搬、P0）。VS Code 的查找条行为细节很多——种子文本、输入即查找、循环、在选区内查找、匹配计数、历史——用户对它的肌肉记忆非常强，任何偏差都会被立刻察觉。

本 issue 在 R19（代码视图 Element） 的装饰与选区接口之上实现这条浮动查找条。**替换功能留到 M3 的 R47（当前文件替换）**：本 issue 不注册替换相关的命令，也不显示替换输入框，但布局与状态机要为它留好位置，避免 M3 时推倒重来。

必读：`FEATURES.md` §4，`docs/compat/KEYBINDINGS.md`（查找替换一节），`SETTINGS.md`（`editor.find.*`），`THEME_COLORS.md`（Editor colors 中的 find 系列、Editor widget colors、Input control）。

## 目标

### 1. 查找条

- 打开与关闭：`actions.find`（`ctrl+f`，when `editorFocus || editorIsOpen`）、`closeFindWidget`（`escape` / `shift+escape`）；设置 `findWidgetVisible`、`findInputFocussed` 上下文键。
- 外观照搬 VS Code：浮动在编辑器右上角，输入框 + 四个开关 + 匹配计数 + 上下箭头 + 关闭按钮；`editor.find.addExtraSpaceOnTop` 为 true 时可滚动到第一行之上。
- 颜色键：`editor.findMatchBackground` / `findMatchForeground` / `findMatchBorder`、`findMatchHighlightBackground` / `findMatchHighlightForeground` / `findMatchHighlightBorder`、`findRangeHighlightBackground` / `findRangeHighlightBorder`、`editorWidget.background` / `border` / `foreground`、`input.*`、`inputOption.activeBackground` / `activeBorder` / `activeForeground` / `hoverBackground`、`inputValidation.*`（正则非法时）。

### 2. 查找选项

- 大小写敏感 `toggleFindCaseSensitive`（`alt+c`）、正则 `toggleFindRegex`（`alt+r`）、全词 `toggleFindWholeWord`（`alt+w`）、在选区内查找 `toggleFindInSelection`（`alt+l`）。
- 正则语义与 VS Code 一致（JavaScript 正则）：多行 `\n`、`\t`、捕获组、`^` / `$` 的行首行尾语义。正则兼容层复用 R12（语言配置） 为 `language-configuration.json` 建立的 JavaScript 正则兼容实现；非法正则时输入框显示校验错误而不是静默无结果。
- 全词判定用 `editor.wordSeparators`。
- `editor.find.autoFindInSelection`（`never` / `always` / `multiline`）决定是否自动打开“在选区内查找”，并绘制 `findRangeHighlight`。

### 3. 导航与匹配计数

- `editor.action.nextMatchFindAction`（`f3`，输入框内 `enter`）、`previousMatchFindAction`（`shift+f3`，输入框内 `shift+enter`）、`editor.action.nextSelectionMatchFindAction`（`ctrl+f3`）、`previousSelectionMatchFindAction`（`ctrl+shift+f3`）。
- 匹配计数显示“第 N 项，共 M 项”“无结果”；超过上限时按 VS Code 显示“超过 M 项结果”。
- `editor.find.loop`（找不到更多时从头开始）、`editor.find.cursorMoveOnType`（输入时光标跟随匹配）、`editor.find.findOnType`（输入即查找）、`editor.find.closeOnResult`。
- 导航到的匹配通过 R19（代码视图 Element） 的滚动定位接口居中显示，并把当前匹配与其他匹配用不同装饰区分。

### 4. 种子文本与历史

- `editor.find.seedSearchStringFromSelection`（默认 `always`）：打开时用选区内容填入；多行选区时按 VS Code 行为改为“在选区内查找”。
- 查找历史：`editor.find.history`（默认 `workspace`），用 R18（通用列表、树与快速输入组件） 的输入历史导航（`history.showPrevious` / `showNext`，`historyNavigationWidgetFocus` 等上下文键）。

### 4.1 用选中内容查找

`actions.findWithSelection`（`KEYBINDINGS.md` §7 附录，P0，无默认键位）：以当前选区为查找文本打开查找条，与 `editor.find.seedSearchStringFromSelection` 共用同一套取词逻辑。

### 5. 大文件与超大文件的接口

查找本身在后台执行、可取消，输入新内容时取消上一次。为 R22（超大文件与超长行的界面） 预留并实现“流式结果 + 进度”的显示能力：匹配计数位置可以显示“已找到 N 处 · 已搜索 37%”，并在超大文件级别隐藏替换按钮。流式查找的引擎与超大文件的具体交互由 R22（超大文件与超长行的界面） 接入，本 issue 提供界面与状态机。

### 6. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。R02（【接口先行 A2】行走骨架） 中没有本 issue 对应的 `SKELETON(...)` 标记，本 issue 只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行，不新增标记。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口（契约变更走单独小 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 替换 UI 与相关命令（R47（当前文件替换），M3）：`editor.action.replaceOne`、`editor.action.replaceAll`、`editor.action.startFindReplaceAction`（`ctrl+h`）、`togglePreserveCase`（`alt+p`）、`editor.find.replaceHistory`、`replaceInputFocussed` 上下文键，本 issue **一律不注册**。
- 全局搜索与搜索视图（R25（全局搜索））、全局替换（R61（全局替换））、搜索编辑器（P2）。
- `editor.action.selectAllMatches`（`alt+enter`）：需要多选区，随 R51（多光标与行操作） 实现。
- 概览标尺上的匹配标记（R55（缩略图与概览标尺））、超大文件的提示条与行索引进度（R22（超大文件与超长行的界面））。
- Markdown 预览内的查找（R29（Markdown 预览），走 WebView2 自带的查找条，颜色键 `simpleFindWidget.sashBorder`）。

## 验收标准

- [ ] `ctrl+f` 打开查找条，种子文本按 `seedSearchStringFromSelection` 填入；`escape` 关闭并清除匹配装饰
- [ ] 四个开关的键位（`alt+c` / `alt+r` / `alt+w` / `alt+l`）与状态持久化（同一窗口内切换文件后保持）与 VS Code 一致
- [ ] `f3` / `shift+f3` / `ctrl+f3` / `ctrl+shift+f3` 行为正确，包括到达末尾时的循环（`editor.find.loop` 为 false 时不循环）
- [ ] 匹配计数正确：空结果、单结果、多结果、超过上限四种情况各有用例
- [ ] 正则用例：`\n` 跨行匹配、捕获组、`^`/`$`、非法正则显示校验错误；与 VS Code 在同一组样例上结果一致（样例写入测试数据）
- [ ] 在选区内查找：匹配限制在选区内，选区范围有 `findRangeHighlight` 装饰，`autoFindInSelection` 三种取值行为正确
- [ ] 查找历史可用 `↑` / `↓` 浏览，按 `workspace` 范围保存
- [ ] 10 万行文件上输入一个字符到匹配计数更新 **< 16 ms**（附测量）；输入过程中不卡顿，上一次查找被取消
- [ ] 查找条可见时 R19（代码视图 Element） 的滚动与帧预算不退化（复跑 R19（代码视图 Element） 的滚动基准，附对比数据）
- [ ] 替换入口完全不存在：命令登记表中没有替换相关命令，界面上没有替换按钮
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增查找条入口的集成测试
- [ ] `docs/ARCHITECTURE.md` 已更新

## 风险与提示

- 匹配装饰要走 R19（代码视图 Element） 的行样式区间接口，并且**只为可见行 + 预留行生成装饰**。在 1 GB 文件上一次性生成几十万个装饰会直接击穿帧预算。
- “当前匹配”和“其他匹配”是两套颜色键（`findMatch*` 与 `findMatchHighlight*`），不要混用。
- 替换虽然不做，但查找模型（匹配列表、当前索引、范围限制）会被 R47（当前文件替换） 和 R22（超大文件与超长行的界面） 复用。把它放在可独立测试的位置，不要和界面状态耦合。
- `ctrl+h`（`editor.action.startFindReplaceAction`）在 KEYBINDINGS.md 中是 P0。本 issue 不注册它，用户按下时不应产生任何反应（未注册的命令本来就会被忽略）；不要做成“打开查找条但没有替换框”的半成品。
