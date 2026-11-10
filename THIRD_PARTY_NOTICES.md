# 第三方来源

## Dark 2026 主题子集

`crates/rove-ui/src/skeleton/appearance.rs` 中的颜色选自 Microsoft VS Code 的
[2026-dark.json](https://github.com/microsoft/vscode/blob/main/extensions/theme-defaults/themes/2026-dark.json)
（2026-11-10 核对，继承 dark_modern / dark_plus / dark_vs）。只包含骨架需要的界面颜色与四种 token 颜色。
原始许可证：[MIT](https://github.com/microsoft/vscode/blob/main/LICENSE.txt)。
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
