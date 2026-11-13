; 语法折叠区间（#118）：标题分节、围栏代码块与 HTML 块，与 VS Code 的 Markdown 折叠一致。
; section 末尾的空行在 Rust 侧裁掉，折叠不会把下一个标题前的空行一起藏起来。
[(section) (fenced_code_block) (html_block)] @fold
