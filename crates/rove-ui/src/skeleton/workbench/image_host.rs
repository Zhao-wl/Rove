//! The image editor shares the ordinary editor resolver, tab and workbench status bar.
#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
use super::*;
use crate::{
    image_preview::{
        decode::{self, DecodeError, Decoded},
        registry::{SIZE, ZOOM},
    },
    shell::status::StatusEntry,
};
use gpui::{
    canvas, AppContext, Canvas, Corners, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, RenderImage, ScrollWheelEvent, Task,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime},
};

const ZOOM_STOPS: &[f32] = &[
    0.1, 0.2, 0.3, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0, 7.0, 10.0, 15.0, 20.0,
];
const WATCH_DELAY: Duration = Duration::from_millis(750);

#[derive(Default)]
pub(super) struct ImagePanels {
    entries: BTreeMap<TabId, Preview>,
    checker: Option<(Hsla, Arc<RenderImage>)>,
}

struct Preview {
    path: PathBuf,
    workspace_path: Option<WorkspacePath>,
    visible: bool,
    cancellation: Option<Arc<AtomicBool>>,
    watch: Option<Task<()>>,
    animation: Option<Task<()>>,
    raster: Option<Task<()>>,
    content: Option<Result<Decoded, DecodeError>>,
    svg_frame: Option<Arc<RenderImage>>,
    raster_resolution: Option<(f32, f32)>,
    frame: usize,
    viewport: Option<Bounds<Pixels>>,
    zoom: Option<f32>,
    pan: Point<Pixels>,
    drag: Option<Drag>,
}

struct Drag {
    start: Point<Pixels>,
    last: Point<Pixels>,
    moved: bool,
}

impl Preview {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            workspace_path: None,
            visible: false,
            cancellation: None,
            watch: None,
            animation: None,
            raster: None,
            content: None,
            svg_frame: None,
            raster_resolution: None,
            frame: 0,
            viewport: None,
            zoom: None,
            pan: point(px(0.0), px(0.0)),
            drag: None,
        }
    }

    fn release(&mut self, window: &mut Window, cx: &mut Context<Workbench>) {
        if let Some(flag) = self.cancellation.take() {
            flag.store(true, Ordering::Relaxed);
        }
        self.watch.take();
        self.animation.take();
        self.raster.take();
        self.raster_resolution = None;
        self.viewport = None;
        self.drag = None;
        if let Some(Ok(image)) = self.content.take() {
            cx.drop_image(image.frames, Some(window));
        }
        if let Some(frame) = self.svg_frame.take() {
            cx.drop_image(frame, Some(window));
        }
    }

    fn scale(&self, viewport: Bounds<Pixels>, dimensions: (u32, u32)) -> f32 {
        self.zoom.unwrap_or_else(|| {
            (f32::from(viewport.size.width) / dimensions.0 as f32)
                .min(f32::from(viewport.size.height) / dimensions.1 as f32)
                .clamp(f32::MIN_POSITIVE, 1.0)
        })
    }

    fn zoom_to(&mut self, desired: f32, anchor: Point<Pixels>) {
        let (Some(viewport), Some(Ok(image))) = (self.viewport, &self.content) else {
            return;
        };
        let previous = self.scale(viewport, image.dimensions);
        let next = desired.clamp(ZOOM_STOPS[0], *ZOOM_STOPS.last().unwrap());
        let center = viewport.center() + self.pan;
        self.pan += (anchor - center) * (1.0 - next / previous);
        self.zoom = Some(next);
    }
}

fn stamp(path: &std::path::Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl Workbench {
    pub(super) fn sync_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut live = BTreeSet::new();
        let mut visible = BTreeMap::new();
        for (group, pane) in &self.editor.groups {
            for tab in &pane.tabs {
                if !crate::editors::settings::is_image_preview(&tab.kind) {
                    continue;
                }
                live.insert(tab.id);
                if pane.active == Some(tab.id)
                    && matches!(tab.content, crate::editors::Content::Ready(_))
                    && self.editor.maximized.is_none_or(|active| active == *group)
                {
                    let paths = match &tab.resource {
                        Resource::Workspace(path) => {
                            (self.root.join(path.as_str()), Some(path.clone()))
                        }
                        Resource::File(path) => (path.clone(), None),
                        Resource::Virtual(_) => continue,
                    };
                    visible.insert(tab.id, paths);
                }
            }
        }
        self.images.entries.retain(|id, entry| {
            if !visible.contains_key(id) && entry.visible {
                entry.release(window, cx);
                entry.visible = false;
            }
            live.contains(id)
        });
        for (id, (path, workspace_path)) in visible {
            let entry = self
                .images
                .entries
                .entry(id)
                .or_insert_with(|| Preview::new(path.clone()));
            if entry.visible && entry.path != path {
                entry.release(window, cx);
                entry.visible = false;
            }
            entry.workspace_path = workspace_path;
            let poll = entry.workspace_path.is_none() || self.workspace_events.is_none();
            if !poll {
                entry.watch.take();
            }
            let start_poll = poll && entry.watch.is_none();
            if !entry.visible {
                entry.path = path;
                entry.visible = true;
                self.load_image(id, cx);
            }
            if start_poll {
                self.watch_image(id, cx);
            }
            self.schedule_image_frame(id, cx);
            self.schedule_svg(id, window.scale_factor(), cx);
        }
        if !self.images.entries.is_empty() {
            let tint = self.ui_color("editor.foreground").opacity(0.10);
            if self
                .images
                .checker
                .as_ref()
                .is_none_or(|(color, _)| *color != tint)
            {
                if let Some((_, old)) = self.images.checker.replace((tint, checker_tile(tint))) {
                    cx.drop_image(old, Some(window));
                }
            }
        }
        self.sync_image_status();
    }

    fn load_image(&mut self, id: TabId, cx: &mut Context<Self>) {
        let entry = self.images.entries.get_mut(&id).unwrap();
        if let Some(flag) = entry.cancellation.take() {
            flag.store(true, Ordering::Relaxed);
        }
        let token = Arc::new(AtomicBool::new(false));
        entry.cancellation = Some(token.clone());
        let path = entry.path.clone();
        let worker_token = token.clone();
        let job = cx.background_spawn(async move { decode::decode(&path, &worker_token) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |view, cx| {
                let Some(entry) = view.images.entries.get_mut(&id) else {
                    return;
                };
                if !entry
                    .cancellation
                    .as_ref()
                    .is_some_and(|active| Arc::ptr_eq(active, &token))
                {
                    return;
                }
                entry.cancellation = None;
                entry.animation.take();
                entry.raster.take();
                entry.raster_resolution = None;
                if let Some(Ok(old)) = entry.content.replace(result) {
                    cx.drop_image(old.frames, None);
                }
                if let Some(old) = entry.svg_frame.take() {
                    cx.drop_image(old, None);
                }
                entry.frame = 0;
                cx.notify();
            });
        })
        .detach();
    }

    fn watch_image(&mut self, id: TabId, cx: &mut Context<Self>) {
        let entry = self.images.entries.get_mut(&id).unwrap();
        let path = entry.path.clone();
        let executor = cx.background_executor().clone();
        entry.watch = Some(cx.spawn(async move |this, cx| {
            let mut previous = executor
                .spawn({
                    let path = path.clone();
                    async move { stamp(&path) }
                })
                .await;
            loop {
                executor.timer(WATCH_DELAY).await;
                let next = executor
                    .spawn({
                        let path = path.clone();
                        async move { stamp(&path) }
                    })
                    .await;
                if next != previous {
                    previous = next;
                    if this.update(cx, |view, cx| view.load_image(id, cx)).is_err() {
                        break;
                    }
                }
            }
        }));
    }

    pub(super) fn refresh_workspace_images(
        &mut self,
        events: &[FileEvent],
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<_> = self
            .images
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                let watched = entry.workspace_path.as_ref()?;
                (entry.visible
                    && events.iter().any(|event| match event {
                        FileEvent::Created(path)
                        | FileEvent::Changed(path)
                        | FileEvent::Removed(path) => path == watched,
                        FileEvent::Renamed { from, to } => from == watched || to == watched,
                        FileEvent::RescanRequired => true,
                    }))
                .then_some(*id)
            })
            .collect();
        for id in ids {
            self.load_image(id, cx);
        }
    }

    fn schedule_image_frame(&mut self, id: TabId, cx: &mut Context<Self>) {
        let entry = self.images.entries.get_mut(&id).unwrap();
        let Some(Ok(image)) = &entry.content else {
            return;
        };
        let count = image.frames.frame_count();
        if count <= 1 || entry.animation.is_some() {
            return;
        }
        let (numerator, denominator) = image.frames.delay(entry.frame).numer_denom_ms();
        let ms = if numerator == 0 {
            100.0
        } else {
            f64::from(numerator) / f64::from(denominator)
        };
        let executor = cx.background_executor().clone();
        entry.animation = Some(cx.spawn(async move |this, cx| {
            executor
                .timer(Duration::from_secs_f64(ms.max(10.0) / 1000.0))
                .await;
            let _ = this.update(cx, |view, cx| {
                if let Some(entry) = view.images.entries.get_mut(&id) {
                    entry.animation.take();
                    entry.frame = (entry.frame + 1) % count;
                    cx.notify();
                }
            });
        }));
    }

    fn schedule_svg(&mut self, id: TabId, dpi: f32, cx: &mut Context<Self>) {
        let entry = self.images.entries.get_mut(&id).unwrap();
        let (Some(Ok(image)), Some(viewport)) = (&entry.content, entry.viewport) else {
            return;
        };
        let Some(tree) = &image.svg else { return };
        let display = entry.scale(viewport, image.dimensions);
        let scale = (display * dpi).max(0.05);
        let resolution = (scale, display);
        if entry.raster.is_some() || entry.raster_resolution == Some(resolution) {
            return;
        }
        entry.raster_resolution = Some(resolution);
        if (scale - 1.0).abs() < f32::EPSILON && entry.svg_frame.is_none() {
            return;
        }
        let tree = tree.clone();
        let job = cx.background_spawn({
            let tree = tree.clone();
            async move { decode::rasterize(&tree, scale) }
        });
        entry.raster = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |view, cx| {
                let Some(entry) = view.images.entries.get_mut(&id) else {
                    return;
                };
                if entry.raster_resolution != Some(resolution)
                    || !entry
                        .content
                        .as_ref()
                        .and_then(|image| image.as_ref().ok())
                        .and_then(|image| image.svg.as_ref())
                        .is_some_and(|current| Arc::ptr_eq(current, &tree))
                {
                    return;
                }
                entry.raster.take();
                if let Some(old) = result.ok().and_then(|frame| entry.svg_frame.replace(frame)) {
                    cx.drop_image(old, None);
                }
                cx.notify();
            });
        }));
    }

    fn sync_image_status(&mut self) {
        let state = self
            .editor
            .active()
            .active
            .and_then(|id| self.images.entries.get(&id))
            .and_then(|entry| {
                Some((
                    entry.content.as_ref()?.as_ref().ok()?.dimensions,
                    entry.zoom,
                ))
            });
        let Some(((w, h), zoom)) = state else {
            self.shell.status_bar.remove(SIZE);
            self.shell.status_bar.remove(ZOOM);
            return;
        };
        let zoom_label = zoom.map_or_else(
            || self.label("imagePreview.fit", &[]),
            |zoom| format!("{:.0}%", zoom * 100.0),
        );
        for (id, text) in [(SIZE, format!("{w}×{h}")), (ZOOM, zoom_label)] {
            self.shell
                .status_bar
                .set(
                    &self.shell.contributions,
                    id,
                    StatusEntry {
                        text,
                        ..Default::default()
                    },
                )
                .expect("registered image preview status");
        }
    }

    pub(super) fn toggle_image_fit(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .editor
            .active()
            .active
            .and_then(|id| self.images.entries.get_mut(&id))
        {
            entry.zoom = entry.zoom.is_none().then_some(1.0);
            entry.pan = point(px(0.0), px(0.0));
            cx.notify();
        }
    }

    pub(super) fn image_viewport(&self, id: TabId, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(entry) = self.images.entries.get(&id) else {
            return div().into_any_element();
        };
        let image = match &entry.content {
            None => {
                return self
                    .centered(self.label("common.loading", &[]))
                    .into_any_element()
            }
            Some(Err(error)) => return self.image_error(id, entry.path.clone(), *error, cx),
            Some(Ok(image)) => image,
        };
        let warning = (image.blocked != 0).then(|| {
            div()
                .flex_none()
                .px_2()
                .py_1()
                .bg(self.ui_color("editorWarning.foreground").opacity(0.14))
                .child(self.label(
                    "imagePreview.blocked",
                    &[("count", image.blocked.to_string())],
                ))
        });
        let background = self.ui_color("editor.background");
        let paint = ImagePaint {
            id,
            workbench: cx.weak_entity(),
            dimensions: image.dimensions,
            frames: entry
                .svg_frame
                .clone()
                .unwrap_or_else(|| image.frames.clone()),
            frame: entry.frame,
            zoom: entry.zoom,
            pan: entry.pan,
            checker: self
                .images
                .checker
                .as_ref()
                .expect("checker initialized")
                .1
                .clone(),
        }
        .canvas();
        let canvas = div()
            .id(("image-preview", id))
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, _| {
                    if let Some(entry) = view.images.entries.get_mut(&id) {
                        entry.drag = Some(Drag {
                            start: event.position,
                            last: event.position,
                            moved: false,
                        });
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                let Some(entry) = view.images.entries.get_mut(&id) else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    entry.drag = None;
                    return;
                }
                let Some(drag) = &mut entry.drag else { return };
                let delta = event.position - drag.start;
                drag.moved |= f32::from(delta.x).abs() >= 3.0 || f32::from(delta.y).abs() >= 3.0;
                if drag.moved {
                    entry.pan += event.position - drag.last;
                    drag.last = event.position;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseUpEvent, _, cx| {
                    let Some(entry) = view.images.entries.get_mut(&id) else {
                        return;
                    };
                    let (Some(drag), Some(viewport), Some(Ok(image))) =
                        (entry.drag.take(), entry.viewport, &entry.content)
                    else {
                        return;
                    };
                    if drag.moved {
                        return;
                    }
                    let current = entry.scale(viewport, image.dimensions);
                    let desired = if event.modifiers.alt {
                        ZOOM_STOPS
                            .iter()
                            .copied()
                            .rev()
                            .find(|scale| *scale < current - 0.001)
                    } else {
                        ZOOM_STOPS
                            .iter()
                            .copied()
                            .find(|scale| *scale > current + 0.001)
                    };
                    if let Some(desired) = desired {
                        entry.zoom_to(desired, event.position);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |view, _: &MouseUpEvent, _, _| {
                    if let Some(entry) = view.images.entries.get_mut(&id) {
                        entry.drag = None;
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(move |view, event: &ScrollWheelEvent, _, cx| {
                if !event.modifiers.control {
                    return;
                }
                let Some(entry) = view.images.entries.get_mut(&id) else {
                    return;
                };
                let (Some(viewport), Some(Ok(image))) = (entry.viewport, &entry.content) else {
                    return;
                };
                let current = entry.scale(viewport, image.dimensions);
                let dy = f32::from(event.delta.pixel_delta(px(25.0)).y);
                if dy != 0.0 {
                    entry.zoom_to(current * (dy * 0.006).exp(), event.position);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(paint);
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .children(warning)
            .child(canvas)
            .into_any_element()
    }

    fn image_error(
        &self,
        id: TabId,
        path: PathBuf,
        error: DecodeError,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = match error {
            DecodeError::Invalid => "imagePreview.invalid",
            DecodeError::Rendering => "imagePreview.rendering",
            DecodeError::TooLarge => "imagePreview.tooLarge",
            _ => "imagePreview.unavailable",
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(self.label(key, &[]))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id(("retry-image", id))
                            .cursor_pointer()
                            .px_2()
                            .py_1()
                            .bg(self.ui_color("tab.hoverBackground"))
                            .child(self.label("editors.retry", &[]))
                            .on_click(cx.listener(move |view, _, _, cx| view.load_image(id, cx))),
                    )
                    .child(
                        div()
                            .id(("reveal-image", id))
                            .cursor_pointer()
                            .px_2()
                            .py_1()
                            .bg(self.ui_color("tab.hoverBackground"))
                            .child(self.label("editors.reveal", &[]))
                            .on_click(move |_, _, cx| cx.reveal_path(&path)),
                    ),
            )
            .into_any_element()
    }
}

struct ImagePaint {
    id: TabId,
    workbench: gpui::WeakEntity<Workbench>,
    dimensions: (u32, u32),
    frames: Arc<RenderImage>,
    frame: usize,
    zoom: Option<f32>,
    pan: Point<Pixels>,
    checker: Arc<RenderImage>,
}

impl ImagePaint {
    fn canvas(self) -> Canvas<()> {
        let Self {
            id,
            workbench,
            dimensions,
            frames,
            frame,
            zoom,
            pan,
            checker,
        } = self;
        let render_target = workbench.clone();
        canvas(
            move |bounds, window, cx| {
                let changed = workbench.upgrade().is_some_and(|view| {
                    view.read(cx)
                        .images
                        .entries
                        .get(&id)
                        .is_some_and(|entry| entry.viewport != Some(bounds))
                });
                if changed {
                    window.defer(cx, move |_, cx| {
                        let _ = workbench.update(cx, |view, cx| {
                            if let Some(entry) = view.images.entries.get_mut(&id) {
                                entry.viewport = Some(bounds);
                                cx.notify();
                            }
                        });
                    });
                }
            },
            move |bounds, (), window, cx| {
                if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
                    return;
                }
                let (w, h) = (dimensions.0 as f32, dimensions.1 as f32);
                let scale = zoom.unwrap_or_else(|| {
                    (f32::from(bounds.size.width) / w)
                        .min(f32::from(bounds.size.height) / h)
                        .min(1.0)
                });
                let size = size(px(w * scale), px(h * scale));
                let origin = bounds.center() + pan - point(size.width / 2.0, size.height / 2.0);
                let image_bounds = Bounds { origin, size };
                let result =
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        checkerboard(bounds, image_bounds, checker, window);
                        window.paint_image(
                            image_bounds,
                            Corners::default(),
                            frames.clone(),
                            frame,
                            false,
                        )
                    });
                if result.is_err() {
                    window.defer(cx, move |window, cx| {
                        let _ = render_target.update(cx, |view, cx| {
                            let Some(entry) = view.images.entries.get_mut(&id) else {
                                return;
                            };
                            let Some(Ok(image)) = &entry.content else {
                                return;
                            };
                            if !Arc::ptr_eq(
                                entry.svg_frame.as_ref().unwrap_or(&image.frames),
                                &frames,
                            ) {
                                return;
                            }
                            entry.animation.take();
                            entry.raster.take();
                            let old = entry.content.replace(Err(DecodeError::Rendering));
                            if let Some(Ok(old)) = old {
                                cx.drop_image(old.frames, Some(window));
                            }
                            if let Some(old) = entry.svg_frame.take() {
                                cx.drop_image(old, Some(window));
                            }
                            cx.notify();
                        });
                    });
                }
            },
        )
        .absolute()
        .inset_0()
    }
}

const CHECKER_CELL: u32 = 12;
const CHECKER_SIDE: u32 = 16 * CHECKER_CELL;

fn checker_tile(tint: Hsla) -> Arc<RenderImage> {
    let tint = gpui::Rgba::from(tint);
    let channel = |value: f32| (value * 255.0).round() as u8;
    let tinted = image::Rgba([
        channel(tint.b),
        channel(tint.g),
        channel(tint.r),
        channel(tint.a),
    ]);
    let rgba = image::RgbaImage::from_fn(CHECKER_SIDE, CHECKER_SIDE, |x, y| {
        if ((x / CHECKER_CELL) + (y / CHECKER_CELL)).is_multiple_of(2) {
            tinted
        } else {
            image::Rgba([0; 4])
        }
    });
    Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
        rgba
    )]))
}

fn checkerboard(
    viewport: Bounds<Pixels>,
    image: Bounds<Pixels>,
    tile: Arc<RenderImage>,
    window: &mut Window,
) {
    const TILE: f32 = CHECKER_SIDE as f32;
    let coordinate = |a, b| (f32::from(a - b) / TILE).floor().max(0.0) as i32;
    let left = coordinate(viewport.left(), image.left());
    let top = coordinate(viewport.top(), image.top());
    let right = coordinate(viewport.right().min(image.right()), image.left());
    let bottom = coordinate(viewport.bottom().min(image.bottom()), image.top());
    if !viewport.intersects(&image) {
        return;
    }
    window.with_content_mask(Some(gpui::ContentMask { bounds: image }), |window| {
        for row in top..=bottom {
            for col in left..=right {
                let origin = image.origin + point(px(col as f32 * TILE), px(row as f32 * TILE));
                let bounds = Bounds {
                    origin,
                    size: size(px(TILE), px(TILE)),
                };
                let _ = window.paint_image(bounds, Corners::default(), tile.clone(), 0, false);
            }
        }
    });
}
