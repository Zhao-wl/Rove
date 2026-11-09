# R52 注释切换、自动缩进、自动闭合、包围、回车规则

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 3 困难 | ui | 第 6 批 | R46（代码视图的编辑交互与输入法）、R12（语言配置） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

这是“输入时编辑器替你做的事”那一组功能：按 `Ctrl+/` 切注释、回车后自动缩进、在 `///` 注释里回车自动续写、输入 `(` 自动补 `)`、选中文本后输入 `"` 自动包围。`FEATURES.md` §4 把它们全部判为“照搬 / P1”，数据来源是 VS Code 内置扩展里的 `language-configuration.json`（`docs/compat/SYNTAX_AND_LANGUAGES.md` §6），由 R12（语言配置） 加载并提供。

难点不在“接上数据”，而在**把 VS Code 的缩进规则引擎原样移植过来**。那套引擎（`indentationRules` 的 `increaseIndentPattern` / `decreaseIndentPattern` / `indentNextLinePattern` / `unIndentedLinePattern`，加上 `onEnterRules` 的 `beforeText` / `afterText` / `previousLineText` / `action`）行为相当细，靠猜一定会偏。`FEATURES.md` §11 明确把“缩进规则等算法”列为可直接移植的 VS Code 资产——连同它的单元测试一起移植。

必读：`FEATURES.md` §4、§11，`docs/compat/SYNTAX_AND_LANGUAGES.md` §6（字段表与“JavaScript 正则”的实现注意），`docs/compat/KEYBINDINGS.md`（注释 4 条），`SETTINGS.md`（`editor.autoClosing*`、`editor.autoIndent*`、`editor.autoSurround`、`editor.comments.*`、`editor.tabSize` / `indentSize` / `insertSpaces` / `detectIndentation`、`editor.trimAutoWhitespace`）。

## 目标

### 1. 注释切换

- 命令：`editor.action.commentLine`（`ctrl+oem_2`，即 `Ctrl+/`）、`editor.action.addCommentLine`（`ctrl+k ctrl+c`）、`editor.action.removeCommentLine`（`ctrl+k ctrl+u`）、`editor.action.blockComment`（`shift+alt+a`）。
- 数据来自 R12（语言配置） 的 `comments.lineComment` / `blockComment`；没有行注释的语言（如 JSON 的某些配置）按 VS Code 退化为块注释。
- 行为照搬：多行选区中**只要有一行未注释就全部注释**，否则全部取消注释；注释前缀插在各行公共缩进处而不是行首；遵循 `editor.comments.insertSpace`（默认 true）与 `editor.comments.ignoreEmptyLines`（默认 true）。
- 多光标 / 多选区下合并为一次事务与一次撤销（模型来自 R45（编辑核心模型），手势来自 R51（多光标与行操作））。

### 2. 自动缩进（移植 VS Code 的缩进规则引擎）

- 移植 `indentationRules` 的四条正则语义（`increaseIndentPattern`、`decreaseIndentPattern`、`indentNextLinePattern`、`unIndentedLinePattern`）与 VS Code 的推导过程；正则走 R12（语言配置） 的 JavaScript 正则兼容层（`fancy-regex`），不自己再写一套。
- 触发时机按 `editor.autoIndent`（默认 `full`，取值 `none` / `keep` / `brackets` / `advanced` / `full` 逐一实现）：回车、输入会触发 `decreaseIndentPattern` 的字符（如 `}`）、`editor.action.reindentlines` 之外的缩进命令。
- 粘贴时的缩进：`editor.autoIndentOnPaste`（默认 false）与 `editor.autoIndentOnPasteWithinString`（默认 true）。
- 缩进单位取 `editor.insertSpaces` / `tabSize` / `indentSize`，**以文档当前的检测结果为准**（检测由 R27（状态栏条目） 在 M1 实现并显示，本 issue 只消费它的结果，不自己检测）。
- `editor.trimAutoWhitespace`（默认 true）：自动插入的尾随空白在光标离开该行时清除。
- **把 VS Code 的缩进相关单元测试一起移植**，逐例断言，不自己重新发明用例。

### 3. `onEnterRules`

实现 `beforeText` / `afterText` / `previousLineText` 三个条件与 `action` 的三种缩进动作（`indent` / `indentOutdent` / `none`）加 `appendText` / `removeText`。这 8 份语言配置里 C# 与 JSON 定义了该字段，典型效果是：

- 在 C# 的 `///` 文档注释行里按回车自动续写 `/// `；
- 在 `/* */` 块注释里按回车自动补 ` * `；
- 在 `{` 与 `}` 之间按回车时，`}` 下移一行并把光标放在缩进后的空行上（`indentOutdent`）。

### 4. 自动闭合与自动包围

- 自动闭合：数据来自 R12（语言配置） 的 `autoClosingPairs`（含 `notIn`，例如字符串与注释中不自动闭合）与 `autoCloseBefore`（这 8 份配置都没有定义，取 VS Code 默认值）。设置：`editor.autoClosingBrackets`、`editor.autoClosingQuotes`、`editor.autoClosingComments`（三者默认 `languageDefined`）、`editor.autoClosingDelete`（删除左括号时一并删除相邻右括号）、`editor.autoClosingOvertype`（输入右括号时越过已自动插入的那个而不是再插一个）。
- 自动包围：选中文本后输入 `(`、`"` 等时用 `surroundingPairs` 包围选区，遵循 `editor.autoSurround`（默认 `languageDefined`，取值 `quotes` / `brackets` / `never` 逐一实现）；多选区时每段各自包围。
- **只有“自动插入的”那个闭合符才参与 overtype 与 autoClosingDelete**。需要跟踪“这对括号是编辑器补的”这一状态，光标离开或发生其他编辑后状态失效——这是 VS Code 行为的关键细节。
- 与改写输入模式（R46（代码视图的编辑交互与输入法） 的 `toggleOvertypeInsertMode`）同时开启时，自动闭合不插入闭合符。

### 5. 登记与替换

新增命令、设置项、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死；语言配置一律经 R12（语言配置） 的接口读取。R02（【接口先行 A2】行走骨架） 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 与 R12（语言配置） 的接口（需要扩展时走契约变更流程单独提 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 语言配置文件的加载、字段解析与 JavaScript 正则兼容层本身（R12（语言配置））。
- 编辑事务、撤销分组、单词边界（R45（编辑核心模型））；基础输入、`tab` / `outdent` / `indentLines` / `outdentLines`、改写输入模式（R46（代码视图的编辑交互与输入法））。
- **缩进风格检测与状态栏的缩进项**（R27（状态栏条目），M1）；**“将缩进转换为空格 / 制表符”**（`editor.action.indentationToSpaces` / `indentationToTabs`，已由 R46（代码视图的编辑交互与输入法） 在 M3 登记并接入状态栏选择器）；**换行符统一**（R48（保存与外部修改））。
- `editor.action.reindentlines` / `reindentselectedlines`：`docs/compat/KEYBINDINGS.md` 没有列出这两个命令（既不在需要实现的键位里，也不在 §7 附录中），因此本 issue **不注册**它们。
- 括号匹配与括号对着色（R20（代码视图）、R56（括号对着色与活动缩进参考线高亮））、括号跳转与选择（R53（智能扩选、括号跳转与选择））。
- 多光标手势与行操作（R51（多光标与行操作））；联动编辑（linkedEditing，P2）；格式化（R60（JSON/YAML/XML））。
- 补全、代码片段、参数提示（`FEATURES.md` §4：**不做**）。

## 验收标准

- [ ] 注释切换：C#（`//`、`/* */`）、JSON、YAML（`#`）、XML（`<!-- -->`）、Markdown 五种语言各有用例；混合注释状态的多行选区按“有一行未注释就全部注释”处理
- [ ] `editor.comments.insertSpace` / `ignoreEmptyLines` 两种取值各有用例；注释前缀插在公共缩进处（含首行缩进更深的情况）
- [ ] **VS Code 的缩进规则单元测试全部移植并通过**（PR 中列出用例来源文件与条数）
- [ ] `editor.autoIndent` 的五种取值逐一有行为差异的用例
- [ ] `onEnterRules`：C# 的 `///` 续写、`/* */` 中补 ` * `、`{` 与 `}` 之间回车的 `indentOutdent`，三条各有用例
- [ ] 自动闭合：`notIn`（字符串 / 注释中不闭合）、`autoCloseBefore`、`autoClosingOvertype`、`autoClosingDelete` 各有用例；`editor.autoClosing*` 的 `never` / `always` / `languageDefined` 三种取值生效
- [ ] “只有自动插入的闭合符参与 overtype / delete”：手工输入的 `)` 不被越过（这是最容易做错的一条，必须单独断言）
- [ ] 自动包围：`editor.autoSurround` 三种取值各有用例；多选区时每段各自包围
- [ ] 所有自动行为在一次按键内只产生**一个**撤销单元（例如回车 + 自动缩进 + `///` 续写合并为一次）
- [ ] 只读文档上全部命令被拒绝且无修改；超大文件上不触发任何自动编辑
- [ ] 新增的命令、设置项、界面字符串都经由登记机制，无硬编码
- [ ] 输入时的自动行为不使按键到上屏超出 **8 ms** 预算（复跑 R46（代码视图的编辑交互与输入法） 的延迟基准并附对比数据）
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增注释切换、自动缩进、自动闭合三类集成测试

## 风险与提示

- **不要靠读 VS Code 的文档来推断缩进规则的行为**。`FEATURES.md` §11 把算法与测试一起列为可复用资产；把测试移植过来，比把实现移植过来更重要——有测试就能自查，没测试就只能等用户报 bug。
- `language-configuration.json` 里的正则是 JavaScript 语法，用了前瞻 / 后顾（`docs/compat/SYNTAX_AND_LANGUAGES.md` §6 实现注意）。一律走 R12（语言配置） 的兼容层；自己用 `regex` crate 重写会在 C# 的 `onEnterRules` 上直接编译失败。
- 自动行为是“帮倒忙”的高发区。VS Code 的细节（只越过自己插入的闭合符、`trimAutoWhitespace` 在光标离行时才生效、`ignoreEmptyLines` 对注释的影响）每一条都是用户抱怨过才加上去的，照搬就好，不要自作主张简化。
- 自动缩进发生在按键的关键路径上，而按键到上屏只有 8 ms。缩进规则的正则匹配要限制在必要的几行上下文内（VS Code 也是这么做的），**不要对全文做匹配**。
- 多光标 + 自动包围会产生很多小区间。确保它们合并成一次事务，否则 `Ctrl+Z` 会变成按光标数逐个回退。
