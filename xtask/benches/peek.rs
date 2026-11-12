//! 速览窗口基准（#41），在真实 GPUI 窗口里测界面侧耗时（查询本身属于 #38 的 `cs_router/` 预算）。
//!
//! - `peek/open_to_first_paint_28k`：最重的符号（M0 实测 28,671 处引用，分布在 2,900 个文件）——
//!   从创建速览窗口（查询替身立即返回同一份 `Arc<[RoutedTarget]>`）到**结果树首次绘制**（窗口宿主的
//!   paint 阶段看到结果树的那一刻）。含分组（排序 + 去重 + 计数）、读第一个文件的片段、建树、
//!   预览的第一个文件；不含 #38 的查询本身。预算 p99 < 100 ms。
//! - `peek/tree_scroll_frame_47k`：设计上限 4.7 万条结果、3,000 个文件**全部展开**后，结果树每帧
//!   滚动 37 行，ListView 的 render 开始到 paint 结束（口径同 #18 的 `list_bench`）。
//! - 反复开关 100 次的工作集 / 私有字节变化（Windows，打印到 stdout，不进 criterion）。
//!
//! 文档是内存里生成的 C#（每个文件约 40 行），不读盘。无法创建窗口时打印「已跳过」并正常退出。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    canvas, div, prelude::*, px, size, App, Application, Bounds, Context, Entity, Render, Window,
    WindowBounds, WindowOptions,
};
use rove_base::{
    Cancellation, DocumentId, Line, Position, Resource, Span, Task, Utf8Column, WorkspacePath,
};
use rove_buffer::{
    Document, DocumentMetadata, Documents, Encoding, Eol, LineEndings, RopeText, SizeClass,
};
use rove_config::{DefaultSettings, SettingRegistry};
use rove_lang_cs::query::{Hover, NavigationKind, Precision, QueryPoint, QueryResult, Target};
use rove_lang_cs::router::{RoutedTarget, TargetRole};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{self, CodeView, CodeViewServices};
use rove_ui::list_widget::view::FrameProbe;
use rove_ui::peek::{
    zone::zone_overlay, Marker, PeekKind, PeekQueries, PeekRequest, PeekServices, PeekWidget,
};

const HEAVY_REFS: usize = 28_671;
const HEAVY_FILES: usize = 2_900;
const MAX_REFS: usize = 47_000;
const MAX_FILES: usize = 3_000;
const LINES_PER_FILE: usize = 40;
const OPEN_RUNS: usize = 40;
const OPEN_WARMUP: usize = 5;
const MEMORY_CYCLES: usize = 100;
/// 再开关一轮同样次数，看增长是否只是第一轮的缓存预热。
const MEMORY_ROUNDS: usize = 2;
const SCROLL_FRAMES: usize = 300;

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn path(i: usize) -> String {
    format!("Assets/Scripts/Module{:02}/Generated{i:05}.cs", i % 37)
}

/// `files` 个文件里共 `total` 处引用（每个文件轮流分到一处，行号在 4..LINES_PER_FILE 之间）。
fn targets(files: usize, total: usize) -> Arc<[RoutedTarget]> {
    (0..total)
        .map(|i| {
            let file = i % files;
            let line = 4 + ((i / files) % (LINES_PER_FILE - 6)) as u64;
            let range = Span::new(pos(line, 16), pos(line, 20)).unwrap();
            RoutedTarget {
                target: Target {
                    resource: Resource::Workspace(WorkspacePath::new(&path(file)).unwrap()),
                    range: range.clone(),
                    selection: range,
                },
                precision: Precision::Exact,
                reason: None,
                role: TargetRole::Reference,
            }
        })
        .collect()
}

fn source(i: usize) -> String {
    let mut s = format!(
        "namespace Game\n{{\n    public sealed class Generated{i} : MonoBehaviour\n    {{\n"
    );
    while s.lines().count() < LINES_PER_FILE - 2 {
        s.push_str("        void Tick() { Move(); Move(); Move(); }\n");
    }
    s.push_str("    }\n}\n");
    s
}

/// 内存文档：每个资源一份 `RopeText`（与读盘后的形态相同）。
struct MemDocs {
    docs: HashMap<Resource, Arc<Document>>,
}

impl MemDocs {
    fn new(files: usize) -> Self {
        let docs = (0..files)
            .map(|i| {
                let resource = Resource::Workspace(WorkspacePath::new(&path(i)).unwrap());
                let doc = Arc::new(Document {
                    id: DocumentId(i as u64 + 1),
                    resource: resource.clone(),
                    metadata: DocumentMetadata {
                        language: "csharp".into(),
                        encoding: Encoding::Utf8,
                        bom: false,
                        line_endings: LineEndings::Uniform(Eol::Lf),
                        read_only: Arc::from([]),
                        size: SizeClass::Normal,
                    },
                    text: Arc::new(RopeText::new(source(i))),
                });
                (resource, doc)
            })
            .collect();
        Self { docs }
    }
}

impl Documents for MemDocs {
    fn find(&self, resource: &Resource) -> Option<Arc<Document>> {
        self.docs.get(resource).cloned()
    }
    fn open(&self, resource: Resource, _: Cancellation) -> Task<Arc<Document>> {
        let doc = self.docs.get(&resource).cloned();
        Box::pin(async move { doc.ok_or(rove_base::Error::NotFound) })
    }
}

/// 查询替身：立即返回同一份结果（只复制 `Arc`）。
struct Canned(Arc<[RoutedTarget]>);

impl PeekQueries for Canned {
    fn navigate(
        &self,
        _: NavigationKind,
        point: &QueryPoint,
        _: &Cancellation,
    ) -> rove_base::Result<QueryResult<Arc<[RoutedTarget]>>> {
        Ok(QueryResult {
            value: self.0.clone(),
            precision: Precision::Exact,
            reason: None,
            version: Some(point.version),
            index_generation: 1,
        })
    }
    fn hover(
        &self,
        point: &QueryPoint,
        _: &Cancellation,
    ) -> rove_base::Result<QueryResult<Option<Hover>>> {
        Ok(QueryResult {
            value: None,
            precision: Precision::Exact,
            reason: None,
            version: Some(point.version),
            index_generation: 1,
        })
    }
}

fn registries() -> (
    Arc<DefaultSettings>,
    Arc<DefaultTheme>,
    Arc<rove_i18n::StringTable>,
) {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::chinese().expect("字符串表");
    rove_buffer::settings::register(&mut settings).expect("编码设置");
    rove_config::register_builtin(&mut settings).expect("内置设置");
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).expect("登记");
    rove_ui::peek::registry::register_colors(&mut colors).expect("速览颜色");
    rove_ui::peek::registry::register_strings(&mut strings).expect("速览字符串");
    rove_ui::list_widget::registry::register_strings(&mut strings).expect("列表字符串");
    (
        Arc::new(DefaultSettings(settings)),
        Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        Arc::new(strings),
    )
}

/// 窗口根：主编辑器 + 嵌入区域里的速览窗口；paint 时记录“结果树首次画出来”的时刻。
struct Host {
    code: Entity<CodeView>,
    peek: Option<Entity<PeekWidget>>,
    painted: Rc<Cell<Option<Instant>>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_tree = self
            .peek
            .as_ref()
            .is_some_and(|p| p.read(cx).list().is_some());
        let painted = self.painted.clone();
        div()
            .relative()
            .size_full()
            .child(self.code.clone())
            .children(
                self.peek
                    .clone()
                    .map(|p| zone_overlay(self.code.clone(), 0, p.into_any_element())),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, _, _| {
                        if has_tree && painted.get().is_none() {
                            painted.set(Some(Instant::now()));
                        }
                    },
                )
                .absolute()
                .size_0(),
            )
    }
}

#[derive(Default)]
struct Out {
    open: Vec<Duration>,
    scroll_cpu: Vec<Duration>,
    /// 第 0、100、200 次开关后的内存。
    memory: Vec<Memory>,
    expand_all: Option<Duration>,
}

#[derive(Clone, Copy, Debug)]
struct Memory {
    working_set: usize,
    private: usize,
}

#[cfg(windows)]
fn memory() -> Option<Memory> {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
    }
    let mut c: Counters = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<Counters>() as u32;
    // SAFETY: 传入当前进程的伪句柄与大小正确的输出结构（PROCESS_MEMORY_COUNTERS_EX）。
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    (ok != 0).then_some(Memory {
        working_set: c.working_set_size,
        private: c.private_usage,
    })
}

#[cfg(not(windows))]
fn memory() -> Option<Memory> {
    None
}

/// 一个场景的状态机，由帧回调推进。
enum Step {
    /// 打开速览，等结果树首次绘制。
    Open {
        n: usize,
        started: Instant,
    },
    /// 关闭后等两帧再开下一次。
    Closed {
        n: usize,
        frames: usize,
    },
    /// 打开 4.7 万条结果并全部展开，等片段读完。
    Expand {
        started: Instant,
        requested: bool,
    },
    Scroll {
        n: usize,
    },
    /// 反复开关 100 次。
    Memory {
        n: usize,
        opened: bool,
        before: Option<Memory>,
    },
    Done,
}

struct Bench {
    host: Entity<Host>,
    services_heavy: Rc<PeekServices>,
    services_max: Rc<PeekServices>,
    line_height: f32,
    probe: Rc<RefCell<FrameProbe>>,
    step: Step,
    out: Rc<RefCell<Out>>,
}

impl Bench {
    fn open(&mut self, services: Rc<PeekServices>, window: &mut Window, cx: &mut App) {
        let request = PeekRequest {
            kind: PeekKind::References,
            point: QueryPoint {
                resource: Resource::Workspace(WorkspacePath::new("Assets/Main.cs").unwrap()),
                position: pos(0, 0),
                version: rove_base::Version(1),
            },
            symbol: "Move".into(),
            marker: Marker::Auto,
            preset: None,
        };
        let lh = self.line_height;
        let widget = cx.new(|cx| PeekWidget::new(services, request, lh, window, cx));
        let height = widget.read(cx).height();
        self.host.update(cx, |h, cx| {
            h.painted.set(None);
            h.code.update(cx, |c, cx| c.set_view_zone(0, height, cx));
            h.peek = Some(widget);
            cx.notify();
        });
    }

    fn close(&mut self, cx: &mut App) {
        self.host.update(cx, |h, cx| {
            h.peek = None;
            h.code.update(cx, |c, cx| c.set_view_zone(0, 0.0, cx));
            cx.notify();
        });
    }

    fn widget(&self, cx: &App) -> Option<Entity<PeekWidget>> {
        self.host.read(cx).peek.clone()
    }

    fn tick(&mut self, window: &mut Window, cx: &mut App) -> bool {
        match std::mem::replace(&mut self.step, Step::Done) {
            Step::Open { n, started } => {
                let painted = self.host.read(cx).painted.get();
                match painted {
                    Some(at) => {
                        if n >= OPEN_WARMUP {
                            self.out.borrow_mut().open.push(at - started);
                        }
                        self.close(cx);
                        self.step = Step::Closed {
                            n: n + 1,
                            frames: 0,
                        };
                    }
                    None => self.step = Step::Open { n, started },
                }
            }
            Step::Closed { n, frames } if frames < 2 => {
                self.step = Step::Closed {
                    n,
                    frames: frames + 1,
                }
            }
            Step::Closed { n, .. } if n < OPEN_WARMUP + OPEN_RUNS => {
                let started = Instant::now();
                self.open(self.services_heavy.clone(), window, cx);
                self.step = Step::Open { n, started };
            }
            Step::Closed { .. } => {
                eprintln!("peek 基准：展开 4.7 万条结果");
                self.open(self.services_max.clone(), window, cx);
                self.step = Step::Expand {
                    started: Instant::now(),
                    requested: false,
                };
            }
            Step::Expand { started, requested } => {
                let Some(w) = self.widget(cx) else {
                    return false;
                };
                self.step = Step::Expand { started, requested };
                let Some(list) = w.read(cx).list().cloned() else {
                    return true;
                };
                let len = list.update(cx, |v, cx| {
                    if !requested {
                        for i in 0..MAX_FILES {
                            let id = rove_ui::lists::ItemId(format!("f{i}").into());
                            let _ = v.widget.expand_id(&id, true);
                        }
                        cx.notify();
                    }
                    v.widget.len()
                });
                self.step = Step::Expand {
                    started,
                    requested: true,
                };
                if len == MAX_FILES + MAX_REFS {
                    self.out.borrow_mut().expand_all = Some(started.elapsed());
                    let probe = self.probe.clone();
                    list.update(cx, |v, _| v.frame_probe = Some(probe));
                    self.step = Step::Scroll { n: 0 };
                }
            }
            Step::Scroll { n } => {
                let w = self.widget(cx).unwrap();
                let list = w.read(cx).list().cloned().unwrap();
                if n >= SCROLL_FRAMES + 20 {
                    let cpu = std::mem::take(&mut self.probe.borrow_mut().cpu_us);
                    self.out.borrow_mut().scroll_cpu = cpu
                        .into_iter()
                        .skip(20)
                        .map(|us| Duration::from_secs_f64(us / 1e6))
                        .collect();
                    self.close(cx);
                    self.step = Step::Memory {
                        n: 0,
                        opened: false,
                        before: None,
                    };
                    return true;
                }
                list.update(cx, |v, cx| {
                    let rows = 37.0 * 22.0;
                    if v.widget.scroll_top() + rows > v.widget.total_height() - v.widget.viewport()
                    {
                        v.widget.set_scroll_top(0.0);
                    } else {
                        v.widget.scroll_by(rows);
                    }
                    cx.notify();
                });
                self.step = Step::Scroll { n: n + 1 };
            }
            Step::Memory { n, opened, before } => {
                if n % MEMORY_CYCLES == 0 && !opened {
                    if let Some(m) = memory() {
                        let mut out = self.out.borrow_mut();
                        if out.memory.len() == n / MEMORY_CYCLES {
                            out.memory.push(m);
                        }
                    }
                }
                if n == MEMORY_CYCLES * MEMORY_ROUNDS {
                    self.step = Step::Done;
                    return false;
                }
                if opened {
                    let ready = self.widget(cx).is_some_and(|w| w.read(cx).list().is_some());
                    if ready {
                        self.close(cx);
                        self.step = Step::Memory {
                            n: n + 1,
                            opened: false,
                            before,
                        };
                    } else {
                        self.step = Step::Memory { n, opened, before };
                    }
                } else {
                    self.open(self.services_heavy.clone(), window, cx);
                    self.step = Step::Memory {
                        n,
                        opened: true,
                        before,
                    };
                }
            }
            Step::Done => return false,
        }
        true
    }
}

fn frame(bench: Rc<RefCell<Bench>>, window: &mut Window, cx: &mut App) {
    let more = bench.borrow_mut().tick(window, cx);
    if more {
        window.on_next_frame(move |w, cx| frame(bench, w, cx));
        window.refresh();
    } else {
        cx.quit();
    }
}

fn services(
    files: usize,
    total: usize,
    registries: &(
        Arc<DefaultSettings>,
        Arc<DefaultTheme>,
        Arc<rove_i18n::StringTable>,
    ),
) -> Rc<PeekServices> {
    let (settings, theme, strings) = registries.clone();
    Rc::new(PeekServices {
        queries: Some(Arc::new(Canned(targets(files, total)))),
        documents: Arc::new(MemDocs::new(files)),
        settings: settings.clone(),
        theme,
        theme_kind: ThemeKind::Dark,
        strings,
        external: Default::default(),
        syntax: Arc::new(
            rove_syntax::highlight::TreeSitterSyntax::default().with_settings(settings),
        ),
    })
}

fn measure() -> Out {
    let out: Rc<RefCell<Out>> = Rc::default();
    let result = out.clone();
    Application::new().run(move |cx: &mut App| {
        let regs = registries();
        let heavy = services(HEAVY_FILES, HEAVY_REFS, &regs);
        let max = services(MAX_FILES, MAX_REFS, &regs);
        let bounds = Bounds::centered(None, size(px(1400.0), px(900.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let (settings, theme) = (regs.0.clone(), regs.1.clone());
        let window = cx.open_window(options, move |_, cx| {
            let code = cx.new(|cx| {
                let mut c = CodeView::new(
                    CodeViewServices {
                        settings,
                        theme,
                        theme_kind: ThemeKind::Dark,
                    },
                    cx,
                );
                c.set_text(Some(Arc::new(RopeText::new(source(0)))), false, cx);
                c
            });
            cx.new(|_| Host {
                code,
                peek: None,
                painted: Rc::default(),
            })
        });
        let window = match window {
            Ok(w) => w,
            Err(err) => {
                eprintln!("已跳过 peek 基准：无法创建窗口（{err}）");
                cx.quit();
                return;
            }
        };
        let Ok(host) = window.update(cx, |_, _, cx| cx.entity()) else {
            cx.quit();
            return;
        };
        let line_height = host.read(cx).code.read(cx).line_height();
        let bench = Rc::new(RefCell::new(Bench {
            host,
            services_heavy: heavy,
            services_max: max,
            line_height,
            probe: Rc::default(),
            step: Step::Closed { n: 0, frames: 0 },
            out: out.clone(),
        }));
        let any: gpui::AnyWindowHandle = window.into();
        let _ = cx.update_window(any, |_, w, _| {
            w.activate_window();
            w.on_next_frame(move |w, cx| frame(bench, w, cx));
            w.refresh();
        });
    });
    result.take()
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

/// 逐次样本写成 criterion 的结果格式（同 `scroll_unique::record`），汇总与 p99 预算照常读取。
fn record(id: &str, samples: &[Duration]) {
    if samples.is_empty() {
        return;
    }
    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.parent()?.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("../target"));
    let dir = target.join("criterion").join("peek").join(id).join("new");
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
    // 看门狗：没有显示设备的环境里窗口可能永远不出帧。
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(300));
        eprintln!("已跳过 peek 基准：300 s 内没有完成（窗口没有产生帧，可能没有显示设备）");
        std::process::exit(0);
    });
    let out = measure();
    if let Ok(note) = std::env::var("ROVE_PERF_MACHINE_NOTE") {
        println!("机器：{note}");
    }
    println!("peek/open_to_first_paint_28k（{HEAVY_REFS} 处引用、{HEAVY_FILES} 个文件）：");
    stats("打开到结果树首次绘制", &out.open);
    if let Some(t) = out.expand_all {
        println!(
            "peek：{MAX_REFS} 条结果、{MAX_FILES} 个文件全部展开（含后台读片段）{:.1} ms",
            t.as_secs_f64() * 1e3
        );
    }
    println!("peek/tree_scroll_frame_47k（全部展开后每帧滚动 37 行）：");
    stats("结果树每帧 CPU", &out.scroll_cpu);
    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
    for (round, pair) in out.memory.windows(2).enumerate() {
        let (before, after) = (pair[0], pair[1]);
        println!(
            "peek：第 {} 轮开关 {MEMORY_CYCLES} 次（{HEAVY_REFS} 处引用）工作集 {:.1} → {:.1} MB（{:+.1}），私有字节 {:.1} → {:.1} MB（{:+.1}）",
            round + 1,
            mb(before.working_set),
            mb(after.working_set),
            mb(after.working_set) - mb(before.working_set),
            mb(before.private),
            mb(after.private),
            mb(after.private) - mb(before.private),
        );
    }
    record("open_to_first_paint_28k", &out.open);
    record("tree_scroll_frame_47k", &out.scroll_cpu);
}
