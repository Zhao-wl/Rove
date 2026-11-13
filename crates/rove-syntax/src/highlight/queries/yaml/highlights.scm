
(boolean_scalar) @constant.language.boolean.yaml
(null_scalar) @constant.language.null.yaml
(integer_scalar) @constant.numeric.integer.decimal.yaml
((integer_scalar) @constant.numeric.integer.hexadecimal.yaml (#match? @constant.numeric.integer.hexadecimal.yaml "^[-+]?0[xX]"))
((integer_scalar) @constant.numeric.integer.octal.yaml (#match? @constant.numeric.integer.octal.yaml "^[-+]?0o"))
(float_scalar) @constant.numeric.float.yaml
(double_quote_scalar) @string.quoted.double.yaml
(single_quote_scalar) @string.quoted.single.yaml
(block_scalar) @string.unquoted.block.yaml
(string_scalar) @string.unquoted.plain.out.yaml
(block_mapping_pair key: (flow_node (plain_scalar (string_scalar) @meta.map.key.yaml @string.unquoted.plain.yaml @entity.name.tag.yaml)))
(flow_mapping (_ key: (flow_node (plain_scalar (string_scalar) @meta.flow.map.key.yaml @string.unquoted.plain.in.yaml @entity.name.tag.yaml))))
(block_mapping_pair key: (flow_node [(double_quote_scalar) (single_quote_scalar)] @entity.name.tag.yaml))
(flow_mapping (_ key: (flow_node [(double_quote_scalar) (single_quote_scalar)] @entity.name.tag.yaml)))
; 块序列整体带 meta.block.sequence.yaml：有的主题（Dark/Light 2026）给 meta.block 定了前景色，
; 少了这一层，序列里没有被更内层规则命中的字符就会用成编辑器前景色。
(block_sequence) @meta.block.sequence.yaml
(comment) @comment.line.number-sign.yaml
; 锚点与别名整体是 keyword.control.flow.*，`&`/`*` 与名字再往里一层（与 VS Code 一致）。
(anchor) @keyword.control.flow.anchor.yaml
(alias) @keyword.control.flow.alias.yaml
(anchor_name) @variable.other.anchor.yaml
(alias_name) @variable.other.alias.yaml
"&" @punctuation.definition.anchor.yaml
"*" @punctuation.definition.alias.yaml
(tag) @storage.type.tag.shorthand.yaml
(yaml_directive) @keyword.other.directive.yaml.yaml
(tag_directive) @keyword.other.directive.tag.yaml
(reserved_directive) @keyword.other.directive.other.yaml
"---" @entity.other.document.begin.yaml
"..." @entity.other.document.end.yaml
"-" @punctuation.definition.block.sequence.item.yaml
":" @punctuation.separator.map.value.yaml
"?" @punctuation.definition.map.key.yaml
">" @keyword.control.flow.block-scalar.folded.yaml
"|" @keyword.control.flow.block-scalar.literal.yaml
"[" @punctuation.definition.sequence.begin.yaml
"]" @punctuation.definition.sequence.end.yaml
"{" @punctuation.definition.mapping.begin.yaml
"}" @punctuation.definition.mapping.end.yaml
(flow_mapping "," @punctuation.separator.mapping.yaml)
(flow_sequence "," @punctuation.separator.sequence.yaml)
