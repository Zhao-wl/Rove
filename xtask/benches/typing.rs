//! 按键到上屏（issue #47 起，口径由 #215 修正，CHARTER §6：p99 < 8 ms）。
//!
//! 在真实 GPUI 窗口里打开 `rove_ui::code_view::CodeView`：10 万行内存生成的 C#（每行唯一）、正式
//! tree-sitter 快照与 Dark 2026 主题着色、编辑入口是 #46 的 `EditSession`，插入符在文件中部（第 5 万行）。
//!
//! ## 口径（#215，详见 `perf/README.md`「按键到上屏」一节）
//!
//! Windows 上这条路径是：垂直同步线程 `RedrawWindow` → `WM_PAINT` → `request_frame`
//! →（帧回调）→ 绘制 → `Present(0)`。键盘消息是**投递消息**，在消息队列里优先于 `WM_PAINT`，
//! 所以按键总是在「这一帧开始绘制之前」被吸收——不需要额外的采样钩子。
//!
//! 一次按键的墙钟时间因此分成三段：
//!
//! | 段 | 谁决定 | 本基准的 id |
//! |---|---|---|
//! | 按键到达 → 下一次垂直同步（**空等**） | 显示器刷新率与按键到达相位 | `typing/key_vsync_wait` |
//! | 按键处理（事务 + 视图增量更新 + 事件） | Rove | `typing/key_handling` |
//! | 这一帧的绘制（帧回调开始 → 代码视图 paint 结束） | Rove | `typing/frame_cpu`（代码视图部分） |
//!
//! - `typing/key_active`：**判定口径**（预算 `keystroke_to_paint`，p99）= 处理 + 整帧绘制，
//!   不含空等。它就是「按键进入 → 包含它的那一帧绘制完成，但不多等一帧」的那部分。
//! - `typing/key_to_paint`：墙钟（含空等），下界由刷新率决定，只记录。
//! - `typing/key_to_frame`：**旧口径**（#47），终点是「这一帧之后的下一次帧回调」，
//!   比 paint 结束晚了整整一个刷新间隔。保留 id 只为了和历史数据对比。
//!
//! 注入相位：默认把注入点**均匀铺开**在一个刷新间隔里（`typing_plan`），而不是像 #47 那样
//! 固定在帧回调里（相位 0）。设 `ROVE_TYPING_PHASE=frame-start` 可以复现 #47 的相位做对比。
//!
//! 注入发生在进程内（不经过系统消息队列与输入法），所以不含 Windows 键盘消息的排队时间；
//! 真正「上屏」还要再经 DWM 合成与扫描输出（至少一个刷新间隔），进程内测不到，见 `perf/README.md`。
//! 无法创建窗口（例如没有图形设备的环境）时打印「已跳过」并正常退出。

// 采样计划与统计跟 `cargo test -p xtask` 共用一份源码（单元测试在那边跑，这里只用其中一部分）。
#[path = "../src/typing_plan.rs"]
#[allow(dead_code, unused_imports)]
mod typing_plan;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    px, size, App, AppContext, Application, Bounds, Entity, Focusable, Keystroke, Window,
    WindowBounds, WindowOptions,
};
use rove_base::{Line, Position, Utf8Column};
use rove_buffer::edit::{EditBuffer, EditOptions, EditSession};
use rove_buffer::{Eol, LineEndings, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};
use rove_syntax::SyntaxService;
use rove_theme::{ColorRegistry, Theme, ThemeKind};
use rove_ui::code_view::{self, CodeView, CodeViewServices, EditHost, Reveal, Selection};
use typing_plan::{estimate_interval, injection_delay, Absorption, PHASE_CEILING};

const LINES: usize = 100_000;
/// 丢弃的预热按键数（缓存、整形、tree-sitter 都要先热起来）。
const WARMUP: usize = 30;
const SAMPLES: usize = 400;
/// 开始注入之前先空跑几帧，用帧回调的间隔估计刷新间隔。
const CALIBRATE: usize = 60;
/// 一次按键超过这么多帧还没被画出来就认定卡住：打印原因并强制刷新，避免基准挂死。
const STALL_FRAMES: u32 = 20;

fn csharp(lines: usize) -> String {
    let mut s = String::from(
        "namespace Game.Systems\n{\n    public sealed class Generated : MonoBehaviour\n    {\n",
    );
    let mut i = 0;
    let mut count = 4;
    while count < lines - 2 {
        s.push_str(&format!(
            "        // Method {i}: returns a value computed from the input\n        private static int Method{i}(int value, string name = \"item{i}\")\n        {{\n            if (value > {i}) {{ return value * {i} + name.Length; }}\n            return Mathf.Max(value, {i}); // fallback\n        }}\n"
        ));
        count += 6;
        i += 1;
    }
    s.push_str("    }\n}\n");
    s
}

fn services(theme: Arc<dyn Theme>) -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).expect("登记");
    rove_buffer::edit::settings::register(&mut settings).expect("编辑设置");
    CodeViewServices {
        settings: Arc::new(DefaultSettings(settings)),
        theme,
        theme_kind: ThemeKind::Dark,
    }
}

#[derive(Default)]
struct Out {
    /// 按键进入 → 包含新字符的那一帧 paint 结束（墙钟，含空等）。
    key_to_paint: Vec<Duration>,
    /// 其中 Rove 自己干活的部分：处理 + 整帧绘制。
    key_active: Vec<Duration>,
    /// 其中空等下一次垂直同步的部分。
    vsync_wait: Vec<Duration>,
    /// 按键处理本身（`dispatch_keystroke` 返回为止）。
    handling: Vec<Duration>,
    /// 这些帧里代码视图的绘制 CPU（prepaint → paint，对照 #19 的滚动帧预算）。
    frame_cpu: Vec<Duration>,
    /// 旧口径（#47）：终点取「这一帧之后的下一次帧回调」。
    key_to_frame: Vec<Duration>,
    /// 按键被第几帧吸收。
    absorption: Absorption,
    /// 实际用到的注入相位（毫秒），用来核对相位确实铺开了。
    phases_ms: Vec<f64>,
    /// 按键没有让窗口变脏、需要强制刷新的次数。
    stalls: u32,
}

/// 等待上屏的那一次按键。
struct Pending {
    at: Instant,
    handling: Duration,
    /// 按键之后文本的版本：画出这个版本（或更新）的那一帧才算包含了这次按键。
    target_version: u64,
    /// 注入时「最近画完的那一帧」的结束时刻，用来数之后又画了几帧。
    seen_end: Option<Instant>,
    draws: u32,
    frames: u32,
    phase: Duration,
}

struct Run {
    view: Entity<CodeView>,
    frames: usize,
    /// 已经注入过的按键数（含预热）。
    keys: usize,
    spread: bool,
    interval: Option<Duration>,
    deltas: Vec<Duration>,
    last_callback: Option<Instant>,
    prev_callback: Option<Instant>,
    pending: Option<Pending>,
    /// 注入任务已经排好、还没落地。
    injecting: bool,
    out: Rc<RefCell<Out>>,
}

impl Run {
    fn done(&self) -> bool {
        self.keys >= WARMUP + SAMPLES && self.pending.is_none() && !self.injecting
    }
}

/// 把按键送进窗口，并记下这次采样的起点。调用点已经在 UI 线程上（帧回调里，或帧绘制之后的任务里）。
fn inject(run: &Rc<RefCell<Run>>, phase: Duration, window: &mut Window, cx: &mut App) {
    let (key, seen_end, before) = {
        let r = run.borrow();
        let f = r.view.read(cx).last_frame();
        (
            (b'a' + (r.keys % 26) as u8) as char,
            f.end,
            r.view
                .read(cx)
                .text()
                .map_or(0, |t: &Arc<dyn TextSource>| t.version().0),
        )
    };
    let at = Instant::now();
    window.dispatch_keystroke(Keystroke::parse(&key.to_string()).expect("按键"), cx);
    let handling = at.elapsed();
    let after = run
        .borrow()
        .view
        .read(cx)
        .text()
        .map_or(0, |t: &Arc<dyn TextSource>| t.version().0);
    let mut r = run.borrow_mut();
    r.keys += 1;
    r.injecting = false;
    r.pending = Some(Pending {
        at,
        handling,
        // 版本没动（例如只读）时退而求其次：任何一次新的绘制都算吸收，并由停滞兜底提示。
        target_version: if after > before { after } else { 0 },
        seen_end,
        draws: 0,
        frames: 0,
        phase,
    });
}

/// 结算一次等待中的按键：这一帧画出了新字符就把样本收下，否则继续等。
fn settle(run: &Rc<RefCell<Run>>, now: Instant, cx: &mut App) {
    let mut r = run.borrow_mut();
    let Some(mut pending) = r.pending.take() else {
        return;
    };
    let frame = r.view.read(cx).last_frame();
    pending.frames += 1;
    if frame.end == pending.seen_end {
        r.pending = Some(pending);
        return;
    }
    pending.seen_end = frame.end;
    pending.draws += 1;
    if frame.text_version < pending.target_version {
        r.pending = Some(pending);
        return;
    }
    let (Some(end), Some(frame_started)) = (frame.end, r.prev_callback) else {
        r.pending = Some(pending);
        return;
    };
    if r.keys > WARMUP {
        // 整帧的绘制：这一帧的帧回调开始 → 代码视图 paint 结束（含 GPUI 自己的每帧开销）。
        // `frame-start` 相位下按键是在帧回调**里**注入的，绘制其实从按键处理完才开始，
        // 取两者的晚者，免得把处理时间在两段里各算一遍。
        let frame_work =
            end.saturating_duration_since(frame_started.max(pending.at + pending.handling));
        let wall = end.saturating_duration_since(pending.at);
        let active = pending.handling + frame_work;
        let mut out = r.out.borrow_mut();
        out.key_to_paint.push(wall);
        out.key_active.push(active);
        out.vsync_wait.push(wall.saturating_sub(active));
        out.handling.push(pending.handling);
        out.frame_cpu.push(frame.cpu);
        out.key_to_frame
            .push(now.saturating_duration_since(pending.at));
        out.absorption.record(pending.draws);
        out.phases_ms.push(pending.phase.as_secs_f64() * 1e3);
    }
}

fn frame(run: Rc<RefCell<Run>>, window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    {
        let mut r = run.borrow_mut();
        if let (Some(prev), None) = (r.last_callback, r.interval) {
            let delta = now - prev;
            r.deltas.push(delta);
        }
        r.prev_callback = r.last_callback;
        r.last_callback = Some(now);
        r.frames += 1;
        if r.interval.is_none() && r.frames >= CALIBRATE {
            let deltas = std::mem::take(&mut r.deltas);
            match estimate_interval(&deltas) {
                Some(interval) => {
                    eprintln!(
                        "typing 基准：刷新间隔估计 {:.3} ms（{} 个样本）",
                        interval.as_secs_f64() * 1e3,
                        deltas.len()
                    );
                    r.interval = Some(interval);
                }
                None => {
                    eprintln!("已跳过 typing 基准：量不到稳定的帧间隔（窗口没有在刷新）");
                    cx.quit();
                    return;
                }
            }
        }
    }

    settle(&run, now, cx);

    // 卡住的兜底：按键没有让窗口变脏时强制刷新，并记一笔。
    let stalled = {
        let mut r = run.borrow_mut();
        match r.pending.as_mut() {
            Some(p) if p.frames >= STALL_FRAMES => {
                p.frames = 0;
                r.out.borrow_mut().stalls += 1;
                true
            }
            _ => false,
        }
    };
    if stalled {
        eprintln!("typing 基准：一次按键 {STALL_FRAMES} 帧内没有画出来，强制刷新");
        window.refresh();
    }

    if run.borrow().done() {
        eprintln!("typing 基准：完成");
        cx.quit();
        return;
    }

    // 排下一次注入。
    let plan = {
        let r = run.borrow();
        (r.pending.is_none() && !r.injecting && r.keys < WARMUP + SAMPLES)
            .then(|| (r.spread, r.interval, r.keys))
    };
    if let Some((spread, Some(interval), keys)) = plan {
        run.borrow_mut().injecting = true;
        if spread {
            // 真实按键落在一帧里的任意相位：从这次帧回调起等一段再注入。任务由前台执行器投递
            // （`PostMessageW`），所以它在**这一帧绘制完成之后**才跑，等待不会推迟本帧的绘制。
            let delay = injection_delay(keys, interval, PHASE_CEILING);
            let run2 = run.clone();
            window
                .spawn(cx, async move |cx| {
                    let _ = cx.update(|window, cx| {
                        let deadline = now + delay;
                        while Instant::now() < deadline {
                            std::hint::spin_loop();
                        }
                        inject(&run2, delay, window, cx);
                    });
                })
                .detach();
        } else {
            // #47 的相位：在帧回调里直接注入（刚过垂直同步、这一帧还没开始绘制）。
            inject(&run, Duration::ZERO, window, cx);
            window.refresh();
        }
    }

    window.on_next_frame(move |w, cx| frame(run, w, cx));
}

fn stats(name: &str, v: &[Duration]) {
    let Some(s) = typing_plan::summarize(v) else {
        return;
    };
    println!(
        "  {name}: n={} mean {:.3} ms  p50 {:.3}  p99 {:.3}  max {:.3}",
        s.n, s.mean_ms, s.p50_ms, s.p99_ms, s.max_ms
    );
}

/// 逐样本写成 criterion 的结果格式（同 `scroll_unique.rs`），`cargo xtask bench` 照常汇总与判定。
fn record(id: &str, samples: &[Duration]) {
    if samples.is_empty() {
        return;
    }
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.parent()?.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("../target"));
    let dir = target.join("criterion").join("typing").join(id).join("new");
    let ns: Vec<f64> = samples.iter().map(|d| d.as_nanos() as f64).collect();
    let n = ns.len() as f64;
    let mean = ns.iter().sum::<f64>() / n;
    let var = ns.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let estimates = serde_json::json!({
        "mean": { "point_estimate": mean, "standard_error": (var / n).sqrt() },
    });
    let sample = serde_json::json!({
        "sampling_mode": "Flat",
        "iters": vec![1.0; ns.len()],
        "times": ns,
    });
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("estimates.json"), estimates.to_string())?;
        std::fs::write(dir.join("sample.json"), sample.to_string())
    };
    if let Err(err) = write() {
        eprintln!("无法写入 {}：{err}", dir.display());
    }
}

fn main() {
    // `ROVE_TYPING_PHASE=frame-start` 复现 #47 的注入相位（帧回调里，相位 0）做对比。
    let spread = std::env::var("ROVE_TYPING_PHASE").as_deref() != Ok("frame-start");
    eprintln!("typing 基准：生成 {LINES} 行 C# 并解析");
    let text = csharp(LINES);
    let session = Rc::new(RefCell::new(EditSession::new(
        EditBuffer::new(&text, LineEndings::Uniform(Eol::Lf)),
        EditOptions::default(),
    )));
    let snapshot: Arc<dyn TextSource> = session.borrow().snapshot();
    let syntax = rove_syntax::highlight::TreeSitterSyntax::default()
        .with_refresh_budget(Duration::from_secs(120));
    let update = match futures_lite::future::block_on(syntax.analyze(
        snapshot.clone(),
        "csharp".into(),
        rove_base::Cancellation::default(),
    )) {
        Ok(u) => u,
        Err(err) => {
            eprintln!("已跳过 typing 基准：语法分析失败（{err}）");
            return;
        }
    };
    let theme: Arc<dyn Theme> = rove_theme::ThemeService::with_catalog(
        Arc::new(DefaultSettings(SettingRegistry::default())),
        rove_theme::ThemeCatalog::builtin(),
        Arc::new(code_view::colors::theme_registry()),
    );
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(240));
        eprintln!("已跳过 typing 基准：240 s 内没有完成（窗口没有产生帧，可能没有显示设备）");
        std::process::exit(0);
    });
    let out: Rc<RefCell<Out>> = Rc::default();
    let result = out.clone();
    Application::new().run(move |cx: &mut App| {
        rove_ui::keys::init(cx);
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let services = services(theme.clone());
        let host: Rc<dyn EditHost> = session.clone();
        let snap = update.snapshot.clone();
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                cx.new(|cx| {
                    let mut v = CodeView::new(services, cx);
                    v.set_text(Some(snapshot), false, cx);
                    v.set_language(Some("csharp".into()), cx);
                    v.set_syntax(Some(snap), None, cx);
                    v.set_edit_host(Some(host), cx);
                    v
                })
            },
        );
        let Ok(window) = window else {
            eprintln!("已跳过 typing 基准：无法创建窗口");
            cx.quit();
            return;
        };
        let Ok(view) = window.update(cx, |_, _, cx| cx.entity()) else {
            cx.quit();
            return;
        };
        let any: gpui::AnyWindowHandle = window.into();
        let out = out.clone();
        let _ = cx.update_window(any, |_, w, cx| {
            w.activate_window();
            w.focus(&view.read(cx).focus_handle(cx));
            let middle = Position {
                line: Line(LINES as u64 / 2),
                column: Utf8Column(8),
            };
            view.update(cx, |v, cx| {
                v.set_selection(Selection::caret(middle), Some(Reveal::Center), cx)
            });
            let run = Rc::new(RefCell::new(Run {
                view,
                frames: 0,
                keys: 0,
                spread,
                interval: None,
                deltas: Vec::new(),
                last_callback: None,
                prev_callback: None,
                pending: None,
                injecting: false,
                out,
            }));
            w.on_next_frame(move |w, cx| frame(run, w, cx));
            w.refresh();
        });
    });
    let out = result.take();
    if out.key_to_paint.is_empty() {
        return;
    }
    if let Ok(note) = std::env::var("ROVE_PERF_MACHINE_NOTE") {
        println!("机器：{note}");
    }
    println!(
        "typing（{LINES} 行 C#，插入符在第 {} 行，带语法着色；注入相位：{}）：",
        LINES / 2,
        if spread {
            "均匀铺开在一个刷新间隔里（#215）"
        } else {
            "固定在帧回调里（#47 旧相位）"
        }
    );
    stats(
        "按键 → 含新字符的帧绘制完成（墙钟，含空等）",
        &out.key_to_paint,
    );
    stats(
        "  其中 Rove 干活（处理 + 整帧绘制）【判定口径】",
        &out.key_active,
    );
    stats("  其中空等下一次垂直同步", &out.vsync_wait);
    stats("  其中按键处理", &out.handling);
    stats("这些帧的代码视图绘制 CPU", &out.frame_cpu);
    stats("旧口径（#47：到下一次帧回调）", &out.key_to_frame);
    let phase_lo = out.phases_ms.iter().copied().fold(f64::MAX, f64::min);
    let phase_hi = out.phases_ms.iter().copied().fold(0.0_f64, f64::max);
    println!("  注入相位：{phase_lo:.3}–{phase_hi:.3} ms");
    println!(
        "  按键被第几帧吸收：{}（不多等一帧：{}/{}）{}",
        out.absorption.histogram(),
        out.absorption.next_frame(),
        out.absorption.total(),
        if out.absorption.all_next_frame() {
            ""
        } else {
            " ← 有按键多等了一帧"
        }
    );
    if out.stalls > 0 {
        println!("  按键没有让窗口变脏（强制刷新）：{} 次", out.stalls);
    }
    record("key_to_paint", &out.key_to_paint);
    record("key_active", &out.key_active);
    record("key_vsync_wait", &out.vsync_wait);
    record("key_handling", &out.handling);
    record("frame_cpu", &out.frame_cpu);
    record("key_to_frame", &out.key_to_frame);
}
