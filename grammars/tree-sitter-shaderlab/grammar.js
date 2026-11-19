// Copyright (c) Rove contributors. Licensed under the MIT license.
module.exports = grammar({
  name: "shaderlab",

  extras: $ => [/\s/, $.comment],
  word: $ => $.identifier,
  externals: $ => [$._cg_body, $._hlsl_body],

  rules: {
    source_file: $ => repeat($.shader),

    shader: $ => seq(
      keyword("Shader"),
      field("name", $.string),
      "{",
      repeat(choice(
        $.properties,
        $.subshader,
        $.category,
        $.fallback,
        $.custom_editor,
        $.dependency,
        $.program_block,
      )),
      "}",
    ),

    properties: $ => seq(keyword("Properties"), "{", repeat($.property), "}"),
    property: $ => seq(
      repeat($.attribute),
      field("name", $.identifier),
      "(",
      field("label", $.string),
      ",",
      field("type", $.property_type),
      ")",
      "=",
      field("default", $.property_default),
    ),
    attribute: $ => seq(
      "[",
      $.identifier,
      optional(seq("(", optional(commaList(choice($.string, $.attribute_text))), ")")),
      "]",
    ),
    // Property drawers also accept unquoted, multi-word arguments such as Header labels.
    attribute_text: _ => token(/[^,)\]"\s][^,)\]"\r\n]*/),
    property_type: $ => choice(
      seq($.identifier, optional(seq("(", $.number, ",", $.number, ")"))),
      $.dimension,
    ),
    property_default: $ => choice(
      $.number,
      $.vector,
      seq($.string, optional($.texture_options)),
    ),
    texture_options: $ => seq("{", repeat($.identifier), "}"),

    category: $ => seq(keyword("Category"), "{", repeat(choice($.subshader, $._pass_item)), "}"),
    subshader: $ => seq(keyword("SubShader"), "{", repeat(choice($.pass, $._pass_item)), "}"),
    pass: $ => seq(choice(keyword("Pass"), keyword("GrabPass")), "{", repeat(choice($.string, $._pass_item)), "}"),
    _pass_item: $ => choice(
      $.tags,
      $.render_state,
      $.stencil,
      $.program_block,
      $.legacy_block,
      $.use_pass,
      $.pass_name,
    ),

    tags: $ => seq(keyword("Tags"), "{", repeat($.tag), "}"),
    tag: $ => seq(field("name", $.string), "=", field("value", $.string)),
    pass_name: $ => seq(keyword("Name"), $.string),
    use_pass: $ => seq(keyword("UsePass"), $.string),
    fallback: $ => seq(keyword("Fallback"), choice($.string, keyword("Off"))),
    custom_editor: $ => choice(
      seq(keyword("CustomEditor"), $.string),
      seq(keyword("CustomEditorForRenderPipeline"), $.string, $.string),
    ),
    dependency: $ => seq(keyword("Dependency"), $.string, "=", $.string),

    render_state: $ => choice(
      seq(choice(
        keyword("Cull"), keyword("ZTest"), keyword("ZWrite"), keyword("ZClip"), keyword("AlphaToMask"), keyword("Conservative"),
        keyword("LOD"), keyword("Lighting"), keyword("SeparateSpecular"), keyword("ColorMaterial"), keyword("Color"),
      ), $._value),
      seq(keyword("Offset"), $._value, ",", $._value),
      seq(keyword("ColorMask"), $._value, optional($.number)),
      prec.right(seq(keyword("AlphaTest"), $._factor, optional($._value))),
      seq(keyword("Blend"), optional($.number), choice(
        keyword("Off"),
        seq($._factor, $._factor, optional(seq(",", $._factor, $._factor))),
      )),
      seq(keyword("BlendOp"), optional($.number), $._factor, optional(seq(",", $._factor))),
    ),
    stencil: $ => seq(keyword("Stencil"), "{", repeat($.stencil_state), "}"),
    stencil_state: $ => seq(choice(
      keyword("Ref"), keyword("ReadMask"), keyword("WriteMask"), keyword("Comp"), keyword("Pass"), keyword("Fail"), keyword("ZFail"),
      keyword("CompFront"), keyword("PassFront"), keyword("FailFront"), keyword("ZFailFront"),
      keyword("CompBack"), keyword("PassBack"), keyword("FailBack"), keyword("ZFailBack"),
    ), $._value),

    legacy_block: $ => seq(
      choice(keyword("Fog"), keyword("Material"), keyword("BindChannels"), seq(keyword("SetTexture"), $.property_reference)),
      "{",
      repeat($._legacy_item),
      "}",
    ),
    _legacy_item: $ => choice($._value, ",", "=", "+", "-", "*", "."),

    program_block: $ => choice(
      seq(
        choice(keyword("CGPROGRAM"), keyword("CGINCLUDE")),
        optional(field("body", alias($._cg_body, $.program_body))),
        keyword("ENDCG"),
      ),
      seq(
        choice(keyword("HLSLPROGRAM"), keyword("HLSLINCLUDE")),
        optional(field("body", alias($._hlsl_body, $.program_body))),
        keyword("ENDHLSL"),
      ),
    ),

    _factor: $ => choice($.identifier, $.property_reference),
    _value: $ => choice($.number, $.identifier, $.string, $.property_reference, $.vector),
    property_reference: $ => seq("[", $.identifier, "]"),
    vector: $ => seq("(", commaList($.number), ")"),
    identifier: _ => /[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*/,
    dimension: _ => /[23][Dd]([Aa][Rr][Rr][Aa][Yy])?/,
    number: _ => /[+-]?(0[xX][0-9a-fA-F]+|([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][+-]?[0-9]+)?[fF]?)/,
    string: _ => token(seq('"', repeat(choice(/[^"\\\r\n]+/, /\\[^\r\n]/)), '"')),
    comment: _ => token(choice(seq("//", /[^\r\n]*/), seq("/*", /[^*]*\*+([^/*][^*]*\*+)*/, "/"))),
  },
});

function keyword(word) {
  const pattern = new RegExp([...word].map(letter =>
    `[${letter.toLowerCase()}${letter.toUpperCase()}]`).join(""));
  return alias(token(prec(1, pattern)), word);
}

function commaList(item) {
  return seq(item, repeat(seq(",", item)));
}
