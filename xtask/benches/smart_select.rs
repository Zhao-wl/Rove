//! #54：2000 行 C# 的真实快捷键到扩选绘制。

use gpui::{
    prelude::*, px, size, AnyWindowHandle, Application, AsyncApp, Bounds, Entity, Focusable,
    Keystroke, WindowBounds, WindowHandle, WindowOptions,
};
use rove_base::{ByteOffset, Cancellation, Resource, WorkspacePath};
use rove_ui::{
    code_view::{CodeView, Selection},
    documents::OpenOptions,
    shell::{layout::LayoutStore, Shell},
    skeleton::Workbench,
};
use serde_json::json;
use std::{
    cell::RefCell,
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
            code.syntax_snapshot()
                .is_some_and(|snapshot| snapshot.version() == code.version())
                && code.last_frame().shaped > 0
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
        let (sent, returned) = any.update(cx, |_, window, app| {
            let key = Keystroke::parse("shift-alt-right").unwrap();
            let sent = Instant::now();
            window.dispatch_keystroke(key, app);
            (sent, Instant::now())
        })?;
        wait_for(cx, |cx| {
            any.update(cx, |_, _, app| {
                let code = code.read(app);
                !code.selection().is_empty() && code.last_frame().end.is_some_and(|end| end >= sent)
            })
            .unwrap_or(false)
        })
        .await?;
        let frame = any.update(cx, |_, _, app| code.read(app).last_frame())?;
        let end = frame.end.unwrap();
        let actual_active = if end <= returned {
            returned - sent
        } else {
            (returned - sent) + frame.cpu
        };
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
    let p99 = sorted[((sorted.len() - 1) as f64 * 0.99).round() as usize];
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
        let exe = std::env::current_exe()?;
        let target = exe.ancestors().nth(3).ok_or("找不到性能目标目录")?;
        record(target, "key_active", &measurements.active)?;
        record(target, "key_to_paint", &measurements.wall)?;
    }
    Ok(())
}
