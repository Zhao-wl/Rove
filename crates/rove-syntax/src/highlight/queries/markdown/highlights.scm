; 标题整体带 markup.heading.markdown：VS Code 的 TextMate 语法把 `#` 标记、标记后的空格
; 和行尾的 `###` 都放在标题 scope 里，只有标题文字另外加 entity.name.section.markdown。
((atx_heading (atx_h1_marker)) @markup.heading.markdown @heading.1.markdown)
((atx_heading (atx_h2_marker)) @markup.heading.markdown @heading.2.markdown)
((atx_heading (atx_h3_marker)) @markup.heading.markdown @heading.3.markdown)
((atx_heading (atx_h4_marker)) @markup.heading.markdown @heading.4.markdown)
((atx_heading (atx_h5_marker)) @markup.heading.markdown @heading.5.markdown)
((atx_heading (atx_h6_marker)) @markup.heading.markdown @heading.6.markdown)
(atx_heading (inline) @entity.name.section.markdown)
; setext 标题：VS Code 只把下划线行算作标题，标题文字仍然是段落。
(setext_heading (setext_h1_underline) @markup.heading.setext.1.markdown)
(setext_heading (setext_h2_underline) @markup.heading.setext.2.markdown)
[
  (atx_h1_marker) (atx_h2_marker) (atx_h3_marker)
  (atx_h4_marker) (atx_h5_marker) (atx_h6_marker)
] @punctuation.definition.heading.markdown
(fenced_code_block) @markup.fenced_code.block.markdown
(fenced_code_block_delimiter) @punctuation.definition.markdown
(info_string (language) @fenced_code.block.language.markdown)
(indented_code_block) @markup.raw.block.markdown
(block_quote) @markup.quote.markdown
(block_quote_marker) @punctuation.definition.quote.begin.markdown
; HTML 注释：tree-sitter-md 只给出整块 html_block，这里按 VS Code 的 comment.block.html 着色；
; 其余 HTML 标签不拆分（Rove 不内置 HTML 语法）。
((html_block) @comment.block.html (#match? @comment.block.html "^<!--"))
[(list_marker_plus) (list_marker_minus) (list_marker_star) (list_marker_dot) (list_marker_parenthesis)] @punctuation.definition.list.begin.markdown
(thematic_break) @meta.separator.markdown
