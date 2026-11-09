## 背景

括号匹配、折叠标记、单词边界、注释切换、自动闭合、自动缩进，这些行为在 VS Code 里全部来自各语言扩展中的 `language-configuration.json`。`FEATURES` §11 把它列为"可以直接复用的 VS Code 资产"，`SYNTAX_AND_LANGUAGES.md` §6 明确：**Rove 直接使用这些配置文件，不自己重写**。这样 Rove 的括号、折叠、单词规则与 VS Code 天然一致。

实现放在 `rove-syntax`，在 [[contracts]] 定义的语言配置数据模型背后；本 PR 不修改该接口。

## 目标

### 1. 加载配置

把 VS Code 内置扩展中各语言的 `language-configuration.json`（MIT，在 `THIRD_PARTY_NOTICES.md` 注明来源与版本）随二进制一起分发，**运行时不下载，也不读取用户安装的扩展**。`SYNTAX_AND_LANGUAGES.md` §6 统计的是 8 份配置；M1 必须覆盖 C#、JSON、JSONC、YAML、XML、Markdown，HLSL 与 Lua 的两份随 [[langs-extra]] 一起接入（本 issue 只要保证加载器能直接吃下它们）。

### 2. 支持的字段（`SYNTAX_AND_LANGUAGES.md` §6）

| 字段 | 本 issue 的职责 | 使用方 |
|---|---|---|
| `comments`（`lineComment`、`blockComment`） | 解析 | [[autoedit]] |
| `brackets` | 解析 | [[codeview-hl]]、[[smartselect]] |
| `colorizedBracketPairs` | 解析；未定义时回落到 `brackets` | [[brackets]] |
| `autoClosingPairs`（含 `notIn`） | 解析 | [[autoedit]] |
| `autoCloseBefore` | 这 8 份都没用，取 VS Code 的默认值并写进代码注释 | [[autoedit]] |
| `surroundingPairs` | 解析 | [[autoedit]] |
| `folding.markers` | 解析，并产出标记折叠区间（C# 的 `#region`） | [[codeview-hl]] |
| `folding.offSide` | 解析（Markdown、YAML：缩进折叠时空行归属下一区块） | [[codeview-hl]] |
| `wordPattern` | 解析并提供"取光标处单词"的查询 | [[codeview-hl]]、[[edit-core]] |
| `indentationRules` | 解析 | [[autoedit]] |
| `onEnterRules` | 解析 | [[autoedit]] |

本 issue 做**数据模型 + 查询**（给定语言与位置，返回括号对、注释符、单词范围、折叠标记），功能本身不在这里实现。

### 3. JavaScript 正则兼容

这些配置里的正则是 JavaScript 语法，部分用到前瞻、后顾。用 `fancy-regex`（或等价的、支持这些特性的库）编译，并处理两种语法差异：命名组写法（`(?<name>...)`）、`\d` / `\w` 的 Unicode 语义。

- 为**每一份配置**写一个测试，确保其中所有正则都能编译；
- 对 `wordPattern`、`indentationRules`、`onEnterRules` 中的每条正则，准备一组样例串，断言匹配结果与 JavaScript 一致（样例来源逐条注明：取自 VS Code 的测试、或手工构造）；
- 不支持的正则特性要**显式报错并指明是哪一份配置的哪个字段**，不允许静默退化成"永不匹配"。

### 4. 语言级设置覆盖

按语言查询时要能叠加 `"[csharp]"` 这类语言级设置覆盖（例如 `editor.maxTokenizationLineLength`）。这个能力由 [[settings]] 提供，本 issue 只消费它，不自己解析设置文件。

## 不包含

- 注释切换、自动闭合、包围、自动缩进、回车规则等功能本身（[[autoedit]]）。
- 括号匹配渲染、折叠界面、缩进参考线（[[codeview-hl]]）、括号对着色（[[brackets]]）。
- 单词操作与边界算法的使用方（[[edit-core]]）。
- 自动检测缩进风格（属于 [[autoedit]] / [[statusbar]]）。

## 验收标准

- [ ] M1 的 6 份配置全部能加载，并有字段级断言（括号对数量、注释符、折叠标记正则等逐项核对原文件）
- [ ] 每份配置中的**所有**正则都有编译测试；新增一份配置时测试自动覆盖（遍历目录而不是逐个写死）
- [ ] `wordPattern` 在 Markdown 与 XML 上的行为与 VS Code 一致，给出对照用例（这两种语言自定义了 `wordPattern`，其余语言用 VS Code 默认值）
- [ ] `folding.markers` 能从一段 C# 代码中产出 `#region` / `#endregion` 的配对区间，嵌套与未配对的情况都有用例
- [ ] `folding.offSide` 在 YAML 与 Markdown 上的空行归属有用例
- [ ] `autoClosingPairs` 的 `notIn`（字符串、注释中不自动闭合）解析正确
- [ ] 不支持的正则特性会报出"哪份配置的哪个字段"，并有一个人为构造的失败用例
- [ ] 新增集成测试覆盖"按语言 ID 取配置 → 查询括号 / 单词 / 折叠标记"；[[skeleton]] 的端到端测试继续通过
- [ ] `cargo tree -p rove-syntax -i gpui` 为空

## 风险与提示

- `indentationRules` 与 `onEnterRules` 的正则最复杂（C# 的 `///` 续写、JSON 的缩进规则），也最容易在 JS 与 Rust 之间出现行为差异。这两项的使用方是 M4 的 [[autoedit]]，所以**本 issue 只要保证解析与匹配正确**，不必追求把规则引擎也实现出来。
- 配置文件里会出现 `// 注释`（它们本身是 JSONC）。用与 [[settings]] 同一套 JSONC 解析，不要再写一份。
- 这些文件是数据资产，升级 VS Code 时要能整体替换。把它们原样放在一个目录里并记下来源版本，不要手工改动其中的内容；确需偏离 VS Code 时，在代码里做覆盖并写明原因。
