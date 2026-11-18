//! 声明细节的语法来源（#59）。
//!
//! L2 索引里每个符号只记了 Roslyn 的 `SymbolKind`（段 3 的 `kind` 字节）与「有没有源码位置 /
//! 是不是外部程序集」两个标志位，**没有** `TypeKind`（class / struct / interface / enum /
//! delegate）、也**没有**修饰符（`static` / `readonly` / `const` / `abstract` / `async`）。
//! 在索引格式补上这两项之前（属于 #32 / #33 的范围，本 issue 不改跨进程格式），
//! 这些细节由**正在着色的这个文件自己的语法树**给出：
//!
//! - 本文件里声明的符号：类型种类与修饰符都是精确的；
//! - 本文件里只被引用、声明在别处的符号：类型种类退回 L1 快速声明索引（它有 `DeclKind`），
//!   修饰符留空（宁可不标也不标错）。
//!
//! 同一个符号在**一个文件里**的全部出现位置共用同一份细节（在声明处读到 `const`，这个文件里
//! 对它的每一次引用也是 `const`），所以同一屏里不会出现同一个符号两种颜色。
//!
//! 只走语法、不绑定：这里拿到的是「这条声明写了哪些关键字」，不是语义结论。

use super::{CsSymbolKind, Modifiers};
use std::collections::HashMap;
use tree_sitter::{Node, Tree};

/// 一条声明的细节。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeclFacts {
    pub kind: CsSymbolKind,
    pub modifiers: Modifiers,
}

/// 一个文件里全部声明的细节，按声明标识符的（行, UTF-8 字节列）索引。
#[derive(Debug, Default)]
pub struct FileFacts {
    by_position: HashMap<(u32, u32), DeclFacts>,
}

/// 递归深度上限：防御畸形或深度嵌套的输入（与 L1 提取一致的量级）。
const MAX_DEPTH: u32 = 200;

impl FileFacts {
    /// 解析一份文本（自己建一次 parser；已有语法树时用 [`Self::from_tree`]）。
    pub fn parse(text: &str) -> Self {
        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .is_err()
        {
            return Self::default();
        }
        match parser.parse(text, None) {
            Some(tree) => Self::from_tree(&tree, text),
            None => Self::default(),
        }
    }

    pub fn from_tree(tree: &Tree, text: &str) -> Self {
        let mut out = Self::default();
        let mut walker = Walker {
            src: text.as_bytes(),
            out: &mut out,
        };
        walker.children(tree.root_node(), None, 0);
        out
    }

    pub fn at(&self, line: u32, column: u32) -> Option<DeclFacts> {
        self.by_position.get(&(line, column)).copied()
    }

    pub fn len(&self) -> usize {
        self.by_position.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_position.is_empty()
    }
}

struct Walker<'a> {
    src: &'a [u8],
    out: &'a mut FileFacts,
}

/// 类型声明节点 → 种类。`record_declaration` 带 `struct` 关键字时是 `record struct`。
fn type_kind(node: Node<'_>) -> Option<CsSymbolKind> {
    Some(match node.kind() {
        "class_declaration" => CsSymbolKind::Class,
        "struct_declaration" => CsSymbolKind::Struct,
        "interface_declaration" => CsSymbolKind::Interface,
        "enum_declaration" => CsSymbolKind::Enum,
        "delegate_declaration" => CsSymbolKind::Delegate,
        "record_struct_declaration" => CsSymbolKind::RecordStruct,
        "record_declaration" => {
            if has_token(node, "struct") {
                CsSymbolKind::RecordStruct
            } else {
                CsSymbolKind::Record
            }
        }
        _ => return None,
    })
}

fn has_token(node: Node<'_>, token: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|c| !c.is_named() && c.kind() == token);
    found
}

impl Walker<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        std::str::from_utf8(&self.src[node.byte_range()]).unwrap_or("")
    }

    /// 声明节点上的 `modifier` 关键字与 `[Obsolete]` 标注。
    fn modifiers_of(&self, node: Node<'_>) -> (Modifiers, bool) {
        let mut m = Modifiers::default();
        let mut is_const = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "modifier" => match self.text(child) {
                    "static" => m.static_ = true,
                    "readonly" => m.readonly = true,
                    "abstract" => m.abstract_ = true,
                    "async" => m.async_ = true,
                    "const" => is_const = true,
                    _ => {}
                },
                "attribute_list" if self.has_obsolete(child) => m.deprecated = true,
                _ => {}
            }
        }
        (m, is_const)
    }

    fn has_obsolete(&self, list: Node<'_>) -> bool {
        let mut cursor = list.walk();
        let found = list.named_children(&mut cursor).any(|attr| {
            let name = attr
                .child_by_field_name("name")
                .map_or("", |n| self.text(n));
            let name = name.rsplit('.').next().unwrap_or(name);
            name == "Obsolete" || name == "ObsoleteAttribute"
        });
        found
    }

    fn record(&mut self, name: Node<'_>, kind: CsSymbolKind, mut modifiers: Modifiers) {
        modifiers.declaration = true;
        let p = name.start_position();
        self.out.by_position.insert(
            (p.row as u32, p.column as u32),
            DeclFacts { kind, modifiers },
        );
    }

    /// `node` 的 `name` 子节点，没有就跳过这条声明。
    fn named(&mut self, node: Node<'_>, kind: CsSymbolKind, modifiers: Modifiers) {
        if let Some(name) = node.child_by_field_name("name") {
            self.record(name, kind, modifiers);
        }
    }

    /// `field_declaration` / `event_field_declaration` / `local_declaration_statement`：
    /// 修饰符在外层节点上，名字在 `variable_declaration > variable_declarator` 里。
    fn variables(&mut self, node: Node<'_>, kind: CsSymbolKind, modifiers: Modifiers) {
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
                self.record(name, kind, modifiers);
            }
        }
    }

    fn children(&mut self, node: Node<'_>, enclosing: Option<CsSymbolKind>, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        for child in children {
            self.visit(child, enclosing, depth);
        }
    }

    fn visit(&mut self, child: Node<'_>, enclosing: Option<CsSymbolKind>, depth: u32) {
        if let Some(kind) = type_kind(child) {
            let (modifiers, _) = self.modifiers_of(child);
            self.named(child, kind, modifiers);
            self.children(child, Some(kind), depth + 1);
            return;
        }
        match child.kind() {
            "field_declaration" | "event_field_declaration" | "local_declaration_statement" => {
                let (modifiers, is_const) = self.modifiers_of(child);
                let kind = match (child.kind(), is_const, enclosing) {
                    (_, true, _) => CsSymbolKind::Constant,
                    ("event_field_declaration", ..) => CsSymbolKind::Event,
                    ("local_declaration_statement", ..) => CsSymbolKind::Local,
                    _ => CsSymbolKind::Field,
                };
                self.variables(child, kind, modifiers);
                self.children(child, enclosing, depth + 1);
            }
            "enum_member_declaration" => {
                let (modifiers, _) = self.modifiers_of(child);
                self.named(child, CsSymbolKind::EnumMember, modifiers);
            }
            "method_declaration" => {
                let (modifiers, _) = self.modifiers_of(child);
                self.named(child, CsSymbolKind::Method, modifiers);
                self.children(child, enclosing, depth + 1);
            }
            "local_function_statement" => {
                let (modifiers, _) = self.modifiers_of(child);
                self.named(child, CsSymbolKind::LocalFunction, modifiers);
                self.children(child, enclosing, depth + 1);
            }
            // 构造函数 / 析构函数的名字就是类型名：按所在类型的种类着色，与 VS Code 一致。
            "constructor_declaration" | "destructor_declaration" => {
                let (modifiers, _) = self.modifiers_of(child);
                self.named(child, enclosing.unwrap_or(CsSymbolKind::Type), modifiers);
                self.children(child, enclosing, depth + 1);
            }
            "property_declaration" | "indexer_declaration" => {
                let (modifiers, _) = self.modifiers_of(child);
                self.named(child, CsSymbolKind::Property, modifiers);
                self.children(child, enclosing, depth + 1);
            }
            "event_declaration" => {
                let (modifiers, _) = self.modifiers_of(child);
                self.named(child, CsSymbolKind::Event, modifiers);
                self.children(child, enclosing, depth + 1);
            }
            "parameter" => {
                self.named(child, CsSymbolKind::Parameter, Modifiers::default());
                self.children(child, enclosing, depth + 1);
            }
            "type_parameter" => {
                self.named(child, CsSymbolKind::TypeParameter, Modifiers::default());
            }
            "from_clause" | "let_clause" | "join_clause" | "query_continuation" => {
                self.named(child, CsSymbolKind::RangeVariable, Modifiers::default());
                self.children(child, enclosing, depth + 1);
            }
            "declaration_expression" | "foreach_statement" | "catch_declaration" => {
                self.named(child, CsSymbolKind::Local, Modifiers::default());
                self.children(child, enclosing, depth + 1);
            }
            _ => self.children(child, enclosing, depth + 1),
        }
    }
}
