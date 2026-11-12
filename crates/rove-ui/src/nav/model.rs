//! #40 的纯逻辑（不依赖 GPUI，单元测试覆盖）：查询结果 → 呈现方式（跳转 / 速览 / 符号导航 / 空状态）、
//! 符号导航模式、近似提示去重、语义同名高亮的区间集合。

use super::settings::GotoMode;
use rove_base::{Line, Position, Resource, TextRange, Utf8Column};
use rove_lang_cs::query::{NavigationKind, Precision, QueryResult, Target};
use rove_lang_cs::router::{RoutedTarget, TargetRole, METADATA_SCHEME};
use std::collections::HashSet;
use std::sync::Arc;

/// 一次 `F12` → 目标行画面完成的耗时分解（毫秒）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavSample {
    /// 后台查询（`CsRouter::navigate_detailed`）。
    pub query_ms: f64,
    /// 从按键到“目标位置进入这一帧”（查询 + 打开文档服务 + 代码视图定位，开始绘制前）。
    pub frame_ms: f64,
    /// 这一帧代码视图的绘制 CPU（prepaint → paint）；`frame_ms + paint_cpu_ms` 即按键到画面完成。
    /// 下一帧回调里读出，测试环境的假平台没有帧回调时为 `None`。
    pub paint_cpu_ms: Option<f64>,
    /// 这一帧里用等宽占位或尚未读到的行数（0 表示目标画面是正式整形结果）。
    pub placeholders: Option<u32>,
    /// 从按键到下一帧回调（含等待下一次垂直同步，是画面完成的上界）。
    pub next_frame_ms: Option<f64>,
    /// 从按键到第一帧没有占位行的帧回调（目标区域整形缓存未命中时晚一两帧；上界，含等待垂直同步）。
    pub full_ms: Option<f64>,
}

impl NavSample {
    /// 按键到画面完成（这一帧绘制结束）。
    pub fn painted_ms(&self) -> Option<f64> {
        Some(self.frame_ms + self.paint_cpu_ms?)
    }
}

/// 用户发起的跳转种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKind {
    /// `revealDefinition`、`Ctrl+点击`。
    Definition,
    /// `goToDeclaration`：C# 中与转到定义等价（同一份数据），只有空状态的措辞不同。
    Declaration,
    /// `goToImplementation`（`ctrl+f12`）：接口、虚方法、抽象方法的实现 / 重写，类型的派生类型。
    Implementation,
}

impl NavKind {
    pub fn query(self) -> NavigationKind {
        match self {
            NavKind::Definition | NavKind::Declaration => NavigationKind::Definition,
            NavKind::Implementation => NavigationKind::Implementations,
        }
    }

    /// 空状态（UX_SPEC §6.3）的字符串 key；近似时追加“（语义索引构建中）”。
    pub fn no_result_key(self, approximate: bool) -> &'static str {
        match (self, approximate) {
            (NavKind::Definition, false) => "nav.noDefinition",
            (NavKind::Definition, true) => "nav.noDefinition.approximate",
            (NavKind::Declaration, false) => "nav.noDeclaration",
            (NavKind::Declaration, true) => "nav.noDeclaration.approximate",
            (NavKind::Implementation, false) => "nav.noImplementation",
            (NavKind::Implementation, true) => "nav.noImplementation.approximate",
        }
    }

    pub fn mode(self, s: &super::settings::NavSettings) -> GotoMode {
        match self {
            NavKind::Definition => s.multiple_definitions,
            NavKind::Declaration => s.multiple_declarations,
            NavKind::Implementation => s.multiple_implementations,
        }
    }
}

/// 交给速览窗口（#41）的请求。标题文案由本模块决定（UX_SPEC §2.2）：精确结果没有任何标记，
/// 近似时为“近似 · N 个候选”。
#[derive(Clone, Debug)]
pub struct PeekRequest {
    pub kind: NavKind,
    /// 查询点所在的资源与位置（速览窗口嵌在这一行下方）。
    pub origin: (Resource, Position),
    pub targets: Arc<[Target]>,
    /// 近似时的标题标记（字符串 key 与参数）；精确为 `None`。
    pub annotation: Option<(&'static str, Vec<(&'static str, String)>)>,
}

/// 一次跳转的呈现方式。
#[derive(Clone, Debug)]
pub enum Plan {
    /// 没有结果：光标处提示（§6.1 第 2 级）。
    NoResult { key: &'static str },
    /// 只有外部符号（元数据视图）。#42 合并前在状态栏提示“未找到 `X` 的元数据”，不打开空标签。
    Metadata { targets: Arc<[Target]> },
    /// 单个候选：直接跳转；近似时状态栏临时提示（同一符号只提示一次）。
    Jump { target: Target, approximate: bool },
    /// 多个候选：按 `editor.gotoLocation.multiple*`。
    Multiple {
        mode: GotoMode,
        targets: Arc<[Target]>,
        approximate: bool,
    },
}

/// 外部符号的元数据目标（#34 / #42）。
pub fn is_metadata(target: &Target) -> bool {
    matches!(&target.resource, Resource::Virtual(uri) if uri.starts_with(METADATA_SCHEME))
}

/// 查询结果 → 呈现方式。`approximate` 按逐条精确度取最弱（契约：混合时用最弱的）；源码目标优先于
/// 元数据目标；同一位置的重复结果合并。
pub fn plan(kind: NavKind, result: &QueryResult<Arc<[RoutedTarget]>>, mode: GotoMode) -> Plan {
    let approximate = result.precision != Precision::Exact
        || result
            .value
            .iter()
            .any(|t| t.precision != Precision::Exact || t.role == TargetRole::Candidate);
    let mut seen = HashSet::new();
    let mut source = Vec::new();
    let mut metadata = Vec::new();
    for routed in result.value.iter() {
        let t = &routed.target;
        let start = t.selection.start();
        let key = (resource_key(&t.resource), start.line.0, start.column.0);
        if !seen.insert(key) {
            continue;
        }
        if is_metadata(t) {
            metadata.push(t.clone());
        } else {
            source.push(t.clone());
        }
    }
    match (source.len(), metadata.is_empty()) {
        (0, true) => Plan::NoResult {
            key: kind.no_result_key(approximate),
        },
        (0, false) => Plan::Metadata {
            targets: metadata.into(),
        },
        (1, _) => Plan::Jump {
            target: source.pop().unwrap(),
            approximate,
        },
        _ => Plan::Multiple {
            mode,
            targets: source.into(),
            approximate,
        },
    }
}

/// 资源的比较键（`Resource` 本身没有实现 `Hash`）。
pub fn resource_key(r: &Resource) -> String {
    match r {
        Resource::Workspace(p) => format!("w:{}", p.as_str()),
        Resource::File(p) => format!("f:{}", p.display()),
        Resource::Virtual(v) => format!("v:{v}"),
    }
}

/// 速览标题的近似标记（UX_SPEC §2.2）：精确结果没有任何标记。
pub fn peek_annotation(
    approximate: bool,
    candidates: usize,
) -> Option<(&'static str, Vec<(&'static str, String)>)> {
    approximate.then(|| {
        (
            "nav.peek.approximateCandidates",
            vec![("count", candidates.to_string())],
        )
    })
}

/// 查找引用速览的标题标记（文案由 #40 决定、#41 呈现）：近似时为“近似”，精确结果没有任何标记。
pub fn references_annotation(
    approximate: bool,
) -> Option<(&'static str, Vec<(&'static str, String)>)> {
    approximate.then(|| ("nav.peek.approximate", Vec::new()))
}

/// 多候选的符号导航模式（VS Code `SymbolNavigationService`）：`hasSymbols` 为真时 `f12` 转到下一个、
/// 到末尾循环，`escape` 退出；状态栏显示“第 N / 共 M 个符号”。
#[derive(Clone, Debug)]
pub struct SymbolNavigation {
    pub targets: Arc<[Target]>,
    pub index: usize,
}

impl SymbolNavigation {
    /// 进入模式时已经跳到了第一个候选。
    pub fn new(targets: Arc<[Target]>) -> Self {
        Self { targets, index: 0 }
    }

    /// 下一个候选（循环）。
    pub fn advance(&mut self) -> Target {
        self.index = (self.index + 1) % self.targets.len();
        self.targets[self.index].clone()
    }

    pub fn current(&self) -> &Target {
        &self.targets[self.index]
    }

    /// 状态栏消息的参数：（第几个，共几个），从 1 起算。
    pub fn position(&self) -> (usize, usize) {
        (self.index + 1, self.targets.len())
    }
}

/// 近似跳转提示的去重：同一次会话里对同一个符号（同一个目标位置）的重复近似跳转只提示一次，
/// 避免 3 秒的状态栏提示连续刷屏。
#[derive(Default, Debug)]
pub struct ApproxNotices {
    seen: HashSet<(String, u64, u64)>,
}

impl ApproxNotices {
    /// 第一次见到这个目标时返回 true（应当提示）。
    pub fn first_time(&mut self, target: &Target) -> bool {
        let p = target.selection.start();
        self.seen
            .insert((resource_key(&target.resource), p.line.0, p.column.0))
    }
}

/// 同名高亮的种类：读 / 写 / 文本匹配回退（`editor.wordHighlight*` 三组颜色）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightKind {
    Read,
    Write,
    Text,
}

/// 当前文件里同一个符号的全部出现（按位置排序）。装饰层按行二分取出，光标仍在其中某一处时不重查。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SymbolHighlights {
    pub ranges: Vec<(TextRange, HighlightKind)>,
}

impl SymbolHighlights {
    /// 由引用查询结果建立：只保留 `resource` 里的结果。索引只区分定义与引用，所以定义（声明）处
    /// 按“写”显示（与 VS Code C# 扩展一致：声明用 `wordHighlightStrong`），其余按“读”。
    pub fn from_references(resource: &Resource, targets: &[RoutedTarget]) -> Self {
        let key = resource_key(resource);
        let mut ranges: Vec<(TextRange, HighlightKind)> = targets
            .iter()
            .filter(|t| resource_key(&t.target.resource) == key)
            .map(|t| {
                let kind = match t.role {
                    TargetRole::Definition => HighlightKind::Write,
                    _ => HighlightKind::Read,
                };
                (t.target.selection.clone(), kind)
            })
            .collect();
        ranges.sort_by_key(|(r, _)| (r.start(), r.end()));
        ranges.dedup_by(|a, b| a.0 == b.0);
        Self { ranges }
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    /// 某行上的出现（UTF-8 字节列，半开）。只处理单行区间；跨行的标识符不存在。
    pub fn on_line(
        &self,
        line: u64,
    ) -> impl Iterator<Item = (std::ops::Range<u32>, HighlightKind)> + '_ {
        let start = self
            .ranges
            .partition_point(|(r, _)| r.start().line.0 < line);
        self.ranges[start..]
            .iter()
            .take_while(move |(r, _)| r.start().line.0 == line)
            .filter(|(r, _)| r.end().line == r.start().line)
            .map(|(r, k)| (r.start().column.0 as u32..r.end().column.0 as u32, *k))
    }

    /// 光标是否落在某一处出现上（含两端）：是的话光标移动不重新查询。
    pub fn contains(&self, p: Position) -> bool {
        let i = self.ranges.partition_point(|(r, _)| r.end() < p);
        self.ranges
            .get(i)
            .is_some_and(|(r, _)| r.start() <= p && p <= r.end())
    }

    /// `wordHighlight.next` / `prev`：从光标处往后 / 往前的下一处（循环）。
    pub fn step(&self, from: Position, forward: bool) -> Option<TextRange> {
        if self.ranges.is_empty() {
            return None;
        }
        let n = self.ranges.len();
        let i = if forward {
            let i = self.ranges.partition_point(|(r, _)| r.start() <= from);
            i % n
        } else {
            // 光标所在的那一处之前的一个。
            let i = self.ranges.partition_point(|(r, _)| r.end() < from);
            (i + n - 1) % n
        };
        Some(self.ranges[i].0.clone())
    }
}

/// 文本模式（非 C# 或近似结果）的 `wordHighlight.next/prev`：整词、大小写敏感的全部出现。
/// 只在执行命令时扫描（不进入绘制路径）。`lines` 依次给出每一行的文本。
pub fn text_occurrences<'a>(
    lines: impl Iterator<Item = (u64, std::borrow::Cow<'a, str>)>,
    word: &str,
    is_word: impl Fn(&str, usize) -> Option<(usize, usize)>,
) -> SymbolHighlights {
    let mut ranges = Vec::new();
    for (line, text) in lines {
        for (i, _) in text.match_indices(word) {
            if is_word(&text, i) == Some((i, i + word.len())) {
                let a = pos(line, i as u64);
                let b = pos(line, (i + word.len()) as u64);
                if let Ok(r) = TextRange::new(a, b) {
                    ranges.push((r, HighlightKind::Text));
                }
            }
        }
    }
    SymbolHighlights { ranges }
}

pub fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

/// 只读视图（元数据视图等）与非 C# 文件不提供语义导航（D2）。
pub fn is_csharp(language: &str) -> bool {
    language == "csharp"
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Version, WorkspacePath};
    use rove_lang_cs::query::ApproximationReason;

    fn res(p: &str) -> Resource {
        Resource::Workspace(WorkspacePath::new(p).unwrap())
    }
    fn range(line: u64, a: u64, b: u64) -> TextRange {
        TextRange::new(pos(line, a), pos(line, b)).unwrap()
    }
    fn target(p: &str, line: u64, col: u64) -> Target {
        Target {
            resource: res(p),
            range: range(line, col, col + 3),
            selection: range(line, col, col + 3),
        }
    }
    fn routed(t: Target, precision: Precision, role: TargetRole) -> RoutedTarget {
        RoutedTarget {
            target: t,
            precision,
            reason: None,
            role,
        }
    }
    fn result(value: Vec<RoutedTarget>, precision: Precision) -> QueryResult<Arc<[RoutedTarget]>> {
        QueryResult {
            value: value.into(),
            precision,
            reason: (precision != Precision::Exact).then_some(ApproximationReason::IndexNotReady),
            version: Some(Version(1)),
            index_generation: 1,
        }
    }

    #[test]
    fn empty_results_use_the_empty_state_and_mention_the_index_when_approximate() {
        let exact = result(vec![], Precision::Exact);
        assert!(matches!(
            plan(NavKind::Definition, &exact, GotoMode::Peek),
            Plan::NoResult {
                key: "nav.noDefinition"
            }
        ));
        let approx = result(vec![], Precision::Approximate);
        assert!(matches!(
            plan(NavKind::Definition, &approx, GotoMode::Peek),
            Plan::NoResult {
                key: "nav.noDefinition.approximate"
            }
        ));
        assert!(matches!(
            plan(NavKind::Implementation, &exact, GotoMode::Peek),
            Plan::NoResult {
                key: "nav.noImplementation"
            }
        ));
    }

    #[test]
    fn one_candidate_jumps_and_weakest_precision_wins() {
        let r = result(
            vec![routed(
                target("A.cs", 3, 4),
                Precision::Exact,
                TargetRole::Definition,
            )],
            Precision::Exact,
        );
        assert!(matches!(
            plan(NavKind::Definition, &r, GotoMode::Peek),
            Plan::Jump {
                approximate: false,
                ..
            }
        ));
        // 一条 L1 候选让整批成为近似。
        let r = result(
            vec![routed(
                target("A.cs", 3, 4),
                Precision::Approximate,
                TargetRole::Candidate,
            )],
            Precision::Exact,
        );
        assert!(matches!(
            plan(NavKind::Definition, &r, GotoMode::Peek),
            Plan::Jump {
                approximate: true,
                ..
            }
        ));
    }

    #[test]
    fn several_candidates_follow_the_setting_and_duplicates_merge() {
        let t = vec![
            routed(
                target("A.cs", 3, 4),
                Precision::Exact,
                TargetRole::Definition,
            ),
            routed(
                target("A.cs", 3, 4),
                Precision::Exact,
                TargetRole::Definition,
            ),
            routed(
                target("B.cs", 1, 0),
                Precision::Exact,
                TargetRole::Definition,
            ),
        ];
        match plan(
            NavKind::Definition,
            &result(t, Precision::Exact),
            GotoMode::Goto,
        ) {
            Plan::Multiple {
                mode,
                targets,
                approximate,
            } => {
                assert_eq!(mode, GotoMode::Goto);
                assert_eq!(targets.len(), 2);
                assert!(!approximate);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn metadata_only_results_do_not_open_an_empty_tab() {
        let meta = Target {
            resource: Resource::Virtual(
                format!("{METADATA_SCHEME}UnityEngine|T:UnityEngine.Transform").into(),
            ),
            range: range(0, 0, 1),
            selection: range(0, 0, 1),
        };
        let r = result(
            vec![routed(meta.clone(), Precision::Exact, TargetRole::Metadata)],
            Precision::Exact,
        );
        assert!(matches!(
            plan(NavKind::Definition, &r, GotoMode::Peek),
            Plan::Metadata { .. }
        ));
        // 有源码目标时源码优先。
        let r = result(
            vec![
                routed(meta, Precision::Exact, TargetRole::Metadata),
                routed(
                    target("A.cs", 1, 1),
                    Precision::Exact,
                    TargetRole::Definition,
                ),
            ],
            Precision::Exact,
        );
        assert!(matches!(
            plan(NavKind::Definition, &r, GotoMode::Peek),
            Plan::Jump { .. }
        ));
    }

    #[test]
    fn peek_title_is_unmarked_for_exact_results() {
        assert_eq!(peek_annotation(false, 3), None);
        let (key, args) = peek_annotation(true, 3).unwrap();
        assert_eq!(key, "nav.peek.approximateCandidates");
        assert_eq!(args, vec![("count", "3".to_string())]);
    }

    #[test]
    fn symbol_navigation_cycles_and_reports_its_position() {
        let mut nav = SymbolNavigation::new(
            vec![
                target("A.cs", 1, 0),
                target("B.cs", 2, 0),
                target("C.cs", 3, 0),
            ]
            .into(),
        );
        assert_eq!(nav.position(), (1, 3));
        assert_eq!(nav.advance().selection.start().line.0, 2);
        assert_eq!(nav.advance().selection.start().line.0, 3);
        assert_eq!(nav.position(), (3, 3));
        assert_eq!(nav.advance().selection.start().line.0, 1);
        assert_eq!(nav.position(), (1, 3));
    }

    #[test]
    fn approximate_notice_is_shown_once_per_symbol() {
        let mut n = ApproxNotices::default();
        assert!(n.first_time(&target("A.cs", 1, 2)));
        assert!(!n.first_time(&target("A.cs", 1, 2)));
        assert!(n.first_time(&target("A.cs", 5, 2)));
    }

    #[test]
    fn highlights_keep_only_this_file_and_mark_definitions_as_writes() {
        let here = res("A.cs");
        let h = SymbolHighlights::from_references(
            &here,
            &[
                routed(
                    target("A.cs", 9, 1),
                    Precision::Exact,
                    TargetRole::Reference,
                ),
                routed(
                    target("B.cs", 1, 1),
                    Precision::Exact,
                    TargetRole::Reference,
                ),
                routed(
                    target("A.cs", 2, 4),
                    Precision::Exact,
                    TargetRole::Definition,
                ),
                routed(
                    target("A.cs", 9, 10),
                    Precision::Exact,
                    TargetRole::Reference,
                ),
            ],
        );
        assert_eq!(h.len(), 3);
        assert_eq!(h.ranges[0].1, HighlightKind::Write);
        let line9: Vec<_> = h.on_line(9).collect();
        assert_eq!(
            line9,
            vec![(1..4, HighlightKind::Read), (10..13, HighlightKind::Read)]
        );
        assert_eq!(h.on_line(3).count(), 0);
        assert!(h.contains(pos(9, 4)));
        assert!(h.contains(pos(2, 4)));
        assert!(!h.contains(pos(9, 5)));
        // 前后循环。
        assert_eq!(h.step(pos(2, 5), true).unwrap().start(), pos(9, 1));
        assert_eq!(h.step(pos(9, 10), true).unwrap().start(), pos(2, 4));
        assert_eq!(h.step(pos(9, 1), false).unwrap().start(), pos(2, 4));
        assert_eq!(h.step(pos(2, 4), false).unwrap().start(), pos(9, 10));
    }

    #[test]
    fn text_occurrences_are_whole_words() {
        let lines = ["int Init; Initialize(); Init();", "Init"];
        let h = text_occurrences(
            lines
                .iter()
                .enumerate()
                .map(|(i, l)| (i as u64, std::borrow::Cow::Borrowed(*l))),
            "Init",
            |t, i| {
                let b = t.as_bytes();
                let mut a = i;
                while a > 0 && (b[a - 1] as char).is_alphanumeric() {
                    a -= 1;
                }
                let mut e = i;
                while e < b.len() && (b[e] as char).is_alphanumeric() {
                    e += 1;
                }
                Some((a, e))
            },
        );
        assert_eq!(h.len(), 3);
        assert!(h.ranges.iter().all(|(_, k)| *k == HighlightKind::Text));
    }
}
