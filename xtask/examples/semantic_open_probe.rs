//! #59: measure native Workbench paint of a 5,000-line C# document with a ready index.
use gpui::{
    prelude::*, px, size, App, Application, Bounds, Entity, Window, WindowBounds, WindowOptions,
};
use rove_base::{Cancellation, Resource, WorkspacePath};
use rove_lang_cs::{
    router::{
        hash_bytes,
        testing::{TestIndex, KIND_FIELD},
        ManualSemantic,
    },
    CsIndex,
};
use rove_theme::Color;
use rove_ui::{
    code_view::CodeView,
    documents::OpenOptions,
    shell::{layout::LayoutStore, Shell},
    skeleton::Workbench,
};
use serde_json::json;
use std::{
    cell::Cell,
    error::Error,
    fmt::Write,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const PATH: &str = "Bench.cs";
const LINES: usize = 5_000;
const PREFIX: &str = "    public const int ";
const MARKER_ROW: u64 = 2;
// Dark 2026 selects this foreground for variable.readonly and a different
// foreground for the syntax-only identifier.
const MARKER_COLOR: Color = Color(121, 192, 255, 255);

fn indexed_document() -> (String, TestIndex) {
    let mut source = String::from("static class Bench\n{\n");
    let mut index = TestIndex::default();
    for n in 0..LINES - 3 {
        writeln!(source, "{PREFIX}Field{n:04} = {n};").expect("C# fixture");
    }
    source.push('}');
    assert_eq!(source.lines().count(), LINES);
    let file = index.file(PATH, hash_bytes(source.as_bytes()));
    for n in 0..LINES - 3 {
        let name = format!("Field{n:04}");
        let symbol = index.symbol(&format!("Bench|F:Bench.{name}"), &name, KIND_FIELD, None);
        index.def(symbol, file, (n + 2) as u32, PREFIX.len() as u32);
    }
    (source, index)
}

#[derive(Default)]
struct Observation {
    first: Cell<Option<Instant>>,
    marked: Cell<Option<(u64, Instant)>>,
    result: Cell<Option<Duration>>,
}

fn on_frame(code: Entity<CodeView>, seen: Rc<Observation>, window: &mut Window, cx: &mut App) {
    if seen.first.get().is_none() {
        let first = code.update(cx, |view, _| {
            let first = view
                .take_frame_samples()
                .into_iter()
                .filter(|frame| frame.glyphs > 0 && frame.text_version != 0)
                .find_map(|frame| frame.end);
            if first.is_some() {
                view.record_frames(false);
            }
            first
        });
        seen.first.set(first);
    }
    let view = code.read(cx);
    let frame = view.last_frame();
    if let Some(end) = frame.end.filter(|_| {
        frame.glyphs > 0
            && frame.shaped == frame.visible_lines
            && frame.placeholders == 0
            && frame.missing == 0
            && view.is_editable()
            && frame.text_version == view.version().0
            && view
                .text()
                .is_some_and(|text| text.lines().known == LINES as u64)
            && view.viewport().is_some_and(|viewport| {
                (viewport.first_line..=viewport.last_line).contains(&MARKER_ROW)
            })
    }) {
        let colored = view.merged_decorations(MARKER_ROW).is_some_and(|row| {
            row.runs.iter().any(|run| {
                run.range.contains(&(PREFIX.len() as u32))
                    && run.style.foreground == Some(MARKER_COLOR)
            })
        });
        // The cache can advance after the previous paint. Require a newer paint
        // after observing the color at a frame boundary to bound that ambiguity.
        let prior_paint = seen
            .marked
            .get()
            .is_some_and(|(version, previous)| version == frame.text_version && end > previous);
        if let Some(first) = seen.first.get().filter(|_| colored && prior_paint) {
            seen.result.set(Some(end - first));
            cx.quit();
            return;
        }
        seen.marked
            .set(colored.then_some((frame.text_version, end)));
    }
    code.update(cx, |_, cx| cx.notify());
    window.on_next_frame(move |window, cx| on_frame(code, seen, window, cx));
}

fn main() -> Result<(), Box<dyn Error>> {
    let project = tempfile::tempdir()?;
    let index_output = tempfile::tempdir()?;
    let (source, index) = indexed_document();
    std::fs::write(project.path().join(PATH), source)?;
    let index_path = index_output.path().join("bench.idx");
    index.write(&index_path)?;
    let index = Arc::new(CsIndex::open(&index_path)?);
    let root = project.path().to_path_buf();
    let result = Rc::new(Observation::default());
    let observed = result.clone();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(10));
        eprintln!("Timed out waiting for a semantic paint in the native Workbench");
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
                        // Use the isolated assembly entry so user caches, hot-exit
                        // backups and update services are outside the probe.
                        let mut workbench = Workbench::with_store(
                            root,
                            Instant::now(),
                            LayoutStore::disabled(),
                            cx,
                        );
                        Shell::attach_window(&mut workbench, window, cx);
                        workbench.attach_semantic(Some(Arc::new(ManualSemantic::ready(index))), cx);
                        workbench
                    })
                },
            )
            .expect("visible native window required");
        cx.activate(true);
        window
            .update(cx, |workbench, window, cx| {
                let code = workbench.code_view().clone();
                code.update(cx, |view, _| view.record_frames(true));
                workbench
                    .open_documents()
                    .open(
                        Resource::Workspace(WorkspacePath::new(PATH).expect("fixture filename")),
                        OpenOptions::default(),
                        Cancellation::default(),
                    )
                    .expect("open fixture");
                window.on_next_frame(move |window, cx| on_frame(code, observed, window, cx));
                cx.notify();
            })
            .expect("open fixture in the Workbench");
    });
    let elapsed = result
        .result
        .get()
        .ok_or("window closed before the semantic paint")?;
    let ms = elapsed.as_secs_f64() * 1_000.0;
    println!(
        "{}",
        json!({ "lines": LINES, "catch_up_upper_bound_ms": ms })
    );
    if ms >= 100.0 {
        return Err("the measured upper bound did not meet the 100 ms catch-up budget".into());
    }
    Ok(())
}
