
(atx_heading (atx_h1_marker) (inline) @markup.heading.markdown @heading.1.markdown @entity.name.section.markdown)
(atx_heading (atx_h2_marker) (inline) @markup.heading.markdown @heading.2.markdown @entity.name.section.markdown)
(atx_heading (atx_h3_marker) (inline) @markup.heading.markdown @heading.3.markdown @entity.name.section.markdown)
(atx_heading (atx_h4_marker) (inline) @markup.heading.markdown @heading.4.markdown @entity.name.section.markdown)
(atx_heading (atx_h5_marker) (inline) @markup.heading.markdown @heading.5.markdown @entity.name.section.markdown)
(atx_heading (atx_h6_marker) (inline) @markup.heading.markdown @heading.6.markdown @entity.name.section.markdown)
(setext_heading (paragraph) @markup.heading.setext.1.markdown (setext_h1_underline))
(setext_heading (paragraph) @markup.heading.setext.2.markdown (setext_h2_underline))
[
  (atx_h1_marker) (atx_h2_marker) (atx_h3_marker)
  (atx_h4_marker) (atx_h5_marker) (atx_h6_marker)
  (setext_h1_underline) (setext_h2_underline)
] @punctuation.definition.heading.markdown
(fenced_code_block_delimiter) @punctuation.definition.markdown
(info_string (language) @fenced_code.block.language.markdown)
(indented_code_block) @markup.raw.block.markdown
(block_quote_marker) @punctuation.definition.quote.begin.markdown
[(list_marker_plus) (list_marker_minus) (list_marker_star) (list_marker_dot) (list_marker_parenthesis)] @punctuation.definition.list.begin.markdown
(thematic_break) @meta.separator.markdown
