//! 覆盖层合并的单元测试：用构造出来的小索引（`router::testing::TestIndex`）与手写的覆盖层段，
//! 不依赖真实项目，也不依赖索引进程。
use super::*;
use crate::router::testing::{TestIndex, KIND_LOCAL, KIND_METHOD, KIND_TYPE};

const FOO_RUN: u32 = 0;
const IFOO_RUN: u32 = 1;
const FOO: u32 = 2;
const IFOO: u32 = 3;
const X: u32 = 4;

/// 基础索引：A.cs 声明 `class Foo : IFoo { void Run() { int x; } }`，B.cs 声明接口，C.cs 引用 Foo 与 Run。
/// 符号按写入端的规则排列：全局键按 Ordinal 序，局部键在后。
fn base() -> (tempfile::TempDir, CsIndex) {
    let mut t = TestIndex::default();
    let a = t.file("Assets/A.cs", 1);
    let b = t.file("Assets/B.cs", 2);
    let c = t.file("Assets/C.cs", 3);
    assert_eq!(
        t.symbol("A|M:Foo.Run", "Run", KIND_METHOD, Some(FOO)),
        FOO_RUN
    );
    assert_eq!(
        t.symbol("A|M:IFoo.Run", "Run", KIND_METHOD, Some(IFOO)),
        IFOO_RUN
    );
    assert_eq!(t.symbol("A|T:Foo", "Foo", KIND_TYPE, None), FOO);
    assert_eq!(t.symbol("A|T:IFoo", "IFoo", KIND_TYPE, None), IFOO);
    assert_eq!(t.local("x", KIND_LOCAL, a, 40), X);
    t.def(FOO, a, 0, 6);
    t.reference(IFOO, a, 0, 12);
    t.def(FOO_RUN, a, 1, 9);
    t.def(X, a, 2, 8);
    t.reference(X, a, 3, 4);
    t.def(IFOO, b, 0, 10);
    t.def(IFOO_RUN, b, 1, 9);
    t.reference(FOO, c, 3, 8);
    t.reference(FOO_RUN, c, 4, 12);
    t.relations.push((FOO, IFOO, 1));
    t.relations.push((FOO_RUN, IFOO_RUN, 2));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("base.rvcsidx");
    t.write(&path).unwrap();
    let index = CsIndex::open(&path).unwrap();
    (dir, index)
}

fn sym(key: &str, name: &str, kind: u8, container: Option<&str>) -> SegSymbol {
    SegSymbol {
        key: OvKey::Global(key.into()),
        name: name.into(),
        display: format!("{name}!"),
        kind,
        flags: 1,
        container: container.map(|c| OvKey::Global(c.into())),
    }
}

fn local(path: &str, offset: u32) -> SegSymbol {
    SegSymbol {
        key: OvKey::Local {
            path: path.into(),
            offset,
        },
        name: "x".into(),
        display: "int x".into(),
        kind: KIND_LOCAL,
        flags: 1,
        container: None,
    }
}

fn occ(line: u32, col: u32, symbol: u32, def: bool) -> SegOccurrence {
    SegOccurrence {
        line,
        col,
        len: 3,
        symbol,
        role: if def { 2 } else { 1 },
    }
}

/// 段 1：A.cs 把 Run 改名为 Go、去掉 `: IFoo`，类型下移一行；新建 D.cs 引用 Foo。
fn rename_segment() -> Segment {
    Segment {
        sequence: 1,
        symbols: vec![
            sym("A|T:Foo", "Foo", KIND_TYPE, None),
            sym("A|M:Foo.Go", "Go", KIND_METHOD, Some("A|T:Foo")),
            local("Assets/A.cs", 70),
        ],
        files: vec![
            SegFile {
                path: "Assets/A.cs".into(),
                hash: 11,
                occurrences: vec![
                    occ(1, 6, 0, true),
                    occ(2, 9, 1, true),
                    occ(3, 8, 2, true),
                    occ(4, 4, 2, false),
                ],
                inactive: vec![InactiveLines { start: 7, end: 9 }],
                declarations: vec![(0, 5), (1, 6)],
                ..Default::default()
            },
            SegFile {
                path: "Assets/D.cs".into(),
                hash: 44,
                occurrences: vec![occ(0, 4, 0, false)],
                ..Default::default()
            },
        ],
        owners: vec![(0, OWNER_SELF), (0, OWNER_MEMBERS), (1, OWNER_SELF)],
        relations: vec![],
    }
}

fn locations(view: &L2View, occs: &[Occurrence]) -> Vec<(String, u32, u64, bool)> {
    occs.iter()
        .map(|o| {
            (
                view.file_path(o.file).unwrap().to_owned(),
                o.line,
                o.col.0,
                Role(o.role).is_def(),
            )
        })
        .collect()
}

fn loc(path: &str, line: u32, col: u64, def: bool) -> (String, u32, u64, bool) {
    (format!("Assets/{path}"), line, col, def)
}

#[test]
fn without_overlay_the_view_is_the_base_index() {
    let (_dir, index) = base();
    let view = L2View::new(&index, None);
    let a = view.file_id("Assets/A.cs").unwrap();
    assert_eq!(view.file_hash(a), Some(1));
    let at = view.occurrence_at(a, 1, Utf8Column(10)).unwrap();
    assert_eq!(at.symbol, FOO_RUN);
    assert_eq!(
        locations(&view, &view.occurrences(FOO)),
        vec![loc("A.cs", 0, 6, true), loc("C.cs", 3, 8, false)]
    );
    assert_eq!(view.bases(FOO).len(), 1);
    assert_eq!(view.implementations(IFOO_RUN)[0].from, FOO_RUN);
}

#[test]
fn changed_file_replaces_its_occurrences_and_symbols() {
    let (_dir, index) = base();
    let overlay = Overlay::build(&index, &[rename_segment()]);
    let view = L2View::new(&index, Some(&overlay));
    let a = view.file_id("Assets/A.cs").unwrap();
    // 文件版本（内容哈希）换成覆盖层的，#38 据此判断缓冲区与索引是否一致
    assert_eq!(view.file_hash(a), Some(11));
    assert_eq!(
        view.inactive_lines(a),
        &[InactiveLines { start: 7, end: 9 }]
    );
    // 旧位置上已经没有 Run 了；新位置是 Foo 与新符号 Go
    assert_eq!(view.occurrence_at(a, 1, Utf8Column(10)), None);
    assert_eq!(view.occurrence_at(a, 1, Utf8Column(7)).unwrap().symbol, FOO);
    let go = view.occurrence_at(a, 2, Utf8Column(10)).unwrap().symbol;
    assert!(go >= index.symbols().len() as u32, "overlay-only symbol");
    assert_eq!(view.global_key(go), Some("A|M:Foo.Go"));
    assert_eq!(view.kind(go), Some(KIND_METHOD));
    assert_eq!(view.display(go), Some("Go!"));
    assert_eq!(view.container(go), Some(FOO));
    assert_eq!(
        view.display(FOO),
        Some("Foo!"),
        "source record from the overlay wins"
    );
    // 定义只剩覆盖层里的；引用合并基础（C.cs）与新文件（D.cs），按路径序
    assert_eq!(
        locations(&view, &view.occurrences(FOO)),
        vec![
            loc("A.cs", 1, 6, true),
            loc("C.cs", 3, 8, false),
            loc("D.cs", 0, 4, false)
        ]
    );
    assert_eq!(
        view.definitions(FOO_RUN),
        vec![],
        "Run's declaration is gone"
    );
    // C.cs 还没有重绑：它对 Run 的引用要等影响分析的后台重绑
    assert_eq!(
        locations(&view, &view.occurrences(FOO_RUN)),
        vec![loc("C.cs", 4, 12, false)]
    );
    // 关系：Foo 自己的基类型与成员实现关系都已被重算（为空）
    assert!(view.bases(FOO).is_empty());
    assert!(view.implementations(IFOO).is_empty());
    assert!(view.implementations(IFOO_RUN).is_empty());
    assert!(view.bases(FOO_RUN).is_empty());
    // 被替换文件里的局部符号：基础的已失效，新的按新偏移
    assert!(view.occurrences(X).is_empty());
    let x = view.occurrence_at(a, 4, Utf8Column(5)).unwrap().symbol;
    assert_eq!(
        locations(&view, &view.occurrences(x)),
        vec![loc("A.cs", 3, 8, true), loc("A.cs", 4, 4, false)]
    );
    // 新文件
    let d = view.file_id("Assets/D.cs").unwrap();
    assert_eq!(d, index.files().len() as u32);
    assert_eq!(view.file_path(d), Some("Assets/D.cs"));
    assert_eq!(view.occurrence_at(d, 0, Utf8Column(4)).unwrap().symbol, FOO);
}

#[test]
fn deleted_file_leaves_no_dangling_occurrences() {
    let (_dir, index) = base();
    let deleted = Segment {
        sequence: 2,
        files: vec![SegFile {
            path: "Assets/C.cs".into(),
            deleted: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let overlay = Overlay::build(&index, &[rename_segment(), deleted]);
    let view = L2View::new(&index, Some(&overlay));
    assert_eq!(view.file_id("Assets/C.cs"), None);
    assert_eq!(view.file_hash(2), None);
    assert_eq!(
        locations(&view, &view.occurrences(FOO)),
        vec![loc("A.cs", 1, 6, true), loc("D.cs", 0, 4, false)]
    );
    assert!(view.occurrences(FOO_RUN).is_empty());
    // 删除接口所在文件：接口的定义、它在基础索引里的关系一并消失
    let gone = Segment {
        sequence: 3,
        files: vec![SegFile {
            path: "Assets/B.cs".into(),
            deleted: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let overlay = Overlay::build(&index, &[gone]);
    let view = L2View::new(&index, Some(&overlay));
    assert!(view.definitions(IFOO).is_empty());
    // Foo 仍声明在 A.cs（未被取代）：它对 IFoo 的关系沿用基础索引，直到 A.cs 被影响分析重绑
    assert_eq!(view.bases(FOO).len(), 1);
}

#[test]
fn later_segments_win_and_relations_follow_their_owner() {
    let (_dir, index) = base();
    // 段 2：A.cs 恢复 `: IFoo` 与 Run（Run 实现 IFoo.Run）
    let restored = Segment {
        sequence: 2,
        symbols: vec![
            sym("A|T:Foo", "Foo", KIND_TYPE, None),
            sym("A|M:Foo.Run", "Run", KIND_METHOD, Some("A|T:Foo")),
            sym("A|T:IFoo", "IFoo", KIND_TYPE, None),
            sym("A|M:IFoo.Run", "Run", KIND_METHOD, Some("A|T:IFoo")),
        ],
        files: vec![SegFile {
            path: "Assets/A.cs".into(),
            hash: 12,
            occurrences: vec![occ(0, 6, 0, true), occ(0, 12, 2, false), occ(5, 9, 1, true)],
            declarations: vec![(0, 5), (1, 7)],
            ..Default::default()
        }],
        owners: vec![(0, OWNER_SELF), (0, OWNER_MEMBERS), (1, OWNER_SELF)],
        relations: vec![(0, 2, 1), (1, 3, 2)],
    };
    let overlay = Overlay::build(&index, &[rename_segment(), restored]);
    assert_eq!((overlay.segments, overlay.sequence), (2, 2));
    let view = L2View::new(&index, Some(&overlay));
    let a = view.file_id("Assets/A.cs").unwrap();
    assert_eq!(view.file_hash(a), Some(12));
    assert_eq!(
        view.occurrence_at(a, 5, Utf8Column(9)).unwrap().symbol,
        FOO_RUN
    );
    assert_eq!(
        locations(&view, &view.definitions(FOO_RUN)),
        vec![loc("A.cs", 5, 9, true)]
    );
    // 关系来自最新的段（基础索引里的同一条已被取代，不重复）
    let bases: Vec<_> = view.bases(FOO).iter().map(|r| (r.to, r.kind)).collect();
    assert_eq!(bases, vec![(IFOO, 1)]);
    let implementations: Vec<_> = view
        .implementations(IFOO_RUN)
        .iter()
        .map(|r| (r.from, r.kind))
        .collect();
    assert_eq!(implementations, vec![(FOO_RUN, 2)]);
    // 段 1 新建的 D.cs 仍在
    assert!(view.file_id("Assets/D.cs").is_some());
    // 段 1 的 Go 只出现在被取代的文件版本里：不再解析成符号，也没有任何出现位置
    let go = overlay
        .extra
        .iter()
        .position(|r| r.key == OvKey::Global("A|M:Foo.Go".into()));
    assert!(go.is_none_or(|g| view
        .occurrences(index.symbols().len() as u32 + g as u32)
        .is_empty()));
}

#[test]
fn local_keys_in_unchanged_files_resolve_to_base_symbols() {
    let (_dir, index) = base();
    // C.cs 被重绑，它引用了 A.cs（未被取代）里偏移 40 的局部符号（如命名实参指向的参数）
    let seg = Segment {
        sequence: 1,
        symbols: vec![local("Assets/A.cs", 40)],
        files: vec![SegFile {
            path: "Assets/C.cs".into(),
            hash: 33,
            occurrences: vec![occ(9, 1, 0, false)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let overlay = Overlay::build(&index, &[seg]);
    let view = L2View::new(&index, Some(&overlay));
    let c = view.file_id("Assets/C.cs").unwrap();
    assert_eq!(view.occurrence_at(c, 9, Utf8Column(2)).unwrap().symbol, X);
    assert_eq!(
        locations(&view, &view.occurrences(X)),
        vec![
            loc("A.cs", 2, 8, true),
            loc("A.cs", 3, 4, false),
            loc("C.cs", 9, 1, false)
        ]
    );
}

#[test]
fn journal_keeps_valid_prefix_only() {
    let (_dir, index) = base();
    let id = index.identity();
    let mut bytes = rename_segment().encode(&id);
    let first = bytes.len();
    assert_eq!(first % 8, 0);
    let mut second = Segment {
        sequence: 2,
        ..Default::default()
    }
    .encode(&id);
    bytes.append(&mut second);
    let (segments, valid) = decode_journal(&bytes, &id);
    assert_eq!((segments.len(), valid), (2, bytes.len()));
    assert_eq!(segments[0], rename_segment());
    // 写到一半的尾部
    let (segments, valid) = decode_journal(&bytes[..bytes.len() - 3], &id);
    assert_eq!((segments.len(), valid), (1, first));
    // 校验和不对
    let mut corrupt = bytes.clone();
    corrupt[first + 44] ^= 1;
    assert_eq!(decode_journal(&corrupt, &id).0.len(), 1);
    // 属于另一个基础索引
    assert!(decode_journal(&bytes, &[9; 16]).0.is_empty());
    // 序号不递增
    let mut repeated = rename_segment().encode(&id);
    repeated.extend(rename_segment().encode(&id));
    assert_eq!(decode_journal(&repeated, &id).0.len(), 1);
}

#[test]
fn find_symbol_uses_utf16_ordinal_order() {
    let (_dir, index) = base();
    assert_eq!(index.find_symbol(SymbolKey::Global("A|T:Foo")), Some(FOO));
    assert_eq!(index.find_symbol(SymbolKey::Global("A|T:Nope")), None);
    assert_eq!(
        index.find_symbol(SymbolKey::Local {
            file: 0,
            offset: ByteOffset(40)
        }),
        Some(X)
    );
    assert_eq!(
        index.find_symbol(SymbolKey::Local {
            file: 0,
            offset: ByteOffset(41)
        }),
        None
    );
}

#[test]
fn superseded_file_versions_lose_their_relation_ownership() {
    let (_dir, index) = base();
    // 段 1：A.cs 被（影响分析）重绑，仍声明 Run，Run 实现 IFoo.Run
    let first = Segment {
        sequence: 1,
        symbols: vec![
            sym("A|T:Foo", "Foo", KIND_TYPE, None),
            sym("A|M:Foo.Run", "Run", KIND_METHOD, Some("A|T:Foo")),
            sym("A|T:IFoo", "IFoo", KIND_TYPE, None),
            sym("A|M:IFoo.Run", "Run", KIND_METHOD, Some("A|T:IFoo")),
        ],
        files: vec![SegFile {
            path: "Assets/A.cs".into(),
            hash: 21,
            occurrences: vec![occ(0, 6, 0, true), occ(0, 12, 2, false), occ(1, 9, 1, true)],
            ..Default::default()
        }],
        owners: vec![(0, OWNER_SELF), (0, OWNER_MEMBERS), (1, OWNER_SELF)],
        relations: vec![(0, 2, 1), (1, 3, 2), (1, 3, 3)],
    };
    // 段 2：A.cs 再被改写，Run 改名为 Go（Run 不再声明）；Foo 的成员关系重算为空
    let mut second = rename_segment();
    second.sequence = 2;
    let overlay = Overlay::build(&index, &[first, second]);
    let view = L2View::new(&index, Some(&overlay));
    assert!(view.definitions(FOO_RUN).is_empty());
    assert!(
        view.bases(FOO_RUN).is_empty(),
        "stale ownership from the superseded version"
    );
    assert!(view.implementations(IFOO_RUN).is_empty());
}
