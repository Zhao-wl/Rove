# 语法高亮与语言配置兼容方案（阶段 0.3c-2、0.3c-3）

> 版本：v1.0（2026-11-09 已确认）
> 相关：D1（tree-sitter + 兼容 VS Code 主题）、D2（其他语言只做高亮和折叠）、[THEME_COLORS.md](THEME_COLORS.md)（界面颜色键）

## 1. 语法高亮的实现方式

### 1.1 关键发现：VS Code 自己就是这样做的

VS Code 已经开始用 tree-sitter 做高亮（`src/vs/editor/common/languages/highlights/typescript.scm`、`css.scm`、`ini.scm`）。它的做法是：**高亮查询的捕获名直接写成完整的 TextMate scope**，例如：

```scheme
(this) @variable.language.this.ts
((string) @string.quoted.single.ts (#match? @string.quoted.single.ts "^'.*'$"))
```

这样，tree-sitter 产生的 token 与原来 TextMate 语法产生的 token 带有**相同的 scope**，现有主题的 `tokenColors` 规则无需任何修改就能生效。

**Rove 采用完全相同的做法**，D1 中原计划的“捕获名 → scope 映射表”因此不再需要单独维护：
- 每种语言写一份 `highlights.scm`，捕获名使用 VS Code 内置 TextMate 语法为同类 token 产生的 scope。
- 主题匹配算法移植自 `vscode-textmate` 的主题解析（MIT 许可）：scope 选择器、父 scope 选择器、规则优先级（specificity）、`fontStyle`（粗体、斜体、下划线、删除线）。
- 用户设置 `editor.tokenColorCustomizations` 按 VS Code 的语义合并。

### 1.2 验证方法：VS Code 的着色测试数据

VS Code 仓库中的 `extensions/vscode-colorize-tests` 为每种语言准备了测试源文件（`colorize-fixtures/`），以及每个 token 在 **10 个内置主题**下的期望颜色（`colorize-results/*.json`，记录了文本、scope、各主题下命中的规则和最终颜色）。

Rove 直接把这些数据当作黄金测试集（MIT 许可，注明来源）：
1. 用 Rove 的 tree-sitter 高亮处理同一个测试源文件；
2. 用同一个主题计算每个字符的颜色；
3. 与 VS Code 的期望颜色**逐字符比较**。比较的是颜色，不是 scope：只要最终颜色一致，scope 有出入也可以接受。

**验收阈值**（以 Dark Modern、Light Modern、Dark+、Light+ 四个主题计）：

| 语言 | 测试数据 | 颜色一致的字符比例 |
|---|---|---|
| C#、JSON、YAML、XML、Markdown | `test_cs`、`test_json`、`test_yaml` 及 3 个 YAML 问题用例、`test_xml`、`test-7115_xml`、`test_md` 及 3 个 Markdown 问题用例 | ≥ 98% |
| HLSL、ShaderLab、Lua | `test_hlsl`、`test_shader`、`test_lua` | ≥ 90% |
| Protobuf | VS Code 没有内置支持，也没有测试数据 | 不设阈值，人工检查 |

不一致的地方逐条列出，并说明原因（例如 tree-sitter 能区分而 TextMate 正则区分不了的情况）。

## 2. 各语言方案

| 语言 | tree-sitter 语法 | 目标 scope 的来源（VS Code 内置语法） | 能力 | 优先级 |
|---|---|---|---|---|
| C# | `tree-sitter/tree-sitter-c-sharp`（MIT，维护活跃） | `extensions/csharp/syntaxes`（上游为 dotnet/csharp-tmLanguage） | 高亮、折叠、大纲、跳转（预计算索引）、语义高亮 | P0 |
| JSON / JSONC | `tree-sitter/tree-sitter-json`（MIT） | `extensions/json/syntaxes` | 高亮、折叠、大纲、结构树、校验 | P0 |
| YAML | `tree-sitter-grammars/tree-sitter-yaml`（MIT；`ikatyang/tree-sitter-yaml` 已停止维护，实现前确认这个维护版本可用） | `extensions/yaml/syntaxes` | 高亮、折叠、大纲、结构树、校验 | P0 |
| XML | `tree-sitter-grammars/tree-sitter-xml`（MIT） | `extensions/xml/syntaxes` | 高亮、折叠、大纲、结构树、校验 | P0 |
| Markdown | `tree-sitter-grammars/tree-sitter-markdown`（MIT，分为块级和行内两个语法） | `extensions/markdown-basics/syntaxes` | 高亮、折叠、大纲；围栏代码块按语言注入 | P0 |
| `.asmdef`、`.asmref` | 同 JSON | 同 JSON | 同 JSON | P0 |
| `.meta`、`.prefab`、`.unity`、`.asset` | 同 YAML | 同 YAML | 同 YAML；Unity 增强为 P2 | P0 |
| HLSL（`.hlsl`、`.cginc`、`.compute`） | `tree-sitter-grammars/tree-sitter-hlsl`（MIT） | `extensions/hlsl/syntaxes` | 高亮、折叠 | P1 |
| ShaderLab（`.shader`） | **自己编写**（见 §3） | `extensions/shaderlab/syntaxes` | 高亮、折叠；着色器代码块中注入 HLSL | P1 |
| Lua | `tree-sitter-grammars/tree-sitter-lua`（MIT） | `extensions/lua/syntaxes` | 高亮、折叠 | P1 |
| Protobuf | `coder3101/tree-sitter-proto`（MIT，维护活跃） | VS Code 没有内置语法，采用通用 scope（`keyword`、`storage.type`、`entity.name.type`、`string`、`comment`……） | 高亮、折叠 | P1 |

所有语法都通过 Cargo 依赖或 vendor 的方式编译进 Rove，**不在运行时下载**。

## 3. ShaderLab：自己编写语法

现有的两个 tree-sitter-shaderlab 都是个人项目（0–1 星，没有许可证），无法使用。ShaderLab 的结构本身很简单，所以自己编写：

- **外层结构**：`Shader "name" { Properties { … } SubShader { Tags { … } Pass { … } } Fallback … CustomEditor … }`，加上渲染状态命令（`Cull`、`ZWrite`、`Blend` 等）和属性声明。
- **代码块注入**：`CGPROGRAM … ENDCG`、`HLSLPROGRAM … ENDHLSL`、`CGINCLUDE`、`HLSLINCLUDE` 中的内容，通过 tree-sitter 的语言注入交给 HLSL 语法处理。这样一个 `.shader` 文件中的着色器代码，和 `.hlsl` 文件获得同样的高亮。
- **验收**：通过 `test_shader` 黄金测试（≥ 90%），并在两个 Unity 测试项目中的全部 `.shader` 文件上解析不报错（允许个别错误节点，但要统计并列出）。
- 编写语法的工作量估计为中等（语法本身约几百行）。语法放在 `grammars/tree-sitter-shaderlab/`，以 MIT 许可发布。

## 4. 语义高亮（C#，P1）

- 来源：C# 预计算索引中的符号种类，可以区分类、结构、接口、枚举、委托、字段、属性、局部变量、参数、方法、事件、命名空间、类型参数等。
- 输出：按 VS Code 标准的语义 token 类型和修饰符（`readonly`、`static`、`deprecated`、`abstract` 等）生成。
- 着色：优先使用主题中的 `semanticTokenColors`；主题没有定义时，按 VS Code 内置的“语义 token → TextMate scope”回退表映射到 scope，再按 `tokenColors` 着色。这张回退表从 VS Code 源码 `tokenClassificationRegistry` 移植。
- 开关：遵循 `editor.semanticHighlighting.enabled` 和主题中的 `semanticHighlighting` 字段，与 VS Code 一致。
- 文件已修改但未保存时，修改过的区域退回到纯语法高亮，直到索引更新完成（与 UX_SPEC §2.2 的近似结果原则一致）。

## 5. Markdown 预览中的代码块

Markdown 预览运行在 WebView2 中（D5）。代码块高亮与 VS Code 一样使用 highlight.js，样式使用 VS Code 的 `highlight.css`（MIT），颜色跟随当前主题。这部分与编辑器中的 tree-sitter 高亮是两套独立的实现，因此预览和编辑器中同一段代码的颜色可能有细微差别；这一点与 VS Code 的行为相同。

## 6. 语言配置（`language-configuration.json`）

Rove 直接使用 VS Code 内置扩展中各语言的 `language-configuration.json`（MIT），不自己重写。这 8 份配置是：`csharp`、`json`（JSONC 共用）、`yaml`、`xml`、`markdown-basics`（M1），以及 `hlsl`、`shaderlab`、`lua`（随 M4 的其他语言一起接入）。下表列出其中实际出现的字段，以及 Rove 的支持情况：

| 字段 | 用到它的语言 | Rove 用它实现的功能 | 优先级 |
|---|---|---|---|
| `comments`（`lineComment`、`blockComment`） | 全部 | 注释切换（`Ctrl+/`、`Shift+Alt+A`） | P1 |
| `brackets` | 全部 | 括号匹配、括号跳转 | P0 |
| `colorizedBracketPairs` | C#、Markdown、XML | 括号对着色（未定义时使用 `brackets`） | P1 |
| `autoClosingPairs`（含 `notIn`） | 全部 | 自动闭合括号和引号 | P1 |
| `autoCloseBefore` | （这 8 份中都没有使用，取 VS Code 默认值） | 决定光标后面是什么字符时才自动闭合 | P1 |
| `surroundingPairs` | 除 JSON 外 | 选中文本后输入括号，用括号包围选区 | P1 |
| `folding.markers` | C#、JSON、Markdown、XML、YAML | 基于标记的折叠（如 C# 的 `#region`） | P0 |
| `folding.offSide` | Markdown、YAML | 基于缩进折叠时，空行归属于下一个区块 | P0 |
| `wordPattern` | Markdown、XML（其他语言使用 VS Code 默认值） | 单词边界：双击选词、同名高亮、按单词移动 | P0 |
| `indentationRules` | JSON、Lua、YAML | 自动缩进 | P1 |
| `onEnterRules` | C#、JSON | 按回车时的缩进与续写（例如在 `///` 注释中换行时自动补上 `///`） | P1 |

**实现注意**：这些配置中的正则表达式是 JavaScript 语法，部分用到了前瞻、后顾等特性。Rust 侧需要使用支持这些特性的正则库（例如 `fancy-regex`），并为每一份配置写一个测试，确保其中所有正则都能编译、行为与 JavaScript 一致。

## 7. 文件图标主题（P1）

内置 VS Code 的 Seti 文件图标主题（`extensions/theme-seti`，MIT 许可；图标字体来自 seti-ui，MIT 许可）。按 VS Code 的文件图标主题格式实现（文件名、扩展名、语言 ID 三种关联方式），以后也可以只读加载用户在 VS Code 中安装的图标主题，与颜色主题的处理方式相同（FEATURES §7）。
