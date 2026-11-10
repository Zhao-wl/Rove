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
