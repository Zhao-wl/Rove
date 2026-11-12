//! L0 兜底：在当前文件的语法树上做作用域查找，解析局部变量、参数、局部函数与 lambda 参数。
//!
//! 只回答“这个名字是不是当前文件里某个局部声明”：从标识符向外逐层找声明，最内层优先（所以
//! 嵌套作用域的遮蔽是正确的）；走到类型声明（成员层）还没找到就放弃，交给 L1。成员访问的右侧
//! （`a.x` 的 `x`）、命名实参与对象初始化器的成员名不是局部名字，直接放弃。
//!
//! 作用域规则按 C# 的“表达式变量”放宽处理：语句里 `out var x`、`is T t` 等引入的变量算在所在
//! 语句块（不进入嵌套的块、lambda 与局部函数），`for`/`foreach`/`using`/`fixed`/`catch` 的变量
//! 算在该语句。名字在声明之前被使用（编译错误）时也解析到这个声明。
use rove_base::{Line, Position, Span, TextRange, Utf8Column};
use std::ops::Range;
use tree_sitter::{Node, Parser, Tree};

/// 局部声明的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Variable,
    Parameter,
    LambdaParameter,
    LocalFunction,
    RangeVariable,
}

/// 一个局部声明：名字的位置、所在作用域与悬停用的声明文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDecl {
    pub name: Range<usize>,
    pub kind: LocalKind,
    /// 声明所在的作用域节点（字节区间）。
    pub scope: Range<usize>,
    /// 悬停用：`int count`、`(string s)`、`void Helper(int a)`……
    pub signature: String,
}

/// 解析好的一份文本（按 `\n` 分行，与 tree-sitter 的行号一致）。
pub struct L0File {
    pub text: String,
    tree: Tree,
    line_starts: Vec<usize>,
}

impl L0File {
    pub fn parse(text: String) -> Option<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .ok()?;
        let tree = parser.parse(&text, None)?;
        let line_starts = std::iter::once(0)
            .chain(memchr::memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1))
            .collect();
        Some(Self {
            text,
            tree,
            line_starts,
        })
    }

    /// 行列 → 字节偏移；越界返回 `None`。
    pub fn offset(&self, position: Position) -> Option<usize> {
        let start = *self
            .line_starts
            .get(usize::try_from(position.line.0).ok()?)?;
        let end = self
            .line_starts
            .get(position.line.0 as usize + 1)
            .map_or(self.text.len(), |next| next - 1);
        let offset = start.checked_add(usize::try_from(position.column.0).ok()?)?;
        (offset <= end).then_some(offset)
    }

    /// 字节偏移 → 行列。
    pub fn position(&self, offset: usize) -> Position {
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        Position {
            line: Line(line as u64),
            column: Utf8Column((offset - self.line_starts[line]) as u64),
        }
    }

    /// 字节区间 → 行列区间。
    pub fn range(&self, range: &Range<usize>) -> TextRange {
        Span::new(self.position(range.start), self.position(range.end)).expect("ordered byte range")
    }

    /// 字节偏移处的标识符所引用的局部声明。
    pub fn resolve(&self, offset: usize) -> Option<LocalDecl> {
        let ident = self.identifier_at(offset)?;
        resolve(ident, &self.text)
    }

    /// 某个局部声明的全部出现位置（含声明本身），按位置排序。
    pub fn references(&self, decl: &LocalDecl) -> Vec<Range<usize>> {
        let root = self.tree.root_node();
        let Some(scope) = root.descendant_for_byte_range(decl.scope.start, decl.scope.end) else {
            return Vec::new();
        };
        let name = ident_text(&self.text[decl.name.clone()]);
        let mut out = Vec::new();
        let mut cursor = scope.walk();
        let mut stack = vec![scope];
        while let Some(node) = stack.pop() {
            if is_identifier(node) && ident_text(node_text(node, &self.text)) == name {
                if resolve(node, &self.text).is_some_and(|d| d.name == decl.name) {
                    out.push(node.byte_range());
                }
                continue;
            }
            stack.extend(node.named_children(&mut cursor));
        }
        out.sort_by_key(|r| r.start);
        out
    }

    fn identifier_at(&self, offset: usize) -> Option<Node<'_>> {
        let node = self
            .tree
            .root_node()
            .descendant_for_byte_range(offset, offset)?;
        (is_identifier(node) && node.start_byte() <= offset && offset < node.end_byte())
            .then_some(node)
    }
}

fn is_identifier(node: Node<'_>) -> bool {
    matches!(node.kind(), "identifier" | "implicit_parameter")
}

fn node_text<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}

/// `@class` 与 `class` 是同一个名字。
fn ident_text(s: &str) -> &str {
    s.strip_prefix('@').unwrap_or(s)
}

fn field_is(parent: Node<'_>, field: &str, child: Node<'_>) -> bool {
    parent.child_by_field_name(field) == Some(child)
}

/// 标识符处在“成员名”的位置：不可能是局部名字。
fn is_member_position(ident: Node<'_>) -> bool {
    let Some(parent) = ident.parent() else {
        return false;
    };
    match parent.kind() {
        "member_access_expression" | "qualified_name" | "member_binding_expression" => {
            field_is(parent, "name", ident)
        }
        "argument" => field_is(parent, "name", ident),
        "assignment_expression" => {
            field_is(parent, "left", ident)
                && parent
                    .parent()
                    .is_some_and(|g| g.kind() == "initializer_expression")
        }
        _ => false,
    }
}

/// 走到这些节点就离开了方法体：剩下的是成员与类型，由 L1/L2 回答。
fn is_member_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "record_declaration"
            | "enum_declaration"
            | "namespace_declaration"
            | "file_scoped_namespace_declaration"
            | "compilation_unit"
    )
}

fn resolve(ident: Node<'_>, text: &str) -> Option<LocalDecl> {
    if is_member_position(ident) {
        return None;
    }
    let name = ident_text(node_text(ident, text));
    let mut node = ident.parent();
    while let Some(scope) = node {
        if let Some(found) = declared_in(scope, name, text) {
            return Some(found);
        }
        if is_member_boundary(scope.kind()) {
            return None;
        }
        node = scope.parent();
    }
    None
}

/// `scope` 这一层直接引入的、名字为 `name` 的局部声明。
fn declared_in<'t>(scope: Node<'t>, name: &str, text: &str) -> Option<LocalDecl> {
    let mut found = None;
    let mut add = |decl: Node<'t>, kind: LocalKind, signature: Signature<'t>| {
        if found.is_none() && ident_text(node_text(decl, text)) == name {
            found = Some(LocalDecl {
                name: decl.byte_range(),
                kind,
                scope: scope.byte_range(),
                signature: signature.render(decl, text),
            });
        }
    };
    let mut cursor = scope.walk();
    match scope.kind() {
        "block" | "switch_section" | "switch_expression_arm" | "global_statement" => {
            for statement in scope.named_children(&mut cursor) {
                match statement.kind() {
                    "local_function_statement" => {
                        if let Some(n) = statement.child_by_field_name("name") {
                            add(n, LocalKind::LocalFunction, Signature::Header(statement));
                        }
                    }
                    kind if kind == "block" || STATEMENT_SCOPED.contains(&kind) => {}
                    _ => expression_variables(statement, &mut add),
                }
            }
        }
        "for_statement" | "using_statement" | "fixed_statement" | "while_statement" => {
            expression_variables(scope, &mut add);
        }
        "foreach_statement" => {
            if let Some(left) = scope.child_by_field_name("left") {
                let signature = match scope.child_by_field_name("type") {
                    Some(_) => Signature::Typed(scope),
                    None => Signature::Name,
                };
                designations(left, signature, &mut add);
            }
        }
        "catch_clause" => {
            for child in scope.named_children(&mut cursor) {
                if child.kind() == "catch_declaration" {
                    if let Some(n) = child.child_by_field_name("name") {
                        add(n, LocalKind::Variable, Signature::Node(child));
                    }
                }
            }
        }
        "lambda_expression" | "anonymous_method_expression" => {
            if let Some(params) = scope.child_by_field_name("parameters") {
                if params.kind() == "implicit_parameter" {
                    add(params, LocalKind::LambdaParameter, Signature::Name);
                } else {
                    parameters(params, LocalKind::LambdaParameter, &mut add);
                }
            } else {
                // anonymous_method_expression 的参数表不一定带字段名。
                for child in scope.named_children(&mut cursor) {
                    if child.kind() == "parameter_list" {
                        parameters(child, LocalKind::LambdaParameter, &mut add);
                    }
                }
            }
        }
        "local_function_statement"
        | "method_declaration"
        | "constructor_declaration"
        | "destructor_declaration"
        | "operator_declaration"
        | "conversion_operator_declaration"
        | "indexer_declaration"
        | "delegate_declaration" => {
            for child in scope.named_children(&mut cursor) {
                if matches!(child.kind(), "parameter_list" | "bracketed_parameter_list") {
                    parameters(child, LocalKind::Parameter, &mut add);
                }
            }
        }
        "query_expression" => {
            for clause in scope.named_children(&mut cursor) {
                match clause.kind() {
                    "from_clause" => {
                        if let Some(n) = clause.child_by_field_name("name") {
                            add(n, LocalKind::RangeVariable, Signature::Name);
                        }
                    }
                    "let_clause" | "join_clause" | "query_continuation" => {
                        let first = (0..clause.named_child_count())
                            .filter_map(|i| clause.named_child(i))
                            .find(|n| n.kind() == "identifier");
                        if let Some(n) = first {
                            add(n, LocalKind::RangeVariable, Signature::Name);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    found
}

/// 这些语句自己就是作用域：它们引入的变量不属于外层语句块。
const STATEMENT_SCOPED: &[&str] = &[
    "for_statement",
    "foreach_statement",
    "using_statement",
    "fixed_statement",
    "while_statement",
    "try_statement",
];

/// 悬停用的声明文本。
#[derive(Clone, Copy)]
enum Signature<'t> {
    /// 整个节点的文本（参数、catch 声明、模式）。
    Node(Node<'t>),
    /// 局部函数：从开头到参数表结束。
    Header(Node<'t>),
    /// 变量声明：类型 + 名字（`int a = 1, b` 中的 `b` 显示为 `int b`）。
    Typed(Node<'t>),
    Name,
}

impl Signature<'_> {
    fn render(self, name: Node<'_>, text: &str) -> String {
        let name = node_text(name, text);
        match self {
            Self::Node(n) => node_text(n, text).trim().to_owned(),
            Self::Header(n) => {
                let end = n
                    .child_by_field_name("parameters")
                    .map_or(n.end_byte(), |p| p.end_byte());
                text[n.start_byte()..end].trim().to_owned()
            }
            Self::Typed(declaration) => match declaration.child_by_field_name("type") {
                Some(t) => format!("{} {name}", node_text(t, text)),
                None => name.to_owned(),
            },
            Self::Name => name.to_owned(),
        }
    }
}

fn parameters<'t>(
    list: Node<'t>,
    kind: LocalKind,
    add: &mut impl FnMut(Node<'t>, LocalKind, Signature<'t>),
) {
    let mut cursor = list.walk();
    for p in list.named_children(&mut cursor) {
        if p.kind() == "parameter" {
            if let Some(n) = p.child_by_field_name("name") {
                add(n, kind, Signature::Node(p));
            }
        }
    }
}

/// 语句里引入的变量：局部变量声明与表达式变量，不进入嵌套的块、lambda、局部函数，
/// 也不进入自成作用域的语句（[`STATEMENT_SCOPED`]）。
fn expression_variables<'t>(
    statement: Node<'t>,
    add: &mut impl FnMut(Node<'t>, LocalKind, Signature<'t>),
) {
    let mut stack = vec![statement];
    let mut cursor = statement.walk();
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if node != statement
            && (STATEMENT_SCOPED.contains(&kind)
                || matches!(
                    kind,
                    "block"
                        | "lambda_expression"
                        | "anonymous_method_expression"
                        | "local_function_statement"
                        | "query_expression"
                ))
        {
            continue;
        }
        match kind {
            "variable_declaration" => {
                for d in node.named_children(&mut cursor) {
                    if d.kind() == "variable_declarator" {
                        if let Some(n) = d.child_by_field_name("name") {
                            add(n, LocalKind::Variable, Signature::Typed(node));
                        }
                    }
                }
            }
            "declaration_expression"
            | "declaration_pattern"
            | "var_pattern"
            | "recursive_pattern" => {
                if let Some(n) = node.child_by_field_name("name") {
                    add(n, LocalKind::Variable, Signature::Typed(node));
                }
                designations(node, Signature::Node(node), add);
            }
            _ => {}
        }
        // 子节点倒序入栈，保持源码顺序（同名时取第一个）。
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
}

/// `var (a, b)`、`foreach (var (k, v) in …)`、`foreach (var x in …)` 中的名字。
fn designations<'t>(
    node: Node<'t>,
    signature: Signature<'t>,
    add: &mut impl FnMut(Node<'t>, LocalKind, Signature<'t>),
) {
    match node.kind() {
        "identifier" => add(node, LocalKind::Variable, signature),
        "parenthesized_variable_designation" | "tuple_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                designations(child, signature, add);
            }
        }
        "declaration_expression" | "var_pattern" | "declaration_pattern" | "recursive_pattern" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if matches!(
                    child.kind(),
                    "parenthesized_variable_designation" | "tuple_pattern"
                ) {
                    designations(child, signature, add);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `⟦`/`⟧` 标出期望的声明，`‸` 标出查询点。返回（去掉标记的文本，查询偏移，期望声明区间）。
    fn marked(src: &str) -> (String, usize, Option<Range<usize>>) {
        let mut text = String::new();
        let (mut at, mut start, mut end) = (None, None, None);
        for c in src.chars() {
            match c {
                '‸' => at = Some(text.len()),
                '⟦' => start = Some(text.len()),
                '⟧' => end = Some(text.len()),
                _ => text.push(c),
            }
        }
        (text, at.expect("‸"), start.zip(end).map(|(s, e)| s..e))
    }

    fn check(src: &str) -> Option<LocalDecl> {
        let (text, at, expected) = marked(src);
        let file = L0File::parse(text).unwrap();
        let got = file.resolve(at);
        assert_eq!(
            got.as_ref().map(|d| d.name.clone()),
            expected,
            "{src}\n{got:?}"
        );
        got
    }

    #[test]
    fn locals_parameters_and_shadowing() {
        check("class C { void F(int ⟦a⟧) { var b = ‸a; } }");
        check("class C { void F() { int ⟦b⟧ = 1; ‸b++; } }");
        check("class C { int x; void F() { int ⟦x⟧ = 2; Use(‸x); } }");
        // 成员访问的右侧不是局部名字。
        check("class C { int x; void F() { int x = 2; Use(this.‸x); } }");
        // 字段：没有局部声明，交给 L1。
        check("class C { int x; void F() { Use(‸x); } }");
    }

    #[test]
    fn lambda_parameters_local_functions_and_nesting() {
        let d = check("class C { void F() { Run(⟦x⟧ => ‸x + 1); } }").unwrap();
        assert_eq!(d.kind, LocalKind::LambdaParameter);
        check("class C { void F() { Run((int ⟦x⟧, int y) => ‸x + y); } }");
        let d = check("class C { void F() { ‸Helper(1); int ⟦Helper⟧(int a) => a; } }").unwrap();
        assert_eq!(d.kind, LocalKind::LocalFunction);
        // 局部函数的参数遮蔽外层的同名局部变量。
        check("class C { void F() { int a = 0; int G(int ⟦a⟧) { return ‸a; } } }");
        // lambda 参数遮蔽外层参数。
        check("class C { void F(int v) { Run(⟦v⟧ => ‸v); } }");
        // 内层块里的变量对外层不可见。
        check("class C { void F() { { int q = 1; } Use(‸q); } }");
        // 外层变量在内层块可见。
        check("class C { void F() { int ⟦q⟧ = 1; { Use(‸q); } } }");
    }

    #[test]
    fn statement_scoped_and_expression_variables() {
        check("class C { void F() { for (int ⟦i⟧ = 0; i < 3; i++) Use(‸i); } }");
        check("class C { void F(int[] xs) { foreach (var ⟦x⟧ in xs) Use(‸x); } }");
        check("class C { void F() { try {} catch (Exception ⟦e⟧) { Log(‸e); } } }");
        check("class C { void F(object o) { if (o is string ⟦s⟧) Use(‸s); } }");
        check("class C { void F() { Parse(out var ⟦n⟧); Use(‸n); } }");
        check("class C { void F() { using (var ⟦r⟧ = Open()) { r.Close(); ‸r.Dispose(); } } }");
        check("class C { void F() { var q = from ⟦p⟧ in ps select ‸p; } }");
    }

    #[test]
    fn references_follow_the_same_declaration() {
        let (text, at, _) = marked(
            "class C { void F() { int a = 1; Use(a); Run(a => a); int G(int a) => a; Use(‸a); } }",
        );
        let file = L0File::parse(text.clone()).unwrap();
        let decl = file.resolve(at).unwrap();
        let refs: Vec<_> = file
            .references(&decl)
            .into_iter()
            .map(|r| r.start)
            .collect();
        // 声明 + Use(a) + Use(‸a)；lambda 与局部函数里的 a 是别的声明。
        assert_eq!(refs.len(), 3, "{refs:?}");
        assert_eq!(refs[0], decl.name.start);
        assert_eq!(*refs.last().unwrap(), at);
    }
}
