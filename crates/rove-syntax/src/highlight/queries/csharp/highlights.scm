(modifier "abstract" @storage.modifier.abstract.cs)
(modifier "async" @storage.modifier.async.cs)
(modifier "const" @storage.modifier.const.cs)
(modifier "extern" @storage.modifier.extern.cs)
(modifier "file" @storage.modifier.file.cs)
(modifier "fixed" @storage.modifier.fixed.cs)
(modifier "internal" @storage.modifier.internal.cs)
(modifier "new" @storage.modifier.new.cs)
(modifier "override" @storage.modifier.override.cs)
(modifier "partial" @storage.modifier.partial.cs)
(modifier "private" @storage.modifier.private.cs)
(modifier "protected" @storage.modifier.protected.cs)
(modifier "public" @storage.modifier.public.cs)
(modifier "readonly" @storage.modifier.readonly.cs)
(modifier "required" @storage.modifier.required.cs)
(modifier "sealed" @storage.modifier.sealed.cs)
(modifier "static" @storage.modifier.static.cs)
(modifier "unsafe" @storage.modifier.unsafe.cs)
(modifier "virtual" @storage.modifier.virtual.cs)
(modifier "volatile" @storage.modifier.volatile.cs)
((predefined_type) @keyword.type.bool.cs (#eq? @keyword.type.bool.cs "bool"))
((predefined_type) @keyword.type.byte.cs (#eq? @keyword.type.byte.cs "byte"))
((predefined_type) @keyword.type.char.cs (#eq? @keyword.type.char.cs "char"))
((predefined_type) @keyword.type.decimal.cs (#eq? @keyword.type.decimal.cs "decimal"))
((predefined_type) @keyword.type.double.cs (#eq? @keyword.type.double.cs "double"))
((predefined_type) @keyword.type.float.cs (#eq? @keyword.type.float.cs "float"))
((predefined_type) @keyword.type.int.cs (#eq? @keyword.type.int.cs "int"))
((predefined_type) @keyword.type.long.cs (#eq? @keyword.type.long.cs "long"))
((predefined_type) @keyword.type.object.cs (#eq? @keyword.type.object.cs "object"))
((predefined_type) @keyword.type.sbyte.cs (#eq? @keyword.type.sbyte.cs "sbyte"))
((predefined_type) @keyword.type.short.cs (#eq? @keyword.type.short.cs "short"))
((predefined_type) @keyword.type.string.cs (#eq? @keyword.type.string.cs "string"))
((predefined_type) @keyword.type.uint.cs (#eq? @keyword.type.uint.cs "uint"))
((predefined_type) @keyword.type.ulong.cs (#eq? @keyword.type.ulong.cs "ulong"))
((predefined_type) @keyword.type.ushort.cs (#eq? @keyword.type.ushort.cs "ushort"))
((predefined_type) @keyword.type.void.cs (#eq? @keyword.type.void.cs "void"))
((predefined_type) @keyword.type.nint.cs (#eq? @keyword.type.nint.cs "nint"))
((predefined_type) @keyword.type.nuint.cs (#eq? @keyword.type.nuint.cs "nuint"))
"class" @storage.type.class.cs
"struct" @storage.type.struct.cs
"interface" @storage.type.interface.cs
"enum" @storage.type.enum.cs
"delegate" @storage.type.delegate.cs
"record" @storage.type.record.cs
"namespace" @storage.type.namespace.cs
"event" @storage.type.event.cs
"operator" @storage.type.operator.cs
"if" @keyword.control.conditional.if.cs
"else" @keyword.control.conditional.else.cs
"switch" @keyword.control.conditional.switch.cs
"case" @keyword.control.conditional.case.cs
"when" @keyword.control.conditional.when.cs
"return" @keyword.control.flow.return.cs
"throw" @keyword.control.flow.throw.cs
"break" @keyword.control.flow.break.cs
"goto" @keyword.control.flow.goto.cs
"yield" @keyword.control.flow.yield.cs
"try" @keyword.control.exception.try.cs
"catch" @keyword.control.exception.catch.cs
"finally" @keyword.control.exception.finally.cs
"do" @keyword.control.loop.do.cs
"for" @keyword.control.loop.for.cs
"foreach" @keyword.control.loop.foreach.cs
"while" @keyword.control.loop.while.cs
"in" @keyword.control.loop.in.cs
"new" @keyword.operator.expression.new.cs
"await" @keyword.operator.expression.await.cs
"as" @keyword.operator.expression.as.cs
"default" @keyword.operator.expression.default.cs
"get" @storage.type.accessor.get.cs
"set" @storage.type.accessor.set.cs
"init" @storage.type.accessor.init.cs
"add" @storage.type.accessor.add.cs
"remove" @storage.type.accessor.remove.cs

(using_directive "using" @keyword.other.directive.using.cs)
(using_statement "using" @keyword.control.context.using.cs)
"lock" @keyword.control.context.lock.cs
(implicit_type) @storage.type.var.cs
"this" @variable.language.this.cs
"true" @constant.language.boolean.true.cs
"false" @constant.language.boolean.false.cs
(null_literal) @constant.language.null.cs
[(integer_literal) (real_literal)] @constant.numeric.decimal.cs
((integer_literal) @constant.numeric.hex.cs (#match? @constant.numeric.hex.cs "^0[xX]"))
((integer_literal) @constant.numeric.binary.cs (#match? @constant.numeric.binary.cs "^0[bB]"))
[(string_literal) (verbatim_string_literal) (raw_string_literal) (interpolated_string_expression)] @string.quoted.double.cs
(character_literal) @string.quoted.single.cs
(escape_sequence) @constant.character.escape.cs
((comment) @comment.line.double-slash.cs (#match? @comment.line.double-slash.cs "^//"))
((comment) @comment.block.cs (#match? @comment.block.cs "^/\\*"))
(class_declaration name: (identifier) @entity.name.type.class.cs)
(struct_declaration name: (identifier) @entity.name.type.struct.cs)
(interface_declaration name: (identifier) @entity.name.type.interface.cs)
(enum_declaration name: (identifier) @entity.name.type.enum.cs)
(delegate_declaration name: (identifier) @entity.name.type.delegate.cs)
(namespace_declaration name: (_) @entity.name.type.namespace.cs)
(file_scoped_namespace_declaration name: (_) @entity.name.type.namespace.cs)
(method_declaration name: (identifier) @entity.name.function.cs)
(constructor_declaration name: (identifier) @entity.name.function.cs)
(local_function_statement name: (identifier) @entity.name.function.cs)
(parameter name: (identifier) @entity.name.variable.parameter.cs)
(property_declaration name: (identifier) @entity.name.variable.property.cs)
(field_declaration (variable_declaration (variable_declarator name: (identifier) @entity.name.variable.field.cs)))
(local_declaration_statement (variable_declaration (variable_declarator name: (identifier) @entity.name.variable.local.cs)))
(enum_member_declaration name: (identifier) @entity.name.variable.enum-member.cs)
(type_parameter name: (identifier) @entity.name.type.type-parameter.cs)
(attribute name: (_) @entity.other.attribute-name.cs)
"{" @punctuation.curlybrace.open.cs
"}" @punctuation.curlybrace.close.cs
"(" @punctuation.parenthesis.open.cs
")" @punctuation.parenthesis.close.cs
"[" @punctuation.squarebracket.open.cs
"]" @punctuation.squarebracket.close.cs
";" @punctuation.terminator.statement.cs
"," @punctuation.separator.comma.cs
"." @punctuation.accessor.cs
"=" @keyword.operator.assignment.cs
["+" "-" "*" "/" "%"] @keyword.operator.arithmetic.cs
["==" "!=" "<" ">" "<=" ">="] @keyword.operator.comparison.cs
["&&" "||" "!"] @keyword.operator.logical.cs
"=>" @keyword.operator.arrow.cs
