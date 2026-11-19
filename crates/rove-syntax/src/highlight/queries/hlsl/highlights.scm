; Scope names follow the VS Code HLSL TextMate grammar.
((comment) @comment.line.double-slash.hlsl
 (#match? @comment.line.double-slash.hlsl "^//"))
((comment) @comment.line.block.hlsl
 (#match? @comment.line.block.hlsl "^/[*]"))

(number_literal) @constant.numeric.decimal.hlsl
((number_literal) @constant.numeric.hex.hlsl
 (#match? @constant.numeric.hex.hlsl "^0[xX]"))
[(true) (false)] @constant.language.hlsl
[(string_literal) (system_lib_string)] @string.quoted.double.hlsl
(escape_sequence) @constant.character.escape.hlsl

["#define" "#elif" "#else" "#endif" "#if" "#ifdef" "#ifndef" "#include"
 (preproc_directive)] @keyword.preprocessor.hlsl
["break" "case" "continue" "default" "discard" "do" "else" "for" "if"
 "return" "switch" "while"] @keyword.control.hlsl
"typedef" @keyword.typealias.hlsl
["struct" "class" "enum" "cbuffer"] @storage.type.structured.hlsl
["in" "inout" "out" "const" "extern" "static" "uniform" "volatile"
 "groupshared" "shared" "globallycoherent" "precise"] @storage.modifier.hlsl
["row_major" "column_major"] @storage.modifier.float.hlsl
["centroid" "linear" "nointerpolation" "noperspective" "sample"] @storage.modifier.interpolation.hlsl
["point" "line" "lineadj" "triangle" "triangleadj"] @storage.modifier.geometryshader.hlsl

(primitive_type) @storage.type.basic.hlsl
((type_identifier) @storage.type.basic.hlsl
 (#match? @storage.type.basic.hlsl "^(bool|int|uint|dword|half|float|double|min(10|16)float|min(12|16)int|min16uint)([1-4](x[1-4])?)?$"))
((type_identifier) @support.type.texture.hlsl
 (#match? @support.type.texture.hlsl "^Texture(1D|2D|3D|Cube)(MS)?(Array)?$"))
((type_identifier) @support.type.object.rw.hlsl
 (#match? @support.type.object.rw.hlsl "^(RW|Append|Consume)(Texture[123]D(Array)?|Buffer|ByteAddressBuffer|StructuredBuffer)$"))
((type_identifier) @support.type.sampler.hlsl
 (#any-of? @support.type.sampler.hlsl "SamplerState" "SamplerComparisonState"))
((type_identifier) @support.type.object.hlsl
 (#any-of? @support.type.object.hlsl "Buffer" "StructuredBuffer" "ByteAddressBuffer"))

(function_declarator declarator: (identifier) @support.function.hlsl)
((call_expression function: (identifier) @support.function.hlsl)
 (#any-of? @support.function.hlsl "abs" "acos" "all" "any" "asin" "atan" "atan2"
   "ceil" "clamp" "clip" "cos" "cross" "ddx" "ddy" "distance" "dot" "exp" "floor"
   "frac" "fwidth" "length" "lerp" "log" "max" "min" "mul" "normalize" "pow" "reflect"
   "refract" "round" "rsqrt" "saturate" "sin" "smoothstep" "sqrt" "step" "tan" "transpose"))
([(semantics (identifier) @support.variable.semantic.hlsl)
  (bitfield_clause (identifier) @support.variable.semantic.hlsl)]
 (#match? @support.variable.semantic.hlsl "^(?i:BINORMAL|BLENDINDICES|BLENDWEIGHT|COLOR|DEPTH|FOG|NORMAL|POSITIONT?|PSIZE|TANGENT|TEXCOORD|VFACE|VPOS)[0-9]*$"))
([(semantics (identifier) @support.variable.semantic.sm4.hlsl)
  (bitfield_clause (identifier) @support.variable.semantic.sm4.hlsl)]
 (#match? @support.variable.semantic.sm4.hlsl "^(?i:SV_(ClipDistance|CullDistance|Coverage|Depth|DispatchThreadID|GroupID|GroupIndex|GroupThreadID|InstanceID|IsFrontFace|Position|PrimitiveID|RenderTargetArrayIndex|SampleIndex|Target|VertexID|ViewportArrayIndex))[0-9]*$"))
([(semantics (identifier) @support.variable.semantic.sm5.hlsl)
  (bitfield_clause (identifier) @support.variable.semantic.sm5.hlsl)]
 (#match? @support.variable.semantic.sm5.hlsl "^(?i:SV_(DepthGreaterEqual|DepthLessEqual|DomainLocation|GSInstanceID|InsideTessFactor|OutputControlPointID|TessFactor))[0-9]*$"))
