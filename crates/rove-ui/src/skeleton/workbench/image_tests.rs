use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba, RgbaImage};
use rove_config::{SettingsOptions, SettingsService};

fn write_png(path: &std::path::Path, dimensions: (u32, u32)) {
    RgbaImage::from_pixel(dimensions.0, dimensions.1, Rgba([243, 32, 90, 120]))
        .save(path)
        .unwrap();
}

fn open_workbench<'a>(
    cx: &'a mut TestAppContext,
    root: PathBuf,
    config: &str,
) -> (
    Entity<Workbench>,
    &'a mut VisualTestContext,
    tempfile::TempDir,
) {
    let user = tempfile::tempdir().unwrap();
    fs::write(user.path().join("settings.json"), config).unwrap();
    let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
    options.watch = None;
    let settings = Arc::new(SettingsService::open(
        options,
        crate::app_settings::registry().unwrap(),
    ));
    let (view, cx) = cx.add_window_view(|_, cx| {
        let editor = EditorSettings::read(&*settings).unwrap();
        Workbench::build(
            root,
            Instant::now(),
            LayoutStore::disabled(),
            settings,
            editor,
            cx,
        )
    });
    cx.run_until_parked();
    (view, cx, user)
}

fn drive_until(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    predicate: impl Fn(&Workbench) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        if view.read_with(cx, |v, _| predicate(v)) {
            return;
        }
        assert!(Instant::now() < deadline, "image preview did not settle");
        std::thread::yield_now();
    }
}

fn open_preview(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    path: &str,
    options: OpenOptions,
) {
    view.update(cx, |view, cx| {
        view.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                options,
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
}

fn active_image(view: &Workbench) -> Option<&Preview> {
    view.images.entries.get(&view.editor.active().active?)
}

#[gpui::test]
fn image_preview_opens_from_explorer_and_uses_status_zoom_and_live_reload(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("A.png");
    write_png(&path, (30, 20));
    let (view, cx, _user) = open_workbench(cx, root.path().to_owned(), "{}");
    cx.simulate_click(point(px(128.0), px(104.0)), Modifiers::default());
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|image| {
            image.viewport.is_some() && image.content.as_ref().is_some_and(Result::is_ok)
        })
    });
    let center = view.read_with(cx, |v, _| {
        assert!(v.tab_read_only(v.editor.active().current().unwrap()));
        assert_eq!(v.shell.status_bar.get(SIZE).unwrap().text, "30×20");
        assert_eq!(
            v.shell.status_bar.get(ZOOM).unwrap().text,
            v.label("imagePreview.fit", &[])
        );
        let preview = active_image(v).unwrap();
        let viewport = preview.viewport.unwrap();
        assert_eq!(preview.scale(viewport, (30, 20)), 1.0);
        viewport.center()
    });
    cx.simulate_click(center, Modifiers::default());
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|p| p.zoom == Some(1.5))
    });
    cx.simulate_click(
        center,
        Modifiers {
            alt: true,
            ..Default::default()
        },
    );
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|p| p.zoom == Some(1.0))
    });
    view.update(cx, |v, cx| {
        v.command(crate::image_preview::registry::TOGGLE_ZOOM, cx)
    });
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|p| p.zoom.is_none())
    });
    view.update(cx, |v, cx| {
        v.command(crate::image_preview::registry::TOGGLE_ZOOM, cx)
    });
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|p| p.zoom == Some(1.0))
    });
    let moved = center + point(px(12.0), px(8.0));
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: center,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseMoveEvent {
        position: moved,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::default(),
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: moved,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|p| p.pan == moved - center)
    });
    write_png(&path, (50, 40));
    view.update(cx, |v, cx| {
        v.refresh_workspace_images(
            &[FileEvent::Changed(WorkspacePath::new("A.png").unwrap())],
            cx,
        );
    });
    drive_until(&view, cx, |v| {
        v.shell
            .status_bar
            .get(SIZE)
            .is_some_and(|s| s.text == "50×40")
    });
    view.read_with(cx, |v, _| {
        assert_eq!(active_image(v).unwrap().zoom, Some(1.0));
        assert_eq!(active_image(v).unwrap().pan, moved - center);
        assert_eq!(v.shell.status_bar.get(ZOOM).unwrap().text, "100%");
    });
}

#[gpui::test]
fn image_preview_respects_explicit_text_associations_and_search_handoffs(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    write_png(&root.path().join("A.png"), (20, 20));
    write_png(&root.path().join("B.jpeg.png"), (21, 20));
    let (view, cx, _user) = open_workbench(
        cx,
        root.path().to_owned(),
        r#"{ "workbench.editorAssociations": { "A.png": "default" } }"#,
    );
    open_preview(&view, cx, "A.png", OpenOptions::default());
    drive_until(&view, cx, |v| {
        v.editor
            .active()
            .current()
            .is_some_and(|t| t.kind == EditorKind::Text)
    });
    view.update(cx, |v, cx| {
        v.open_search_result(
            &crate::search::view::OpenRequest {
                path: WorkspacePath::new("B.jpeg.png").unwrap(),
                selection: rove_base::TextRange::new(Default::default(), Default::default())
                    .unwrap(),
                side: false,
                preserve_focus: false,
                pinned: true,
            },
            cx,
        )
    });
    drive_until(&view, cx, |v| {
        v.shell
            .status_bar
            .get(SIZE)
            .is_some_and(|s| s.text == "21×20")
    });
}

fn write_gif(path: &std::path::Path) {
    let mut encoder = GifEncoder::new(fs::File::create(path).unwrap());
    encoder
        .encode_frames(
            [[255, 0, 0, 255], [0, 0, 255, 255]]
                .into_iter()
                .map(|color| {
                    Frame::from_parts(
                        RgbaImage::from_pixel(4, 3, Rgba(color)),
                        0,
                        0,
                        Delay::from_numer_denom_ms(250, 1),
                    )
                }),
        )
        .unwrap();
}

#[gpui::test]
fn image_preview_gif_timer_stops_when_the_tab_is_hidden(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    write_gif(&root.path().join("A.gif"));
    fs::write(root.path().join("B.txt"), "example").unwrap();
    let executor = cx.background_executor.clone();
    let (view, cx, _user) = open_workbench(cx, root.path().to_owned(), "{}");
    open_preview(
        &view,
        cx,
        "A.gif",
        OpenOptions {
            preview_tab: false,
            ..Default::default()
        },
    );
    drive_until(&view, cx, |v| {
        active_image(v).is_some_and(|p| p.animation.is_some())
    });
    let id = view.read_with(cx, |v, _| v.editor.active().active.unwrap());
    assert_eq!(view.read_with(cx, |v, _| v.images.entries[&id].frame), 0);
    executor.advance_clock(Duration::from_millis(250));
    drive_until(&view, cx, |v| v.images.entries[&id].frame == 1);
    open_preview(&view, cx, "B.txt", OpenOptions::default());
    drive_until(&view, cx, |v| {
        v.editor.active().tab(id).is_some()
            && v.images
                .entries
                .get(&id)
                .is_some_and(|entry| !entry.visible)
    });
    executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let hidden = &v.images.entries[&id];
        assert_eq!(hidden.frame, 1);
        assert!(hidden.animation.is_none());
        assert!(hidden.content.is_none());
        assert!(v.shell.status_bar.get(ZOOM).is_none());
    });
}

#[test]
fn image_preview_decodes_five_formats_and_rejects_corruption_and_size() {
    let root = tempfile::tempdir().unwrap();
    let path = |name| root.path().join(name);
    write_png(&path("a.png"), (4, 3));
    image::RgbImage::from_pixel(4, 3, image::Rgb([9, 8, 7]))
        .save(path("a.jpg"))
        .unwrap();
    RgbaImage::from_pixel(4, 3, Rgba([6, 5, 4, 3]))
        .save(path("a.tga"))
        .unwrap();
    write_gif(&path("a.gif"));
    fs::write(path("a.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"><rect width="2" height="2" fill="red"/></svg>"#).unwrap();
    let token = AtomicBool::new(false);
    for name in ["a.png", "a.jpg", "a.tga", "a.gif", "a.svg"] {
        let image = decode::decode(&path(name), &token).unwrap();
        assert_eq!(image.dimensions, (4, 3), "{name}");
        assert_eq!(
            image.frames.frame_count(),
            if name.ends_with(".gif") { 2 } else { 1 }
        );
    }
    fs::copy(path("a.png"), path("wrong.jpg")).unwrap();
    fs::write(path("broken.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    fs::write(path("huge.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" width="17000" height="17000"><rect width="1" height="1"/></svg>"#).unwrap();
    for name in ["wrong.jpg", "broken.png"] {
        assert!(matches!(
            decode::decode(&path(name), &token),
            Err(DecodeError::Invalid)
        ));
    }
    assert!(matches!(
        decode::decode(&path("huge.svg"), &token),
        Err(DecodeError::TooLarge)
    ));
}

#[test]
fn image_preview_svg_blocks_external_images_and_fonts_and_rasterizes_at_four_times() {
    use std::net::TcpListener;
    let socket = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    socket.set_nonblocking(true).unwrap();
    let endpoint = socket.local_addr().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("remote.svg");
    fs::write(
        &path,
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
        <style>@font-face {{ font-family: blocked; src: url(http://{endpoint}/font); }}</style>
        <image href="http://{endpoint}/asset.png" x="5" width="2" height="2"/>
        <script>alert(1)</script><rect width="4" height="4" fill="red"/>
        </svg>"#
        ),
    )
    .unwrap();
    let decoded = decode::decode(&path, &AtomicBool::new(false)).unwrap();
    assert_eq!(decoded.blocked, 2);
    assert_eq!(&decoded.frames.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
    let four = decode::rasterize(&decoded.svg.unwrap(), 4.0).unwrap();
    assert_eq!(four.size(0).width.0, 40);
    assert_eq!(four.size(0).height.0, 40);
    assert_eq!(
        socket.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[gpui::test]
fn image_preview_opens_real_unity_assets(cx: &mut TestAppContext) {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let project =
        rove_test_support::require_unity_project!(rove_test_support::UnityProject::Medium);
    let mut pending = vec![project.join("Assets")];
    let mut candidates = Vec::new();
    while let Some(folder) = pending.pop() {
        let Ok(children) = fs::read_dir(folder) else {
            continue;
        };
        for entry in children.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ["png", "jpg", "jpeg", "gif", "svg", "tga"]
                            .iter()
                            .any(|candidate| candidate.eq_ignore_ascii_case(ext))
                    })
            {
                let mut hash = DefaultHasher::new();
                path.strip_prefix(&project).unwrap().hash(&mut hash);
                candidates.push((hash.finish(), path));
            }
        }
    }
    if candidates.len() < 20 {
        eprintln!("已跳过：中型 Unity 项目中的受支持图片不足 20 张");
        return;
    }
    candidates.sort_unstable_by_key(|(key, _)| *key);
    let png_2048 = candidates
        .iter()
        .map(|(_, path)| path)
        .find(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
                && image::ImageReader::open(path)
                    .and_then(|r| r.with_guessed_format())
                    .is_ok_and(|r| {
                        r.into_dimensions()
                            .is_ok_and(|dimensions| dimensions == (2048, 2048))
                    })
        })
        .cloned();
    let empty = tempfile::tempdir().unwrap();
    let (view, cx, _user) = open_workbench(cx, empty.path().to_owned(), "{}");
    let mut elapsed = Vec::new();
    for (path, selected_png) in candidates
        .into_iter()
        .take(20)
        .map(|(_, path)| (path, false))
        .chain(png_2048.into_iter().map(|path| (path, true)))
    {
        let start = Instant::now();
        view.update(cx, |v, cx| {
            v.open_documents()
                .open(
                    Resource::File(path.clone()),
                    OpenOptions {
                        source: OpenSource::Explorer,
                        preview_tab: true,
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            cx.notify();
        });
        drive_until(&view, cx, |v| {
            active_image(v).is_some_and(|p| {
                p.path == path
                    && p.viewport.is_some()
                    && p.content.as_ref().is_some_and(Result::is_ok)
            })
        });
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        if selected_png {
            eprintln!("2048×2048 PNG 打开至测试窗口绘制：{ms:.2} ms");
        } else {
            elapsed.push(ms);
        }
    }
    eprintln!(
        "Unity 随机样本：{} 张均绘制，测试窗口绘制耗时最大 {:.2} ms",
        elapsed.len(),
        elapsed.into_iter().fold(0.0_f64, f64::max)
    );
}
