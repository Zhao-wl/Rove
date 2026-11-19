; Protobuf has no built-in VS Code TextMate grammar. Use general theme scopes.
["syntax" "edition" "import" "package" "message" "enum" "service" "rpc" "returns"
 "stream" "repeated" "optional" "required" "oneof" "option" "reserved" "to" "max"
 "extend" "extensions" "group" "map" "public" "weak" "export" "local"] @keyword
[(type) (key_type)] @storage.type
(message_or_enum_type) @entity.name.type
[(message_name) (enum_name) (service_name) (extension_name)] @entity.name.type
(rpc_name) @entity.name.function
(string) @string
(escape_sequence) @constant.character.escape
(comment) @comment
[(int_lit) (float_lit)] @constant.numeric
(bool) @constant.language
