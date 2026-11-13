//! 跨子模块的组合测试（#22）。界面层（真实 GPUI 窗口）的入口测试在
//! `skeleton/workbench/large_file_tests.rs`。

use super::*;
use crate::code_view::decorations::LineDecorations;
use rove_buffer::RopeText;
use rove_i18n::StringTable;

fn strings() -> StringTable {
    let mut table = StringTable::default();
    registry::register_strings(&mut table).unwrap();
    crate::code_view::commands::register_strings(&mut table).unwrap();
    table
}

/// 超大文件的整条链路：提示条 → 状态栏 → 索引进度 → 索引建完后进度条目消失。
#[test]
fn a_huge_file_reports_a_bar_a_status_mark_and_index_progress() {
    let strings = strings();
    let bytes = 1_288_490_189u64;

    // 1. 提示条：只有超大文件有，文案按 UX_SPEC §3.2，体积已格式化。
    let bar = notice::notice_for(SizeClass::Huge, bytes).expect("超大文件有提示条");
    assert_eq!(bar.size, "1.2 GB");
    assert!(notice::notice_for(SizeClass::Large, bytes).is_none());

    // 2. 状态栏：超大文件 · 只读 + 被关闭的功能。
    let disabled = vec![Feature::SyntaxHighlighting, Feature::Folding];
    let entry = status_entry(&strings, SizeClass::Huge, &disabled).expect("有条目");
    assert_eq!(entry.text, "超大文件 · 只读");
    assert!(entry.tooltip.as_ref().unwrap().contains("语法高亮"));
    assert!(status_entry(&strings, SizeClass::Normal, &disabled).is_none());

    // 3. 索引进度：建完前显示百分比、建完后条目消失（永远不弹通知，级别由 #16 决定）。
    let indexing = IndexProgress {
        known: 1_000_000,
        indexed_bytes: bytes / 3,
        total_bytes: bytes,
        complete: false,
    };
    let entry = index_status_entry(&strings, indexing).expect("索引中有条目");
    assert_eq!(entry.text, "行索引 33%");
    let done = IndexProgress {
        complete: true,
        indexed_bytes: bytes,
        ..indexing
    };
    assert!(index_status_entry(&strings, done).is_none());

    // 4. 超大文件不提供替换（#48 必须遵守）。
    assert!(!allows_replace(SizeClass::Huge));
    assert!(allows_replace(SizeClass::Large));
    assert!(allows_replace(SizeClass::Normal));
}

fn decorate(marks: &TruncationMarks, line: u64, text: &str) -> LineDecorations {
    let mut out = LineDecorations::default();
    marks.decorate(line, text, &mut out);
    out
}

/// 截断标记：只给超过上限的行，列与 #19 的截断点一致，字符数是整行的字符数。
#[test]
fn truncation_marks_only_cover_lines_past_the_render_limit() {
    let strings = strings();
    let long = "a".repeat(50);
    let text: Arc<dyn TextSource> = Arc::new(RopeText::from_text(&format!("short\n{long}\n")));
    let marks = TruncationMarks::new(text, Some(10), None, &strings);

    // 短行没有标记。
    assert!(decorate(&marks, 0, "short").overlays.is_empty());

    // 长行有一条灰色覆盖物，列正好是第 10 个字符之后。
    let out = decorate(&marks, 1, &long);
    assert_eq!(out.overlays.len(), 1);
    assert_eq!(out.overlays[0].column, 10);
    assert_eq!(&*out.overlays[0].text, "…（已截断，共 50 个字符）");

    // 不限上限时没有标记。
    let text: Arc<dyn TextSource> = Arc::new(RopeText::from_text(&long));
    let unlimited = TruncationMarks::new(text, None, None, &strings);
    assert!(decorate(&unlimited, 0, &long).overlays.is_empty());
}

/// 截断按**字素簇**：不会把一个 emoji 劈开，列落在簇的边界上。
#[test]
fn truncation_never_splits_a_grapheme_cluster() {
    let strings = strings();
    // 每个家庭 emoji 是一个字素簇、7 个 Unicode 标量值。
    let family = "👨‍👩‍👧‍👦";
    let line = family.repeat(4);
    let text: Arc<dyn TextSource> = Arc::new(RopeText::from_text(&line));
    // 上限 10 个字符，每个簇 7 个字符：和 #19 的 `DisplayLine::build` 一样「先判断再吃一个簇」，
    // 所以渲染两个完整的簇（14 个字符），截断点落在第三个簇的簇首，不会把 emoji 劈开。
    let marks = TruncationMarks::new(text, Some(10), None, &strings);
    let out = decorate(&marks, 0, &line);
    assert_eq!(out.overlays.len(), 1);
    let column = out.overlays[0].column as usize;
    assert_eq!(column, family.len() * 2, "落在第三个簇的簇首");
    assert!(line.is_char_boundary(column));
    assert!(out.overlays[0]
        .text
        .contains(&(family.chars().count() * 4).to_string()));
}

/// 截断点与 #19 实际渲染出来的长度**逐例一致**：标记既不会压在最后一个字形上，
/// 也不会和它之间留出空档。上限、字素簇与 Tab 的组合都对上。
#[test]
fn the_mark_column_matches_what_the_code_view_actually_renders() {
    use crate::code_view::columns::DisplayLine;
    let strings = strings();
    let cases = [
        "abcdefghijklmnopqrstuvwxyz",
        "你好世界，这是一行很长的中文文本，用来检查截断点",
        "👨‍👩‍👧‍👦🙂👍🏽ascii混排👨‍👩‍👧‍👦",
        "\ta\tb\tccccccccccccccccccccc",
    ];
    for line in cases {
        for limit in [1usize, 2, 3, 5, 8, 13, 21] {
            let text: Arc<dyn TextSource> = Arc::new(RopeText::from_text(line));
            let marks = TruncationMarks::new(text, Some(limit), None, &strings);
            let display = DisplayLine::build(line, 4, Some(limit));
            let overlays = decorate(&marks, 0, line).overlays;
            assert_eq!(
                overlays.len(),
                usize::from(display.truncated),
                "{line:?} limit={limit}：是否截断要和 #19 一致"
            );
            if let Some(overlay) = overlays.first() {
                assert_eq!(
                    overlay.column,
                    display.rendered_source_len(),
                    "{line:?} limit={limit}：截断点要和 #19 渲染出来的长度一致"
                );
            }
        }
    }
}

/// 字符数来自整行（可能远超渲染上限），不是渲染出来的那一段。
#[test]
fn the_character_count_is_the_whole_line_not_the_rendered_part() {
    let strings = strings();
    let line = "x".repeat(20_000);
    let text: Arc<dyn TextSource> = Arc::new(RopeText::from_text(&line));
    let marks = TruncationMarks::new(text, Some(100), None, &strings);
    let mark = marks.mark_text(0, &line).unwrap();
    assert!(mark.contains("20000"), "{mark}");
    // 再问一次走缓存，结果一致。
    assert_eq!(marks.mark_text(0, &line).unwrap(), mark);
}

/// 行索引推进时滚动条的总行数估计平滑收敛，并且永远不小于已知行数
/// （滚动范围只扩不缩，视口顶端的行不受影响）。
#[test]
fn the_scroll_estimate_converges_without_a_jump_at_completion() {
    let total_bytes = 90_000_000u64;
    let per_line = 60u64;
    let actual = total_bytes / per_line;
    let mut previous = 0u64;
    for percent in [1u64, 10, 40, 75, 99] {
        let indexed = total_bytes * percent / 100;
        let progress = IndexProgress {
            known: indexed / per_line,
            indexed_bytes: indexed,
            total_bytes,
            complete: false,
        };
        let estimate = progress.estimated_lines();
        assert!(estimate >= progress.known);
        assert!(progress.estimate_error(actual) < 0.01, "{percent}%");
        assert!(estimate as f64 >= previous as f64 * 0.98);
        previous = estimate;
    }
    let done = IndexProgress {
        known: actual,
        indexed_bytes: total_bytes,
        total_bytes,
        complete: true,
    };
    // 完成时就是真实行数：没有“一次性切换”。
    assert_eq!(done.estimated_lines(), actual);
    assert!((done.estimated_lines() as f64 - previous as f64).abs() / (actual as f64) < 0.01);
}
