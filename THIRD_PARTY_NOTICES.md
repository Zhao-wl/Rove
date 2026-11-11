# 第三方来源

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

许可：[Creative Commons Attribution 4.0 International（CC-BY-4.0）](https://creativecommons.org/licenses/by/4.0/)，
© Microsoft Corporation。Rove 未修改字体本身，只读取其字形用于界面图标。

## 列表与快速输入组件（#18）

- `crates/rove-ui/src/list_widget/text_input.rs` 的单行输入框（含 IME 输入处理）改写自 GPUI 0.2.2 的
  `examples/input.rs`（Zed Industries，Apache License 2.0，许可证见 `vendor/gpui/LICENSE-APACHE`）。
- `crates/rove-ui/src/list_widget/registry.rs` 的颜色默认值、命令 ID、键位与 `when` 条件，以及
  `widget.rs` 中翻页、全选、展开/折叠等行为，参照 Microsoft VS Code 1.145（`listWidget.ts`、
  `listCommands.ts`、`colorRegistry` 的列表颜色，MIT，许可见上文）；为 Rove 重写，未复制源码。
