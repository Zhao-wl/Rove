use super::*;
use crate::{documents::OpenOptions, shell::layout::LayoutStore, skeleton::Workbench};
use gpui::{AppContext, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use rove_base::{Cancellation, Resource, WorkspacePath};

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..10 {
        cx.run_until_parked();
        cx.update(|window, app| {
            window.refresh();
            app.refresh_windows();
        });
    }
    cx.run_until_parked();
}

#[gpui::test]
fn minimap_workbench_builds_themed_tiles_and_drags_through_code_view(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    let mut source = "class Program {\n    // MARK: - Startup\n    #region Helpers\n".to_owned();
    for row in 0..5000 {
        source.push_str(&format!("    int field_{row};\n"));
    }
    source.push_str("    #endregion\n}\n");
    std::fs::write(folder.path().join("Program.cs"), source).unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_store(
            folder.path().into(),
            std::time::Instant::now(),
            LayoutStore::disabled(),
            cx,
        )
    });
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new("Program.cs").unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());

    let (top, end) = code.update(cx, |code, _| {
        let panel = code.minimap.panel.expect("opened document has a minimap");
        let tiles = panel.track.tiles();
        assert!(tiles.end >= 2);
        assert!(tiles.clone().all(|tile| code.minimap.cache.contains(tile)));

        let foreground = code
            .merged_decorations(0)
            .unwrap()
            .runs
            .iter()
            .find_map(|run| {
                (run.range.start == 0)
                    .then_some(run.style.foreground)
                    .flatten()
            })
            .expect("class keyword has a theme color");
        let tile = code.minimap.cache.get(0).unwrap();
        let pixels = tile.as_bytes(0).unwrap();
        assert_eq!(
            &pixels[..4],
            &[foreground.2, foreground.1, foreground.0, foreground.3]
        );
        let headers = &code.minimap.headers[&0];
        assert!(headers
            .iter()
            .any(|header| &*header.title.label == "Startup" && header.title.separator));
        assert!(headers
            .iter()
            .any(|header| &*header.title.label == "Helpers"));
        assert!(code.minimap.overview.is_some());
        let x = panel.bounds.origin.x + panel.bounds.size.width / 2.0;
        let y = panel.bounds.origin.y;
        (
            point(x, y + px(3.0)),
            point(x, y + panel.bounds.size.height - px(1.0)),
        )
    });

    cx.simulate_mouse_down(top, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    settle(cx);
    code.read_with(cx, |code, _| {
        assert!(code.viewport().unwrap().first_line > 4900);
        assert!(code.scroll_position().top.is_finite());
    });

    code.update(cx, |code, cx| code.set_document_level(SizeClass::Large, cx));
    settle(cx);
    code.read_with(cx, |code, _| {
        assert!(code
            .disabled_features()
            .contains(&super::super::Feature::Minimap));
        assert!(code.minimap.panel.is_none());
        assert!(code.minimap.overview.is_some());
    });
}

#[test]
fn minimap_slider_uses_f64_even_at_fourteen_million_lines() {
    let metrics = Metrics {
        line_height: 20.0,
        viewport_height: 800.0,
        viewport_width: 800.0,
        line_count: 14_290_000,
        padding_top: 0.0,
        padding_bottom: 0.0,
        scroll_beyond_last_line: true,
        content_width: 800.0,
        beyond_last_column: 0.0,
        sticky_height: 0.0,
    };
    let zones = ViewZones::default();
    let last = metrics.max_top(&zones);
    let sampling = Sampling::new(minimap::Size::Fit, 1, 800.0, metrics.line_count);
    let track = Track::new(sampling, 800.0, last, last);
    let thumb = track.thumb(&metrics, &zones, last).unwrap();
    assert_eq!(
        scroll::pos_for_thumb(
            track.height,
            metrics.viewport_lines(),
            metrics.total_lines(&zones),
            thumb.start
        ),
        last
    );
    assert_eq!(
        ScrollPosition { top: last, x: 0.0 }
            .first_visible(&metrics, &zones)
            .0,
        14_289_999
    );
}
