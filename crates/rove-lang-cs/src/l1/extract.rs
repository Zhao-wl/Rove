//! 单个 `.cs` 文件的声明提取：只走声明容器（编译单元、命名空间、类型体、`#if` 分支），
//! 不进入方法体，所以比高亮解析便宜得多。不做绑定、不推断类型。

use super::{DeclKind, UsingKind};
use tree_sitter::{Node, Parser, Point};

/// 行 + UTF-8 字节列（半开区间），与 `rove_base::Position` 同一单位，存成 u32 省内存。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Range32 {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Range32 {
    fn from_points(start: Point, end: Point) -> Self {
        Self {
            start_line: start.row as u32,
            start_col: start.column as u32,
            end_line: end.row as u32,
            end_col: end.column as u32,
        }
    }

    fn of(node: Node<'_>) -> Self {
        Self::from_points(node.start_position(), node.end_position())
    }

    pub fn contains(&self, line: u32, col: u32) -> bool {
        (self.start_line, self.start_col) <= (line, col)
            && (line, col) <= (self.end_line, self.end_col)
    }
}

pub(crate) const NONE: u32 = u32::MAX;

/// 一条声明。`N` 在提取阶段是 `String`，并入索引后换成驻留的名称 ID。
#[derive(Clone, Debug)]
pub(crate) struct Decl<N> {
    pub name: N,
    pub kind: DeclKind,
    /// 泛型参数个数（类型、委托与泛型方法）。
    pub arity: u8,
    /// 外层类型声明在本文件 `decls` 中的下标；顶层为 `NONE`。
    pub parent: u32,
    /// 所在命名空间作用域（`scopes` 下标，0 是编译单元）。
    pub scope: u32,
    pub range: Range32,
    pub selection: Range32,
}

/// 命名空间作用域。`ns` 是完整命名空间名（全局为空串）。
#[derive(Clone, Debug)]
pub(crate) struct Scope<N> {
    pub ns: N,
    pub parent: u32,
    pub range: Range32,
}

#[derive(Clone, Debug)]
pub(crate) struct Using<N> {
    pub scope: u32,
    pub kind: UsingKind,
    pub global: bool,
    /// 别名（只有 `UsingKind::Alias` 有）。
    pub alias: N,
    /// 去掉空白的目标文本，例如 `System.Collections.Generic`、`List<int>`。
    pub target: N,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Parsed<N> {
    pub decls: Vec<Decl<N>>,
    pub scopes: Vec<Scope<N>>,
    pub usings: Vec<Using<N>>,
}

pub(crate) fn new_parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .expect("bundled C# grammar is compatible with tree-sitter");
    parser
}

/// 解析一份已解码的 UTF-8 源码。解析失败（被取消或 grammar 异常）时返回 None。
pub(crate) fn extract(parser: &mut Parser, source: &str) -> Option<Parsed<String>> {
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    let mut walker = Walker {
        src: source.as_bytes(),
        eof: root.end_position(),
        out: Parsed::default(),
    };
    walker.out.scopes.push(Scope {
        ns: String::new(),
        parent: NONE,
        range: Range32::of(root),
    });
    walker.children(root, 0, NONE, 0);
    Some(walker.out)
}

/// 递归只沿声明容器走；深度上限防止病态的 ERROR 嵌套耗尽栈。
const MAX_DEPTH: u32 = 256;

struct Walker<'a> {
    src: &'a [u8],
    eof: Point,
    out: Parsed<String>,
}

fn type_kind(kind: &str) -> Option<DeclKind> {
    Some(match kind {
        "class_declaration" => DeclKind::Class,
        "struct_declaration" => DeclKind::Struct,
        "interface_declaration" => DeclKind::Interface,
        "enum_declaration" => DeclKind::Enum,
        "record_declaration" | "record_struct_declaration" => DeclKind::Record,
        "delegate_declaration" => DeclKind::Delegate,
        _ => return None,
    })
}

impl Walker<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        std::str::from_utf8(&self.src[node.byte_range()]).unwrap_or("")
    }

    /// 标识符：去掉逐字前缀 `@`。
    fn ident(&self, node: Node<'_>) -> String {
        let t = self.text(node);
        t.strip_prefix('@').unwrap_or(t).to_owned()
    }

    /// 名称类文本（命名空间、using 目标）：去掉空白与注释之外的一切格式差异。
    fn compact(&self, node: Node<'_>) -> String {
        self.text(node)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    fn children(&mut self, node: Node<'_>, scope: u32, parent: u32, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut scope = scope;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "namespace_declaration" => {
                    let new = self.namespace(child, scope, Range32::of(child));
                    if let Some(body) = child.child_by_field_name("body") {
                        self.children(body, new, NONE, depth + 1);
                    }
                }
                "file_scoped_namespace_declaration" => {
                    // 文件级命名空间：作用域一直延伸到文件末尾，后续兄弟节点都属于它。
                    let range = Range32::from_points(child.start_position(), self.eof);
                    scope = self.namespace(child, scope, range);
                    self.children(child, scope, NONE, depth + 1);
                }
                "using_directive" => self.using(child, scope),
                "field_declaration" | "event_field_declaration" => {
                    let kind = if child.kind() == "field_declaration" {
                        DeclKind::Field
                    } else {
                        DeclKind::Event
                    };
                    self.variables(child, kind, scope, parent);
                }
                "declaration_list"
                | "enum_member_declaration_list"
                | "preproc_if"
                | "preproc_elif"
                | "preproc_else"
                | "ERROR" => self.children(child, scope, parent, depth + 1),
                kind => {
                    if let Some(kind) = type_kind(kind) {
                        self.type_decl(child, kind, scope, parent, depth);
                    } else {
                        self.member(child, scope, parent);
                    }
                }
            }
        }
    }

    fn namespace(&mut self, node: Node<'_>, scope: u32, range: Range32) -> u32 {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.compact(n))
            .unwrap_or_default();
        let outer = &self.out.scopes[scope as usize].ns;
        let ns = match (outer.is_empty(), name.is_empty()) {
            (_, true) => outer.clone(),
            (true, false) => name,
            (false, false) => format!("{outer}.{name}"),
        };
        self.out.scopes.push(Scope {
            ns,
            parent: scope,
            range,
        });
        (self.out.scopes.len() - 1) as u32
    }

    fn using(&mut self, node: Node<'_>, scope: u32) {
        let alias = node.child_by_field_name("name");
        let (mut global, mut is_static, mut target) = (false, false, None);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "global" if !child.is_named() => global = true,
                "static" if !child.is_named() => is_static = true,
                _ if child.is_named() && Some(child.id()) != alias.map(|a| a.id()) => {
                    target = Some(child)
                }
                _ => {}
            }
        }
        let Some(target) = target else { return };
        let (kind, alias) = match alias {
            Some(a) => (UsingKind::Alias, self.ident(a)),
            None if is_static => (UsingKind::Static, String::new()),
            None => (UsingKind::Namespace, String::new()),
        };
        self.out.usings.push(Using {
            scope,
            kind,
            global,
            alias,
            target: self.compact(target),
        });
    }

    fn push(&mut self, decl: Decl<String>) -> u32 {
        self.out.decls.push(decl);
        (self.out.decls.len() - 1) as u32
    }

    fn type_decl(&mut self, node: Node<'_>, kind: DeclKind, scope: u32, parent: u32, depth: u32) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let mut cursor = node.walk();
        let arity = node
            .named_children(&mut cursor)
            .find(|c| c.kind() == "type_parameter_list")
            .map_or(0, type_params);
        let index = self.push(Decl {
            name: self.ident(name),
            kind,
            arity,
            parent,
            scope,
            range: Range32::of(node),
            selection: Range32::of(name),
        });
        if let Some(body) = node.child_by_field_name("body") {
            self.children(body, scope, index, depth + 1);
        }
    }

    fn variables(&mut self, node: Node<'_>, kind: DeclKind, scope: u32, parent: u32) {
        let mut cursor = node.walk();
        let Some(declaration) = node
            .named_children(&mut cursor)
            .find(|c| c.kind() == "variable_declaration")
        else {
            return;
        };
        let mut cursor = declaration.walk();
        for declarator in declaration.named_children(&mut cursor) {
            if declarator.kind() != "variable_declarator" {
                continue;
            }
            if let Some(name) = declarator.child_by_field_name("name") {
                self.push(Decl {
                    name: self.ident(name),
                    kind,
                    arity: 0,
                    parent,
                    scope,
                    range: Range32::of(node),
                    selection: Range32::of(name),
                });
            }
        }
    }

    fn member(&mut self, node: Node<'_>, scope: u32, parent: u32) {
        let (kind, name, selection, arity) = match node.kind() {
            "method_declaration" => {
                let Some(n) = node.child_by_field_name("name") else {
                    return;
                };
                let arity = node
                    .child_by_field_name("type_parameters")
                    .map_or(0, type_params);
                (DeclKind::Method, self.ident(n), n, arity)
            }
            "property_declaration" | "event_declaration" | "enum_member_declaration" => {
                let Some(n) = node.child_by_field_name("name") else {
                    return;
                };
                let kind = match node.kind() {
                    "property_declaration" => DeclKind::Property,
                    "event_declaration" => DeclKind::Event,
                    _ => DeclKind::EnumMember,
                };
                (kind, self.ident(n), n, 0)
            }
            "constructor_declaration" => {
                let Some(n) = node.child_by_field_name("name") else {
                    return;
                };
                (DeclKind::Constructor, self.ident(n), n, 0)
            }
            "destructor_declaration" => {
                let Some(n) = node.child_by_field_name("name") else {
                    return;
                };
                (DeclKind::Destructor, format!("~{}", self.ident(n)), n, 0)
            }
            "indexer_declaration" => {
                let Some(this) = find_token(node, "this") else {
                    return;
                };
                (DeclKind::Indexer, "this".to_owned(), this, 0)
            }
            "operator_declaration" => {
                let Some(op) = node.child_by_field_name("operator") else {
                    return;
                };
                (
                    DeclKind::Operator,
                    format!("operator {}", self.text(op)),
                    op,
                    0,
                )
            }
            "conversion_operator_declaration" => {
                let Some(keyword) = find_token(node, "operator") else {
                    return;
                };
                let direction = find_token(node, "implicit")
                    .or_else(|| find_token(node, "explicit"))
                    .map_or("", |n| self.text(n));
                let target = node
                    .child_by_field_name("type")
                    .map(|t| self.compact(t))
                    .unwrap_or_default();
                let name = format!("{direction} operator {target}")
                    .trim_start()
                    .to_owned();
                (DeclKind::Operator, name, keyword, 0)
            }
            _ => return,
        };
        self.push(Decl {
            name,
            kind,
            arity,
            parent,
            scope,
            range: Range32::of(node),
            selection: Range32::of(selection),
        });
    }
}

fn type_params(list: Node<'_>) -> u8 {
    let mut cursor = list.walk();
    let count = list
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "type_parameter")
        .count();
    count.min(u8::MAX as usize) as u8
}

fn find_token<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| !c.is_named() && c.kind() == kind);
    found
}
