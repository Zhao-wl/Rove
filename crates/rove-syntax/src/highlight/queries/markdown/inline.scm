
(emphasis) @markup.italic.markdown
(strong_emphasis) @markup.bold.markdown
(strikethrough) @markup.strikethrough.markdown
(emphasis (emphasis_delimiter) @punctuation.definition.italic.markdown)
(strong_emphasis (emphasis_delimiter) @punctuation.definition.bold.markdown)
(code_span) @markup.inline.raw.string.markdown
(code_span_delimiter) @punctuation.definition.raw.markdown
(backslash_escape) @constant.character.escape.markdown
(link_text) @string.other.link.title.markdown
(link_destination) @markup.underline.link.markdown
(link_title) @string.other.link.description.title.markdown
(uri_autolink) @markup.underline.link.markdown
; 行内 HTML 注释：与块级 html_block 的处理一致。
((html_tag) @comment.block.html (#match? @comment.block.html "^<!--"))
; 数学公式：VS Code 把 $...$ / $$...$$ 的内容注入 TeX 语法，外层是 meta.embedded.math.markdown。
; Rove 没有 TeX 语法，只给出同样的外层 scope，内部的 TeX 命令不再细分。
(latex_block) @markup.math.inline.markdown @meta.embedded.math.markdown
(latex_block (latex_span_delimiter) @punctuation.definition.math.markdown)
