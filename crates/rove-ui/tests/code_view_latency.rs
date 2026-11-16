//! 按键到上屏的**口径**集成测试（#215）。
//!
//! `perf/budgets.toml` 的 `keystroke_to_paint` 判的是「按键进入 → 包含它的那一帧绘制完成」，
//! 其中有一条不靠计时器也能判的硬要求：**按键必须被紧接着的那一帧吸收，不能多等一帧**。
//! 绝对耗时由 `cargo bench -p xtask --bench typing` 在真实窗口里测（受显示器刷新率影响，
//! 不适合放进单元测试）；这里只钉住「哪一帧画了哪个版本」这件与机器无关的事，
//! 顺带守住基准依赖的两个字段：`FrameStats::text_version` 与 `FrameStats::end`。
//!
//! 运行：`cargo test -p rove-ui --test code_view_latency`

use gpui::{Context, Entity, EntityInputHandler, Focusable, Keystroke, TestAppContext, Window};
use rove_buffer::edit::{EditBuffer, EditOptions, EditSession};
use rove_buffer::{Eol, LineEndings, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, selection::pos, CodeView, CodeViewServices, EditHost, FrameStats, Selection,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ------------------------------------------------------------------ 夹具

fn services() -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    rove_buffer::edit::settings::register(&mut settings).unwrap();
    CodeViewServices {
        settings: Arc::new(DefaultSettings(settings)),
        theme: Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        theme_kind: ThemeKind::Dark,
    }
}

struct Fixture<'a> {
    view: Entity<CodeView>,
    cx: &'a mut gpui::VisualTestContext,
}

fn open<'a>(cx: &'a mut TestAppContext, text: &str) -> Fixture<'a> {
    let session = Rc::new(RefCell::new(EditSession::new(
        EditBuffer::new(text, LineEndings::Uniform(Eol::Lf)),
        EditOptions::default(),
    )));
    let snapshot: Arc<dyn TextSource> = session.borrow().snapshot();
    let s = services();
    let host: Rc<dyn EditHost> = session.clone();
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(snapshot), false, cx);
        v.set_edit_host(Some(host), cx);
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
    });
    let mut f = Fixture { view, cx };
    f.settle();
    f
}

impl Fixture<'_> {
    /// 让空闲准备切片全部跑完并重绘。
    fn settle(&mut self) {
        for _ in 0..4 {
            self.cx.update(|window, _| window.refresh());
            self.cx.run_until_parked();
        }
    }

    fn version(&mut self) -> u64 {
        self.view
            .update(self.cx, |v, _| v.text().map_or(0, |t| t.version().0))
    }

    /// 清空录制缓冲，从下一帧开始记。
    fn start_recording(&mut self) {
        self.view.update(self.cx, |v, _| {
            v.record_frames(true);
            v.take_frame_samples();
        });
    }

    fn recorded(&mut self) -> Vec<FrameStats> {
        self.view.update(self.cx, |v, _| v.take_frame_samples())
    }

    fn caret(&mut self, line: u64, col: u64) {
        self.view.update(self.cx, |v, cx| {
            v.set_selection(Selection::caret(pos(line, col)), None, cx)
        });
    }

    /// 走真实按键路径：键位服务 → 按键事件 → 输入法接口 → 事务 → 视图增量更新。
    fn press(&mut self, key: &str) {
        let keystroke = Keystroke::parse(key).expect("按键");
        self.cx
            .update(|window, cx| window.dispatch_keystroke(keystroke, cx));
    }

    /// 输入法提交（和按键走同一个输入法接口）。
    fn commit(&mut self, text: &str) {
        let view = self.view.clone();
        let text = text.to_owned();
        self.cx.update(|window, cx| {
            view.update(cx, |v, cx| v.replace_text_in_range(None, &text, window, cx))
        });
    }
}

fn sample_text() -> String {
    (0..200)
        .map(|i| format!("    private static int Method{i}(int value) {{ return {i}; }}\n"))
        .collect()
}

// ------------------------------------------------------------------ 用例

/// 核心判据：按键之后**第一帧**就画出了含新字符的版本，不多等一帧。
#[gpui::test]
fn a_printable_key_is_painted_by_the_very_next_frame(cx: &mut TestAppContext) {
    let mut f = open(cx, &sample_text());
    f.caret(100, 8);
    f.settle();
    let before = f.version();
    f.start_recording();
    f.press("x");
    f.settle();
    let after = f.version();
    assert!(after > before, "按键没有改到文本：{before} → {after}");

    let frames = f.recorded();
    let first = frames.first().expect("按键之后至少要画一帧");
    assert_eq!(
        first.text_version, after,
        "第一帧画的是旧版本（多等了一帧）：{frames:?}"
    );
    assert_eq!(
        first.placeholders + first.missing,
        0,
        "新字符所在的那一帧里还有占位行或空白行：{first:?}"
    );
}

/// 连续键入：每一次按键都只用一帧就上屏（而不是只有第一次这样）。
#[gpui::test]
fn every_key_in_a_run_is_painted_by_the_very_next_frame(cx: &mut TestAppContext) {
    let mut f = open(cx, &sample_text());
    f.caret(100, 8);
    f.settle();
    for (i, key) in "abcdefghij".chars().enumerate() {
        f.start_recording();
        f.press(&key.to_string());
        f.settle();
        let after = f.version();
        let frames = f.recorded();
        let first = frames
            .first()
            .unwrap_or_else(|| panic!("第 {i} 次按键之后没有画任何一帧"));
        assert_eq!(
            first.text_version, after,
            "第 {i} 次按键多等了一帧：{frames:?}"
        );
    }
}

/// 输入法提交走的是同一个输入法接口，同样不许多等一帧。
#[gpui::test]
fn an_ime_commit_is_painted_by_the_very_next_frame(cx: &mut TestAppContext) {
    let mut f = open(cx, &sample_text());
    f.caret(100, 8);
    f.settle();
    f.start_recording();
    f.commit("你好世界");
    f.settle();
    let after = f.version();
    let frames = f.recorded();
    let first = frames.first().expect("提交之后至少要画一帧");
    assert_eq!(
        first.text_version, after,
        "输入法提交多等了一帧：{frames:?}"
    );
}

/// 基准拿 `text_version` 认帧、拿 `end` 当终点，这两个字段的不变量在这里钉住。
#[gpui::test]
fn frame_stats_carry_the_painted_version_and_the_paint_end(cx: &mut TestAppContext) {
    let mut f = open(cx, &sample_text());
    f.caret(100, 8);
    f.settle();
    f.start_recording();
    for key in "abc".chars() {
        f.press(&key.to_string());
        f.settle();
    }
    let frames = f.recorded();
    assert!(frames.len() >= 3, "至少应该画出三帧：{frames:?}");
    let mut last_end = None;
    let mut last_version = 0;
    for frame in &frames {
        let end = frame.end.expect("每一帧都要记下 paint 结束时刻");
        if let Some(prev) = last_end {
            assert!(end >= prev, "paint 结束时刻倒退了：{frames:?}");
        }
        last_end = Some(end);
        assert!(
            frame.text_version >= last_version,
            "画出来的文本版本倒退了：{frames:?}"
        );
        last_version = frame.text_version;
    }
    assert_eq!(
        last_version,
        f.version(),
        "最后一帧画的不是当前版本：{frames:?}"
    );
    // `cpu` 的口径是 prepaint → paint，`end` 是它的终点，两者必须同时有值。
    assert!(frames.iter().all(|s| s.cpu > std::time::Duration::ZERO));
}
