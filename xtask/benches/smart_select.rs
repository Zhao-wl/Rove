//! #54：2000 行 C# 的真实快捷键到扩选绘制。

use gpui::{
    prelude::*, px, size, AnyWindowHandle, Application, AsyncApp, Bounds, Entity, Focusable,
    Keystroke, Window, WindowBounds, WindowHandle, WindowOptions,
};
use rove_base::{ByteOffset, Cancellation, Resource, WorkspacePath};
use rove_ui::{
    code_view::{CodeView, FrameStats, Selection},
    documents::OpenOptions,
    shell::{layout::LayoutStore, Shell},
    skeleton::Workbench,
};
use serde_json::json;
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

const SAMPLES: usize = 60;
const WARMUP: usize = 12;
const NAME: &str = "SelectionFixture.cs";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Measurements {
    active: Vec<f64>,
    wall: Vec<f64>,
}

async fn wait_for(cx: &mut AsyncApp, mut ready: impl FnMut(&mut AsyncApp) -> bool) -> Result<()> {
    let start = Instant::now();
    let timer = cx.background_executor().clone();
    loop {
        if ready(cx) {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(8) {
            return Err("选择探针等待首个可用绘制超时".into());
        }
        timer.timer(Duration::from_millis(3)).await;
    }
}

type FrameReadback = Rc<Cell<Option<(Instant, FrameStats)>>>;

fn read_first_frame(window: &mut Window, code: Entity<CodeView>, readback: FrameReadback) {
    window.on_next_frame(move |window, _| {
        let frame_started = Instant::now();
        // GPUI 先执行回调再绘制。下一次回调读回前一帧的结果，避免轮询混入更晚的帧。
        window.on_next_frame(move |_, cx| {
            readback.set(Some((frame_started, code.read(cx).last_frame())));
        });
    });
}

async fn measure(
    window: WindowHandle<Workbench>,
    offset: ByteOffset,
    cx: &mut AsyncApp,
) -> Result<Measurements> {
    let code: Entity<CodeView> = window.update(cx, |workbench, _, cx| {
        workbench
            .open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(NAME).unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
        workbench.code_view().clone()
    })?;
    let any = AnyWindowHandle::from(window);
    any.update(cx, |_, window, cx| {
        window.activate_window();
        window.focus(&code.focus_handle(cx));
    })?;
    wait_for(cx, |cx| {
        any.update(cx, |_, _, app| {
            let code = code.read(app);
            code.current_syntax_version().is_some() && code.last_frame().glyphs > 0
        })
        .unwrap_or(false)
    })
    .await?;
    let timer = cx.background_executor().clone();
    let mut active = Vec::with_capacity(SAMPLES);
    let mut wall = Vec::with_capacity(SAMPLES);
    for i in 0..WARMUP + SAMPLES {
        any.update(cx, |_, window, app| {
            window.focus(&code.focus_handle(app));
            code.update(app, |code, cx| {
                let position = code.text().unwrap().position(offset).unwrap();
                code.set_selection(Selection::caret(position), None, cx);
            });
        })?;
        timer.timer(Duration::from_millis(24)).await;
        let readback: FrameReadback = Rc::default();
        let (sent, returned, immediate) = any.update(cx, |_, window, app| {
            let key = Keystroke::parse("shift-alt-right").unwrap();
            let sent = Instant::now();
            window.dispatch_keystroke(key, app);
            let returned = Instant::now();
            let frame = code.read(app).last_frame();
            let immediate = frame.end.is_some_and(|end| end >= sent);
            if !immediate {
                read_first_frame(window, code.clone(), readback.clone());
            }
            (sent, returned, immediate.then_some(frame))
        })?;
        let (frame_started, frame) = if let Some(frame) = immediate {
            (returned, frame)
        } else {
            wait_for(cx, |_| readback.get().is_some()).await?;
            readback.get().unwrap()
        };
        let end = frame
            .end
            .filter(|end| *end >= sent && (immediate.is_some() || *end >= frame_started))
            .ok_or("按键后第一个帧请求没有绘制代码视图")?;
        if !frame.selection_painted {
            return Err(format!("样本 {i} 未在按键后的第一帧绘制选区").into());
        }
        let actual_active = returned - sent + end.saturating_duration_since(frame_started);
        if i >= WARMUP {
            active.push(actual_active.as_nanos() as f64);
            wall.push((end - sent).as_nanos() as f64);
        }
    }
    Ok(Measurements { active, wall })
}

fn record(parent: &Path, id: &str, samples: &[f64]) -> Result<()> {
    let dir = parent.join("criterion/smart_select").join(id).join("new");
    std::fs::create_dir_all(&dir)?;
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let mean = json!({ "point_estimate": mean });
    let estimates = json!({ "mean": mean });
    let sample = json!({ "iters": vec![1; samples.len()], "times": samples });
    std::fs::write(dir.join("estimates.json"), serde_json::to_vec(&estimates)?)?;
    std::fs::write(dir.join("sample.json"), serde_json::to_vec(&sample)?)?;
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (sorted.len() as f64 * 0.99).ceil() as usize;
    let p99 = sorted[rank - 1];
    println!(
        "smart_select/{id} n={}, p99={:.3}ms, max={:.3}ms",
        samples.len(),
        p99 / 1_000_000.0,
        sorted[sorted.len() - 1] / 1_000_000.0
    );
    Ok(())
}

fn main() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut source = "class SelectionFixture {\n".to_owned();
    for i in 0..1998 {
        source.push_str(&format!(
            "    int M{i}(int value) {{ return (value + 42) * {i}; }}\n"
        ));
    }
    source.push_str("}\n");
    assert_eq!(source.lines().count(), 2000);
    let offset = ByteOffset((source.find("return (value").unwrap() + "return (v".len()) as u64);
    std::fs::write(dir.path().join(NAME), source)?;
    let root: PathBuf = dir.path().into();
    let out = Rc::new(RefCell::new(None));
    let out_for_run = out.clone();
    Application::new().run(move |cx| {
        rove_ui::keys::init(cx);
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx.open_window(options, |window, cx| {
            cx.new(|cx| {
                let mut workbench =
                    Workbench::with_store(root, Instant::now(), LayoutStore::disabled(), cx);
                Shell::attach_window(&mut workbench, window, cx);
                workbench
            })
        });
        let Ok(window) = window else {
            eprintln!("已跳过智能扩选实窗基准：无法创建窗口");
            cx.quit();
            return;
        };
        cx.activate(true);
        cx.spawn(async move |cx| {
            *out_for_run.borrow_mut() = Some(measure(window, offset, cx).await);
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    if let Some(measurements) = out.borrow_mut().take() {
        let measurements = measurements?;
        println!(
            "智能扩选在下一帧绘制：{}/{}",
            measurements.active.len(),
            SAMPLES
        );
        let exe = std::env::current_exe()?;
        let target = exe.ancestors().nth(3).ok_or("找不到性能目标目录")?;
        record(target, "key_active", &measurements.active)?;
        record(target, "key_to_paint", &measurements.wall)?;
    }
    Ok(())
}
