//! 查找模型（#21）：查找文本、选项、范围、匹配列表、当前项与搜索状态机。
//!
//! 不依赖 GPUI、不直接读文本；界面层（[`super::host`]）用 [`FindModel::start_search`] 取一个
//! [`SearchJob`] 放到后台执行，再把结果交回 [`FindModel::finish`]。每次改动查找条件都会取消上一次
//! 搜索并让它的结果作废（代次不符的结果被丢弃）。#22 在超大文件上用同一状态机，只需换引擎的
//! 流式入口；#48 的替换复用匹配列表、当前项与范围。

use super::engine::{
    self, FindOptions, Match, Progress, Query, QueryError, SearchOutput, Searcher, MATCHES_LIMIT,
};
use rove_base::{Cancellation, Position, Result, TextRange};
use rove_buffer::TextSource;
use std::sync::Arc;

/// 搜索状态。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SearchState {
    /// 没有进行中的搜索（含没有查找文本）。
    Idle,
    /// 后台正在搜索；`progress` 在需要显示进度（文本很大）时为 `Some`。
    Searching { found: usize, progress: Option<f32> },
}

/// 匹配计数处显示的内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountLabel {
    /// “无结果”。
    NoResults,
    /// “第 N 项，共 M 项”；`index` 为空时只显示总数；`over_limit` 时总数是“至少”。
    Matches {
        index: Option<usize>,
        total: usize,
        over_limit: bool,
    },
    /// “已找到 N 处 · 已搜索 P%”（流式查找）。
    Searching { found: usize, percent: Option<u8> },
}

/// 文本不到这个大小时不显示进度（一帧内就能完成）。
pub const PROGRESS_MIN_BYTES: u64 = 8 << 20;

/// 交给后台线程的一次搜索。
pub struct SearchJob {
    pub generation: u64,
    pub searcher: Arc<Searcher>,
    pub scope: Option<TextRange>,
    pub cancel: Cancellation,
    pub limit: usize,
}

impl SearchJob {
    /// 在后台线程执行；按块调用 `progress`（第二个参数是这一块新找到的匹配，供 #22
    /// 的流式查找逐步显示），被取消时返回 `Err(Cancelled)`。
    pub fn run(
        &self,
        text: &dyn TextSource,
        progress: &mut dyn FnMut(Progress, &[Match]),
    ) -> Result<SearchOutput> {
        engine::search_all(
            text,
            &self.searcher,
            self.scope.as_ref(),
            self.limit,
            &self.cancel,
            progress,
        )
    }
}

/// 查找模型。
pub struct FindModel {
    search: String,
    options: FindOptions,
    in_selection: bool,
    scope: Option<TextRange>,
    word_separators: Arc<str>,
    matches: Arc<Vec<Match>>,
    limit_exceeded: bool,
    current: Option<usize>,
    state: SearchState,
    error: Option<QueryError>,
    generation: u64,
    cancel: Cancellation,
    limit: usize,
    searcher: Option<Arc<Searcher>>,
    /// 已经为哪一代搜索发布过流式部分结果（#22）。
    partial: Option<u64>,
}

impl Default for FindModel {
    fn default() -> Self {
        Self {
            search: String::new(),
            options: FindOptions::default(),
            in_selection: false,
            scope: None,
            word_separators: "".into(),
            matches: Arc::new(Vec::new()),
            limit_exceeded: false,
            current: None,
            state: SearchState::Idle,
            error: None,
            generation: 0,
            cancel: Cancellation::default(),
            limit: MATCHES_LIMIT,
            searcher: None,
            partial: None,
        }
    }
}

impl FindModel {
    pub fn new(word_separators: Arc<str>) -> Self {
        Self {
            word_separators,
            ..Self::default()
        }
    }

    pub fn search(&self) -> &str {
        &self.search
    }

    pub fn options(&self) -> FindOptions {
        self.options
    }

    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_string();
    }

    pub fn set_options(&mut self, options: FindOptions) {
        self.options = options;
    }

    pub fn set_word_separators(&mut self, separators: Arc<str>) {
        self.word_separators = separators;
    }

    /// “在选区内查找”开关是否打开（范围由 [`FindModel::set_scope`] 给定）。
    pub fn in_selection(&self) -> bool {
        self.in_selection
    }

    /// 打开 / 关闭“在选区内查找”。打开时必须同时给出范围。
    pub fn set_in_selection(&mut self, scope: Option<TextRange>) {
        self.in_selection = scope.is_some();
        self.scope = scope;
    }

    /// 生效的搜索范围（仅在“在选区内查找”打开时有值）。
    pub fn scope(&self) -> Option<&TextRange> {
        self.scope.as_ref()
    }

    pub fn matches(&self) -> &Arc<Vec<Match>> {
        &self.matches
    }

    pub fn limit_exceeded(&self) -> bool {
        self.limit_exceeded
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn state(&self) -> SearchState {
        self.state
    }

    /// 查询校验错误（非法正则）：界面在输入框上显示。
    pub fn error(&self) -> Option<&QueryError> {
        self.error.as_ref()
    }

    /// 当前编译好的查询（导航用）；没有查找文本或正则非法时为空。
    pub fn searcher(&self) -> Option<&Arc<Searcher>> {
        self.searcher.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 清空结果（关闭查找条时）。同时取消进行中的搜索。
    pub fn clear(&mut self) {
        self.cancel.cancel();
        self.generation += 1;
        self.matches = Arc::new(Vec::new());
        self.limit_exceeded = false;
        self.current = None;
        self.state = SearchState::Idle;
        self.error = None;
        self.searcher = None;
        self.partial = None;
    }

    /// 按当前条件开始一次新搜索：取消上一次，编译查询。空文本或非法正则时清空匹配并返回 `None`。
    pub fn start_search(&mut self) -> Option<SearchJob> {
        self.cancel.cancel();
        self.cancel = Cancellation::default();
        self.generation += 1;
        let query = Query {
            text: self.search.clone(),
            options: self.options,
            word_separators: self.word_separators.clone(),
        };
        match Searcher::compile(&query) {
            Ok(Some(searcher)) => {
                self.error = None;
                let searcher = Arc::new(searcher);
                self.searcher = Some(searcher.clone());
                self.state = SearchState::Searching {
                    found: self.matches.len(),
                    progress: None,
                };
                Some(SearchJob {
                    generation: self.generation,
                    searcher,
                    scope: self.scope.clone(),
                    cancel: self.cancel.clone(),
                    limit: self.limit,
                })
            }
            Ok(None) => {
                self.empty_result(None);
                None
            }
            Err(e) => {
                self.empty_result(Some(e));
                None
            }
        }
    }

    fn empty_result(&mut self, error: Option<QueryError>) {
        self.matches = Arc::new(Vec::new());
        self.limit_exceeded = false;
        self.current = None;
        self.state = SearchState::Idle;
        self.error = error;
        self.searcher = None;
        self.partial = None;
    }

    /// 流式查找（#22）：后台每扫完一块就把**新找到的**匹配交回界面，结果逐步出现，
    /// 不必等整份文件扫完就能在已找到的结果之间跳转。代次不符或没有新匹配时返回 `false`。
    ///
    /// 一次搜索内第一次发布时丢掉上一次搜索遗留的匹配（[`FindModel::start_search`] 故意保留
    /// 它们以免结果闪断），之后只追加。
    pub fn publish_partial(&mut self, generation: u64, batch: &[Match]) -> bool {
        if generation != self.generation || batch.is_empty() {
            return false;
        }
        let mut matches = if self.partial == Some(generation) {
            self.matches.as_ref().clone()
        } else {
            self.partial = Some(generation);
            self.limit_exceeded = false;
            self.current = None;
            Vec::with_capacity(batch.len())
        };
        matches.extend_from_slice(batch);
        self.matches = Arc::new(matches);
        true
    }

    /// 后台报告进度；代次不符（已被新搜索取代）返回 `false`。
    pub fn progress(&mut self, generation: u64, p: Progress) -> bool {
        if generation != self.generation {
            return false;
        }
        self.state = SearchState::Searching {
            found: p.found,
            progress: (p.total_bytes >= PROGRESS_MIN_BYTES && p.fraction() < 1.0)
                .then(|| p.fraction()),
        };
        true
    }

    /// 后台结果回到界面；代次不符返回 `false`（结果作废）。取消不是错误，也按作废处理。
    pub fn finish(&mut self, generation: u64, result: Result<SearchOutput>) -> bool {
        if generation != self.generation {
            return false;
        }
        match result {
            Ok(out) => {
                self.matches = Arc::new(out.matches);
                self.limit_exceeded = out.limit_exceeded;
                self.current = None;
                self.state = SearchState::Idle;
                self.partial = None;
                true
            }
            Err(_) => {
                self.state = SearchState::Idle;
                // 取消：保留已经流式发布的结果（#22：取消后已找到的结果仍可浏览）。
                if self.partial.take() != Some(generation) {
                    self.matches = Arc::new(Vec::new());
                    self.limit_exceeded = false;
                    self.current = None;
                }
                true
            }
        }
    }

    /// 让当前项跟随选区：选区恰好等于某个匹配时该匹配是当前项，否则没有当前项。
    pub fn set_current_from_selection(&mut self, selection: Option<(Position, Position)>) {
        self.current = selection.and_then(|(start, end)| {
            let i = self.matches.partition_point(|m| m.start < start);
            self.matches
                .get(i)
                .filter(|m| m.start == start && m.end == end)
                .map(|_| i)
        });
    }

    /// 匹配计数处该显示什么。
    pub fn count_label(&self) -> CountLabel {
        if let SearchState::Searching { found, progress } = self.state {
            if progress.is_some() {
                return CountLabel::Searching {
                    found,
                    percent: progress.map(|f| (f * 100.0) as u8),
                };
            }
        }
        if self.matches.is_empty() {
            return CountLabel::NoResults;
        }
        CountLabel::Matches {
            index: self.current.map(|i| i + 1),
            total: self.matches.len(),
            over_limit: self.limit_exceeded,
        }
    }

    /// 当前搜索范围里匹配的边界：用来判断“上一个 / 下一个”按钮是否可用（`loop` 关闭时）。
    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }
}

/// `editor.find.seedSearchStringFromSelection`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedMode {
    Never,
    Always,
    Selection,
}

impl SeedMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "never" => Self::Never,
            "selection" => Self::Selection,
            _ => Self::Always,
        }
    }
}

/// `editor.find.autoFindInSelection`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoFindInSelection {
    Never,
    Always,
    Multiline,
}

impl AutoFindInSelection {
    pub fn parse(value: &str) -> Self {
        match value {
            "always" => Self::Always,
            "multiline" => Self::Multiline,
            _ => Self::Never,
        }
    }
}

/// 打开查找条时编辑器的状态。
pub struct OpenContext<'a> {
    pub selection: TextRange,
    /// 选区文本（单行选区才需要）。
    pub selected_text: Option<&'a str>,
    /// 插入符所在的单词。
    pub word: Option<&'a str>,
}

/// 打开查找条的决定：用什么种子、要不要打开“在选区内查找”。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenPlan {
    pub seed: Option<String>,
    pub scope: Option<TextRange>,
}

/// 选区种子的最大长度（VS Code 同值）。
pub const MAX_SEED_CHARS: usize = 524_288;

/// VS Code `getSelectionSearchString` 与 `autoFindInSelection` 的组合：
/// - 空选区：`always` 取插入符所在单词，`selection` 不取；
/// - 非空单行选区：取选区文本；多行选区不取；
/// - 非空选区时 `autoFindInSelection` 为 `always`，或为 `multiline` 且选区跨行，则打开范围。
pub fn plan_open(seed: SeedMode, auto: AutoFindInSelection, ctx: &OpenContext) -> OpenPlan {
    let empty = ctx.selection.start() == ctx.selection.end();
    let multiline = ctx.selection.start().line != ctx.selection.end().line;
    let text = if seed == SeedMode::Never {
        None
    } else if empty {
        (seed == SeedMode::Always)
            .then_some(ctx.word)
            .flatten()
            .map(str::to_string)
    } else if multiline {
        None
    } else {
        ctx.selected_text
            .filter(|t| t.chars().count() <= MAX_SEED_CHARS)
            .map(str::to_string)
    }
    .filter(|t| !t.is_empty());
    let scope = (!empty
        && match auto {
            AutoFindInSelection::Never => false,
            AutoFindInSelection::Always => true,
            AutoFindInSelection::Multiline => multiline,
        })
    .then(|| ctx.selection.clone());
    OpenPlan { seed: text, scope }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::engine::tests::pos;
    use rove_base::Span;
    use rove_buffer::RopeText;

    fn model(text: &str) -> FindModel {
        let mut m = FindModel::new("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into());
        m.set_search(text);
        m
    }

    fn run(m: &mut FindModel, doc: &str) {
        let rope = RopeText::from_text(doc);
        if let Some(job) = m.start_search() {
            let out = job.run(&rope, &mut |_, _| {});
            assert!(m.finish(job.generation, out));
        }
    }

    #[test]
    fn count_labels_cover_empty_single_many_and_over_limit() {
        let mut m = model("x");
        run(&mut m, "abc");
        assert_eq!(m.count_label(), CountLabel::NoResults);
        run(&mut m, "x");
        assert_eq!(
            m.count_label(),
            CountLabel::Matches {
                index: None,
                total: 1,
                over_limit: false
            }
        );
        m.set_current_from_selection(Some((pos(0, 0), pos(0, 1))));
        assert_eq!(
            m.count_label(),
            CountLabel::Matches {
                index: Some(1),
                total: 1,
                over_limit: false
            }
        );
        run(&mut m, "x x x");
        m.set_current_from_selection(Some((pos(0, 2), pos(0, 3))));
        assert_eq!(
            m.count_label(),
            CountLabel::Matches {
                index: Some(2),
                total: 3,
                over_limit: false
            }
        );
        // 选区不是匹配：没有当前项。
        m.set_current_from_selection(Some((pos(0, 0), pos(0, 2))));
        assert_eq!(m.current(), None);
        // 超过上限。
        let mut big = model("x");
        let doc = "x ".repeat(MATCHES_LIMIT + 5);
        run(&mut big, &doc);
        assert_eq!(
            big.count_label(),
            CountLabel::Matches {
                index: None,
                total: MATCHES_LIMIT,
                over_limit: true
            }
        );
    }

    #[test]
    fn invalid_regex_sets_error_and_clears_matches() {
        let mut m = model("a");
        run(&mut m, "a a");
        assert!(m.has_matches());
        m.set_options(FindOptions {
            regex: true,
            ..Default::default()
        });
        m.set_search("(");
        run(&mut m, "a a");
        assert!(matches!(m.error(), Some(QueryError::InvalidRegex(_))));
        assert!(!m.has_matches());
        m.set_search("a");
        run(&mut m, "a a");
        assert!(m.error().is_none());
        assert_eq!(m.matches().len(), 2);
    }

    #[test]
    fn stale_results_are_discarded_and_previous_search_is_cancelled() {
        let mut m = model("a");
        let rope = RopeText::from_text("a a a");
        let first = m.start_search().unwrap();
        m.set_search("b");
        let second = m.start_search().unwrap();
        // 上一次已被取消。
        assert!(first.cancel.check().is_err());
        let out = first.run(&rope, &mut |_, _| {});
        assert!(out.is_err());
        assert!(!m.finish(first.generation, out));
        assert!(!m.progress(
            first.generation,
            Progress {
                found: 1,
                scanned_bytes: 1,
                total_bytes: 1
            }
        ));
        let out = second.run(&rope, &mut |_, _| {});
        assert!(m.finish(second.generation, out));
        assert_eq!(m.count_label(), CountLabel::NoResults);
    }

    #[test]
    fn progress_is_only_shown_for_large_texts() {
        let mut m = model("a");
        let job = m.start_search().unwrap();
        m.progress(
            job.generation,
            Progress {
                found: 3,
                scanned_bytes: 10,
                total_bytes: 100,
            },
        );
        assert_eq!(
            m.state(),
            SearchState::Searching {
                found: 3,
                progress: None
            }
        );
        m.progress(
            job.generation,
            Progress {
                found: 5,
                scanned_bytes: 37 << 20,
                total_bytes: 100 << 20,
            },
        );
        assert_eq!(
            m.count_label(),
            CountLabel::Searching {
                found: 5,
                percent: Some(37)
            }
        );
    }

    #[test]
    fn clear_resets_everything_and_cancels() {
        let mut m = model("a");
        let job = m.start_search().unwrap();
        m.clear();
        assert!(job.cancel.check().is_err());
        assert_eq!(m.state(), SearchState::Idle);
        assert!(!m.finish(job.generation, Ok(SearchOutput::default())));
    }

    fn sel(a: (u64, u64), b: (u64, u64)) -> TextRange {
        Span::new(pos(a.0, a.1), pos(b.0, b.1)).unwrap()
    }

    #[test]
    fn open_plan_seeds_like_vs_code() {
        let caret = sel((0, 3), (0, 3));
        let ctx = |selection: TextRange, text, word| OpenContext {
            selection,
            selected_text: text,
            word,
        };
        let p = plan_open(
            SeedMode::Always,
            AutoFindInSelection::Never,
            &ctx(caret.clone(), None, Some("word")),
        );
        assert_eq!(
            p,
            OpenPlan {
                seed: Some("word".into()),
                scope: None
            }
        );
        let p = plan_open(
            SeedMode::Selection,
            AutoFindInSelection::Never,
            &ctx(caret.clone(), None, Some("word")),
        );
        assert_eq!(p.seed, None);
        let p = plan_open(
            SeedMode::Never,
            AutoFindInSelection::Never,
            &ctx(sel((0, 0), (0, 3)), Some("foo"), None),
        );
        assert_eq!(p.seed, None);
        let single = sel((0, 0), (0, 3));
        let p = plan_open(
            SeedMode::Selection,
            AutoFindInSelection::Never,
            &ctx(single.clone(), Some("foo"), None),
        );
        assert_eq!(
            p,
            OpenPlan {
                seed: Some("foo".into()),
                scope: None
            }
        );
        // 多行选区：不取种子；`multiline` 才打开范围；`always` 对单行选区也打开范围。
        let multi = sel((0, 0), (2, 1));
        let p = plan_open(
            SeedMode::Always,
            AutoFindInSelection::Never,
            &ctx(multi.clone(), Some("a\nb"), None),
        );
        assert_eq!(
            p,
            OpenPlan {
                seed: None,
                scope: None
            }
        );
        let p = plan_open(
            SeedMode::Always,
            AutoFindInSelection::Multiline,
            &ctx(multi.clone(), Some("a\nb"), None),
        );
        assert_eq!(p.scope, Some(multi.clone()));
        let p = plan_open(
            SeedMode::Always,
            AutoFindInSelection::Multiline,
            &ctx(single.clone(), Some("foo"), None),
        );
        assert_eq!(
            p,
            OpenPlan {
                seed: Some("foo".into()),
                scope: None
            }
        );
        let p = plan_open(
            SeedMode::Always,
            AutoFindInSelection::Always,
            &ctx(single.clone(), Some("foo"), None),
        );
        assert_eq!(p.scope, Some(single));
        // 空选区不会打开范围。
        let p = plan_open(
            SeedMode::Always,
            AutoFindInSelection::Always,
            &ctx(caret, None, None),
        );
        assert_eq!(
            p,
            OpenPlan {
                seed: None,
                scope: None
            }
        );
    }

    #[test]
    fn setting_values_parse() {
        assert_eq!(SeedMode::parse("never"), SeedMode::Never);
        assert_eq!(SeedMode::parse("selection"), SeedMode::Selection);
        assert_eq!(SeedMode::parse("bogus"), SeedMode::Always);
        assert_eq!(
            AutoFindInSelection::parse("multiline"),
            AutoFindInSelection::Multiline
        );
        assert_eq!(AutoFindInSelection::parse("x"), AutoFindInSelection::Never);
    }
}
