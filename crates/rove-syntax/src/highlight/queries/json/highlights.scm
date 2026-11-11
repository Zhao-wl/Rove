
(string) @string.quoted.double.json
(string "\"" @punctuation.definition.string.begin.json . (string_content)? . "\"" @punctuation.definition.string.end.json)
(pair key: (string) @string.json @support.type.property-name.json)
(pair key: (string . "\"" @punctuation.support.type.property-name.begin.json))
(pair key: (string "\"" @punctuation.support.type.property-name.end.json .))
(escape_sequence) @constant.character.escape.json
(number) @constant.numeric.json
[(true) (false) (null)] @constant.language.json
((comment) @comment.block.json (#match? @comment.block.json "^/\\*"))
((comment) @comment.line.double-slash.js (#match? @comment.line.double-slash.js "^//"))
"{" @punctuation.definition.dictionary.begin.json
"}" @punctuation.definition.dictionary.end.json
"[" @punctuation.definition.array.begin.json
"]" @punctuation.definition.array.end.json
":" @punctuation.separator.dictionary.key-value.json
(object "," @punctuation.separator.dictionary.pair.json)
(array "," @punctuation.separator.array.json)
