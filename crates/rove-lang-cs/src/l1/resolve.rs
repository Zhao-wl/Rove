//! 类型引用解析：C# 名称查找规则里 L1 能做到的部分——外层类型的嵌套类型、由内到外的命名空间、
//! 各层的 `using` / `using static` / `using X = Y;`、全局 using。不看基类（不知道继承链），
//! 不做重载、不推断类型。

use super::{extract::NONE, type_segment, State, UsingKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Segment<'a> {
    pub name: &'a str,
    pub arity: u8,
}

/// 引用处写出的类型文本，拆成带泛型参数个数的段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypeText<'a> {
    /// `global::` 或外部别名限定：只在全局命名空间查。
    pub global: bool,
    pub segments: Vec<Segment<'a>>,
}

impl<'a> TypeText<'a> {
    pub fn parse(text: &'a str) -> Option<Self> {
        let mut text = text.trim();
        // 去掉可空、数组、指针后缀（只在最外层）。
        loop {
            let t = text.trim_end();
            if let Some(rest) = t.strip_suffix('?').or_else(|| t.strip_suffix('*')) {
                text = rest;
            } else if t.ends_with(']') {
                let open = t.rfind('[')?;
                text = &t[..open];
            } else {
                text = t;
                break;
            }
        }
        let mut global = false;
        if let Some(i) = find_top(text, "::") {
            global = true;
            text = &text[i + 2..];
        }
        let mut segments = Vec::new();
        let mut depth = 0i32;
        let mut start = 0;
        let bytes = text.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'<' => depth += 1,
                b'>' => depth -= 1,
                b'.' if depth == 0 => {
                    segments.push(Segment::parse(&text[start..i])?);
                    start = i + 1;
                }
                _ => {}
            }
        }
        segments.push(Segment::parse(&text[start..])?);
        Some(Self { global, segments })
    }
}

impl<'a> Segment<'a> {
    fn parse(seg: &'a str) -> Option<Self> {
        let seg = seg.trim();
        let (name, args) = match seg.find('<') {
            Some(i) => (&seg[..i], Some(&seg[i + 1..])),
            None => (seg, None),
        };
        let name = name.trim();
        let name = name.strip_prefix('@').unwrap_or(name);
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let arity = match args {
            None => 0,
            Some(args) => {
                let mut depth = 0i32;
                let mut commas = 0usize;
                for b in args.bytes() {
                    match b {
                        b'<' | b'(' | b'[' => depth += 1,
                        b'>' | b')' | b']' => depth -= 1,
                        b',' if depth == 0 => commas += 1,
                        _ => {}
                    }
                }
                (commas + 1).min(u8::MAX as usize) as u8
            }
        };
        Some(Self { name, arity })
    }
}

fn find_top(text: &str, pat: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            _ if depth == 0 && text[i..].starts_with(pat) => return Some(i),
            _ => {}
        }
    }
    None
}

fn key(prefix: &str, segments: &[Segment<'_>]) -> String {
    let mut key = String::from(prefix);
    for s in segments {
        if !key.is_empty() {
            key.push('.');
        }
        key.push_str(&type_segment(s.name, s.arity));
    }
    key
}

fn lookup_key(state: &State, key: &str) -> Option<u32> {
    let name = state.names.lookup(key)?;
    state.types.get(&name).copied()
}

/// `prefix`（命名空间或类型键）下按段查找；段为空时查 `prefix` 本身。
pub(crate) fn lookup_path(state: &State, prefix: &str, segments: &[Segment<'_>]) -> Option<u32> {
    lookup_key(state, &key(prefix, segments))
}

/// using 目标文本在“声明它的作用域”中的候选前缀：由内到外加命名空间前缀，最后是原样（全局）。
fn using_prefixes(state: &State, fid: u32, scope: u32, target: &str) -> Vec<String> {
    let file = state.file(fid);
    let Some(text) = TypeText::parse(target) else {
        return Vec::new();
    };
    let base = key("", &text.segments);
    let mut out = Vec::new();
    if !text.global {
        let mut s = scope;
        while s != NONE {
            let ns = state.names.get(file.scopes[s as usize].ns);
            if !ns.is_empty() {
                out.push(format!("{ns}.{base}"));
            }
            s = file.scopes[s as usize].parent;
        }
    }
    out.push(base);
    out
}

pub(crate) fn resolve(
    state: &State,
    fid: u32,
    line: u32,
    col: u32,
    text: &TypeText<'_>,
) -> Vec<u32> {
    let segments = &text.segments;
    if text.global {
        return lookup_path(state, "", segments).into_iter().collect();
    }
    let file = state.file(fid);

    // 1. 外层类型（由内到外）的嵌套类型。先序存储，最后一个包含位置的类型声明就是最内层。
    let innermost_type = file
        .decls
        .iter()
        .enumerate()
        .rev()
        .find(|(_, d)| d.kind.is_type() && d.range.contains(line, col))
        .map(|(i, _)| i as u32);
    let mut t = innermost_type.unwrap_or(NONE);
    while t != NONE {
        let sid = file.symbols[t as usize];
        let type_key = state.names.get(state.symbols[sid as usize].key);
        if let Some(found) = lookup_path(state, type_key, segments) {
            return vec![found];
        }
        t = file.decls[t as usize].parent;
    }

    // 2. 由内到外的命名空间；每层先查命名空间成员，再查该层的 using。
    let innermost_scope = file
        .scopes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| s.range.contains(line, col))
        .map_or(0, |(i, _)| i as u32);
    let mut s = innermost_scope;
    while s != NONE {
        let scope = &file.scopes[s as usize];
        let ns = state.names.get(scope.ns);
        if let Some(found) = lookup_path(state, ns, segments) {
            return vec![found];
        }
        let mut found = Vec::new();
        let local = file
            .usings
            .iter()
            .filter(|u| u.scope == s)
            .map(|u| (fid, u));
        let global: Vec<(u32, &super::Using<super::Name>)> = if scope.parent == NONE {
            state
                .global_using_files
                .iter()
                .filter(|&&g| g != fid)
                .flat_map(|&g| {
                    state
                        .file(g)
                        .usings
                        .iter()
                        .filter(|u| u.global)
                        .map(move |u| (g, u))
                })
                .collect()
        } else {
            Vec::new()
        };
        for (ufid, using) in local.chain(global) {
            let target = state.names.get(using.target);
            let hit = match using.kind {
                UsingKind::Alias => {
                    let first = &segments[0];
                    if first.arity != 0 || &**state.names.get(using.alias) != first.name {
                        continue;
                    }
                    using_prefixes(state, ufid, using.scope, target)
                        .iter()
                        .find_map(|p| lookup_path(state, p, &segments[1..]))
                }
                UsingKind::Namespace | UsingKind::Static => {
                    using_prefixes(state, ufid, using.scope, target)
                        .iter()
                        .find_map(|p| lookup_path(state, p, segments))
                }
            };
            if let Some(hit) = hit {
                if !found.contains(&hit) {
                    found.push(hit);
                }
            }
        }
        if !found.is_empty() {
            return found;
        }
        // `namespace A.B.C` 等价于逐层嵌套：回到父作用域之前先查隐含的 A.B（该层没有 using）。
        let outer_ns = match scope.parent {
            NONE => "",
            p => state.names.get(file.scopes[p as usize].ns),
        };
        let mut implicit: &str = ns;
        while let Some(dot) = implicit.rfind('.') {
            implicit = &implicit[..dot];
            if implicit.len() <= outer_ns.len() {
                break;
            }
            if let Some(found) = lookup_path(state, implicit, segments) {
                return vec![found];
            }
        }
        s = scope.parent;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(name: &str, arity: u8) -> Segment<'_> {
        Segment { name, arity }
    }

    #[test]
    fn parses_generic_arity_qualifiers_and_suffixes() {
        let t =
            TypeText::parse("System.Collections.Generic.Dictionary<string, List<int>>").unwrap();
        assert!(!t.global);
        assert_eq!(
            t.segments,
            vec![
                seg("System", 0),
                seg("Collections", 0),
                seg("Generic", 0),
                seg("Dictionary", 2)
            ]
        );
        assert_eq!(
            TypeText::parse("List<>").unwrap().segments,
            vec![seg("List", 1)]
        );
        assert_eq!(
            TypeText::parse("Func<,,>").unwrap().segments,
            vec![seg("Func", 3)]
        );
        assert_eq!(
            TypeText::parse("Outer<int>.Inner[]?").unwrap().segments,
            vec![seg("Outer", 1), seg("Inner", 0)]
        );
        let g = TypeText::parse("global::Game.Player").unwrap();
        assert!(g.global);
        assert_eq!(g.segments, vec![seg("Game", 0), seg("Player", 0)]);
        assert_eq!(
            TypeText::parse("@class").unwrap().segments,
            vec![seg("class", 0)]
        );
        assert!(TypeText::parse("a+b").is_none());
        assert!(TypeText::parse("").is_none());
    }
}
