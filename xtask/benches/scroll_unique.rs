//! 代码视图连续滚动基准（issue #4 预留、#19 实现）。
//!
//! 在真实 GPUI 窗口里打开 `rove_ui::code_view::CodeView`，每帧滚动固定行数，记录：
//! - **每帧 CPU**：`CodeView` 的 prepaint 开始到 paint 结束（口径同 `M0_UI_SPIKE_REPORT` §2.5，
//!   不含 present 的 vsync 阻塞），由 `CodeView::record_frames` 采集；
//! - **帧间隔**：相邻两次 `on_next_frame`（帧已绘制）回调的墙钟间隔。
//!
//! 两个场景：
//! - `scroll/frame_cpu_cache_hit`：`small.txt`（7 种模式循环），300 帧 × 3 行，整形缓存命中；
//! - `scroll/frame_interval_cache_miss`、`scroll/frame_cpu_cache_miss`：`uniq.txt`（每行唯一），
//!   400 帧 × 37 行，整形缓存全未命中（M0：帧间隔 p99 16.83 ms）。
//!
//! 测量在窗口里跑完后，逐帧样本按 criterion 的结果格式写入 `target/criterion/scroll/`（见 [`record`]），
//! 这样 `cargo xtask bench` 的汇总、基线比较与预算判定（mean / p99）照常工作。
//! 无法创建窗口（例如没有图形设备的环境）时打印「已跳过」并正常退出。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    px, size, App, AppContext, Application, Bounds, Entity, Window, WindowBounds, WindowOptions,
};
use rove_buffer::{open_path, LoadOptions, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{self, CodeView, CodeViewServices, ScrollPosition};

fn testdata_dir() -> PathBuf {
    std::env::var_os("ROVE_PERF_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/perf-testdata"))
}

struct Scenario {
    file: &'static str,
    frames: usize,
    step: f64,
    /// 来回滚动的范围（行）；small.txt 只有 401 行，循环滚动保证命中。
    wrap: Option<f64>,
}

#[derive(Default, Clone)]
struct Samples {
    cpu: Vec<Duration>,
    layout: Vec<Duration>,
    glyphs: u64,
    interval: Vec<Duration>,
    placeholders: u64,
    frames_with_placeholders: usize,
}

fn services() -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    rove_buffer::settings::register(&mut settings).expect("编码设置");
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).expect("登记");
    CodeViewServices {
        settings: Arc::new(DefaultSettings(settings)),
        theme: Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        theme_kind: ThemeKind::Dark,
    }
}

fn load(file: &str) -> Option<Arc<dyn TextSource>> {
    let path = testdata_dir().join(file);
    let mut registry = SettingRegistry::default();
    rove_buffer::settings::register(&mut registry).ok()?;
    let settings = DefaultSettings(registry);
    match open_path(&path, &LoadOptions::new("plaintext", &settings)) {
        Ok(loaded) => Some(loaded.text),
        Err(err) => {
            eprintln!("已跳过 {file}：无法打开 {}（{err}）", path.display());
            None
        }
    }
}

/// 在一个 GPUI 应用里依次跑完所有场景。
fn measure(scenarios: Vec<(Scenario, Arc<dyn TextSource>)>) -> Vec<Samples> {
    let results: Rc<RefCell<Vec<Samples>>> = Rc::default();
    let out = results.clone();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let services = services();
        let window = cx.open_window(options, |_, cx| cx.new(|cx| CodeView::new(services, cx)));
        let window = match window {
            Ok(w) => w,
            Err(err) => {
                eprintln!("已跳过 scroll 基准：无法创建窗口（{err}）");
                cx.quit();
                return;
            }
        };
        eprintln!("scroll 基准：窗口已创建");
        let queue: Queue = Rc::new(RefCell::new(scenarios.into_iter()));
        let out = out.clone();
        let Ok(view) = window.update(cx, |_, _, cx| cx.entity()) else {
            cx.quit();
            return;
        };
        // 用 AnyWindowHandle 更新：不占用根视图实体，run_next 里才能 update 它。
        let any: gpui::AnyWindowHandle = window.into();
        let _ = cx.update_window(any, |_, w, cx| {
            w.activate_window();
            run_next(view, queue, out, w, cx);
        });
    });
    results.take()
}

type Queue = Rc<RefCell<std::vec::IntoIter<(Scenario, Arc<dyn TextSource>)>>>;

struct Run {
    view: Entity<CodeView>,
    scenario: Scenario,
    warmup: usize,
    n: usize,
    last: Option<Instant>,
    samples: Samples,
    queue: Queue,
    out: Rc<RefCell<Vec<Samples>>>,
}

/// 开始下一个场景。必须在窗口上下文里调用（帧回调里不能再 `WindowHandle::update` 同一个窗口）。
fn run_next(
    view: Entity<CodeView>,
    queue: Queue,
    out: Rc<RefCell<Vec<Samples>>>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((scenario, text)) = queue.borrow_mut().next() else {
        eprintln!("scroll 基准：全部场景完成");
        cx.quit();
        return;
    };
    eprintln!(
        "scroll 基准：开始 {}（{} 帧 × {} 行）",
        scenario.file, scenario.frames, scenario.step
    );
    view.update(cx, |v, cx| {
        v.set_text(Some(text), false, cx);
        v.record_frames(false);
    });
    // 缓存命中场景的预热要覆盖整个循环范围。
    let warmup = 30
        + scenario
            .wrap
            .map_or(0, |w| (w / scenario.step).ceil() as usize + 1);
    let run = Rc::new(RefCell::new(Run {
        view,
        scenario,
        warmup,
        n: 0,
        last: None,
        samples: Samples::default(),
        queue,
        out,
    }));
    window.on_next_frame(move |w, cx| frame(run, w, cx));
    window.refresh();
}

/// 每帧回调（帧已绘制）：记录帧间隔，滚动固定行数，登记下一帧。
fn frame(run: Rc<RefCell<Run>>, window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    let mut r = run.borrow_mut();
    r.n += 1;
    let n = r.n;
    if n > r.warmup + 1 {
        if let Some(prev) = r.last {
            r.samples.interval.push(now - prev);
        }
    }
    r.last = Some(now);
    let view = r.view.clone();
    if n > r.warmup + r.scenario.frames {
        let frames = view.update(cx, |v, _| v.take_frame_samples());
        let mut samples = std::mem::take(&mut r.samples);
        for f in frames {
            samples.cpu.push(f.cpu);
            samples.layout.push(f.layout);
            samples.glyphs += f.glyphs as u64;
            samples.placeholders += (f.placeholders + f.missing) as u64;
            if f.placeholders + f.missing > 0 {
                samples.frames_with_placeholders += 1;
            }
        }
        r.out.borrow_mut().push(samples);
        let (queue, out) = (r.queue.clone(), r.out.clone());
        drop(r);
        run_next(view, queue, out, window, cx);
        return;
    }
    // 预热：缓存命中场景先把整份文件滚一遍（字形图集、整形缓存都热起来），再回到顶端计时；
    // 未命中场景只空转，避免提前整形。
    let warming = n <= r.warmup;
    let step = match (warming, r.scenario.wrap) {
        (true, Some(_)) => r.scenario.step,
        (true, None) => 0.0,
        (false, _) => r.scenario.step,
    };
    let wrap = r.scenario.wrap;
    let start_recording = n == r.warmup + 1;
    drop(r);
    view.update(cx, |v, cx| {
        if start_recording {
            v.record_frames(true);
        }
        let mut top = if start_recording {
            0.0
        } else {
            v.scroll_position().top + step
        };
        if let Some(wrap) = wrap {
            if top > wrap {
                top = 0.0;
            }
        }
        v.set_scroll_position(ScrollPosition { top, x: 0.0 }, false, cx);
    });
    window.on_next_frame(move |w, cx| frame(run, w, cx));
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

/// 把逐帧样本写成 criterion 的结果格式（`<target>/criterion/scroll/<id>/new/{estimates,sample}.json`），
/// 一个样本恰好是一帧，`cargo xtask bench` 的汇总、基线比较与 p99 预算判定照常读取。
/// 不经过 criterion 的计时循环：帧时间是在窗口里实测的，重新“计时”只会把样本平均掉。
fn record(id: &str, samples: &[Duration]) {
    if samples.is_empty() {
        return;
    }
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.parent()?.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("../target"));
    let dir = target.join("criterion").join("scroll").join(id).join("new");
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
    eprintln!("scroll 基准：加载测试数据 {}", testdata_dir().display());
    let mut scenarios = vec![];
    let mut names = vec![];
    if let Some(text) = load("small.txt") {
        scenarios.push((
            Scenario {
                file: "small.txt",
                frames: 300,
                step: 3.0,
                wrap: Some(300.0),
            },
            text,
        ));
        names.push("hit");
    }
    if let Some(text) = load("uniq.txt") {
        scenarios.push((
            Scenario {
                file: "uniq.txt",
                frames: 400,
                step: 37.0,
                wrap: None,
            },
            text,
        ));
        names.push("miss");
    }
    if scenarios.is_empty() {
        eprintln!("已跳过 scroll 基准：缺少 small.txt / uniq.txt（先 cargo xtask gen-testdata）");
        return;
    }
    // 看门狗：没有显示设备的环境里窗口可能永远不出帧，超时就跳过而不是挂住 CI。
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(180));
        eprintln!("已跳过 scroll 基准：180 s 内没有完成（窗口没有产生帧，可能没有显示设备）");
        std::process::exit(0);
    });
    let files: Vec<&str> = scenarios.iter().map(|(s, _)| s.file).collect();
    let results = measure(scenarios);
    if results.is_empty() {
        return;
    }
    if let Ok(note) = std::env::var("ROVE_PERF_MACHINE_NOTE") {
        println!("机器：{note}");
    }
    for ((name, file), s) in names.iter().zip(files).zip(&results) {
        println!(
            "scroll/{name}（{file}）：占位帧 {} / {}，占位行累计 {}",
            s.frames_with_placeholders,
            s.cpu.len(),
            s.placeholders
        );
        stats("每帧 CPU", &s.cpu);
        stats("  其中布局", &s.layout);
        println!(
            "  平均每帧绘制字形 {:.0}",
            s.glyphs as f64 / s.cpu.len().max(1) as f64
        );
        stats("帧间隔", &s.interval);
        match *name {
            "hit" => {
                record("frame_cpu_cache_hit", &s.cpu);
                record("frame_interval_cache_hit", &s.interval);
            }
            _ => {
                record("frame_cpu_cache_miss", &s.cpu);
                record("frame_interval_cache_miss", &s.interval);
            }
        }
    }
}
