## 背景

`FEATURES.md` §5 对语言检测的判定是**改造**、P1，理由写得很直白：**VS Code 用机器学习模型，Rove 只做基于扩展名和内容的启发式判断**。扩展名那一半 [[syntax]] 已经做完了（按扩展名识别 + `files.associations` + Unity 的 `.asmdef` / `.meta` 等关联），并明确把「基于内容的启发式检测」留给本 issue。

所以本 issue 的范围很窄：**只处理扩展名给不出答案的文件**——无标题文件、没有扩展名的文件、扩展名未知的文件。它还要负责一件同样重要的事：把 VS Code 那几个 ML 相关的设置项**明确判为不支持并静默忽略**，不要让用户以为 Rove 也有模型。

必读：`FEATURES.md` §5（语言检测一行）、§13 规则 2（不支持的设置项静默忽略），`UX_SPEC.md` §1.5（状态栏语言模式）、§6.1，`docs/compat/SYNTAX_AND_LANGUAGES.md` §2（各语言与扩展名），`docs/compat/SETTINGS.md`（`files.associations`，以及「工作台」不支持段中的 `workbench.editor.languageDetection` 等四项），`KEYBINDINGS.md`（阅读一节的 `editor.detectLanguage`）。

## 目标

### 1. 启发式规则

在 `rove-syntax` 中实现一个纯函数：输入「文件名（可能没有扩展名）+ 文本的前若干 KB」，输出「语言 ID + 置信度」或「无结论」。规则按优先级排列，**第一个命中的即为结果**：

| 顺序 | 线索 | 例子 |
|---|---|---|
| 1 | shebang（`#!`） | `#!/usr/bin/env lua` → Lua；`#!/bin/sh` → 无结论（Rove 不支持 shell） |
| 2 | 首行标志 | `<?xml` → XML；`<!DOCTYPE html` → 无结论；`---` 开头的多文档 → YAML |
| 3 | JSON 嗅探 | 去掉前导空白与注释后以 `{` 或 `[` 开头，且能被 JSON/JSONC 解析器解析完整 → JSON / JSONC |
| 4 | XML 嗅探 | 以 `<` 开头且首个标签结构完整 → XML |
| 5 | YAML 嗅探 | 连续若干行形如 `key: value` 或 `- item`，缩进一致 → YAML |
| 6 | 其余 | 无结论，保持纯文本 |

- 候选语言只限 Rove 实际支持的那几种（`SYNTAX_AND_LANGUAGES.md` §2 的表）；**宁可「无结论」也不要猜一个支持不了的语言**。
- 结果带置信度，置信度不足时同样视为无结论。规则表写成数据并逐条测试，便于以后加语言只加一行。

### 2. 生效范围与优先级

- **只在扩展名给不出答案时生效**：有扩展名并且 [[syntax]] 能识别、或被 `files.associations` 覆盖时，一律以扩展名 / 关联为准，检测不介入。这条优先级要有测试。
- 无标题文件（`Ctrl+N` 新建、[[tail]] 与 [[structure]] 产生的只读临时标签）在内容变化时可以重新检测；已经由用户手动指定过语言模式的文档**不再自动改回**。
- 检测结果通过 [[contracts]] 的打开文档服务写入文档的语言字段，由此自动驱动 [[codeview-hl]] 的高亮、[[outline]] 的符号与 [[statusbar]] 的语言模式条目。本 issue **不自己画任何界面**。

### 3. 命令

`editor.detectLanguage`（`shift+alt+d`，when `editorTextFocus && !notebookEditable`，P1）：对当前文档强制运行一次检测并应用结果。检测不出结果时按 `UX_SPEC.md` §6.1 第 1 级在状态栏临时显示一行「未能识别此文件的语言」，3 秒后消失——**不弹通知、不弹模态框**。

### 4. 明确不支持的设置项

VS Code 的 ML 检测相关设置项在 `SETTINGS.md` 中已判为不支持，本 issue 确认并**静默忽略**（`FEATURES.md` §13 规则 2：不报错、不提示）：`workbench.editor.languageDetection`、`workbench.editor.languageDetectionHints`、`workbench.editor.historyBasedLanguageDetection`、`workbench.editor.preferHistoryBasedLanguageDetection`。

「基于历史的检测」（用户过去把类似文件设成什么语言）同样不做：它需要持久化用户行为，收益与 Rove 的定位不匹配。这一条要写进 `docs/ARCHITECTURE.md` 的对照表，避免以后被当成遗漏。

### 5. 性能与大文件

- **只读前若干 KB**（建议 8 KB 或前 100 行，取先到者），绝不扫全文；超大文件（内存映射）只看首块。
- 预算：单次检测 **< 1 ms**，并且**不得在界面线程上做 I/O**——文本从 [[contracts]] 的 `TextSource` 取，不自己读文件。
- 打开文件的总耗时不受影响（`UX_SPEC.md` §6.2：100 ms 内完成时不显示加载状态）。

### 6. 登记与替换

新增命令与界面字符串通过 [[contracts]] 的登记机制加入，不得写死。不得在本 PR 中修改 [[contracts]] 的接口。`rove-syntax` 的依赖树中不得出现 `gpui`。

## 不包含

- 按扩展名识别、`files.associations`、Unity 的 `.asmdef` / `.asmref` / `.meta` / `.prefab` / `.unity` / `.asset` 关联（[[syntax]]）；HLSL / ShaderLab / Lua / Protobuf 的扩展名（[[langs-extra]]）。
- 状态栏的语言模式条目与「切换语言模式」的界面（[[statusbar]]）。
- 基于机器学习或用户历史的检测（见目标第 4 节）。
- 编码识别与 BOM（[[buffer]]，`UX_SPEC.md` §3.5）——那是「用什么字符集读」，与「这是什么语言」是两件事。
- 二进制文件判定与图片预览（[[buffer]]、[[image]]）。

## 验收标准

- [ ] 六条规则逐条有单元测试，含正例与反例；候选语言限于 Rove 支持的集合，不支持的语言一律返回「无结论」
- [ ] 优先级有测试：有已知扩展名时不介入；`files.associations` 覆盖时不介入；用户手动指定过语言后不再自动改回
- [ ] 无扩展名的 JSON、XML、YAML 文件各一例，打开后语言模式正确、高亮生效
- [ ] 带 shebang 的 Lua 文件能识别；`#!/bin/sh` 返回「无结论」并保持纯文本
- [ ] `shift+alt+d` 可用，when 条件与 `KEYBINDINGS.md` 一致；检测不出结果时状态栏临时提示「未能识别此文件的语言」并在 3 秒后消失
- [ ] 四个 ML / 历史相关设置项被静默忽略（断言不产生任何日志、提示或行为变化），并已写入 `docs/ARCHITECTURE.md` 对照表
- [ ] 单次检测 < 1 ms，且只读取前若干 KB（断言读取的字节数上限）；超大文件只看首块
- [ ] 新增的命令与界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（打开无扩展名的 JSON → 断言语言字段与首行 token 的 scope）
- [ ] `cargo tree -p rove-syntax -i gpui` 为空

## 风险与提示

- 启发式最大的风险不是「猜不出」，而是「猜错」。猜错会让高亮、大纲、折叠一起走偏，用户还不知道为什么。所以每条规则都要定得保守：宁可返回无结论、让文件按纯文本显示，也不要靠「看起来像」去赌。
- JSON 嗅探不要只看首字符。`{` 开头的文件也可能是别的东西，必须真的用 JSONC 解析器尝试解析（解析失败就是无结论）。好在前 8 KB 的解析成本极低。
- YAML 嗅探是最容易误报的一条：很多日志和配置文件都长得像 `key: value`。建议要求连续若干行同时成立且缩进一致，并把阈值写成常量方便调。
- 「用户手动指定过就不再自动改回」这条状态要存在文档上而不是全局表里，否则关闭再打开同一个文件会出现行为不一致。
- 不要顺手支持 shell、Python、TypeScript 这些 VS Code 能检测而 Rove 不支持的语言。检测出一个没有语法、没有语言配置的语言 ID，只会让 [[codeview-hl]] 和 [[outline]] 拿到一个空壳。
