; Scope names follow the VS Code Lua TextMate grammar.
(identifier) @variable.other.lua
((identifier) @variable.language.self.lua
 (#eq? @variable.language.self.lua "self"))
["do" "else" "elseif" "end" "for" "function" "if" "in" "repeat" "return"
 "then" "until" "while" (break_statement)] @keyword.control.lua
"goto" @keyword.control.goto.lua
"local" @keyword.local.lua
"global" @keyword.global.lua
["and" "or" "not"] @keyword.operator.logical.lua
["+" "-" "*" "/" "//" "%" "^" "#" "&" "|" "~" "<<" ">>" ".." "<" ">" "<="
 ">=" "==" "~=" "="] @keyword.operator.lua
[(nil) (true) (false) (vararg_expression)] @constant.language.lua
(number) @constant.numeric.float.lua
((number) @constant.numeric.integer.hexadecimal.lua
 (#match? @constant.numeric.integer.hexadecimal.lua "^0[xX][0-9A-Fa-f]+$"))
((number) @constant.numeric.float.hexadecimal.lua
 (#match? @constant.numeric.float.hexadecimal.lua "^0[xX].*[.pP]"))

(comment start: "--") @comment.line.double-dash.lua
(comment start: "[[") @comment.block.lua
(hash_bang_line) @comment.line.shebang.lua
(string start: "\"") @string.quoted.double.lua
(string start: "'") @string.quoted.single.lua
(string start: "[[") @string.quoted.other.multiline.lua
(escape_sequence) @constant.character.escape.lua

(function_declaration name: (identifier) @entity.name.function.lua)
(function_declaration name: (dot_index_expression field: (identifier) @entity.name.function.lua))
(function_declaration name: (method_index_expression method: (identifier) @entity.name.function.lua))
(parameters name: (identifier) @variable.parameter.function.lua)
(parameters "(" @punctuation.definition.parameters.begin.lua)
(parameters ")" @punctuation.definition.parameters.finish.lua)
(parameters "," @punctuation.separator.arguments.lua)
(function_call name: (identifier) @support.function.any-method.lua)
(function_call name: (method_index_expression method: (identifier) @support.function.any-method.lua))
((function_call name: (identifier) @support.function.lua)
 (#any-of? @support.function.lua "assert" "collectgarbage" "dofile" "error" "getmetatable"
   "ipairs" "load" "loadfile" "next" "pairs" "pcall" "print" "rawequal" "rawget" "rawlen"
   "rawset" "require" "select" "setmetatable" "tonumber" "tostring" "type" "xpcall"))
((function_call name: (dot_index_expression table: (identifier)) @support.function.library.lua)
 (#match? @support.function.library.lua "^(coroutine|debug|io|math|os|package|string|table|utf8)[.]"))
