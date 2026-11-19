//! #155 真实窗口中的全部替换探针。执行 cargo run --release -p xtask --example replace_all_probe。
//! 默认验证预备事务命中。设置 ROVE_REPLACE_PREPARATION=cold 可验证模板刚变化时的同步路径。
//! 输出代码视图 paint 及帧回调时间。需要判定实际显示间隔时，按输出的 PID 另采集 Windows GPU/显示事件。
#[path = "../src/replace_fixture.rs"]
mod replace_fixture;
#[allow(dead_code)]
#[path = "../src/typing_plan.rs"]
mod typing_plan;

use gpui::{
    prelude::*, px, size, AnyWindowHandle, App, Application, AsyncApp, Bounds, Entity, Focusable,
    Keystroke, Window, WindowBounds, WindowHandle, WindowOptions,
};
use rove_base::{ByteOffset, Cancellation, Resource, Span, WorkspacePath};
use rove_config::{SettingsOptions, SettingsService};
use rove_ui::{
    code_view::{CodeView, FrameStats},
    documents::OpenOptions,
    find::host::FindHost,
    shell::{layout::LayoutStore, Shell},
    skeleton::Workbench,
};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    error::Error,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FILE: &str = "replace.txt";
const SEARCH: &str = "TargetComponent";
const REPLACEMENT: &str = "RenamedComponent";
const ALTERNATE: &str = "UpdatedComponent";
const WARMUP: usize = 3;
const SAMPLES: usize = 24;
const CALIBRATION_FRAMES: usize = 80;
const FOLLOWUP_FRAMES: usize = 6;
const SEARCH_SETTLE: Duration = Duration::from_millis(500);
const STEP_TIMEOUT: Duration = Duration::from_secs(12);

type ProbeResult<T> = Result<T, Box<dyn Error>>;

struct FrameMark {
    callback_at: Instant,
    drawn: FrameStats,
}

type Frames = Rc<RefCell<Vec<FrameMark>>>;

fn pulse(code: Entity<CodeView>, marks: Frames, window: &mut Window, cx: &mut App) {
    marks.borrow_mut().push(FrameMark {
        callback_at: Instant::now(),
        drawn: code.read(cx).last_frame(),
    });
    code.update(cx, |_, cx| cx.notify());
    window.on_next_frame(move |window, cx| pulse(code, marks, window, cx));
}

fn press(key: &str, window: &mut Window, cx: &mut App) {
    let key = Keystroke::parse(key).expect("probe keyboard shortcut");
    window.dispatch_keystroke(key, cx);
}

fn type_word(word: &str, window: &mut Window, cx: &mut App) {
    for c in word.chars() {
        window.dispatch_keystroke(
            Keystroke {
                key: c.to_ascii_lowercase().to_string(),
                key_char: Some(c.to_string()),
                ..Default::default()
            },
            cx,
        );
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn relative_ms(at: Instant, origin: Instant) -> f64 {
    if let Some(elapsed) = at.checked_duration_since(origin) {
        ms(elapsed)
    } else {
        -ms(origin - at)
    }
}

fn callback_gaps(marks: &[FrameMark]) -> Vec<Duration> {
    marks
        .windows(2)
        .map(|pair| pair[1].callback_at - pair[0].callback_at)
        .collect()
}

fn paint_gaps(marks: &[FrameMark]) -> Vec<Duration> {
    let mut paints: Vec<_> = marks.iter().filter_map(|frame| frame.drawn.end).collect();
    paints.dedup();
    paints.windows(2).map(|pair| pair[1] - pair[0]).collect()
}

fn state(window: WindowHandle<Workbench>, cx: &mut AsyncApp) -> ProbeResult<String> {
    let snapshot = window.update(cx, |workbench, _, cx| {
        workbench.code_view().read(cx).text().cloned()
    })?;
    let snapshot = snapshot.ok_or("missing document snapshot")?;
    let range = Span::new(ByteOffset(0), ByteOffset(snapshot.byte_len()))?;
    let mut text = String::with_capacity(snapshot.byte_len() as usize);
    snapshot.visit_chunks(range, &mut |chunk| text.push_str(chunk))?;
    Ok(text)
}

async fn wait_for(
    cx: &mut AsyncApp,
    label: &str,
    mut ready: impl FnMut(&mut AsyncApp) -> bool,
) -> ProbeResult<()> {
    let timer = cx.background_executor().clone();
    let started = Instant::now();
    while !ready(cx) {
        if started.elapsed() >= STEP_TIMEOUT {
            return Err(format!("timed out: {label}").into());
        }
        timer.timer(Duration::from_millis(5)).await;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Injection {
    before: u64,
    at: Instant,
    wall_at: SystemTime,
    returned_at: Instant,
    phase: Duration,
    prepared: bool,
}

async fn inject(
    window: AnyWindowHandle,
    code: &Entity<CodeView>,
    find: &Entity<FindHost>,
    interval: Duration,
    sample: usize,
    cold_template: Option<&'static str>,
    cx: &mut AsyncApp,
) -> ProbeResult<Injection> {
    let injection: Rc<Cell<Option<Injection>>> = Rc::default();
    let capture = injection.clone();
    let code = code.clone();
    let find = find.clone();
    let phase = typing_plan::injection_delay(sample, interval, typing_plan::PHASE_CEILING);
    window.update(cx, |_, window, _| {
        window.on_next_frame(move |window, cx| {
            let frame_start = Instant::now();
            let deadline = frame_start + phase;
            let timer = cx.background_executor().clone();
            window
                .spawn(cx, async move |cx| {
                    // Wait off the UI thread so the probe cannot delay a frame
                    // while it waits for the intended input phase.
                    timer
                        .timer(deadline.saturating_duration_since(Instant::now()))
                        .await;
                    let _ = cx.update(|window, cx| {
                        if let Some(template) = cold_template {
                            let focus = find.read(cx).replace_input_focus(cx);
                            window.focus(&focus);
                            press("ctrl-a", window, cx);
                            type_word(template, window, cx);
                        }
                        let prepared = find.read(cx).replacement_ready(cx);
                        let before = code.read(cx).text().expect("loaded text").version().0;
                        let wall_at = SystemTime::now();
                        let at = Instant::now();
                        press("ctrl-alt-enter", window, cx);
                        capture.set(Some(Injection {
                            before,
                            at,
                            wall_at,
                            returned_at: Instant::now(),
                            phase: at.saturating_duration_since(frame_start),
                            prepared,
                        }));
                    });
                })
                .detach();
        });
    })?;
    wait_for(cx, "replace all shortcut", |_| injection.get().is_some()).await?;
    Ok(injection.get().expect("completed injection"))
}

async fn run(
    window: WindowHandle<Workbench>,
    source: &str,
    cold: bool,
    started: Instant,
    cx: &mut AsyncApp,
) -> ProbeResult<Vec<Value>> {
    let timer = cx.background_executor().clone();
    window.update(cx, |workbench, _, cx| {
        workbench.open_documents().open(
            Resource::Workspace(WorkspacePath::new(FILE).expect("fixture filename")),
            OpenOptions::default(),
            Cancellation::default(),
        )?;
        cx.notify();
        Ok::<_, rove_base::Error>(())
    })??;
    let input_window = AnyWindowHandle::from(window);
    input_window.update(cx, |_, window, _| window.activate_window())?;

    wait_for(cx, "loaded document", |cx| {
        window
            .update(cx, |workbench, _, cx| {
                workbench
                    .code_view()
                    .read(cx)
                    .text()
                    .is_some_and(|text| text.byte_len() == source.len() as u64)
            })
            .unwrap_or(false)
    })
    .await?;
    let (code, find) = window.update(cx, |workbench, _, _| {
        (workbench.code_view().clone(), workbench.find_host().clone())
    })?;
    let expected = source.replace(SEARCH, REPLACEMENT);
    let alternate = source.replace(SEARCH, ALTERNATE);
    let marks: Frames = Rc::default();
    // Input dispatch may draw the root immediately. Do not lease the Workbench while dispatching.
    input_window.update(cx, |_, w, cx| {
        let view = code.clone();
        let frames = marks.clone();
        w.focus(&code.focus_handle(cx));
        w.on_next_frame(move |w, cx| pulse(view, frames, w, cx));
        press("ctrl-f", w, cx);
    })?;
    timer.timer(Duration::from_millis(150)).await;
    input_window.update(cx, |_, w, cx| {
        press("alt-c", w, cx);
        type_word(SEARCH, w, cx);
    })?;
    timer.timer(SEARCH_SETTLE).await;
    input_window.update(cx, |_, w, cx| press("ctrl-h", w, cx))?;
    timer.timer(Duration::from_millis(150)).await;
    input_window.update(cx, |_, w, cx| type_word(REPLACEMENT, w, cx))?;
    timer.timer(SEARCH_SETTLE).await;

    let baseline_from = marks.borrow().len();
    wait_for(cx, "foreground frame calibration", |_| {
        marks.borrow().len() >= baseline_from + CALIBRATION_FRAMES
    })
    .await?;
    let mut output = Vec::with_capacity(SAMPLES + 1);
    let interval = {
        let marks = marks.borrow();
        let baseline = &marks[baseline_from..];
        let callbacks = callback_gaps(baseline);
        let paints = paint_gaps(baseline);
        if paints.len() < CALIBRATION_FRAMES / 2 {
            return Err("code view did not repaint during calibration".into());
        }
        let interval =
            typing_plan::estimate_interval(&callbacks).ok_or("unstable frame callbacks")?;
        output.push(json!({
            "phase": "baseline",
            "pid": std::process::id(),
            "fixture_bytes": source.len(),
            "preparation_mode": if cold { "cold" } else { "warm" },
            "callback_period_ms": ms(interval),
            "two_periods_ms": ms(interval * 2),
            "max_callback_gap_ms": ms(callbacks.into_iter().max().unwrap_or_default()),
            "max_paint_gap_ms": ms(paints.into_iter().max().unwrap_or_default()),
        }));
        interval
    };

    for sample in 0..(WARMUP + SAMPLES) {
        let changed_template = if sample % 2 == 0 {
            ALTERNATE
        } else {
            REPLACEMENT
        };
        let injection = inject(
            input_window,
            &code,
            &find,
            interval,
            sample,
            cold.then_some(changed_template),
            cx,
        )
        .await?;
        if injection.prepared == cold {
            return Err(format!(
                "sample {sample}: unexpected prepared state {}",
                injection.prepared
            )
            .into());
        }
        let acceptance = |frames: &[FrameMark]| {
            frames.iter().position(|frame| {
                frame.drawn.text_version > injection.before
                    && frame.drawn.end.is_some_and(|end| end >= injection.at)
            })
        };
        wait_for(
            cx,
            "new document version painted and steady frames resumed",
            |_| {
                let frames = marks.borrow();
                acceptance(&frames).is_some_and(|index| frames.len() > index + FOLLOWUP_FRAMES)
            },
        )
        .await?;

        if sample >= WARMUP {
            let frames = marks.borrow();
            let first_index = acceptance(&frames).expect("new paint");
            let end = frames[first_index].drawn.end.expect("new paint end");
            let begin = frames
                .partition_point(|frame| frame.callback_at < injection.at)
                .saturating_sub(2);
            let window_frames = &frames[begin..=first_index + FOLLOWUP_FRAMES];
            let max_callback = callback_gaps(window_frames)
                .into_iter()
                .max()
                .unwrap_or_default();
            let max_paint = paint_gaps(window_frames)
                .into_iter()
                .max()
                .unwrap_or_default();
            let timeline: Vec<_> = window_frames
                .iter()
                .map(|frame| {
                    json!({
                        "callback_from_key_ms": relative_ms(frame.callback_at, injection.at),
                        "paint_from_key_ms": frame.drawn.end.map(|at| relative_ms(at, injection.at)),
                        "code_view_cpu_ms": ms(frame.drawn.cpu),
                        "code_view_layout_ms": ms(frame.drawn.layout),
                        "text_version": frame.drawn.text_version,
                    })
                })
                .collect();
            output.push(json!({
                "phase": "replace",
                "sample": sample - WARMUP + 1,
                "inject_at_ms": ms(injection.at - started),
                "inject_unix_ms": ms(injection.wall_at.duration_since(UNIX_EPOCH)?),
                "inject_phase_ms": ms(injection.phase),
                "prepared_at_dispatch": injection.prepared,
                // Workbench can defer execution until after the key dispatch returns.
            "keystroke_dispatch_ms": ms(injection.returned_at - injection.at),
                "first_new_paint_ms": ms(end - injection.at),
                "max_callback_gap_ms": ms(max_callback),
                "max_paint_gap_ms": ms(max_paint),
                "text_version": frames[first_index].drawn.text_version,
                "timeline": timeline,
            }));
        }

        let expected_text = if cold && changed_template == ALTERNATE {
            &alternate
        } else {
            &expected
        };
        if state(window, cx)? != *expected_text {
            return Err(format!(
                "sample {sample}: result differed from exactly 5,000 replacements"
            )
            .into());
        }
        input_window.update(cx, |_, w, cx| {
            w.focus(&code.focus_handle(cx));
            press("ctrl-z", w, cx);
        })?;
        timer.timer(SEARCH_SETTLE).await;
        if state(window, cx)? != source {
            return Err(format!("sample {sample}: one undo did not restore the original").into());
        }
    }
    for metric in [
        "keystroke_dispatch_ms",
        "first_new_paint_ms",
        "max_callback_gap_ms",
        "max_paint_gap_ms",
    ] {
        let durations: Vec<_> = output
            .iter()
            .filter(|row| row["phase"] == "replace")
            .map(|row| Duration::from_secs_f64(row[metric].as_f64().expect("metric") / 1_000.0))
            .collect();
        let summary = typing_plan::summarize(&durations).expect("completed samples");
        output.push(json!({
            "phase": "summary",
            "metric": metric,
            "samples": summary.n,
            "p50_ms": summary.p50_ms,
            "p99_ms": summary.p99_ms,
            "max_ms": summary.max_ms,
            "above_two_periods": durations.iter().filter(|duration| **duration > interval * 2).count(),
        }));
    }
    Ok(output)
}

fn main() -> ProbeResult<()> {
    let source = replace_fixture::replace_source();
    assert_eq!(source.lines().count(), 100_000);
    assert_eq!(source.matches(SEARCH).count(), 5_000);
    let cold = match std::env::var("ROVE_REPLACE_PREPARATION").as_deref() {
        Ok("cold") => true,
        Ok("warm") | Err(std::env::VarError::NotPresent) => false,
        _ => return Err("ROVE_REPLACE_PREPARATION must be warm or cold".into()),
    };
    let project = tempfile::tempdir()?;
    let user = tempfile::tempdir()?;
    std::fs::write(project.path().join(FILE), &source)?;
    std::fs::write(
        user.path().join("settings.json"),
        serde_json::to_vec(&json!({
            "rove.update.mode": "none",
            "rove.index.enabled": false,
            "editor.find.seedSearchStringFromSelection": "never"
        }))?,
    )?;
    let mut settings = SettingsOptions::new(user.path(), user.path().join("backup"));
    settings.watch = None;
    let settings = Arc::new(SettingsService::open(
        settings,
        rove_ui::app_settings::registry()?,
    ));
    let root = project.path().to_path_buf();
    let started = Instant::now();
    let result: Rc<RefCell<Option<ProbeResult<Vec<Value>>>>> = Rc::default();
    let completed = result.clone();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(120));
        eprintln!("replace_all_probe: real window did not complete within 120s");
        std::process::exit(1);
    });
    Application::new().run(move |cx| {
        rove_ui::keys::init(cx);
        let _ = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(
            rove_ui::shell::icons::FONT,
        )]);
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let mut workbench = Workbench::with_settings(
                            root,
                            started,
                            LayoutStore::disabled(),
                            settings,
                            cx,
                        )
                        .expect("workbench");
                        Shell::attach_window(&mut workbench, window, cx);
                        workbench
                    })
                },
            )
            .expect("visible native window required");
        cx.activate(true);
        cx.spawn(async move |cx| {
            let output = run(window, &source, cold, started, cx).await;
            *completed.borrow_mut() = Some(output);
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    for row in result
        .borrow_mut()
        .take()
        .ok_or("probe exited without results")??
    {
        println!("REPLACE_ALL_PROBE {row}");
    }
    Ok(())
}
