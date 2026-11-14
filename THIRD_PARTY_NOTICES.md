# 第三方来源

> 本文件随安装包与便携版一起分发（`packaging/package-contents.toml`），「关于」对话框里的
> 「第三方声明」按钮打开的就是安装目录里的这一份。维护规则见 `AGENTS.md` §5.1：
> 移植第三方代码或资产时在这里注明来源与许可。

## 总表（#76）

| 来源 | 许可 | 用途 | 详见 |
|---|---|---|---|
| VS Code（microsoft/vscode） | MIT | 默认主题、语言配置、默认键位表、设置项 schema 与说明文字、`markdown.css` / `highlight.css`、colorize 测试数据、`vscode-textmate` 的主题匹配、模糊匹配 / 单词边界 / 缩进规则等算法 | 下文各节 |
| codicons（@vscode/codicons） | **CC-BY-4.0（必须署名）** | 界面图标字体 | [Codicons 产品图标](#codicons-产品图标16) |
| theme-seti + seti-ui 字体 | MIT | 文件图标主题 | 见下文「Seti 文件图标主题」 |
| tree-sitter 与各语言 grammar | MIT（逐个列出） | 语法高亮 | [Tree-sitter 语法与高亮查询](#tree-sitter-语法与高亮查询9) |
| Mermaid / KaTeX / highlight.js | MIT / MIT / BSD-3-Clause | Markdown 预览（离线打包） | [Markdown 预览的打包资源](#markdown-预览的打包资源29) |
| gpui | **Apache-2.0**（单一许可，不是 MIT/Apache 双授权） | 界面框架 | [GPUI 0.2.2](#gpui-022) |
| .NET 9 运行时与 Roslyn | MIT | 可选的 C# 索引组件（独立包，不在主安装包里） | 组件包里的 `THIRD_PARTY_NOTICES.txt` |
| 其余 Rust 依赖 | 自动汇总 | 构建与运行时依赖 | [`packaging/licenses/rust-dependencies.md`](packaging/licenses/rust-dependencies.md) |

Rust 依赖的完整许可清单由 `cargo xtask licenses --write` 从 `cargo metadata` 生成，
并由 CI 的「依赖许可汇总」任务校验（`cargo xtask licenses --check`）：
**新增一个许可未登记的依赖会失败**，允许列表在 `packaging/licenses/allowed.toml`。

## VS Code 默认主题与主题引擎（#10）

以下内容来自 Microsoft VS Code **1.145.2**（提交 `c4453f8ea430795f941c1b2ff6c9321135a74a02`，MIT 许可）：

- `crates/rove-theme/data/themes/*.json`：`extensions/theme-defaults/themes/` 下的十个默认主题文件，
  原样复制（Dark 2026、Light 2026、Dark Modern、Light Modern、Dark+、Light+ 以及它们 include 的
  Visual Studio 与高对比度主题）。文件清单与哈希见该目录的 `README.md`。
- `crates/rove-theme/src/defaults/generated.rs`：由 `scripts/theme/gen_color_defaults.py` 从
  `src/vs/**/*.ts` 中的 `registerColor` 调用提取的颜色默认值表。
- `crates/rove-theme/src/color.rs`：移植 `src/vs/base/common/color.ts` 的颜色运算（HSLA、
  `lighten`/`darken`/`transparent`/`makeOpaque`/`mix`、相对亮度）。
- `crates/rove-theme/src/defaults/mod.rs`：移植 `src/vs/platform/theme/common/colorUtils.ts` 的
  默认值求值（`resolveColorValue`/`executeTransform`）。
- `crates/rove-theme/src/file.rs`、`resolved.rs`：移植 `src/vs/workbench/services/themes/common/`
  中 `colorThemeData.ts` 与 `themeCompatibility.ts` 的主题加载、`include` 合并与用户覆盖语义。

来源：<https://github.com/microsoft/vscode/tree/1.145.2>
原始许可证：<https://github.com/microsoft/vscode/blob/1.145.2/LICENSE.txt>
以下保留版权及许可声明；完整第三方依赖分发清单由 #75 维护。

MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## GPUI 0.2.2

Source: https://crates.io/crates/gpui/0.2.2 (Zed Industries).
License: Apache License, Version 2.0. The original license is retained in
`vendor/gpui/LICENSE-APACHE`. Rove modifications and reapplication instructions
are recorded in `vendor/gpui/ROVE_PATCHES.md`.

## gpui_http_client 0.2.2（被 Rove 的替身取代）

Source: https://crates.io/crates/gpui_http_client/0.2.2 (Zed Industries).
License: Apache License, Version 2.0（同 `vendor/gpui/LICENSE-APACHE`）。

`gpui` 强制依赖这个 crate。#74 用 workspace 的 `[patch.crates-io]` 把它整个换成
`crates/rove-gpui-http-stub`（包名同为 `gpui_http_client`），所以**上游实现不在依赖图、
也不在发布产物里**；reqwest / hyper / rustls / ring / tokio 随之消失。

替身是 **Rove 自己的代码**（MIT，与本仓库一致），没有网络实现。但为了做到「drop-in 替换」，
它的公开 API 形状照着上游 0.2.2 写：`HttpClient` trait 的方法签名、`AsyncBody` 的构造与
`futures::AsyncRead` 实现、`HttpClientWithProxy` / `HttpClientWithUrl` / `RedirectPolicy` /
`HttpRequestExt` / `read_proxy_from_env` 等少量平凡实现与上游基本一致。**这部分视为派生自上游的
Apache-2.0 代码**，故在此保留来源与许可声明；删减项与理由见
`crates/rove-gpui-http-stub/src/lib.rs` 顶部与 `vendor/gpui/ROVE_PATCHES.md`。

## VS Code 语言配置（`language-configuration.json`）

`crates/rove-syntax/data/vscode-extensions/` 下的五个语言配置文件原样取自 Microsoft VS Code
**1.145.2**（提交 `c4453f8ea430795f941c1b2ff6c9321135a74a02`，MIT 许可），用于括号、注释、
自动闭合、缩进规则、单词边界与折叠标记。文件清单、哈希与升级步骤见该目录的 `README.md`：

- `extensions/csharp/language-configuration.json`
- `extensions/json/language-configuration.json`（`json` 与 `jsonc` 共用）
- `extensions/markdown-basics/language-configuration.json`
- `extensions/xml/xml.language-configuration.json`
- `extensions/yaml/language-configuration.json`

来源：<https://github.com/microsoft/vscode/tree/1.145.2/extensions>
原始许可证：<https://github.com/microsoft/vscode/blob/1.145.2/LICENSE.txt>

以下保留版权及许可声明。

MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Tree-sitter 语法与高亮查询（#9）

以下 MIT 语法通过 Cargo 编译进 Rove，版本以 `Cargo.lock` 为准。

| 语法 | Cargo 版本 | 上游 |
|---|---|---|
| C# | `tree-sitter-c-sharp 0.23.5` | [tree-sitter/tree-sitter-c-sharp](https://github.com/tree-sitter/tree-sitter-c-sharp/tree/v0.23.5) |
| JSON / JSONC | `tree-sitter-json 0.24.8` | [tree-sitter/tree-sitter-json](https://github.com/tree-sitter/tree-sitter-json/tree/v0.24.8) |
| YAML | `tree-sitter-yaml 0.7.2` | [tree-sitter-grammars/tree-sitter-yaml](https://github.com/tree-sitter-grammars/tree-sitter-yaml/tree/v0.7.2) |
| XML | `tree-sitter-xml 0.7.0` | [tree-sitter-grammars/tree-sitter-xml](https://github.com/tree-sitter-grammars/tree-sitter-xml/tree/v0.7.0) |
| Markdown（块级和行内） | `tree-sitter-md 0.5.4` | [tree-sitter-grammars/tree-sitter-markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown/tree/v0.5.4) |

`crates/rove-syntax/src/highlight/queries/` 的高亮查询为 Rove 编写。
捕获名参考 [VS Code 内置 TextMate 语法](https://github.com/microsoft/vscode/tree/ffe08d127413e67baa2d3cf31a62bdfdcf218f71/extensions)
（`csharp`、`json`、`yaml`、`xml`、`markdown-basics`，Microsoft，MIT，许可见上文）。

Markdown 围栏注入查询的节点关系参考 [tree-sitter-md 0.5.4 的块级注入查询](https://github.com/tree-sitter-grammars/tree-sitter-markdown/blob/v0.5.4/tree-sitter-markdown/queries/injections.scm)（MIT）。

## VS Code 默认设置（#13）

`crates/rove-config/data/settings-catalog.json` 中的设置键名、VS Code 默认值与默认语言覆盖
（`"[csharp]": {...}` 等）取自 Microsoft VS Code **1.145.2** 导出的默认设置
（`docs/compat/vscode/default-settings.jsonc`），由 `scripts/compat/gen_settings_catalog.py`
按 `scripts/compat/rules.py` 过滤生成；说明文字不进入该文件。`rove.*` 设置项为 Rove 自有。

来源：<https://github.com/microsoft/vscode/tree/1.145.2>
原始许可证：MIT（Copyright (c) 2015 - present Microsoft Corporation，全文见上文“VS Code 语言配置”一节）。

## vscode-textmate 主题匹配（#10）

`crates/rove-theme/src/textmate.rs` 移植自 Microsoft
[vscode-textmate](https://github.com/microsoft/vscode-textmate) **9.3.2** 的 `src/theme.ts`
（scope 选择器解析、父 scope 与子代组合符、规则前缀树与特异性排序），其单元测试移植自
`src/tests/themes.test.ts`。原始许可证：MIT。

The MIT License (MIT)

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Codicons 产品图标（#16）

`crates/rove-ui/assets/codicons/codicon.ttf` 是 Microsoft [vscode-codicons](https://github.com/microsoft/vscode-codicons)
的图标字体（npm 包 `@vscode/codicons` 0.0.45，字体内部版本 1.15，SHA-256
`2bb558cb693451e73c28c33fe64aa89bc19b1a4b70f95948322c243f93476920`），取自本机 VS Code 扩展
GitLens 19.5.0 随附的未修改副本（`dist/webviews/codicon.ttf`，其 `package.json` 依赖 `@vscode/codicons` 0.0.45）。
`mapping.json` 由 `scripts/codicons/gen_mapping.py` 从字体 cmap 生成（图标名 → 码位）。

**署名（CC-BY-4.0 要求，不能只写一行许可名）：**

- 作品：**Visual Studio Code Icons（codicons）**，`@vscode/codicons` 0.0.45 的 `codicon.ttf`；
- 作者 / 版权：**© Microsoft Corporation**；
- 来源：<https://github.com/microsoft/vscode-codicons>；
- 许可：[Creative Commons Attribution 4.0 International（CC-BY-4.0）](https://creativecommons.org/licenses/by/4.0/)，
  许可全文见 <https://github.com/microsoft/vscode-codicons/blob/main/LICENSE>；
- 修改说明：**Rove 未修改字体文件本身**（逐字节相同，哈希见上），只读取其字形用于界面图标；
  `mapping.json`（图标名 → 码位）是 Rove 从字体的 cmap 生成的衍生数据。

同一份署名随发布包分发（本文件就在安装目录里），并可从「关于」对话框打开。

## 列表与快速输入组件（#18）

- `crates/rove-ui/src/list_widget/text_input.rs` 的单行输入框（含 IME 输入处理）改写自 GPUI 0.2.2 的
  `examples/input.rs`（Zed Industries，Apache License 2.0，许可证见 `vendor/gpui/LICENSE-APACHE`）。
- `crates/rove-ui/src/list_widget/registry.rs` 的颜色默认值、命令 ID、键位与 `when` 条件，以及
  `widget.rs` 中翻页、全选、展开/折叠等行为，参照 Microsoft VS Code 1.145（`listWidget.ts`、
  `listCommands.ts`、`colorRegistry` 的列表颜色，MIT，许可见上文）；为 Rove 重写，未复制源码。

## C# L1 声明索引（#37）

- C# 声明提取直接使用 #9 已登记的 `tree-sitter 0.25` 与 `tree-sitter-c-sharp 0.23.5`（MIT），没有复制额外的查询文件。
- `crates/rove-lang-cs/src/l1/names.rs` 的 `FxHasher` 按 [rustc-hash](https://github.com/rust-lang/rustc-hash)（MIT OR Apache-2.0）的 FxHash 思路（乘法-旋转散列与种子常量）重新实现，没有复制源码。

## 编辑核心模型（#46）

以下移植自 Microsoft VS Code **1.145.2**（提交 `c4453f8ea430795f941c1b2ff6c9321135a74a02`，MIT 许可，
版权与许可全文见上文“VS Code 默认主题与主题引擎”一节）：

- `crates/rove-buffer/src/edit/word.rs`：`src/vs/editor/common/cursor/cursorWordOperations.ts`
  （`WordOperations` / `WordPartOperations` 的单词边界、删除区间与选词）与
  `src/vs/editor/common/core/wordCharacterClassifier.ts`（字符分类）。
- `crates/rove-buffer/tests/edit_word_operations.rs`：逐例翻译自
  `src/vs/editor/contrib/wordOperations/test/browser/wordOperations.test.ts` 与
  `src/vs/editor/contrib/wordPartOperations/test/browser/wordPartOperations.test.ts`（用例文本与期望值原样保留），
  辅助函数改写自同目录的 `wordTestUtils.ts`。
- `crates/rove-buffer/src/edit/history.rs`：撤销分组规则取自 `src/vs/editor/common/cursor/cursorTypeEditOperations.ts`
  （`getTypingOperation`、`shouldPushStackElementBetween`）与 `cursorDeleteOperations.ts`；光标撤销取自
  `src/vs/editor/contrib/cursorUndo/browser/cursorUndo.ts`。
- `crates/rove-buffer/src/edit/selection.rs`：多光标合并取自 `src/vs/editor/common/cursor/cursorCollection.ts`
  （`_mergeCursorsIfNecessary`），可见列换算取自 `cursorColumns.ts` 与 `src/vs/base/common/strings.ts`
  （`isFullWidthCharacter`）。
- `crates/rove-buffer/src/edit/clipboard.rs`：`src/vs/editor/common/viewModel/viewModelImpl.ts`
  （`getPlainTextToCopy`）、`cursorDeleteOperations.ts`（`cut`）与 `cursorTypeEditOperations.ts`（`PasteOperation`）。
- `crates/rove-buffer/src/edit/rewrite.rs`：`src/vs/editor/contrib/indentation/browser/indentation.ts`
  （`getIndentationEditOperations`）。
- `crates/rove-buffer/src/edit/word.rs` 中 `editor.wordSegmenterLocales` 的分词停靠逻辑移植自
  `wordCharacterClassifier.ts`（`findPrevIntlWordBeforeOrAtOffset` / `findNextIntlWordAtOrAfterOffset`）。

可选 feature `intl-segmenter`（默认关闭）依赖 Unicode Consortium 的 ICU4X
[`icu_segmenter`](https://crates.io/crates/icu_segmenter) 2.4 及其编译进二进制的分词数据
（`icu_segmenter_data`：中日文词典、东南亚文字 LSTM 模型），许可为
[Unicode License v3](https://www.unicode.org/license.txt)（`Unicode-3.0`）。启用该 feature 发布时，
需按 #75 的分发清单附上该许可全文。

来源：<https://github.com/microsoft/vscode/tree/1.145.2>

## VS Code 模糊匹配与快速打开排序（#24）

`crates/rove-fuzzy` 逐条移植 Microsoft VS Code 的下列文件（MIT 许可，取自 `main` 分支提交
`7d28f73cae52e9fda9662bf6cdc815333b461720`，其中 `fuzzyScorer.ts` 最后一次修改的提交是
`f6c56f75d0f9ff0774c14303046f43acfde240f3`）：

- `src/vs/base/common/fuzzyScorer.ts` → `src/scorer.rs`：`scoreFuzzy`、`scoreFuzzy2`、`scoreItemFuzzy`、
  `compareItemsByFuzzyScore`、`prepareQuery`、`pieceToQuery`。
- `src/vs/base/common/filters.ts` 中被它用到的部分 → `src/filters.rs`（`fuzzyScore`、`createMatches`、
  `matchesPrefix`）与 `src/words.rs`（命令面板的 `WORD_FILTER`：`matchesWords`、`matchesContiguousSubString`）。
- `src/vs/base/common/comparers.ts` 中的 `compareAnything`、`compareByPrefix`、`compareFileNames` → `src/compare.rs`
  （没有 ICU：`Intl.Collator` 换成数字按数值、忽略大小写的近似比较，只影响分数与长度都相同之后的决胜）。
- `src/vs/base/test/common/fuzzyScorer.test.ts` → `src/tests.rs`：全部 68 个用例一对一移植，其中 1 个
  （校验 VS Code 内部 32 位 `hash` 缓存键）因实现不同只留带说明的 `#[ignore]` 空壳。

来源：<https://github.com/microsoft/vscode/tree/7d28f73cae52e9fda9662bf6cdc815333b461720>
原始许可证：<https://github.com/microsoft/vscode/blob/7d28f73cae52e9fda9662bf6cdc815333b461720/LICENSE.txt>
（MIT，Copyright (c) 2015 - present Microsoft Corporation，全文见上文“VS Code 语言配置”一节）。

## Markdown 预览的打包资源（#29）

以下文件位于 `crates/rove-ui/assets/markdown/vendor/`，由 `crates/rove-ui/build.rs` 在构建时 DEFLATE 压缩后嵌入 `rove.exe`，
运行时在内存里解压、经 WebView2 的资源请求事件返回，**不联网、不读安装目录**。原始许可证全文保存在同目录的
`LICENSES/` 下（不嵌入二进制）。合计约 4.29 MB（含 Rove 自写的 `bridge.js`、`rove.css`、`blocked-image.svg`）。

| 文件 | 来源 | 版本 | 许可 | SHA-256（前 16 位） |
|---|---|---|---|---|
| `markdown.css`、`highlight.css` | Microsoft VS Code `extensions/markdown-language-features/media/`（提交 `c4453f8ea430795f941c1b2ff6c9321135a74a02`） | 1.145.2 | MIT（`LICENSES/vscode.LICENSE`） | `31acba626be3846f`、`b73e0ecc7a5b9153` |
| `highlight.min.js` | highlight.js，npm 包 `@highlightjs/cdn-assets`（常用语言包） | 11.11.1 | BSD-3-Clause（`LICENSES/highlight.js.LICENSE`） | `c4a399dd6f488bc9` |
| `katex/katex.min.js`、`katex/katex.min.css`、`katex/fonts/*.woff2`（20 个） | KaTeX，npm 包 `katex` 的 `dist/` | 0.16.25 | MIT（`LICENSES/KaTeX.LICENSE`） | `76d534cf11670670`、`0c8126645bb983a7` |
| `mermaid.min.js` | Mermaid，npm 包 `mermaid` 的 `dist/mermaid.min.js` | 11.17.2 | MIT（`LICENSES/mermaid.LICENSE`）；压缩包内嵌的依赖（d3 等）的许可注释保留在文件内，完整分发清单由 #75 维护 | `581ed7d74bd90488` |

`highlight.css` 文件头注明其改编自 highlight.js 的 `vs2015.css`。

Rust 依赖（只记直接依赖，许可以上游 crate 为准）：`pulldown-cmark` 0.13（MIT）、`ammonia` 4（MIT OR Apache-2.0）、
`webview2-com` 0.38（MIT）、`windows` 0.61 与 `raw-window-handle` 0.6（MIT OR Apache-2.0）。WebView2 运行时是
Windows 的系统组件，不随 Rove 分发，也不由 Rove 下载。

## VS Code 着色测试数据（#11）

`tests/golden/colorize-fixtures/` 与 `tests/golden/colorize-results/` 原样取自 Microsoft VS Code
**1.145.2**（MIT 许可）的 `extensions/vscode-colorize-tests/test/`：前者是各语言的测试源文件，
后者是每个 token 在十个内置主题下的期望颜色。Rove 用它们做着色黄金测试（逐字符比对最终颜色），
**只在测试里使用，不进二进制**。按文件名取用与 Rove 语言清单相关的部分，文件内容不做加工；
清单、哈希与升级步骤见该目录的 `README.md`。

来源：<https://github.com/microsoft/vscode/tree/1.145.2/extensions/vscode-colorize-tests>
## 注释切换、自动缩进、自动闭合与回车规则（#53）

`crates/rove-ui/src/code_view/autoedit/` 逐段移植 Microsoft VS Code 1.145.2（MIT）的下列文件：

- `src/vs/editor/common/languages/autoIndent.ts`（`IndentRulesSupport`、`getInheritIndentForLine`、
  `getGoodIndentForLine`、`getIndentForEnter`、`getIndentActionForType`）、
  `src/vs/editor/common/commands/shiftCommand.ts`（`shiftIndent` / `unshiftIndent`）与
  `src/vs/editor/common/model/textModel.ts`（`normalizeIndentation`）→ `autoedit/indent.rs`；
  `src/vs/editor/contrib/indentation/browser/indentation.ts` 的 `AutoIndentOnPaste` → 同文件的 `reindent_lines`。
- `src/vs/editor/common/languages/supports/onEnter.ts`、`enterAction.ts` 与
  `src/vs/editor/common/cursor/cursorTypeOperations.ts` 的 `_enter` → `autoedit/on_enter.rs`。
- `src/vs/editor/common/cursor/cursorTypeOperations.ts`（`_findAutoClosingPairOpen`、
  `_isAutoClosingOpenCharType`、`_isAutoClosingOvertype`、`_isSurroundSelectionType`）与
  `cursorDeleteOperations.ts`（`_isAutoClosingPairDelete`）→ `autoedit/autoclose.rs`。
- `src/vs/editor/contrib/comment/browser/lineCommentCommand.ts`、`blockCommentCommand.ts` → `autoedit/comments.rs`。

单元测试移植自 `src/vs/editor/test/common/modes/supports/onEnter.test.ts` 的 `uses brackets`（16 例）与
`uses regExpRules`（24 例），夹具 `javascriptOnEnterRules.ts` 与 `javascriptIndentationRules.ts` 的正则一并照抄。

来源：<https://github.com/microsoft/vscode/tree/1.145.2>
原始许可证：<https://github.com/microsoft/vscode/blob/1.145.2/LICENSE.txt>
（MIT，Copyright (c) 2015 - present Microsoft Corporation，全文见上文“VS Code 语言配置”一节）。

## Seti 文件图标主题（#69）

VS Code 内置的 **Seti 文件图标主题**（`extensions/theme-seti/`：`icons/seti.woff`、
`icons/vs-seti-icon-theme.json`）与它的上游 [jesseweed/seti-ui](https://github.com/jesseweed/seti-ui)
都是 MIT 许可（seti-ui © Jesse Weed；VS Code 的打包副本 © Microsoft Corporation，MIT 全文见上文
「VS Code 默认主题与主题引擎」一节）。

Rove 只读加载该主题，不修改字体与图标定义。**#69 合并前仓库里还没有这些文件**；
合并时由 #69 在这里补上文件清单、版本与哈希（做法与上面的 codicons 一节相同）。
发布包的体积分项（`packaging/package-contents.toml` 的 `[[embedded]]`）也要同时加上一条。

## Rust 依赖的完整清单（#76）

见 [`packaging/licenses/rust-dependencies.md`](packaging/licenses/rust-dependencies.md)：
由 `cargo xtask licenses --write` 从 `cargo metadata --all-features` 生成（覆盖全部 feature 的
依赖图，因此也包含只在构建、基准与测试时使用、不进 `rove.exe` 的包），CI 校验它与依赖图一致。
允许的许可及理由在 [`packaging/licenses/allowed.toml`](packaging/licenses/allowed.toml)。

其中需要特别说明的几类：

- **MPL-2.0**（`dwrote`、`cssparser`、`dtoa-short`、`option-ext`、`cbindgen`）：文件级 copyleft。
  Rove 不修改这些 crate 的源码，只需随附本声明并指出其源码可从 crates.io 获取。
- **Unicode-3.0**（ICU4X 的 `icu_*`、`unicode-ident` 等）：[Unicode License v3](https://www.unicode.org/license.txt)。
  可选 feature `intl-segmenter`（默认关闭）启用时会把 ICU4X 的分词数据编进二进制，
  届时需随发布包附上该许可全文。
- **Apache-2.0 单授权**：`gpui`（见上文）。
