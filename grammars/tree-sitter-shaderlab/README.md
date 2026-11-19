## 语法构建

该语法由 Rove 编写并以 MIT 许可发布。外层结构参考 Unity 的 [ShaderLab 语法](https://docs.unity3d.com/Manual/SL-Shader.html)。程序块里的 HLSL 使用 Rove 的 HLSL 语法和查询。

`src/parser.c` 及 `src/tree_sitter/` 由 Tree-sitter CLI 0.25.10 生成。修改 `grammar.js` 或扫描器后，在本目录运行 `tree-sitter generate` 和 `tree-sitter test`。Rove 的构建直接编译提交的 C 源码。
