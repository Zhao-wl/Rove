; Scope names follow the VS Code ShaderLab TextMate grammar.
[
 "Shader" "Properties" "SubShader" "Category" "GrabPass"
] @storage.type.structure.shaderlab
(pass "Pass" @storage.type.structure.shaderlab)
[
 "Tags" "Fallback" "CustomEditor" "CustomEditorForRenderPipeline" "Dependency"
 "UsePass" "Name" "Cull" "ZWrite" "ZTest" "ZClip" "AlphaToMask" "Conservative"
 "LOD" "Blend" "BlendOp" "ColorMask" "Offset" "Stencil" "AlphaTest" "Lighting"
 "SeparateSpecular" "ColorMaterial" "Color" "Fog" "Material" "BindChannels" "SetTexture"
 "Ref" "ReadMask" "WriteMask" "Comp" "Fail" "ZFail"
 "CompFront" "PassFront" "FailFront" "ZFailFront"
 "CompBack" "PassBack" "FailBack" "ZFailBack"
] @support.type.propertyname.shaderlab
(stencil_state "Pass" @support.type.propertyname.shaderlab)

((comment) @comment.line.double-slash.shaderlab
 (#match? @comment.line.double-slash.shaderlab "^//"))
(string) @string.quoted.double.shaderlab
(number) @constant.numeric.shaderlab
(property_type [(identifier) (dimension)] @support.type.basic.shaderlab)
(property name: (identifier) @support.variable.declaration.shaderlab)
(attribute) @meta.attribute.shaderlab
(attribute . (identifier) @support.type.attributename.shaderlab)
(property_reference (identifier) @support.variable.reference.shaderlab)

"Off" @support.constant.property-value.shaderlab
((render_state (identifier) @support.constant.property-value.shaderlab)
 (#any-of? @support.constant.property-value.shaderlab "On" "Off" "Front" "Back"))
((render_state (identifier) @support.constant.property-value.comparisonfunction.shaderlab)
 (#any-of? @support.constant.property-value.comparisonfunction.shaderlab "Never" "Less" "Equal" "LEqual" "Greater" "NotEqual" "GEqual" "Always"))
((render_state (identifier) @support.constant.property-value.blendfactors.shaderlab)
 (#any-of? @support.constant.property-value.blendfactors.shaderlab "One" "Zero" "SrcColor" "SrcAlpha" "DstColor" "DstAlpha" "OneMinusSrcColor" "OneMinusSrcAlpha" "OneMinusDstColor" "OneMinusDstAlpha"))
((render_state (identifier) @support.constant.property-value.blendoperations.shaderlab)
 (#any-of? @support.constant.property-value.blendoperations.shaderlab "Add" "Sub" "RevSub" "Min" "Max"))
((stencil_state (identifier) @support.constant.property-value.stenciloperation.shaderlab)
 (#any-of? @support.constant.property-value.stenciloperation.shaderlab "Keep" "Zero" "Replace" "IncrSat" "DecrSat" "Invert" "IncrWrap" "DecrWrap"))

(program_block ["CGPROGRAM" "CGINCLUDE"]) @meta.cgblock
(program_block ["HLSLPROGRAM" "HLSLINCLUDE"]) @meta.hlslblock
["CGPROGRAM" "CGINCLUDE" "ENDCG" "HLSLPROGRAM" "HLSLINCLUDE" "ENDHLSL"] @keyword.other
