//! C#：命名空间、类型、成员（字段、属性、方法、构造函数、事件、枚举成员、运算符、索引器）。
//!
//! 名称格式按 VS Code C# 大纲（Roslyn 文档符号）：类型带类型参数 `List<T>`，方法与构造函数带参数
//! 类型 `Move(Vector3, float)`，索引器 `this[int]`，析构函数 `~Foo()`。`#if` 等预处理分支里的成员照常
//! 列出（Unity 脚本常见 `#if UNITY_EDITOR`）。局部函数、访问器不进入大纲。
use super::{
    kinds,
    source::{named_children, span, Source},
};
use crate::{Symbol, SymbolKind};
use rove_base::Result;
use tree_sitter::Node;

pub(super) fn extract(src: &Source) -> Result<Vec<Symbol>> {
    let tree = src.parse(tree_sitter_c_sharp::LANGUAGE.into())?;
    let root = tree.root_node();
    members(src, root, root.end_byte())
}

/// 一个容器（编译单元、命名空间、类型体、预处理分支）里的成员，按文档顺序。
fn members(src: &Source, container: Node, end: usize) -> Result<Vec<Symbol>> {
    let mut out = Vec::new();
    let children = named_children(container);
    for (i, node) in children.iter().enumerate() {
        if node.kind() == "file_scoped_namespace_declaration" {
            // `namespace A.B;` 之后的声明都属于它，直到容器结束。
            let mut inner = Vec::new();
            for sibling in &children[i + 1..] {
                member(src, *sibling, &mut inner)?;
            }
            let name = node.child_by_field_name("name");
            out.push(src.symbol(
                name.map(|n| src.compact(n)).unwrap_or_default(),
                SymbolKind::Namespace,
                (node.start_byte(), end),
                name.map_or(span(*node), span),
                inner,
            )?);
            break;
        }
        member(src, *node, &mut out)?;
    }
    Ok(out)
}

fn member(src: &Source, node: Node, out: &mut Vec<Symbol>) -> Result<()> {
    let kind = match node.kind() {
        "namespace_declaration" => SymbolKind::Namespace,
        "class_declaration" => SymbolKind::Class,
        "struct_declaration" => SymbolKind::Struct,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        "record_declaration" => {
            if has_token(node, "struct") {
                SymbolKind::Struct
            } else {
                SymbolKind::Class
            }
        }
        "delegate_declaration" => SymbolKind::Function,
        // 局部函数只会在语法错误恢复时出现在成员位置（见 `recover`）。
        "method_declaration" | "destructor_declaration" | "local_function_statement" => {
            SymbolKind::Method
        }
        "constructor_declaration" => SymbolKind::Constructor,
        "property_declaration" | "indexer_declaration" => SymbolKind::Property,
        "event_declaration" => SymbolKind::Event,
        "enum_member_declaration" => SymbolKind::Other(kinds::ENUM_MEMBER.into()),
        "operator_declaration" | "conversion_operator_declaration" => {
            SymbolKind::Other(kinds::OPERATOR.into())
        }
        "field_declaration" | "event_field_declaration" => return fields(src, node, out),
        // `#if`/`#elif`/`#else` 分支与声明列表：成员属于外层容器。
        "ERROR" => return recover(src, node, out),
        k if k.starts_with("preproc_") || k == "declaration_list" => {
            out.extend(members(src, node, node.end_byte())?);
            return Ok(());
        }
        _ => return Ok(()),
    };
    let name_node = node.child_by_field_name("name");
    let mut name = String::new();
    if let Some(spec) = child_of_kind(node, "explicit_interface_specifier") {
        name.push_str(&src.compact(spec));
    }
    let mut selection = name_node.map(span);
    match node.kind() {
        "indexer_declaration" => {
            let this = child_token(node, "this");
            selection = this.map(span);
            name.push_str("this");
            if let Some(params) = node.child_by_field_name("parameters") {
                name.push_str(&parameter_types(src, params, '[', ']'));
            }
        }
        "operator_declaration" => {
            let op = node.child_by_field_name("operator");
            selection = op.map(span);
            name.push_str("operator ");
            name.push_str(op.map(|n| src.text(n)).unwrap_or_default());
        }
        "conversion_operator_declaration" => {
            for token in ["implicit", "explicit"] {
                if let Some(t) = child_token(node, token) {
                    name.push_str(token);
                    name.push(' ');
                    selection = Some(span(t));
                }
            }
            name.push_str("operator ");
            if let Some(ty) = node.child_by_field_name("type") {
                name.push_str(&src.compact(ty));
            }
        }
        "destructor_declaration" => {
            name.push('~');
            name.push_str(name_node.map(|n| src.text(n)).unwrap_or_default());
        }
        _ => name.push_str(name_node.map(|n| src.text(n)).unwrap_or_default()),
    }
    if let Some(params) = node
        .child_by_field_name("type_parameters")
        .or_else(|| child_of_kind(node, "type_parameter_list"))
    {
        name.push_str(&src.compact(params));
    }
    if matches!(
        node.kind(),
        "method_declaration"
            | "local_function_statement"
            | "constructor_declaration"
            | "destructor_declaration"
            | "delegate_declaration"
            | "operator_declaration"
            | "conversion_operator_declaration"
    ) {
        if let Some(params) = node.child_by_field_name("parameters") {
            name.push_str(&parameter_types(src, params, '(', ')'));
        } else {
            name.push_str("()");
        }
    }
    let children = match node
        .child_by_field_name("body")
        .filter(|b| b.kind() == "declaration_list" || b.kind() == "enum_member_declaration_list")
        .or_else(|| child_of_kind(node, "enum_member_declaration_list"))
    {
        Some(body) if is_type(node.kind()) || node.kind() == "namespace_declaration" => {
            members(src, body, body.end_byte())?
        }
        _ => Vec::new(),
    };
    out.push(src.symbol(
        name,
        kind,
        span(node),
        selection.unwrap_or((node.start_byte(), node.start_byte())),
        children,
    )?);
    Ok(())
}

/// 语法错误：tree-sitter 把未闭合的类型体放进 ERROR 节点，只留下 `class` 记号与名称。
/// 认出“类型关键字 + 名称”时，把它之后的成员归到这个类型下，区间到 ERROR 结束（与 Roslyn
/// 对未闭合类型的大纲一致）；否则成员属于外层容器。
fn recover(src: &Source, node: Node, out: &mut Vec<Symbol>) -> Result<()> {
    let mut cursor = node.walk();
    let all: Vec<Node> = node.children(&mut cursor).collect();
    let keyword = all.iter().enumerate().find_map(|(i, c)| {
        let kind = match c.kind() {
            "class" | "record" if !c.is_named() => SymbolKind::Class,
            "struct" if !c.is_named() => SymbolKind::Struct,
            "interface" if !c.is_named() => SymbolKind::Interface,
            "enum" if !c.is_named() => SymbolKind::Enum,
            _ => return None,
        };
        let name = all.get(i + 1).filter(|n| n.kind() == "identifier")?;
        Some((i, kind, *name))
    });
    let Some((at, kind, name)) = keyword else {
        for child in named_children(node) {
            member(src, child, out)?;
        }
        return Ok(());
    };
    // 关键字前的修饰符与特性属于这个类型。
    let start = all[..at]
        .iter()
        .rev()
        .take_while(|c| matches!(c.kind(), "modifier" | "attribute_list"))
        .last()
        .unwrap_or(&all[at])
        .start_byte();
    for child in all[..at].iter().filter(|c| c.is_named()) {
        if child.start_byte() < start {
            member(src, *child, out)?;
        }
    }
    let mut inner = Vec::new();
    for child in all[at + 2..].iter().filter(|c| c.is_named()) {
        member(src, *child, &mut inner)?;
    }
    out.push(src.symbol(
        src.text(name),
        kind,
        (start, node.end_byte()),
        span(name),
        inner,
    )?);
    Ok(())
}

/// `int a, b;` 产出两个字段；只有一个变量时用整条声明的区间（含特性），否则用各自声明符的区间。
fn fields(src: &Source, node: Node, out: &mut Vec<Symbol>) -> Result<()> {
    let event = node.kind() == "event_field_declaration";
    let constant = named_children(node)
        .iter()
        .any(|c| c.kind() == "modifier" && src.text(*c) == "const");
    let kind = if event {
        SymbolKind::Event
    } else if constant {
        SymbolKind::Constant
    } else {
        SymbolKind::Field
    };
    let Some(declaration) = child_of_kind(node, "variable_declaration") else {
        return Ok(());
    };
    let declarators: Vec<_> = named_children(declaration)
        .into_iter()
        .filter(|c| c.kind() == "variable_declarator")
        .collect();
    let single = declarators.len() == 1;
    for d in declarators {
        let name = d.child_by_field_name("name").or_else(|| d.named_child(0));
        out.push(src.symbol(
            name.map(|n| src.text(n)).unwrap_or_default(),
            kind.clone(),
            if single { span(node) } else { span(d) },
            name.map_or(span(d), span),
            Vec::new(),
        )?);
    }
    Ok(())
}

fn is_type(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration"
            | "struct_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
    )
}

/// `(int, ref Vector3, params string[])`：参数类型与 ref/out/in/params 修饰符，不含参数名与 `this`。
fn parameter_types(src: &Source, list: Node, open: char, close: char) -> String {
    let mut out = String::new();
    out.push(open);
    let mut first = true;
    for param in named_children(list) {
        if param.kind() != "parameter" {
            continue;
        }
        if !first {
            out.push_str(", ");
        }
        first = false;
        for m in named_children(param) {
            if m.kind() == "modifier" && matches!(src.text(m), "ref" | "out" | "in" | "params") {
                out.push_str(src.text(m));
                out.push(' ');
            }
        }
        match param.child_by_field_name("type") {
            Some(ty) => out.push_str(&src.compact(ty)),
            None => out.push_str(
                param
                    .child_by_field_name("name")
                    .map(|n| src.text(n))
                    .unwrap_or_default(),
            ),
        }
    }
    // `params T[] x` 在这一版语法里直接挂在参数列表上（`params` 记号 + type/name 字段）。
    if child_token(list, "params").is_some() {
        if let Some(ty) = list.child_by_field_name("type") {
            if !first {
                out.push_str(", ");
            }
            out.push_str("params ");
            out.push_str(&src.compact(ty));
        }
    }
    out.push(close);
    out
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|c| c.kind() == kind);
    found
}

fn child_token<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|c| !c.is_named() && c.kind() == kind);
    found
}

fn has_token(node: Node, kind: &str) -> bool {
    child_token(node, kind).is_some()
}
