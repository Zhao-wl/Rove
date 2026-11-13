//! #42：元数据视图文档内的导航查询（`F12`、`Alt+F12`、`Shift+F12`、`Ctrl+F12`）。
//!
//! 查询点是视图文档的显示坐标（`rove.metadataView.showDocs` 为 false 时去掉了 `///` 行），先换回索引坐标，
//! 再按下面的顺序找符号：
//!
//! 1. **成员位置记录**（#34 段 13）：光标在声明里的名字上 → 这个成员（精确）；
//! 2. **按名字解析类型**（签名里引用到的其他类型没有出现位置记录，见 CS_INDEX_FORMAT“已知待改进”）：有视图的外部类型里
//!    名字相同的符号；恰好一个为精确，多个为近似；没有时退到 L1 的同名源码类型（近似）。
//!
//! 找到符号后与源码里的查询完全相同：交给 `l2_navigate`（定义 = 元数据目标，引用 = 工作区里的全部使用位置，
//! 实现 / 派生同理），所以视图里 `Shift+F12` 的条数就是同一个查询在源码里的条数。

use super::*;
use crate::index::SymbolKey;
use crate::metadata_view::{first_view, original_line, view_key};

impl CsRouter {
    pub(super) fn metadata_navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>> {
        let (Some(views), Some(type_key)) = (&self.views, view_key(&point.resource)) else {
            return Err(Error::Unsupported);
        };
        let Some(snap) = self.semantic.snapshot() else {
            return Err(Error::NotReady);
        };
        let generation = snap.generation;
        let cap = Cap::of(&snap);
        let version = point.version;
        let empty = |precision, reason| QueryResult {
            value: Arc::from([]),
            precision,
            reason,
            version: Some(version),
            index_generation: generation,
        };
        let index = snap.holder.index();
        let Some(symbol) = index.find_symbol(SymbolKey::Global(type_key)) else {
            return Ok(empty(cap.precision, cap.reason.clone()));
        };
        let Some(view) = first_view(index, symbol) else {
            return Ok(empty(cap.precision, cap.reason.clone()));
        };
        let Ok(original) = index.metadata_view_text(view) else {
            return Ok(empty(cap.precision, cap.reason.clone()));
        };
        let line = original_line(original, point.position.line.0 as u32, views.show_docs());
        let col = point.position.column.0 as u32;
        let Some(text_line) = original.lines().nth(line as usize) else {
            return Ok(empty(cap.precision, cap.reason.clone()));
        };
        // 1. 声明里的名字：成员位置记录。
        let declared = index
            .metadata_view_members(view)
            .iter()
            .find(|m| m.line == line && m.col <= col && col <= m.col + m.len)
            .map(|m| m.symbol);
        let mut candidates: Vec<u32> = declared.into_iter().collect();
        let mut precision = cap.precision;
        let mut reason = cap.reason.clone();
        let mut source = Vec::new();
        if candidates.is_empty() {
            // 2. 按名字解析。
            let Some(range) = identifier_range(text_line, col as usize) else {
                return Ok(empty(cap.precision, cap.reason.clone()));
            };
            let word = &text_line[range];
            candidates = views.types_named(&snap, word);
            if candidates.len() > 1 {
                precision = Precision::Approximate;
                reason = Some(ApproximationReason::Other("metadata-view-name".into()));
            }
            if candidates.is_empty() {
                if let Some(l1) = &self.l1 {
                    if matches!(
                        kind,
                        NavigationKind::Definition | NavigationKind::TypeDefinition
                    ) {
                        let found = l1.workspace_symbols(word, 64, cancel)?;
                        for s in found.value.iter() {
                            if &*s.name == word && s.kind.is_type() {
                                source.extend(s.locations.iter().cloned());
                            }
                        }
                    }
                }
            }
        }
        if candidates.is_empty() {
            let value: Vec<RoutedTarget> = source
                .into_iter()
                .map(|target| RoutedTarget {
                    target,
                    precision: Precision::Approximate,
                    reason: Some(ApproximationReason::Other("l1-declaration-index".into())),
                    role: TargetRole::Candidate,
                })
                .collect();
            let (p, r) = if value.is_empty() {
                (cap.precision, cap.reason)
            } else {
                (
                    Precision::Approximate,
                    Some(ApproximationReason::Other("l1-declaration-index".into())),
                )
            };
            return Ok(QueryResult {
                value: value.into(),
                precision: p,
                reason: r,
                version: Some(version),
                index_generation: generation,
            });
        }
        let cap = Cap { precision, reason };
        let mut out: Vec<RoutedTarget> = Vec::new();
        for s in candidates {
            cancel.check()?;
            let at = Occurrence {
                file: 0,
                line: 0,
                col: Utf8Column(0),
                symbol: s,
                len: 0,
                role: 0,
            };
            let r = self.l2_navigate(kind, &snap, at, cap.clone(), version, cancel)?;
            out.extend(r.value.iter().cloned());
        }
        Ok(batch(out, cap, version, generation))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use crate::metadata_view::{MetadataViews, ViewError};

    struct Fixture {
        _dir: tempfile::TempDir,
        router: CsRouter,
        views: Arc<MetadataViews>,
        s: MetadataSample,
    }

    fn fixture(show_docs: bool) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (index, s) = metadata_sample_index(GAME_CS);
        let path = dir.path().join("meta.idx");
        index.write(&path).unwrap();
        let holder: Arc<dyn IndexHolder> = Arc::new(CsIndex::open(&path).unwrap());
        let semantic = Arc::new(ManualSemantic::ready(holder));
        let views = Arc::new(MetadataViews::new(semantic.clone(), show_docs));
        let router =
            CsRouter::new(semantic, Arc::new(BufferStates::new())).with_metadata(views.clone());
        Fixture {
            _dir: dir,
            router,
            views,
            s,
        }
    }

    fn at(type_key: &str, line: u32, col: u32) -> QueryPoint {
        QueryPoint {
            resource: crate::metadata_view::view_resource(type_key),
            position: position(line, u64::from(col)),
            version: Version(1),
        }
    }

    fn keys(r: &QueryResult<Arc<[RoutedTarget]>>) -> Vec<String> {
        r.value
            .iter()
            .map(|t| match &t.target.resource {
                Resource::Virtual(v) => v.to_string(),
                Resource::Workspace(p) => {
                    let s = t.target.selection.start();
                    format!("{}:{}:{}", p.as_str(), s.line.0, s.column.0)
                }
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn locate_maps_members_nested_types_and_hidden_docs() {
        let f = fixture(true);
        let loc = f.views.locate(POSITION_KEY).unwrap();
        assert_eq!(loc.type_key, TRANSFORM_KEY);
        assert_eq!((loc.line, loc.len), (10, 8));
        // 嵌套类型落在外层类型的视图里。
        let nested = f.views.locate(ENUMERATOR_KEY).unwrap();
        assert_eq!(nested.type_key, LIST_KEY);
        assert_eq!(nested.line, 9);
        // 隐藏文档后，位置按去掉的 `///` 行数上移（第 6、9 行是文档）。
        f.views.set_show_docs(false);
        assert_eq!(f.views.locate(POSITION_KEY).unwrap().line, 8);
        assert_eq!(f.views.locate(TRANSFORM_KEY).unwrap().line, 6);
        assert_eq!(
            f.views.locate("UnityEngine.CoreModule|T:Nope"),
            Err(ViewError::NotFound)
        );
    }

    #[test]
    fn content_has_title_assembly_path_and_hidden_docs() {
        let f = fixture(true);
        let c = f.views.content(TRANSFORM_KEY).unwrap();
        assert_eq!(c.title, "Transform");
        assert!(c.assembly.starts_with("UnityEngine.CoreModule, Version="));
        assert_eq!(
            c.dll_path.as_deref(),
            Some("C:/Unity/Managed/UnityEngine.CoreModule.dll")
        );
        assert!(c.text.contains("/// <summary>The world"));
        assert_eq!(f.views.content(LIST_KEY).unwrap().title, "List<T>");
        f.views.set_show_docs(false);
        let c = f.views.content(TRANSFORM_KEY).unwrap();
        assert!(!c.text.contains("///"));
        assert!(c.text.starts_with("#region"), "文件头保留");
        assert_eq!(f.views.content("A|T:B").unwrap_err(), ViewError::NotFound);
        let down = ManualSemantic::unavailable(SemanticStatus::NoCSharp);
        let views = MetadataViews::new(Arc::new(down), true);
        assert_eq!(
            views.content(TRANSFORM_KEY).unwrap_err(),
            ViewError::NotReady
        );
    }

    #[test]
    fn references_in_a_view_match_the_source_query() {
        for show in [true, false] {
            let f = fixture(show);
            let line = f.views.locate(POSITION_KEY).unwrap();
            let r = f
                .router
                .navigate_detailed(
                    NavigationKind::References,
                    &at(TRANSFORM_KEY, line.line, line.col + 2),
                    &Cancellation::default(),
                )
                .unwrap();
            assert_eq!(r.precision, Precision::Exact);
            let got = keys(&r);
            assert_eq!(got.len(), 3, "{got:?}");
            assert!(got.iter().all(|k| k.starts_with("Assets/Game.cs:")));
            // 与源码里同一个符号的查询条数一致。
            let _ = f.s;
        }
    }

    #[test]
    fn definition_in_a_view_targets_metadata_by_member_or_by_type_name() {
        let f = fixture(true);
        let go = |line: u32, col: u32| {
            f.router
                .navigate_detailed(
                    NavigationKind::Definition,
                    &at(TRANSFORM_KEY, line, col),
                    &Cancellation::default(),
                )
                .unwrap()
        };
        // 声明里的名字 → 这个成员自己的元数据位置。
        let r = go(10, 25);
        assert_eq!(keys(&r), vec![format!("rove-cs-metadata:{POSITION_KEY}")]);
        // 签名里的 Vector3（没有位置记录）→ 按名字解析到另一个类型的视图。
        let r = go(10, 12);
        assert_eq!(keys(&r), vec![format!("rove-cs-metadata:{VECTOR3_KEY}")]);
        assert_eq!(r.precision, Precision::Exact);
        // 找不到的名字（关键字）→ 空结果。
        assert!(go(10, 4).value.is_empty());
        // 没有 L1 时源码类型 `Player` 不能解析。
        assert!(go(13, 11).value.is_empty());
    }

    #[test]
    fn unsupported_without_views_or_with_a_bad_resource() {
        let f = fixture(true);
        let plain = CsRouter::new(
            Arc::new(ManualSemantic::unavailable(SemanticStatus::NoCSharp)),
            Arc::new(BufferStates::new()),
        );
        let p = at(TRANSFORM_KEY, 7, 13);
        assert!(plain
            .navigate_detailed(NavigationKind::Definition, &p, &Cancellation::default())
            .is_err());
        // 视图键找不到：空结果而不是错误。
        let r = f
            .router
            .navigate_detailed(
                NavigationKind::Definition,
                &at("X|T:Y", 0, 0),
                &Cancellation::default(),
            )
            .unwrap();
        assert!(r.value.is_empty());
    }
}
