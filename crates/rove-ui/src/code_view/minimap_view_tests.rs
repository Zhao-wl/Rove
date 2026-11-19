use super::*;
use crate::{documents::OpenOptions, shell::layout::LayoutStore, skeleton::Workbench};
use gpui::{
    Focusable, Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext,
};
use rove_base::{Cancellation, Line, Position, Resource, Utf8Column, WorkspacePath};

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

    let (top, end, first, second) = code.update(cx, |code, _| {
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
        let first = tile.id;
        let second = code.minimap.cache.get(1).unwrap().id;
        let x = panel.bounds.origin.x + panel.bounds.size.width / 2.0;
        let y = panel.bounds.origin.y;
        (
            point(x, y + px(3.0)),
            point(x, y + panel.bounds.size.height - px(1.0)),
            first,
            second,
        )
    });

    code.update(cx, |code, cx| {
        let row = rove_base::Line(17);
        let changed = rove_base::Span::new(row, rove_base::Line(18)).unwrap();
        let snapshot = code.syntax_snapshot().unwrap();
        code.set_syntax(Some(snapshot), Some(&[changed]), cx);
    });
    settle(cx);
    code.update(cx, |code, _| {
        assert_ne!(code.minimap.cache.get(0).unwrap().id, first);
        assert_eq!(code.minimap.cache.get(1).unwrap().id, second);
    });
    code.update(cx, |code, cx| {
        let theme = Arc::new(rove_theme::DefaultTheme {
            colors: super::super::colors::theme_registry(),
            kind: rove_theme::ThemeKind::Light,
        });
        code.set_theme(theme, rove_theme::ThemeKind::Light, cx);
    });
    settle(cx);
    code.update(cx, |code, _| {
        assert_ne!(code.minimap.cache.get(1).unwrap().id, second);
    });

    let (before_semantic, after_theme_second, opacity) = code.update(cx, |code, cx| {
        use crate::code_view::semantic::SemanticStyles;
        use rove_lang_cs::query::SemanticToken;
        use rove_theme::semantic::SemanticStyle;

        let first = code.minimap.cache.get(0).unwrap().id;
        let second = code.minimap.cache.get(1).unwrap().id;
        code.set_semantic_enabled(true, cx);
        let token = SemanticToken {
            line: Line(0),
            column: Utf8Column(0),
            length_bytes: 5,
            token_type: "keyword".into(),
            modifiers: Arc::from([]),
        };
        code.set_semantic_styles(
            SemanticStyles::build(&[token], &|_, _| SemanticStyle {
                foreground: Some(Color(127, 63, 41, 255)),
                ..Default::default()
            }),
            cx,
        );
        code.set_inactive_regions(std::iter::once(0..1).collect(), cx);
        let opacity = code.semantic.borrow().unnecessary_opacity.unwrap();
        (first, second, opacity)
    });
    settle(cx);
    code.update(cx, |code, _| {
        let foreground_opacity = code
            .minimap_color("minimap.foregroundOpacity")
            .map_or(255, |color| color.3);
        let alpha = (u16::from(opacity) * u16::from(foreground_opacity) / 255) as u8;
        let tile = code.minimap.cache.get(0).unwrap();
        assert_ne!(tile.id, before_semantic);
        assert_eq!(&tile.as_bytes(0).unwrap()[..4], &[41, 63, 127, alpha]);
        assert_eq!(code.minimap.cache.get(1).unwrap().id, after_theme_second);
    });

    cx.simulate_mouse_down(top, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    settle(cx);
    code.read_with(cx, |code, _| {
        assert!(code.viewport().unwrap().first_line > 4900);
        assert!(code.scroll_position().top.is_finite());
    });

    code.update(cx, |code, cx| {
        code.set_scroll_position(ScrollPosition::default(), false, cx)
    });
    settle(cx);
    cx.simulate_event(ScrollWheelEvent {
        position: top,
        delta: ScrollDelta::Lines(point(0.0, -4.0)),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    settle(cx);
    let after_wheel = code.read_with(cx, |code, _| code.viewport().unwrap().first_line);
    assert!(after_wheel > 0);
    cx.simulate_mouse_down(end, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    settle(cx);
    assert!(code.read_with(cx, |code, _| code.viewport().unwrap().first_line) > after_wheel);

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

#[gpui::test]
fn minimap_marks_follow_editor_inputs_and_registered_theme_colors(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("notes.txt"),
        "heading\nneedle\nalpha\nneedle\nbeta\ntail\n",
    )
    .unwrap();
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
                Resource::Workspace(WorkspacePath::new("notes.txt").unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("needle");
    settle(cx);
    code.read_with(cx, |code, _| {
        assert_eq!(code.minimap.visual_marks[&MarkerKind::Find], [1..2, 3..4]);
        let color = code
            .minimap_color(MarkerKind::Find.style().minimap.unwrap())
            .unwrap();
        assert!(color.3 > 0);
    });

    cx.simulate_keystrokes("escape");
    settle(cx);
    code.update(cx, |code, cx| {
        code.set_selection(
            super::super::Selection::new(
                Position {
                    line: Line(1),
                    column: Utf8Column(0),
                },
                Position {
                    line: Line(1),
                    column: Utf8Column(6),
                },
            ),
            None,
            cx,
        );
        code.set_minimap_markers(
            "diagnostics",
            vec![Marker {
                lines: 4..5,
                kind: MarkerKind::Warning,
            }],
            cx,
        );
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    settle(cx);
    code.read_with(cx, |code, _| {
        assert!(!code.minimap.visual_marks.contains_key(&MarkerKind::Find));
        assert_eq!(code.minimap.visual_marks[&MarkerKind::Selection], [1..2]);
        assert!(code.minimap.visual_marks[&MarkerKind::Occurrence].contains(&(3..4)));
        assert_eq!(code.minimap.visual_marks[&MarkerKind::Warning], [4..5]);
        let overview = code.minimap.overview.as_ref().unwrap();
        for (kind, lane) in [
            (MarkerKind::Selection, 1),
            (MarkerKind::Occurrence, 1),
            (MarkerKind::Warning, 2),
        ] {
            let style = kind.style();
            assert!(code.minimap_color(style.minimap.unwrap()).unwrap().3 > 0);
            let ruler_color = code.minimap_color(style.ruler).unwrap();
            assert!(overview
                .bands
                .iter()
                .any(|(column, _, color)| *column == lane && *color == ruler_color));
        }
    });
}

#[gpui::test]
fn minimap_open_path_classifies_big_documents_and_keeps_the_ruler(cx: &mut TestAppContext) {
    use rove_config::{SettingsOptions, SettingsService};
    use std::io::Write;

    let folder = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    std::fs::write(
        profile.path().join("settings.json"),
        r#"{"rove.largeFile.threshold": 23068672}"#,
    )
    .unwrap();
    let mut options = SettingsOptions::new(profile.path(), profile.path().join("backup"));
    options.watch = None;
    let settings = Arc::new(SettingsService::open(
        options,
        crate::app_settings::registry().unwrap(),
    ));

    let line = format!("{}\n", "word ".repeat(45));
    let chunk = line.repeat(2048);
    for (name, target) in [
        ("Large.txt", 20 * 1024 * 1024),
        ("Huge.txt", 23 * 1024 * 1024),
    ] {
        let mut file = std::fs::File::create(folder.path().join(name)).unwrap();
        for _ in 0..(target as usize).div_ceil(chunk.len()) {
            file.write_all(chunk.as_bytes()).unwrap();
        }
    }

    let (view, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_settings(
            folder.path().into(),
            std::time::Instant::now(),
            LayoutStore::disabled(),
            settings,
            cx,
        )
        .unwrap()
    });
    for (name, size) in [
        ("Large.txt", SizeClass::Large),
        ("Huge.txt", SizeClass::Huge),
    ] {
        view.update(cx, |v, cx| {
            v.open_documents()
                .open(
                    Resource::Workspace(WorkspacePath::new(name).unwrap()),
                    OpenOptions::default(),
                    Cancellation::default(),
                )
                .unwrap();
            cx.notify();
        });
        settle(cx);
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.read_with(cx, |code, _| {
            assert_eq!(code.document_level(), size);
            assert!(code
                .disabled_features()
                .contains(&super::super::Feature::Minimap));
            assert!(code.minimap.panel.is_none());
            assert!(code.minimap.overview.is_some());
        });
    }
}

#[gpui::test]
fn minimap_toggle_writes_the_registered_setting(cx: &mut TestAppContext) {
    use rove_config::{Scope, SettingsOptions, SettingsService};
    let folder = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
    options.watch = None;
    let settings = Arc::new(SettingsService::open(
        options,
        crate::app_settings::registry().unwrap(),
    ));
    std::fs::write(folder.path().join("note.txt"), "first\nsecond\n").unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_settings(
            folder.path().into(),
            std::time::Instant::now(),
            LayoutStore::disabled(),
            settings.clone(),
            cx,
        )
        .unwrap()
    });
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new("note.txt").unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| {
        code.update(cx, |code, cx| {
            assert!(code.execute(minimap::TOGGLE, window, cx))
        });
    });
    settle(cx);
    assert!(!minimap::ENABLED
        .read(&*settings, &Scope::default())
        .unwrap());
    code.read_with(cx, |code, _| {
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
