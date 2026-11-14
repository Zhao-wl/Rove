//! Direct syntax-tree walk that replaces the C# highlight query (#117).
//!
//! `queries/csharp/highlights.scm` stays the specification: every rule below is one of its
//! patterns, in the same order, so the captures and their relative priorities are the same.
//! The walk visits each node once and decides from a table indexed by the node kind id,
//! which costs a fraction of running the query engine over a whole megabyte of C#.
//!
//! `tests::walk_matches_the_query` compares the two capture streams character by character
//! on a fixture that exercises every rule, on the golden-colors fixture and on the real
//! Unity sources, so the query remains the reference implementation.

use super::{
    grammar::{leading_whitespace, Capture, Work},
    source::Source,
};
use rove_base::Result;
use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};
use tree_sitter::{Language, Node, Tree};

/// Scope of every rule, in the order the patterns appear in `highlights.scm`. Only the
/// relative order matters downstream: captures are sorted by `(start, -end, priority)`.
const SCOPES: [&str; 146] = [
    // 0–19 `(modifier "…")`
    "storage.modifier.abstract.cs",
    "storage.modifier.async.cs",
    "storage.modifier.const.cs",
    "storage.modifier.extern.cs",
    "storage.modifier.file.cs",
    "storage.modifier.fixed.cs",
    "storage.modifier.internal.cs",
    "storage.modifier.new.cs",
    "storage.modifier.override.cs",
    "storage.modifier.partial.cs",
    "storage.modifier.private.cs",
    "storage.modifier.protected.cs",
    "storage.modifier.public.cs",
    "storage.modifier.readonly.cs",
    "storage.modifier.required.cs",
    "storage.modifier.sealed.cs",
    "storage.modifier.static.cs",
    "storage.modifier.unsafe.cs",
    "storage.modifier.virtual.cs",
    "storage.modifier.volatile.cs",
    // 20–37 `(predefined_type)`, dispatched by text
    "keyword.type.bool.cs",
    "keyword.type.byte.cs",
    "keyword.type.char.cs",
    "keyword.type.decimal.cs",
    "keyword.type.double.cs",
    "keyword.type.float.cs",
    "keyword.type.int.cs",
    "keyword.type.long.cs",
    "keyword.type.object.cs",
    "keyword.type.sbyte.cs",
    "keyword.type.short.cs",
    "keyword.type.string.cs",
    "keyword.type.uint.cs",
    "keyword.type.ulong.cs",
    "keyword.type.ushort.cs",
    "keyword.type.void.cs",
    "keyword.type.nint.cs",
    "keyword.type.nuint.cs",
    // 38–46 declaration keywords
    "storage.type.class.cs",
    "storage.type.struct.cs",
    "storage.type.interface.cs",
    "storage.type.enum.cs",
    "storage.type.delegate.cs",
    "storage.type.record.cs",
    "storage.type.namespace.cs",
    "storage.type.event.cs",
    "storage.type.operator.cs",
    // 47–73 control-flow, expression and accessor keywords
    "keyword.control.conditional.if.cs",
    "keyword.control.conditional.else.cs",
    "keyword.control.conditional.switch.cs",
    "keyword.control.conditional.case.cs",
    "keyword.control.conditional.when.cs",
    "keyword.control.flow.return.cs",
    "keyword.control.flow.throw.cs",
    "keyword.control.flow.break.cs",
    "keyword.control.flow.goto.cs",
    "keyword.control.flow.yield.cs",
    "keyword.control.exception.try.cs",
    "keyword.control.exception.catch.cs",
    "keyword.control.exception.finally.cs",
    "keyword.control.loop.do.cs",
    "keyword.control.loop.for.cs",
    "keyword.control.loop.foreach.cs",
    "keyword.control.loop.while.cs",
    "keyword.control.loop.in.cs",
    "keyword.operator.expression.new.cs",
    "keyword.operator.expression.await.cs",
    "keyword.operator.expression.as.cs",
    "keyword.operator.expression.default.cs",
    "storage.type.accessor.get.cs",
    "storage.type.accessor.set.cs",
    "storage.type.accessor.init.cs",
    "storage.type.accessor.add.cs",
    "storage.type.accessor.remove.cs",
    // 74–76 `using`
    "keyword.other.directive.using.cs",
    "entity.name.type.namespace.cs",
    "keyword.control.context.using.cs",
    // 77–82 literals and `lock` / `var` / `this`
    "keyword.control.context.lock.cs",
    "storage.type.var.cs",
    "variable.language.this.cs",
    "constant.language.boolean.true.cs",
    "constant.language.boolean.false.cs",
    "constant.language.null.cs",
    // 83–90 numbers, strings and comments
    "constant.numeric.decimal.cs",
    "constant.numeric.hex.cs",
    "constant.numeric.binary.cs",
    "string.quoted.double.cs",
    "string.quoted.single.cs",
    "constant.character.escape.cs",
    "comment.line.double-slash.cs",
    "comment.block.cs",
    // 91–107 declaration names
    "entity.name.type.class.cs",
    "entity.name.type.struct.cs",
    "entity.name.type.interface.cs",
    "entity.name.type.enum.cs",
    "entity.name.type.delegate.cs",
    "entity.name.type.namespace.cs",
    "entity.name.type.namespace.cs",
    "entity.name.function.cs",
    "entity.name.function.cs",
    "entity.name.function.cs",
    "entity.name.variable.parameter.cs",
    "entity.name.variable.property.cs",
    "entity.name.variable.field.cs",
    "entity.name.variable.local.cs",
    "entity.name.variable.enum-member.cs",
    "entity.name.type.type-parameter.cs",
    "entity.other.attribute-name.cs",
    // 108–112 type references
    "entity.name.type.cs",
    "entity.name.type.cs",
    "entity.name.type.cs",
    "entity.name.type.cs",
    "entity.name.type.cs",
    // 113–126 plain identifiers in expressions
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "variable.other.readwrite.cs",
    "entity.name.variable.local.cs",
    "variable.other.readwrite.cs",
    // 127–131 member access and calls
    "variable.other.object.cs",
    "variable.other.object.property.cs",
    "entity.name.function.cs",
    "entity.name.function.cs",
    "entity.name.function.cs",
    // 132–145 punctuation and operators
    "punctuation.curlybrace.open.cs",
    "punctuation.curlybrace.close.cs",
    "punctuation.parenthesis.open.cs",
    "punctuation.parenthesis.close.cs",
    "punctuation.squarebracket.open.cs",
    "punctuation.squarebracket.close.cs",
    "punctuation.terminator.statement.cs",
    "punctuation.separator.comma.cs",
    "punctuation.accessor.cs",
    "keyword.operator.assignment.cs",
    "keyword.operator.arithmetic.cs",
    "keyword.operator.comparison.cs",
    "keyword.operator.logical.cs",
    "keyword.operator.arrow.cs",
];

// Rules that are referenced by name below.
const MODIFIER_NEW: u16 = 7;
const PREDEFINED_FIRST: u16 = 20;
const KEYWORD_NEW: u16 = 65;
const USING_DIRECTIVE: u16 = 74;
const USING_NAMESPACE: u16 = 75;
const USING_STATEMENT: u16 = 76;
const NUMERIC_DECIMAL: u16 = 83;
const NUMERIC_HEX: u16 = 84;
const NUMERIC_BINARY: u16 = 85;
const COMMENT_LINE: u16 = 89;
const COMMENT_BLOCK: u16 = 90;
const NAMESPACE_NAME: u16 = 96;
const FILE_SCOPED_NAMESPACE_NAME: u16 = 97;
const FIELD_NAME_RULE: u16 = 103;
const LOCAL_NAME_RULE: u16 = 104;
const ATTRIBUTE_NAME: u16 = 107;
const TYPE_REFERENCE: u16 = 108;
const GENERIC_TYPE_REFERENCE: u16 = 109;
const BASE_GENERIC_TYPE: u16 = 111;
const GENERIC_CALL: u16 = 130;
const MEMBER_CALL: u16 = 131;

/// What to do with a node of one kind id.
#[derive(Clone, Copy, Default)]
enum Action {
    #[default]
    Skip,
    /// Every node of this kind gets `0`.
    Token(u16),
    /// Only inside a parent of kind `parent`.
    InParent { parent: u16, rule: u16 },
    /// `rule` everywhere, plus `extra` inside a parent of kind `parent` (`new`).
    TokenInParent { rule: u16, parent: u16, extra: u16 },
    /// Anonymous `using`: a directive keyword or a statement keyword.
    Using,
    /// `name: (_)` of this node gets `0` (namespaces and attribute names).
    WildcardName(u16),
    /// `(predefined_type)`: the rule follows from the text.
    Predefined,
    /// `(integer_literal)`: decimal, plus hex or binary from the prefix.
    Integer,
    /// `(comment)`: line or block, line comments reach back over the indent.
    Comment,
    /// `(identifier)`: the rules follow from the field and the ancestors.
    Identifier,
    /// `(qualified_name)` directly under a using directive.
    QualifiedName,
}

/// Rules for an `(identifier)` child of one parent kind, by the child's field.
#[derive(Clone, Copy, Default)]
struct ParentRules {
    name: Option<u16>,
    left: Option<u16>,
    right: Option<u16>,
    expression: Option<u16>,
    function: Option<u16>,
    /// Matches an identifier child whatever its field.
    any: Option<u16>,
}

struct Walker {
    scopes: Vec<Arc<[Arc<str>]>>,
    actions: Vec<Action>,
    parents: Vec<ParentRules>,
    field_name: u16,
    field_type: u16,
    field_left: u16,
    field_right: u16,
    field_expression: u16,
    field_function: u16,
    generic_name: u16,
    base_list: u16,
    invocation_expression: u16,
    member_access_expression: u16,
    using_directive: u16,
    using_statement: u16,
    variable_declarator: u16,
    variable_declaration: u16,
    field_declaration: u16,
    local_declaration_statement: u16,
}

static WALKER: LazyLock<Walker> = LazyLock::new(Walker::new);

fn language() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}

impl Walker {
    fn new() -> Self {
        let language = language();
        // One `Arc` per distinct scope, as the query's capture table does.
        let mut distinct: Vec<(&str, Arc<[Arc<str>]>)> = Vec::new();
        let scopes = SCOPES
            .iter()
            .map(
                |&name| match distinct.iter().find(|(known, _)| *known == name) {
                    Some((_, scopes)) => scopes.clone(),
                    None => {
                        let scopes: Arc<[Arc<str>]> = Arc::from([Arc::from(name)]);
                        distinct.push((name, scopes.clone()));
                        scopes
                    }
                },
            )
            .collect();

        let kind = |name: &str, named: bool| -> u16 {
            let id = language.id_for_node_kind(name, named);
            assert!(id != 0, "unknown C# node kind {name:?} (named: {named})");
            id
        };
        let field = |name: &str| -> u16 {
            language
                .field_id_for_name(name)
                .unwrap_or_else(|| panic!("unknown C# field {name:?}"))
                .get()
        };

        let modifier = kind("modifier", true);
        let binary_expression = kind("binary_expression", true);
        let mut actions = vec![Action::Skip; language.node_kind_count()];
        let mut set = |id: u16, action: Action| actions[id as usize] = action;

        // 0–19: modifier keywords. `new` is also an expression keyword (65).
        for (offset, name) in [
            "abstract",
            "async",
            "const",
            "extern",
            "file",
            "fixed",
            "internal",
            "new",
            "override",
            "partial",
            "private",
            "protected",
            "public",
            "readonly",
            "required",
            "sealed",
            "static",
            "unsafe",
            "virtual",
            "volatile",
        ]
        .into_iter()
        .enumerate()
        {
            let rule = offset as u16;
            let action = if rule == MODIFIER_NEW {
                Action::TokenInParent {
                    rule: KEYWORD_NEW,
                    parent: modifier,
                    extra: MODIFIER_NEW,
                }
            } else {
                Action::InParent {
                    parent: modifier,
                    rule,
                }
            };
            set(kind(name, false), action);
        }
        set(kind("predefined_type", true), Action::Predefined);

        // 38–73: plain keywords. `new` is already set above.
        for (offset, name) in [
            "class",
            "struct",
            "interface",
            "enum",
            "delegate",
            "record",
            "namespace",
            "event",
            "operator",
            "if",
            "else",
            "switch",
            "case",
            "when",
            "return",
            "throw",
            "break",
            "goto",
            "yield",
            "try",
            "catch",
            "finally",
            "do",
            "for",
            "foreach",
            "while",
            "in",
            "new",
            "await",
            "as",
            "default",
            "get",
            "set",
            "init",
            "add",
            "remove",
        ]
        .into_iter()
        .enumerate()
        {
            let rule = 38 + offset as u16;
            if rule != KEYWORD_NEW {
                set(kind(name, false), Action::Token(rule));
            }
        }

        let using_directive = kind("using_directive", true);
        let using_statement = kind("using_statement", true);
        set(kind("using", false), Action::Using);
        set(kind("qualified_name", true), Action::QualifiedName);
        set(kind("lock", false), Action::Token(77));
        set(kind("implicit_type", true), Action::Token(78));
        set(kind("this", false), Action::Token(79));
        set(kind("true", false), Action::Token(80));
        set(kind("false", false), Action::Token(81));
        set(kind("null_literal", true), Action::Token(82));
        set(kind("integer_literal", true), Action::Integer);
        set(kind("real_literal", true), Action::Token(NUMERIC_DECIMAL));
        for name in [
            "string_literal",
            "verbatim_string_literal",
            "raw_string_literal",
            "interpolated_string_expression",
        ] {
            set(kind(name, true), Action::Token(86));
        }
        set(kind("character_literal", true), Action::Token(87));
        set(kind("escape_sequence", true), Action::Token(88));
        set(kind("comment", true), Action::Comment);

        // 96, 97 and 107 capture `name: (_)`, so they are read from the parent instead of
        // from every named node.
        set(
            kind("namespace_declaration", true),
            Action::WildcardName(NAMESPACE_NAME),
        );
        set(
            kind("file_scoped_namespace_declaration", true),
            Action::WildcardName(FILE_SCOPED_NAMESPACE_NAME),
        );
        set(
            kind("attribute", true),
            Action::WildcardName(ATTRIBUTE_NAME),
        );
        set(kind("identifier", true), Action::Identifier);

        // 132–145: punctuation and operators.
        for (offset, name) in ["{", "}", "(", ")", "[", "]", ";", ",", ".", "="]
            .into_iter()
            .enumerate()
        {
            set(kind(name, false), Action::Token(132 + offset as u16));
        }
        for name in ["+", "-", "*", "/", "%"] {
            set(kind(name, false), Action::Token(142));
        }
        for name in ["==", "!=", "<", ">", "<=", ">="] {
            set(
                kind(name, false),
                Action::InParent {
                    parent: binary_expression,
                    rule: 143,
                },
            );
        }
        for name in ["&&", "||", "!"] {
            set(kind(name, false), Action::Token(144));
        }
        set(kind("=>", false), Action::Token(145));

        let mut parents = vec![ParentRules::default(); language.node_kind_count()];
        let mut named = |name: &str, rules: ParentRules| parents[kind(name, true) as usize] = rules;
        let with_name = |rule: u16| ParentRules {
            name: Some(rule),
            ..Default::default()
        };
        let with_any = |rule: u16| ParentRules {
            any: Some(rule),
            ..Default::default()
        };
        named("class_declaration", with_name(91));
        named("struct_declaration", with_name(92));
        named("interface_declaration", with_name(93));
        named("enum_declaration", with_name(94));
        named("delegate_declaration", with_name(95));
        named("method_declaration", with_name(98));
        named("constructor_declaration", with_name(99));
        named("local_function_statement", with_name(100));
        named("parameter", with_name(101));
        named("property_declaration", with_name(102));
        named("enum_member_declaration", with_name(105));
        named("type_parameter", with_name(106));
        named("base_list", with_any(110));
        named("type_argument_list", with_any(112));
        named("using_directive", with_any(USING_NAMESPACE));
        named(
            "binary_expression",
            ParentRules {
                left: Some(113),
                right: Some(114),
                ..Default::default()
            },
        );
        named("prefix_unary_expression", with_any(115));
        named("postfix_unary_expression", with_any(116));
        named("parenthesized_expression", with_any(117));
        named(
            "assignment_expression",
            ParentRules {
                left: Some(118),
                right: Some(119),
                ..Default::default()
            },
        );
        named("argument", with_any(120));
        named("return_statement", with_any(121));
        named("interpolation", with_any(122));
        named(
            "element_access_expression",
            ParentRules {
                expression: Some(123),
                ..Default::default()
            },
        );
        named(
            "foreach_statement",
            ParentRules {
                left: Some(125),
                right: Some(124),
                ..Default::default()
            },
        );
        named("conditional_expression", with_any(126));
        named(
            "member_access_expression",
            ParentRules {
                expression: Some(127),
                name: Some(128),
                ..Default::default()
            },
        );
        named(
            "invocation_expression",
            ParentRules {
                function: Some(129),
                ..Default::default()
            },
        );

        Self {
            scopes,
            actions,
            parents,
            field_name: field("name"),
            field_type: field("type"),
            field_left: field("left"),
            field_right: field("right"),
            field_expression: field("expression"),
            field_function: field("function"),
            generic_name: kind("generic_name", true),
            base_list: kind("base_list", true),
            invocation_expression: kind("invocation_expression", true),
            member_access_expression: kind("member_access_expression", true),
            using_directive,
            using_statement,
            variable_declarator: kind("variable_declarator", true),
            variable_declaration: kind("variable_declaration", true),
            field_declaration: kind("field_declaration", true),
            local_declaration_statement: kind("local_declaration_statement", true),
        }
    }

    fn push(&'static self, output: &mut Vec<Capture>, rule: u16, start: usize, end: usize) {
        if start >= end {
            return;
        }
        output.push(Capture {
            start,
            end,
            priority: (0, rule as usize, 0),
            scopes: &self.scopes[rule as usize],
        });
    }
}

/// Ancestors of the node being visited, outermost first, as `(kind id, field id)`.
type Ancestors = Vec<(u16, u16)>;

/// Captures of `scan` from a C# tree, equivalent to running the bundled highlight query.
pub(super) fn capture(
    tree: &Tree,
    source: &Source,
    scan: Range<usize>,
    output: &mut Vec<Capture>,
    work: &Work<'_>,
) -> Result<()> {
    if scan.is_empty() {
        return Ok(());
    }
    work.check()?;
    let walker: &'static Walker = &WALKER;
    let root = tree.root_node();
    // Pre-order visits start offsets in non-decreasing order, so the walk can stop at the
    // first node that begins past the range and skip subtrees that end before it.
    let clip = scan.start > root.start_byte() || scan.end < root.end_byte();
    let mut cursor = root.walk();
    let mut ancestors: Ancestors = Vec::with_capacity(32);
    let mut field = 0u16;
    let mut steps = 0u32;
    loop {
        steps = steps.wrapping_add(1);
        if steps.is_multiple_of(8192) {
            work.check()?;
        }
        let node = cursor.node();
        let kind = node.kind_id();
        let mut skip = false;
        if clip {
            if node.start_byte() >= scan.end {
                break;
            }
            skip = node.end_byte() <= scan.start;
        }
        if !skip {
            visit(walker, &node, kind, field, &ancestors, source, output);
            // Within a range, enter at the first child that reaches into it: letting
            // tree-sitter skip the earlier siblings costs much less than stepping the
            // cursor over them one call at a time (an edit to one member of a large type).
            let descended = if clip {
                cursor.goto_first_child_for_byte(scan.start).is_some()
            } else {
                cursor.goto_first_child()
            };
            if descended {
                ancestors.push((kind, field));
                field = cursor.field_id().map_or(0, |id| id.get());
                continue;
            }
        }
        loop {
            if cursor.goto_next_sibling() {
                field = cursor.field_id().map_or(0, |id| id.get());
                break;
            }
            if ancestors.pop().is_none() {
                return work.check();
            }
            assert!(cursor.goto_parent(), "cursor and ancestors disagree");
        }
    }
    work.check()
}

fn visit(
    walker: &'static Walker,
    node: &Node<'_>,
    kind: u16,
    field: u16,
    ancestors: &Ancestors,
    source: &Source,
    output: &mut Vec<Capture>,
) {
    let parent = ancestors.last().map_or(0, |&(kind, _)| kind);
    // `ERROR` and `MISSING` nodes carry ids outside the table.
    match walker
        .actions
        .get(kind as usize)
        .copied()
        .unwrap_or_default()
    {
        Action::Skip => {}
        Action::Token(rule) => walker.push(output, rule, node.start_byte(), node.end_byte()),
        Action::InParent {
            parent: wanted,
            rule,
        } => {
            if parent == wanted {
                walker.push(output, rule, node.start_byte(), node.end_byte());
            }
        }
        Action::TokenInParent {
            rule,
            parent: wanted,
            extra,
        } => {
            let (start, end) = (node.start_byte(), node.end_byte());
            if parent == wanted {
                walker.push(output, extra, start, end);
            }
            walker.push(output, rule, start, end);
        }
        Action::Using => {
            let rule = if parent == walker.using_directive {
                USING_DIRECTIVE
            } else if parent == walker.using_statement {
                USING_STATEMENT
            } else {
                return;
            };
            walker.push(output, rule, node.start_byte(), node.end_byte());
        }
        Action::WildcardName(rule) => {
            if let Some(name) = node.child_by_field_id(walker.field_name) {
                if name.is_named() {
                    walker.push(output, rule, name.start_byte(), name.end_byte());
                }
            }
        }
        Action::QualifiedName => {
            if parent == walker.using_directive {
                walker.push(output, USING_NAMESPACE, node.start_byte(), node.end_byte());
            }
        }
        Action::Predefined => {
            let (start, end) = (node.start_byte(), node.end_byte());
            let rule = match source.bytes(start..end).as_ref() {
                b"bool" => 0,
                b"byte" => 1,
                b"char" => 2,
                b"decimal" => 3,
                b"double" => 4,
                b"float" => 5,
                b"int" => 6,
                b"long" => 7,
                b"object" => 8,
                b"sbyte" => 9,
                b"short" => 10,
                b"string" => 11,
                b"uint" => 12,
                b"ulong" => 13,
                b"ushort" => 14,
                b"void" => 15,
                b"nint" => 16,
                b"nuint" => 17,
                _ => return,
            };
            walker.push(output, PREDEFINED_FIRST + rule, start, end);
        }
        Action::Integer => {
            let (start, end) = (node.start_byte(), node.end_byte());
            walker.push(output, NUMERIC_DECIMAL, start, end);
            // The prefix rules match the node's own text, so a stray `x` after a lone `0`
            // in broken input is not a hexadecimal literal.
            if end - start >= 2 && source.byte(start) == Some(b'0') {
                match source.byte(start + 1) {
                    Some(b'x' | b'X') => walker.push(output, NUMERIC_HEX, start, end),
                    Some(b'b' | b'B') => walker.push(output, NUMERIC_BINARY, start, end),
                    _ => {}
                }
            }
        }
        Action::Comment => {
            let (start, end) = (node.start_byte(), node.end_byte());
            if end - start < 2 || source.byte(start) != Some(b'/') {
                return;
            }
            match source.byte(start + 1) {
                Some(b'/') => {
                    walker.push(output, COMMENT_LINE, leading_whitespace(source, start), end)
                }
                Some(b'*') => walker.push(output, COMMENT_BLOCK, start, end),
                _ => {}
            }
        }
        Action::Identifier => identifier(walker, node, field, ancestors, output),
    }
}

fn identifier(
    walker: &'static Walker,
    node: &Node<'_>,
    field: u16,
    ancestors: &Ancestors,
    output: &mut Vec<Capture>,
) {
    let (start, end) = (node.start_byte(), node.end_byte());
    if start >= end {
        return;
    }
    let Some(&(parent, parent_field)) = ancestors.last() else {
        return;
    };
    let mut push = |rule: u16| walker.push(output, rule, start, end);
    let rules = walker
        .parents
        .get(parent as usize)
        .copied()
        .unwrap_or_default();
    if let Some(rule) = rules.any {
        push(rule);
    }
    if field == walker.field_name {
        if let Some(rule) = rules.name {
            push(rule);
        }
    } else if field == walker.field_left {
        if let Some(rule) = rules.left {
            push(rule);
        }
    } else if field == walker.field_right {
        if let Some(rule) = rules.right {
            push(rule);
        }
    } else if field == walker.field_expression {
        if let Some(rule) = rules.expression {
            push(rule);
        }
    } else if field == walker.field_function {
        if let Some(rule) = rules.function {
            push(rule);
        }
    }
    // `(_ type: (identifier))`, whatever the parent is.
    if field == walker.field_type {
        push(TYPE_REFERENCE);
    }
    let grandparent = ancestors
        .len()
        .checked_sub(2)
        .map_or(0, |index| ancestors[index].0);
    if parent == walker.generic_name {
        // `(_ type: (generic_name (identifier)))`, `(base_list (generic_name (identifier)))`
        // and `(invocation_expression function: (generic_name (identifier)))`.
        if parent_field == walker.field_type {
            push(GENERIC_TYPE_REFERENCE);
        }
        if grandparent == walker.base_list {
            push(BASE_GENERIC_TYPE);
        }
        if parent_field == walker.field_function && grandparent == walker.invocation_expression {
            push(GENERIC_CALL);
        }
    } else if parent == walker.member_access_expression
        && field == walker.field_name
        && parent_field == walker.field_function
        && grandparent == walker.invocation_expression
    {
        push(MEMBER_CALL);
    } else if parent == walker.variable_declarator
        && field == walker.field_name
        && grandparent == walker.variable_declaration
    {
        let great = ancestors
            .len()
            .checked_sub(3)
            .map_or(0, |index| ancestors[index].0);
        if great == walker.field_declaration {
            push(FIELD_NAME_RULE);
        } else if great == walker.local_declaration_statement {
            push(LOCAL_NAME_RULE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::grammar::Grammar;
    use rove_base::Cancellation;

    /// Captures of both implementations in assembly order, as comparable rows.
    type Rows = Vec<(usize, usize, String)>;

    fn both(text: &str, scan: Range<usize>) -> (Rows, Rows) {
        let cancel = Cancellation::default();
        let work = Work {
            cancel: &cancel,
            deadline: None,
        };
        let source = Source::from_str(text);
        let tree = Grammar::Csharp.parse(&source, None, &work).unwrap();
        let rows = |mut captures: Vec<Capture>| -> Rows {
            super::super::csharp_parts::sort_captures(&mut captures);
            captures
                .into_iter()
                .map(|c| (c.start, c.end, c.scopes[0].to_string()))
                .collect::<Vec<_>>()
        };
        let mut query = Vec::new();
        Grammar::Csharp
            .capture_with_query(&tree, &source, scan.clone(), &mut query, &work)
            .unwrap();
        let mut walk = Vec::new();
        capture(&tree, &source, scan, &mut walk, &work).unwrap();
        (rows(query), rows(walk))
    }

    #[track_caller]
    fn assert_same(text: &str, scan: Range<usize>) {
        let (query, walk) = both(text, scan.clone());
        // The query cursor also reports captures of matches that merely overlap the range,
        // so compare only what assembly can see.
        let inside = |rows: Rows| {
            rows.into_iter()
                .filter(|(start, end, _)| *end > scan.start && *start < scan.end)
                .collect::<Vec<_>>()
        };
        assert_eq!(inside(query), inside(walk));
    }

    const EVERY_RULE: &str = include_str!("../../tests/data/csharp_every_rule.cs");

    #[test]
    fn walk_matches_the_query_on_every_rule() {
        assert_same(EVERY_RULE, 0..EVERY_RULE.len());
    }

    #[test]
    fn walk_matches_the_query_on_sub_ranges() {
        let len = EVERY_RULE.len();
        for start in (0..len).step_by(97) {
            assert_same(EVERY_RULE, start..(start + 240).min(len));
        }
    }

    #[test]
    fn walk_matches_the_query_on_the_golden_fixture() {
        let text = include_str!("../../../../tests/golden/colorize-fixtures/test.cs");
        assert_same(text, 0..text.len());
    }

    #[test]
    fn walk_matches_the_query_on_broken_input() {
        for text in [
            "",
            "class",
            "class C { int }",
            "namespace",
            "void M() { x. }",
            "class C { void M() { int a = 0x; } }",
            "/*",
            "// comment only",
            "#if X\nclass C {}\n#endif\n",
        ] {
            assert_same(text, 0..text.len().max(1));
        }
    }

    #[test]
    fn walk_matches_the_query_on_real_unity_sources() {
        let project =
            rove_test_support::require_unity_project!(rove_test_support::UnityProject::Medium);
        let mut checked = 0;
        for path in sample_cs_files(&project, 400) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let (query, walk) = both(&text, 0..text.len());
            assert_eq!(query, walk, "{}", path.display());
            checked += 1;
        }
        assert!(checked > 0, "没有找到可比对的 .cs 文件");
        eprintln!("逐 capture 比对了 {checked} 个真实 C# 文件");
    }

    /// Up to `limit` `.cs` files spread evenly over the whole project, so the sample is
    /// not one plugin directory.
    fn sample_cs_files(root: &std::path::Path, limit: usize) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "cs") {
                    found.push(path);
                }
            }
        }
        found.sort();
        let stride = found.len().div_ceil(limit).max(1);
        found.into_iter().step_by(stride).collect()
    }
}
