//! Native Windows measurement of a large followed log while a separate process appends to it.
use super::*;
use std::{
    process::{Command, Stdio},
    sync::Mutex,
};

#[test]
#[ignore = "requires a native Windows window and a 200 MiB log fixture"]
fn r66_native_tail_replay() {
    let fixture =
        PathBuf::from(std::env::var_os("ROVE_TAIL_PROBE_FIXTURE").expect("fixture directory"));
    let temp = tempfile::tempdir_in(&fixture).expect("disposable fixture on the same drive");
    let source = fixture.join("Editor.scale.log");
    let source_bytes = source.metadata().unwrap().len();
    assert!(source_bytes >= 200 * 1024 * 1024);
    let path = temp.path().join("replay-rove.log");
    std::fs::copy(source, &path).expect("copy the immutable fixture");
    let writer = temp.path().join("append-log.py");
    std::fs::copy(fixture.join("append-log.py"), &writer).expect("copy the append driver");
    let root = temp.path().to_owned();
    let user = tempfile::tempdir().unwrap();
    let mut settings = rove_config::SettingsOptions::new(user.path(), user.path().join("backup"));
    settings.watch = None;
    let settings = Arc::new(rove_config::SettingsService::open(
        settings,
        crate::app_settings::registry().unwrap(),
    ));
    let result = Arc::new(Mutex::new(None));
    let report = result.clone();

    let (_temp, _user) = (temp, user);
    Application::new().run(move |cx| {
        cx.text_system().add_fonts(vec![Cow::Borrowed(crate::shell::icons::FONT)]).unwrap();
        crate::keys::init(cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1200.), px(840.)), cx))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let mut view = Workbench::with_settings(root.clone(), Instant::now(), LayoutStore::disabled(), settings.clone(), cx).unwrap();
                    Shell::attach_window(&mut view, window, cx);
                    view.open_documents().open(
                        Resource::Workspace(WorkspacePath::new("replay-rove.log").unwrap()),
                        OpenOptions::default(),
                        Cancellation::default(),
                    ).unwrap();
                    window.focus(&view.focus);
                    view
                })
            },
        ).unwrap();
        cx.spawn(async move |cx| {
            let executor = cx.background_executor().clone();
            let initial = Instant::now();
            let outcome: Result<Vec<(crate::code_view::FrameStats, bool)>, String> = async {
                loop {
                    let opened = window.update(cx, |view, _, _| view.active_large_file_document().is_some())
                        .map_err(|e| e.to_string())?;
                    if opened { break; }
                    if initial.elapsed() >= Duration::from_secs(30) {
                        return Err("initial large file did not open".to_owned());
                    }
                    executor.timer(Duration::from_millis(20)).await;
                }
                window.update(cx, |_, window, cx| {
                    let action = crate::keys::action_for(crate::follow_tail::FOLLOW).unwrap();
                    window.dispatch_action(action, cx);
                }).map_err(|e| e.to_string())?;
                let (base_version, base_bytes, base_lines, base_appends, base_reopens) = loop {
                    let value = window.update(cx, |view, _, cx| {
                        let Some(doc) = view.active_large_file_document() else { return (None, "no active large file".to_owned()) };
                        let lines = doc.text.lines();
                        let follows = view.shell.status_bar.get(crate::follow_tail::FOLLOW_STATUS).is_some();
                        let at_end = view.code_view().read(cx).viewport()
                            .is_some_and(|viewport| viewport.last_line + 1 >= lines.known);
                        let state = (lines.complete && follows && at_end).then(|| (
                            doc.text.version(), doc.text.byte_len(), lines.known,
                            view.follow_tail.counts.appends, view.follow_tail.counts.reopens,
                        ));
                        (state, format!("lines={lines:?}, follows={follows}, at_end={at_end}, reopens={}", view.follow_tail.counts.reopens))
                    }).map_err(|e| e.to_string())?;
                    if let (Some(value), _) = value { break value; }
                    if initial.elapsed() >= Duration::from_secs(30) {
                        return Err(format!("armed follow baseline did not finish indexing and displaying: {}", value.1));
                    }
                    executor.timer(Duration::from_millis(20)).await;
                };
                if base_bytes != source_bytes || base_lines < 1_000_000 {
                    return Err(format!("unexpected fixture baseline: {base_bytes} bytes, {base_lines} lines"));
                }
                window.update(cx, |view, _, cx| {
                    view.code_view().update(cx, |code, _| code.record_frames(true));
                }).map_err(|e| e.to_string())?;
                executor.timer(Duration::from_millis(350)).await;
                let started = Instant::now();
                let mut child = Command::new("python")
                    .arg(writer)
                    .arg(path)
                    .args(["--seconds", "10"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn().map_err(|e| e.to_string())?;
                let mut samples = Vec::new();
                let mut off_tail_frames = 0;
                let mut writer_updates = [0; 4];
                let mut observed_updates = 0;
                let mut previous_version = base_version.0;
                let until = started + Duration::from_secs(15);
                loop {
                    executor.timer(Duration::from_millis(40)).await;
                    let (frames, lines, bytes, follows, appended, reopened, at_end) = window
                        .update(cx, |view, _, cx| {
                            let document = view.active_large_file_document().unwrap();
                            let lines = document.text.lines().known;
                            let follows = view.shell.status_bar.get(crate::follow_tail::FOLLOW_STATUS)
                                .is_some_and(|entry| entry.text == "跟随末尾");
                            let code = view.code_view();
                            let at_end = code.read(cx).viewport().is_some_and(|vp| vp.last_line + 1 >= lines);
                            let frames = code.update(cx, |code, _| code.take_frame_samples());
                            (frames, lines, document.text.byte_len(), follows, view.follow_tail.counts.appends, view.follow_tail.counts.reopens, at_end)
                        }).map_err(|e| e.to_string())?;
                    let appended = appended - base_appends;
                    let reopened = reopened - base_reopens;
                    let writing = child.try_wait().map_err(|e| e.to_string())?.is_none();
                    for frame in frames {
                        if frame.text_version <= base_version.0 {
                            continue;
                        }
                        let new_snapshot = previous_version != frame.text_version;
                        samples.push((frame, new_snapshot));
                        off_tail_frames += usize::from(frame.last_line + 1 < frame.known_lines);
                        if new_snapshot {
                            observed_updates += 1;
                            previous_version = frame.text_version;
                            if writing {
                                let elapsed = frame.end.unwrap().saturating_duration_since(started);
                                let quarter = (elapsed.as_millis() / 2_500).min(3) as usize;
                                writer_updates[quarter] += 1;
                            }
                        }
                    }
                    if !writing && lines == base_lines + 10_000 && at_end && follows
                    {
                        let output = child.wait_with_output().map_err(|e| e.to_string())?;
                        if !output.status.success() || reopened != 0 || appended == 0 {
                            return Err(format!("writer/append counters: {:?}, appends={appended} reopens={reopened}", output.status));
                        }
                        println!("R66_NATIVE_WRITER {}", String::from_utf8_lossy(&output.stdout).trim());
                        println!("R66_NATIVE_STATE base_bytes={base_bytes} final_bytes={bytes} base_lines={base_lines} final_lines={lines} append_indexes={appended} full_reopens={reopened} observed_updates={observed_updates} writer_active_updates_by_quartile={writer_updates:?} off_tail_frames={off_tail_frames}");
                        if writer_updates.iter().any(|&updates| updates < 3) || off_tail_frames != 0 {
                            return Err("writing must update the painted tail throughout all four quarters".into());
                        }
                        break;
                    }
                    if Instant::now() >= until {
                        let _ = child.kill();
                        let output = child.wait_with_output().map_err(|e| e.to_string())?;
                        return Err(format!("tail did not settle: {lines} vs {}; follows={follows} at_end={at_end}; writer={}", base_lines + 10_000, String::from_utf8_lossy(&output.stderr)));
                    }
                }
                Ok(samples)
            }.await;
            *report.lock().unwrap() = Some(outcome);
            cx.update(|cx| cx.quit()).unwrap();
        }).detach();
    });
    let mut samples = result
        .lock()
        .unwrap()
        .take()
        .expect("the native probe completed")
        .unwrap();
    assert!(
        samples.len() >= 20,
        "insufficient measured frames: {}",
        samples.len()
    );
    samples.sort_by_key(|(frame, _)| frame.cpu);
    let ms = |duration: Duration| duration.as_secs_f64() * 1_000.0;
    let p50 = ms(samples[samples.len() / 2].0.cpu);
    let p99 = ms(samples[(samples.len() - 1) * 99 / 100].0.cpu);
    let max = ms(samples.last().unwrap().0.cpu);
    let over = samples
        .iter()
        .filter(|(frame, _)| ms(frame.cpu) >= 1.0)
        .count();
    let over_on_new_snapshot = samples
        .iter()
        .filter(|(frame, new_snapshot)| *new_snapshot && ms(frame.cpu) >= 1.0)
        .count();
    println!(
        "R66_NATIVE_FRAMES frames={} p50_ms={p50:.3} p99_ms={p99:.3} max_ms={max:.3} over_budget={over} over_budget_on_new_snapshot={over_on_new_snapshot}",
        samples.len()
    );
    for (frame, new_snapshot) in samples.iter().rev().take(5) {
        println!(
            "R66_NATIVE_SLOW_FRAME cpu_ms={:.3} layout_ms={:.3} gap_ms={:.3} paint_ms={:.3} glyph_ms={:.3} gutter_ms={:.3} guides_ms={:.3} minimap_ms={:.3} links_ms={:.3} glyphs={} visible_lines={} shaped={} placeholders={} missing={} new_snapshot={new_snapshot}",
            ms(frame.cpu),
            ms(frame.layout),
            ms(frame.paint_gap),
            ms(frame.paint),
            ms(frame.glyph_cpu),
            ms(frame.gutter_cpu),
            ms(frame.guides_cpu),
            ms(frame.minimap_cpu),
            ms(frame.links_cpu),
            frame.glyphs,
            frame.visible_lines,
            frame.shaped,
            frame.placeholders,
            frame.missing,
        );
    }
    assert!(max < 1.0, "continuous following frame CPU max {max:.3} ms");
}
