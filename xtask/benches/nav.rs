//! `F12` → 目标行画面完成（issue #40 的端到端预算，目标文件已打开时 p99 < 50 ms）。
//!
//! 在真实 GPUI 窗口里打开工作台（`rove_ui::skeleton::workbench::Workbench`），装上固定的语义索引
//! （#38 的测试写出器生成的 RVCSIDX v2，读取端是真实的 `CsIndex`），两个文件都已打开：
//! `Caller.cs` 有 200 处调用，`Target.cs` 有 2 万行、200 个方法均匀分布（每次跳到不同的区域，整形缓存
//! 不命中）。每一轮：插入符放到第 i 处调用 → 经键位服务的命令派发 `editor.action.revealDefinition`
//! → 等目标位置画到屏幕上（`on_next_frame`）→ `workbench.action.navigateBack` 回到调用处。
//!
//! 分解（写入 `target/criterion/nav/`，`cargo xtask bench` 汇总、比较、按预算判定）：
//! - `nav/f12_to_paint`：按键到目标帧绘制完成（预算 `goto_definition_e2e`，p99）；
//! - `nav/f12_to_frame`：按键到目标位置进入这一帧（开始绘制前）；
//! - `nav/f12_query`：其中的后台查询（`CsRouter::navigate_detailed`）；
//! - `nav/f12_to_next_frame`：按键到下一帧回调（多等一次垂直同步，画面完成的上界）；
//! - `nav/f12_to_full_paint`：按键到第一帧没有等宽占位行的帧回调（目标区域整形缓存未命中时，代码视图
//!   先画占位、空闲切片整形后替换，#19 的设计；上界）。
//!
//! `f12_to_paint` = `f12_to_frame` + 这一帧代码视图的绘制 CPU（prepaint → paint，取自代码视图的帧统计）。
//!
//! 无法创建窗口（例如没有图形设备的环境）时打印「已跳过」并正常退出。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    px, size, App, AppContext, Application, Bounds, Entity, Focusable, Window, WindowBounds,
    WindowOptions,
};
use rove_base::{Cancellation, Line, Position, Resource, Utf8Column, WorkspacePath};
use rove_lang_cs::router::testing::{TestIndex, KIND_METHOD, KIND_TYPE};
use rove_lang_cs::router::{hash_bytes, ManualSemantic};
use rove_ui::documents::OpenOptions;
use rove_ui::skeleton::Workbench;

const METHODS: usize = 200;
const SPACING: usize = 100;
const WARMUP: usize = 10;

fn target_cs() -> String {
    let mut s = String::from("namespace Game\n{\n    public static class Target\n    {\n");
    for i in 0..METHODS {
        s.push_str(&format!(
            "        public static int M{i}(int value)\n        {{\n"
        ));
        for k in 0..SPACING - 4 {
            s.push_str(&format!(
                "            value = value * {k} + {i}; // filler line {k}\n"
            ));
        }
        s.push_str("            return value;\n        }\n");
    }
    s.push_str("    }\n}\n");
    s
}

fn caller_cs() -> String {
    let mut s = String::from(
        "namespace Game\n{\n    public class Caller\n    {\n        void Run()\n        {\n",
    );
    for i in 0..METHODS {
        s.push_str(&format!("            Target.M{i}(1);\n"));
    }
    s.push_str("        }\n    }\n}\n");
    s
}

fn pos(line: usize, col: usize) -> Position {
    Position {
        line: Line(line as u64),
        column: Utf8Column(col as u64),
    }
}

/// 第 i 个方法：定义在 Target.cs 的（行, 列），调用在 Caller.cs 的（行, 列）。
fn def_at(i: usize) -> (usize, usize) {
    (4 + i * SPACING, "        public static int ".len())
}
fn call_at(i: usize) -> (usize, usize) {
    (6 + i, "            Target.".len())
}

fn project() -> std::io::Result<(tempfile::TempDir, Arc<rove_lang_cs::CsIndex>)> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("Assets"))?;
    let (target, caller) = (target_cs(), caller_cs());
    std::fs::write(dir.path().join("Assets/Caller.cs"), &caller)?;
    std::fs::write(dir.path().join("Assets/Target.cs"), &target)?;
    let mut t = TestIndex::default();
    let c = t.file("Assets/Caller.cs", hash_bytes(caller.as_bytes()));
    let f = t.file("Assets/Target.cs", hash_bytes(target.as_bytes()));
    t.symbol("Game|T:Game.Target", "Target", KIND_TYPE, None);
    for i in 0..METHODS {
        let name = format!("M{i}");
        let sym = t.symbol(
            &format!("Game|M:Game.Target.M{i}(System.Int32)"),
            &name,
            KIND_METHOD,
            None,
        );
        let (dl, dc) = def_at(i);
        t.def(sym, f, dl as u32, dc as u32);
        let (cl, cc) = call_at(i);
        t.reference(sym, c, cl as u32, cc as u32);
    }
    let file = dir.path().join("bench.idx");
    t.write(&file)?;
    let index = Arc::new(rove_lang_cs::CsIndex::open(&file)?);
    Ok((dir, index))
}

#[derive(Default)]
struct Out {
    paint: Vec<Duration>,
    next: Vec<Duration>,
    full: Vec<Duration>,
    placeholder_frames: usize,
    frame: Vec<Duration>,
    query: Vec<Duration>,
}

struct Run {
    view: Entity<Workbench>,
    i: usize,
    total: usize,
    /// 等待 F12 的样本（样本数，开始等待的帧数）。
    waiting: Option<(usize, usize)>,
    back: bool,
    frames: usize,
    out: Rc<RefCell<Out>>,
}

fn main() {
    if std::env::args().any(|a| a == "--list") {
        println!("nav/f12_to_paint: benchmark");
        return;
    }
    // `cargo xtask bench --quick` 传 criterion 的 `--sample-size`：少跑几轮。
    let quick = std::env::args().any(|a| a == "--sample-size");
    let total = if quick { 40 } else { METHODS };
    let (dir, index) = match project() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("已跳过 nav 基准：无法生成测试项目（{err}）");
            return;
        }
    };
    let root: PathBuf = dir.path().into();
    let out: Rc<RefCell<Out>> = Rc::default();
    let results = out.clone();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx.open_window(options, |_, cx| {
            cx.new(|cx| Workbench::new(root.clone(), Instant::now(), cx))
        });
        let window = match window {
            Ok(w) => w,
            Err(err) => {
                eprintln!("已跳过 nav 基准：无法创建窗口（{err}）");
                cx.quit();
                return;
            }
        };
        let Ok(view) = window.update(cx, |_, _, cx| cx.entity()) else {
            cx.quit();
            return;
        };
        let any: gpui::AnyWindowHandle = window.into();
        let semantic = Arc::new(ManualSemantic::ready(index.clone()));
        let _ = cx.update_window(any, |_, w, cx| {
            w.activate_window();
            view.update(cx, |v, cx| {
                v.attach_semantic(Some(semantic), cx);
                for path in ["Assets/Target.cs", "Assets/Caller.cs"] {
                    let resource = Resource::Workspace(WorkspacePath::new(path).unwrap());
                    // 固定标签（不是预览）：打开第二个文件时不替换第一个。
                    let options = OpenOptions {
                        preview_tab: false,
                        ..Default::default()
                    };
                    let _ = v
                        .open_documents()
                        .open(resource, options, Cancellation::default());
                }
            });
            let run = Rc::new(RefCell::new(Run {
                view: view.clone(),
                i: 0,
                total: total + WARMUP,
                waiting: None,
                back: false,
                frames: 0,
                out: out.clone(),
            }));
            w.on_next_frame(move |w, cx| frame(run, w, cx));
            w.refresh();
        });
    });
    report(&results.borrow());
    drop(dir);
}

fn active_is(view: &Entity<Workbench>, path: &str, cx: &App) -> bool {
    view.read(cx).active_path().as_deref() == Some(path)
}

fn frame(run: Rc<RefCell<Run>>, window: &mut Window, cx: &mut App) {
    let mut r = run.borrow_mut();
    r.frames += 1;
    let view = r.view.clone();
    if r.frames.is_multiple_of(500) {
        eprintln!(
            "nav 基准：第 {} 帧，第 {} 轮，活动 {:?}，等待 {:?}，回退 {}，样本 {}",
            r.frames,
            r.i,
            view.read(cx).active_path(),
            r.waiting,
            r.back,
            view.read(cx).navigation_samples().len()
        );
    }
    if r.frames > 20_000 {
        eprintln!("nav 基准：超时，已完成 {} 轮", r.i);
        cx.quit();
        return;
    }
    if let Some((_, since)) = r.waiting {
        // 第一次按键时文档可能还没装进代码视图：长时间没有样本就重按。
        if r.frames - since > 600 {
            eprintln!("nav 基准：第 {} 轮等待超时，重试", r.i);
            r.waiting = None;
        }
    }
    let ready = {
        let code = view.read(cx).code_view().clone();
        r.frames > 30 && code.read(cx).text().is_some()
    };
    if !ready {
    } else if let Some((before, _)) = r.waiting {
        // 等目标帧绘制完成的样本。
        let sample = view
            .read(cx)
            .navigation_samples()
            .get(before)
            .copied()
            .filter(|s| s.painted_ms().is_some() && s.full_ms.is_some());
        if let Some(sample) = sample {
            if r.i >= WARMUP {
                let ms = |v: f64| Duration::from_secs_f64(v / 1000.0);
                let mut out = r.out.borrow_mut();
                out.paint.push(ms(sample.painted_ms().unwrap()));
                out.frame.push(ms(sample.frame_ms));
                out.next.push(ms(sample.next_frame_ms.unwrap_or(0.0)));
                out.full.push(ms(sample.full_ms.unwrap_or(0.0)));
                out.placeholder_frames += usize::from(sample.placeholders.unwrap_or(0) > 0);
                out.query.push(ms(sample.query_ms));
            }
            r.waiting = None;
            r.back = true;
            // 回到调用处（导航历史）。
            if let Some(action) = rove_ui::keys::action_for("workbench.action.navigateBack") {
                focus_editor(&view, window, cx);
                window.dispatch_action(action, cx);
            }
        }
    } else if r.back {
        if active_is(&view, "Assets/Caller.cs", cx) {
            r.back = false;
            r.i += 1;
        }
    } else if r.i >= r.total {
        eprintln!("nav 基准：完成 {} 轮", r.total - WARMUP);
        cx.quit();
        return;
    } else if active_is(&view, "Assets/Caller.cs", cx) {
        // 插入符放到第 i 处调用，按 F12（与键位服务分发的是同一个 action）。
        let i = r.i % METHODS;
        let (line, col) = call_at(i);
        let code = view.read(cx).code_view().clone();
        code.update(cx, |c, cx| {
            c.set_selection(
                rove_ui::code_view::Selection::caret(pos(line, col + 1)),
                None,
                cx,
            )
        });
        focus_editor(&view, window, cx);
        let before = view.read(cx).navigation_samples().len();
        r.waiting = Some((before, r.frames));
        if let Some(action) = rove_ui::keys::action_for("editor.action.revealDefinition") {
            window.dispatch_action(action, cx);
        }
    }
    drop(r);
    window.on_next_frame(move |w, cx| frame(run, w, cx));
    window.refresh();
}

fn focus_editor(view: &Entity<Workbench>, window: &mut Window, cx: &mut App) {
    let code = view.read(cx).code_view().clone();
    window.focus(&code.focus_handle(cx));
}

fn stats(name: &str, v: &[Duration]) {
    if v.is_empty() {
        println!("  {name}: 没有样本");
        return;
    }
    let mut s: Vec<f64> = v.iter().map(|d| d.as_secs_f64() * 1e3).collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| s[((s.len() as f64 - 1.0) * p).round() as usize];
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    println!(
        "  {name}: n={} mean {:.3} ms  p50 {:.3}  p99 {:.3}  max {:.3}",
        s.len(),
        mean,
        pct(0.5),
        pct(0.99),
        s[s.len() - 1]
    );
}

fn report(out: &Out) {
    println!("nav（F12 → 目标行画面完成，目标文件已打开）");
    stats("f12_to_paint", &out.paint);
    stats("f12_to_frame", &out.frame);
    stats("f12_query", &out.query);
    stats("f12_to_next_frame", &out.next);
    stats("f12_to_full_paint", &out.full);
    println!(
        "  目标帧里有占位行的样本：{} / {}",
        out.placeholder_frames,
        out.paint.len()
    );
    record("f12_to_paint", &out.paint);
    record("f12_to_frame", &out.frame);
    record("f12_query", &out.query);
    record("f12_to_next_frame", &out.next);
    record("f12_to_full_paint", &out.full);
}

/// 逐个样本按 criterion 的结果格式写入 `target/criterion/nav/<id>/new/`（与 scroll 基准相同）。
fn record(id: &str, samples: &[Duration]) {
    if samples.is_empty() {
        return;
    }
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.parent()?.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("../target"));
    let dir = target.join("criterion").join("nav").join(id).join("new");
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
