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
; These literal rules share a single predefined_type query at runtime.
((predefined_type) @keyword.type.bool.cs (#rove-predefined-type? @keyword.type.bool.cs "bool"))
((predefined_type) @keyword.type.byte.cs (#rove-predefined-type? @keyword.type.byte.cs "byte"))
((predefined_type) @keyword.type.char.cs (#rove-predefined-type? @keyword.type.char.cs "char"))
((predefined_type) @keyword.type.decimal.cs (#rove-predefined-type? @keyword.type.decimal.cs "decimal"))
((predefined_type) @keyword.type.double.cs (#rove-predefined-type? @keyword.type.double.cs "double"))
((predefined_type) @keyword.type.float.cs (#rove-predefined-type? @keyword.type.float.cs "float"))
((predefined_type) @keyword.type.int.cs (#rove-predefined-type? @keyword.type.int.cs "int"))
((predefined_type) @keyword.type.long.cs (#rove-predefined-type? @keyword.type.long.cs "long"))
((predefined_type) @keyword.type.object.cs (#rove-predefined-type? @keyword.type.object.cs "object"))
((predefined_type) @keyword.type.sbyte.cs (#rove-predefined-type? @keyword.type.sbyte.cs "sbyte"))
((predefined_type) @keyword.type.short.cs (#rove-predefined-type? @keyword.type.short.cs "short"))
((predefined_type) @keyword.type.string.cs (#rove-predefined-type? @keyword.type.string.cs "string"))
((predefined_type) @keyword.type.uint.cs (#rove-predefined-type? @keyword.type.uint.cs "uint"))
((predefined_type) @keyword.type.ulong.cs (#rove-predefined-type? @keyword.type.ulong.cs "ulong"))
((predefined_type) @keyword.type.ushort.cs (#rove-predefined-type? @keyword.type.ushort.cs "ushort"))
((predefined_type) @keyword.type.void.cs (#rove-predefined-type? @keyword.type.void.cs "void"))
((predefined_type) @keyword.type.nint.cs (#rove-predefined-type? @keyword.type.nint.cs "nint"))
((predefined_type) @keyword.type.nuint.cs (#rove-predefined-type? @keyword.type.nuint.cs "nuint"))
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
(using_directive [(identifier) (qualified_name)] @entity.name.type.namespace.cs)
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
; 类型引用：写在 type 字段上的标识符与泛型名（变量声明、参数、返回值、new、强制转换……）。
(_ type: (identifier) @entity.name.type.cs)
(_ type: (generic_name (identifier) @entity.name.type.cs))
(base_list (identifier) @entity.name.type.cs)
(base_list (generic_name (identifier) @entity.name.type.cs))
(type_argument_list (identifier) @entity.name.type.cs)
; 表达式里的普通标识符：VS Code 的 C# 语法统一给 variable.other.readwrite.cs。
; 这里按位置列出而不是一网打尽，免得给声明名、类型名也套上变量 scope。
(binary_expression left: (identifier) @variable.other.readwrite.cs)
(binary_expression right: (identifier) @variable.other.readwrite.cs)
(prefix_unary_expression (identifier) @variable.other.readwrite.cs)
(postfix_unary_expression (identifier) @variable.other.readwrite.cs)
(parenthesized_expression (identifier) @variable.other.readwrite.cs)
(assignment_expression left: (identifier) @variable.other.readwrite.cs)
(assignment_expression right: (identifier) @variable.other.readwrite.cs)
(argument (identifier) @variable.other.readwrite.cs)
(return_statement (identifier) @variable.other.readwrite.cs)
(interpolation (identifier) @variable.other.readwrite.cs)
(element_access_expression expression: (identifier) @variable.other.readwrite.cs)
(foreach_statement right: (identifier) @variable.other.readwrite.cs)
(foreach_statement left: (identifier) @entity.name.variable.local.cs)
(conditional_expression (identifier) @variable.other.readwrite.cs)
; 成员访问：`System.Console.WriteLine` 的前两段是对象与属性，最后一段按调用处理。
(member_access_expression expression: (identifier) @variable.other.object.cs)
(member_access_expression name: (identifier) @variable.other.object.property.cs)
; 调用：放在成员访问之后，作为更靠内的一层覆盖掉上面的属性 scope。
(invocation_expression function: (identifier) @entity.name.function.cs)
(invocation_expression function: (generic_name (identifier) @entity.name.function.cs))
(invocation_expression function: (member_access_expression name: (identifier) @entity.name.function.cs))
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
; 比较运算符限定在二元表达式里：`<`/`>` 也是泛型实参列表的括号，那里不是运算符。
(binary_expression ["==" "!=" "<" ">" "<=" ">="] @keyword.operator.comparison.cs)
["&&" "||" "!"] @keyword.operator.logical.cs
"=>" @keyword.operator.arrow.cs
