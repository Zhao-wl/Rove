//! 按键到上屏（issue #47，CHARTER §6：p99 < 8 ms）。
//!
//! 在真实 GPUI 窗口里打开 `rove_ui::code_view::CodeView`：10 万行内存生成的 C#（每行唯一）、正式
//! tree-sitter 快照与 Dark 2026 主题着色、编辑入口是 #46 的 `EditSession`，插入符在文件中部（第 5 万行）。
//! 每个样本：在上一帧的回调里记下时间，经窗口的按键派发注入一个可打印字符（`Window::dispatch_keystroke`：
//! 键位服务的拦截器 → 按键事件 → 输入法接口 `replace_text_in_range` → 事务 → 增量更新），再在下一帧
//! 回调（这一帧已经绘制，含新字符）里记下时间。
//!
//! - `typing/key_to_frame`：按键进入 → 包含新字符的帧绘制完成（预算 `keystroke_to_paint`，p99）；
//! - `typing/key_handling`：其中按键处理本身（派发返回为止：事务、视图增量更新、事件）；
//! - `typing/frame_cpu`：这些帧里代码视图的绘制 CPU（prepaint → paint，对照 #19 的滚动帧预算）。
//!
//! 注入发生在进程内（不经过系统消息队列与输入法），所以不含 Windows 键盘消息的排队时间；实机
//! 端到端（含输入法）的手测见 PR。语法重解析不在这条路径上：视图先用旧 token 画新文本。
//! 无法创建窗口（例如没有图形设备的环境）时打印「已跳过」并正常退出。

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

const LINES: usize = 100_000;
const WARMUP: usize = 30;
const SAMPLES: usize = 400;

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
    key_to_frame: Vec<Duration>,
    handling: Vec<Duration>,
    frame_cpu: Vec<Duration>,
}

struct Run {
    view: Entity<CodeView>,
    n: usize,
    pending: Option<Instant>,
    out: Rc<RefCell<Out>>,
}

fn frame(run: Rc<RefCell<Run>>, window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    let mut r = run.borrow_mut();
    if let Some(start) = r.pending.take() {
        if r.n > WARMUP {
            let cpu = r.view.read(cx).last_frame().cpu;
            let mut out = r.out.borrow_mut();
            out.key_to_frame.push(now - start);
            out.frame_cpu.push(cpu);
        }
    }
    r.n += 1;
    if r.n > WARMUP + SAMPLES {
        eprintln!("typing 基准：完成");
        cx.quit();
        return;
    }
    let n = r.n;
    let out = r.out.clone();
    drop(r);
    // 注入一个可打印字符（字母循环），记下按键进入的时间。
    let key = (b'a' + (n % 26) as u8) as char;
    let start = Instant::now();
    window.dispatch_keystroke(Keystroke::parse(&key.to_string()).expect("按键"), cx);
    if n > WARMUP {
        out.borrow_mut().handling.push(start.elapsed());
    }
    run.borrow_mut().pending = Some(start);
    window.on_next_frame(move |w, cx| frame(run, w, cx));
    window.refresh();
}

fn stats(name: &str, v: &[Duration]) {
    if v.is_empty() {
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
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("已跳过 typing 基准：180 s 内没有完成（窗口没有产生帧，可能没有显示设备）");
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
                n: 0,
                pending: None,
                out,
            }));
            w.on_next_frame(move |w, cx| frame(run, w, cx));
            w.refresh();
        });
    });
    let out = result.take();
    if out.key_to_frame.is_empty() {
        return;
    }
    if let Ok(note) = std::env::var("ROVE_PERF_MACHINE_NOTE") {
        println!("机器：{note}");
    }
    println!(
        "typing（{LINES} 行 C#，插入符在第 {} 行，带语法着色）：",
        LINES / 2
    );
    stats("按键 → 含新字符的帧绘制完成", &out.key_to_frame);
    stats("  其中按键处理", &out.handling);
    stats("这些帧的代码视图绘制 CPU", &out.frame_cpu);
    record("key_to_frame", &out.key_to_frame);
    record("key_handling", &out.handling);
    record("frame_cpu", &out.frame_cpu);
}
