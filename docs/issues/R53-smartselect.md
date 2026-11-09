# R53 智能扩选、括号跳转与选择

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 2 中等 | ui | 第 6 批 | R46（代码视图的编辑交互与输入法）、R09（tree-sitter 集成）、R20（代码视图） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

智能扩选是阅读代码时最顺手的一个键：光标放在一个标识符上，按几下 `Shift+Alt+→` 就从单词扩到表达式、到参数列表、到整个方法体。VS Code 用语言服务提供选区区间，Rove 改为**用 tree-sitter 语法树**（`FEATURES.md` §4：改造 / P1）——语法树本来就在 R09（tree-sitter 集成） 里，不需要任何额外的语言服务。

括号跳转与选择放在同一个 issue，是因为它们用的是同一类信息：`editor.action.jumpToBracket` 与 `selectToBracket` 依赖括号配对，而括号配对已经由 R20（代码视图） 在 M1 实现（`editorBracketMatch.*` 的高亮就来自它）。本 issue 复用它的配对结果，不重新算一遍。

必读：`FEATURES.md` §4，`docs/compat/KEYBINDINGS.md`（智能扩选 2 条、括号 2 条，以及 §7 附录中“P1 · 智能扩选”的 `editor.action.smartSelect.grow` 与“P1 · 括号”的 `editor.action.selectToBracket`），`SETTINGS.md`（`editor.smartSelect.selectLeadingAndTrailingWhitespace`、`editor.smartSelect.selectSubwords`）。

## 目标

### 1. 扩选与缩选

- 命令：`editor.action.smartSelect.expand`（`shift+alt+right`，when `editorTextFocus`）、`editor.action.smartSelect.shrink`（`shift+alt+left`）、`editor.action.smartSelect.grow`（§7 附录，无默认键位，**不要自行分配**，行为与 `expand` 一致，VS Code 保留它作为别名）。
- 区间来源：R09（tree-sitter 集成） 的语法树。从光标位置向上取祖先节点，生成一条**由内向外、严格包含、去重**的区间链；同一层级内多个等价节点只保留一个。
- 缩选沿同一条链回退：扩选 N 次后缩选 N 次必须精确回到起点，中途发生编辑或光标移动时链失效，重新开始。
- 对**不支持语法树的文件**（纯文本、没有 tree-sitter 语法的语言）退化为 VS Code 的通用层级：单词 → 行 → 括号块 → 全文；退化时不给任何提示（与 `UX_SPEC.md` §2.2 对“近似”的克制原则一致）。

### 2. 子词与空白

- `editor.smartSelect.selectSubwords`（默认 true）：链的最内层先给驼峰 / 下划线子词（`fooBar` 中的 `foo`），再给完整单词。子词边界复用 R45（编辑核心模型） 移植的 `wordPartOperations` 规则，不另写一套。
- `editor.smartSelect.selectLeadingAndTrailingWhitespace`（默认 true）：区间是否包含前导与尾随空白。
- 单词层的边界用 `editor.wordSeparators` 与 R12（语言配置） 的 `wordPattern`，与双击选词（R20（代码视图））保持一致。

### 3. 括号跳转与选择

- `editor.action.jumpToBracket`（`ctrl+shift+oem_5`）：在配对括号之间来回跳；光标不在括号上时跳到最近的外层括号对（照搬 VS Code）。
- `editor.action.selectToBracket`（§7 附录，无默认键位）：选中到配对括号为止，带参数决定是否包含括号本身。
- `editor.action.removeBrackets`（`ctrl+alt+backspace`）：删除光标所在的一对括号，两处删除合并为 R45（编辑核心模型） 的**一次**事务与一次撤销。
- 括号配对一律取自 R20（代码视图） 的括号匹配结果（数据源是 R12（语言配置） 的 `brackets`），不自己扫描；跳转与选择后的滚动定位走 R19（代码视图 Element） 的接口。

### 4. 性能

扩选链在按下第一次时一次性算好并缓存，后续扩 / 缩直接取缓存，不重复遍历语法树。整条链的计算必须在一次按键的预算内完成（按键到上屏 < 8 ms，`PROJECT_CHARTER.md` §6）；语法树尚未解析完成（大文件后台高亮阶段）时退化为通用层级，不等待。

### 5. 登记与替换

新增命令、设置项、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。R02（【接口先行 A2】行走骨架） 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口（契约变更走单独小 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- tree-sitter 集成与增量解析（R09（tree-sitter 集成））、语言配置（R12（语言配置））。
- 括号匹配的计算与 `editorBracketMatch.*` 高亮（R20（代码视图））、括号对着色与活动缩进参考线高亮（R56（括号对着色与活动缩进参考线高亮））。
- 选区模型、单词边界规则、编辑事务（R45（编辑核心模型））；基础光标与选区手势（R19（代码视图 Element））；多光标（R51（多光标与行操作））。
- `editor.doubleClickSelectsBlock`（依赖括号信息，归 R20（代码视图））。
- 折叠（R20（代码视图））、大纲与面包屑（R26（大纲））、粘性滚动（R54（粘性滚动））——它们用的是同一棵语法树，但各有各的区间类型。
- 基于语义索引的选区（例如“选中整个符号的引用”）：不做。

## 验收标准

- [ ] `Shift+Alt+→` / `Shift+Alt+←` 在 C# 文件上逐级扩选 / 缩选：标识符 → 表达式 → 语句 → 块 → 方法 → 类，逐级断言区间
- [ ] **扩选 N 次后缩选 N 次精确回到起点**（N ≥ 6）；中途编辑或移动光标后链正确失效
- [ ] 在 JSON、YAML、XML、Markdown 四种语言上各有一组扩选用例；没有语法树的纯文本退化为单词 → 行 → 全文且无提示
- [ ] `editor.smartSelect.selectSubwords` / `selectLeadingAndTrailingWhitespace` 两种取值各有用例（`fooBarBaz`、带前导空白的行）
- [ ] 子词边界与 R45（编辑核心模型） 的 `wordPartOperations`、双击选词的结果三者一致（同一组样例上交叉断言）
- [ ] `Ctrl+Shift+\` 在嵌套括号中来回跳转正确；光标不在括号上时跳到最近外层括号对
- [ ] `editor.action.selectToBracket` 含 / 不含括号两种参数各有用例；`Ctrl+Alt+Backspace` 删除一对括号后一次撤销全部回退
- [ ] 选区在 CJK、emoji、Tab 混排行上的边界正确（复用 R19（代码视图 Element） 的命中测试用例数据）
- [ ] 扩选链的计算不使按键到上屏超出 **8 ms**（在一个 2000 行的 C# 文件上取样，附数据）
- [ ] 只读文档上扩选与跳转可用、`removeBrackets` 被拒绝
- [ ] 新增的命令、设置项、界面字符串都经由登记机制，无硬编码；`editor.action.smartSelect.grow` 与 `selectToBracket` **没有被分配默认键位**
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增扩选与括号跳转入口的集成测试

## 风险与提示

- tree-sitter 的节点层级与 VS Code 语言服务给出的层级**不完全一样**：语法树里会多出很多中间节点（`expression_statement` 包着 `invocation_expression` 包着 `argument_list`），逐个暴露给用户会让扩选变得“按很多下才有变化”。必须做去重与合并，规则写进 PR 说明并用 C# 的典型结构（方法调用、`if`/`else`、lambda、泛型参数、属性访问器）验证。
- 扩选链不要在每次按键时重算。语法树遍历在深层嵌套的 C# 文件里不便宜，而这是按键关键路径。
- 括号跳转在**语法错误**的文件里会找不到配对（编辑过程中很常见）。按 VS Code 的做法静默不动，不要弹提示。
- `editor.action.smartSelect.grow` 与 `editor.action.selectToBracket` 在 VS Code 中没有默认键位。按 R19（代码视图 Element） 的同类约定，登记命令但不分配键位。
