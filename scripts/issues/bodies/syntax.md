## 背景

D1 决定用 tree-sitter 代替 TextMate 正则引擎做高亮。关键做法在 `docs/compat/SYNTAX_AND_LANGUAGES.md` §1.1：**高亮查询的捕获名直接写成完整的 TextMate scope**，与 VS Code 内置 TextMate 语法为同类 token 产生的 scope 一致。VS Code 自己就是这么做的（`src/vs/editor/common/languages/highlights/*.scm`，例如 `(this) @variable.language.this.ts`）。这样现有主题的 `tokenColors` 无需任何修改就能生效，原计划的"捕获名 → scope 映射表"不再需要单独维护。

实现放在 `rove-syntax`，在 [[contracts]] 定义的高亮接口背后；本 PR 不修改这些接口。替换 [[skeleton]] 中的 `SKELETON(syntax)` 标记（"只接入 tree-sitter C#，用少量查询规则产生 scope"），并更新 `docs/ARCHITECTURE.md` 的对照表。

## 目标

### 1. 集成 tree-sitter 与五种语法

按 `SYNTAX_AND_LANGUAGES.md` §2 的仓库与许可（全部 MIT，在 `THIRD_PARTY_NOTICES.md` 注明来源与版本）：

- C#：`tree-sitter/tree-sitter-c-sharp`
- JSON / JSONC：`tree-sitter/tree-sitter-json`
- YAML：`tree-sitter-grammars/tree-sitter-yaml`（**不要用已停止维护的 `ikatyang/tree-sitter-yaml`**，实现前先确认维护版本可用）
- XML：`tree-sitter-grammars/tree-sitter-xml`
- Markdown：`tree-sitter-grammars/tree-sitter-markdown`（块级与行内两个语法）

全部通过 Cargo 依赖或 vendor 的方式**编译进 Rove，运行时不下载**。

### 2. `highlights.scm`

每种语言一份，捕获名使用对应 VS Code 内置语法（`extensions/{csharp,json,yaml,xml,markdown-basics}/syntaxes`）为同类 token 产生的 scope。**不要发明自己的类别名**，也不要在本 crate 里维护一张映射表。scope 的正确性由 [[golden]] 逐字符验证。

### 3. 语言识别与文件关联

按扩展名识别，并遵循 `files.associations`。Unity 相关的关联（`FEATURES` §9、`SYNTAX_AND_LANGUAGES.md` §2，均为 P0）：

- `.asmdef`、`.asmref` → JSON
- `.meta`、`.prefab`、`.unity`、`.asset` → YAML

基于内容的启发式语言检测是 [[langdetect]]，本 issue 只做扩展名与关联。

### 4. 高亮结果

按行查询 token：`(字节区间, scope 列表)`。版本号随文本版本推进；**结果过期时可以继续使用旧结果**（接口上要能表达"这是上一版的结果"），这是 UX 的明确要求（先显示已有内容、逐步补全，`UX_SPEC` §6.2）。

### 5. 增量解析

接收编辑（`InputEdit`），只重新解析变化的部分，并计算出**哪些行的高亮发生了变化**，交给视图做局部失效。行级差集要尽量紧，不能动不动返回"全文件变了"。

### 6. 后台解析

解析放在后台执行器，不阻塞界面线程（`PROJECT_CHARTER` §5）。超过时间预算时返回上一版结果并标记过期。配合 `UX_SPEC` §3.1：大文件的语法高亮在后台完成，完成前先显示纯文本；超大文件不解析。遵循 `editor.maxTokenizationLineLength`（默认 20000，`[csharp]` 覆盖为 2500）。

### 7. Markdown 围栏代码块注入

用 tree-sitter 的语言注入，把围栏代码块的内容交给对应语言的语法处理（至少支持本 issue 内置的 5 种；未知语言不报错，按纯文本）。

## 不包含

- 折叠区间与文档符号（接口归 `rove-syntax`，但由 [[codeview-hl]] 与 [[outline]] 实现）。
- 主题着色与 scope 匹配（[[theme]]）、颜色逐字符比对（[[golden]]）。
- 语言配置 `language-configuration.json`（[[langcfg]]）。
- C# 语义高亮（[[semantic]]）、HLSL / ShaderLab / Lua / Protobuf（[[langs-extra]]）。
- Markdown 预览中的代码块高亮（预览用 highlight.js，与这里是两套实现，`SYNTAX_AND_LANGUAGES.md` §5）。

## 验收标准

- [ ] 五种语言各有一份"按行 token（字节区间 + scope）"的快照测试
- [ ] `.asmdef` 按 JSON、`.meta` / `.prefab` / `.unity` / `.asset` 按 YAML 处理，各有测试
- [ ] Markdown 中的 ` ```csharp ` 代码块内部产生 C# 的 scope（注入生效）
- [ ] 基准：5000 行 C# 文件中做一次单字符编辑，重新解析 + 高亮更新 < 2 ms（`LANGUAGE_INTELLIGENCE` §1）
- [ ] 基准：1 MB C# 文件首次全量高亮 < 50 ms
- [ ] 增量更新返回的"变化行集合"有测试：在方法体内改一个字符时，不应标记整个文件
- [ ] 结果过期时能继续返回旧结果，并带有"已过期"标记
- [ ] `SKELETON(syntax)` 标记已删除，`docs/ARCHITECTURE.md` 的对照表已更新
- [ ] 新增集成测试覆盖"打开文件 → 后台解析 → 按行取 token"；[[skeleton]] 的端到端测试继续通过
- [ ] `cargo tree -p rove-syntax -i gpui` 为空

## 风险与提示

- `highlights.scm` 的 scope 命名是整个主题兼容性的地基。写之前先把对应语言的 VS Code `.tmLanguage.json` 读一遍，按它实际产生的 scope 写，不要凭直觉。写完立刻用 [[golden]] 的方式抽查，不要等到 [[golden]] 整个做完才发现命名跑偏。
- tree-sitter 在某些地方比 TextMate 分得更细（或更粗）。差异本身不是 bug，但要能解释；[[golden]] 会要求逐条说明不一致的原因。
- 不要让 `rove-syntax` 依赖 `rove-buffer` 的具体类型，面向 [[contracts]] 的 `TextSource` 编程，否则两个 issue 会互相卡住。
