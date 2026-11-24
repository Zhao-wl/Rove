use super::Workbench;
use crate::file_icons::{FileIcons, FontCache, PreparedIcons};
use gpui::{App, Context, Global, Window, WindowAppearance};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rove_base::Value;
use rove_config::{Scope, Settings};
use rove_theme::extensions::{ExtensionScanner, ExtensionThemes};
use rove_theme::file_icons;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Themes(Option<Arc<ExtensionThemes>>);

impl Global for Themes {}

#[derive(Default)]
pub(super) struct AppIcons(pub(super) FileIcons, FontCache);

impl Global for AppIcons {}

struct StartAfterPaint(async_channel::Sender<()>);

impl Global for StartAfterPaint {}

pub(super) fn after_first_frame(cx: &App) {
    if let Some(start) = cx.try_global::<StartAfterPaint>() {
        let _ = start.0.try_send(());
    }
}

pub(super) const ICON_THEME: &str = "workbench.iconTheme";

pub(super) fn start(override_root: Option<PathBuf>, settings: Arc<dyn Settings>, cx: &mut App) {
    cx.set_global(Themes::default());
    cx.set_global(AppIcons::default());
    let (first_frame, ready) = async_channel::bounded(1);
    cx.set_global(StartAfterPaint(first_frame));
    let root = override_root
        .or_else(|| std::env::var_os("ROVE_VSCODE_EXTENSIONS_DIR").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .map(|home| home.join(".vscode").join("extensions"))
        });
    let root = root.and_then(|root| std::path::absolute(root).ok());
    let (signal, changed) = async_channel::bounded(1);
    let refresh = Arc::new(AtomicBool::new(true));
    let refresh_watcher = refresh.clone();
    let settings_signal = signal.clone();
    let subscription = settings.changes().subscribe(Arc::new(move |change| {
        if change.keys.is_empty() || change.keys.iter().any(|key| key.as_ref() == ICON_THEME) {
            let _ = settings_signal.try_send(());
        }
    }));
    let background = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        if ready.recv().await.is_err() {
            return;
        }
        drop(ready);
        let watch_root = root.clone();
        let _watcher = background
            .spawn(async move { start_watcher(watch_root.as_deref()?, signal, refresh_watcher) })
            .await;
        let _settings_subscription = subscription;
        let mut scan = Arc::new(ExtensionThemes::default());
        let mut scanner = ExtensionScanner::default();
        loop {
            if refresh.swap(false, Ordering::AcqRel) {
                if let Some(root) = &root {
                    let path = root.clone();
                    (scanner, scan) = background
                        .spawn(async move {
                            let start = Instant::now();
                            let scan = scanner.scan(&path);
                            for diagnostic in &scan.diagnostics {
                                eprintln!("rove-theme: {diagnostic}");
                            }
                            eprintln!(
                                "rove-theme: extensions scanned in {} ms",
                                start.elapsed().as_millis()
                            );
                            (scanner, Arc::new(scan))
                        })
                        .await;
                }
                if cx
                    .update_global(|state: &mut Themes, _| state.0 = Some(scan.clone()))
                    .is_err()
                {
                    break;
                }
            }
            let key = icon_theme_setting(&*settings);
            let chosen = key
                .as_deref()
                .and_then(|id| scan.icons.iter().find(|icon| icon.id.as_ref() == id))
                .cloned();
            let prepared = background
                .spawn(async move {
                    let id = key?;
                    let parsed = if id.as_ref() == "vs-seti" {
                        file_icons::seti()
                    } else {
                        chosen
                            .as_ref()
                            .map_or_else(|| Err(rove_base::Error::NotFound), file_icons::load)
                            .or_else(|error| {
                                eprintln!("rove-theme: file icon theme {id:?}: {error}");
                                file_icons::seti()
                            })
                    };
                    parsed.ok().map(|theme| {
                        for diagnostic in &theme.diagnostics {
                            eprintln!("rove-theme: {diagnostic}");
                        }
                        PreparedIcons::new(theme)
                    })
                })
                .await;
            if cx
                .update_global(|state: &mut AppIcons, cx| {
                    state.0 = prepared.map_or(FileIcons::Disabled, |icons| {
                        icons.register(cx, &mut state.1)
                    });
                })
                .is_err()
            {
                break;
            }
            if changed.recv().await.is_err() {
                break;
            }
            background.timer(Duration::from_millis(180)).await;
            while changed.try_recv().is_ok() {}
        }
    })
    .detach();
}

pub(super) fn icon_theme_setting(settings: &dyn Settings) -> Option<Arc<str>> {
    match settings.read(ICON_THEME, &Scope::default()) {
        Ok(Value::String(theme)) => Some(theme),
        Ok(Value::Null) => None,
        _ => Some("vs-seti".into()),
    }
}

fn start_watcher(
    root: &Path,
    signal: async_channel::Sender<()>,
    dirty: Arc<AtomicBool>,
) -> Option<RecommendedWatcher> {
    if !rove_theme::extensions::is_local_directory(root) {
        return None;
    }
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<Event>| match event {
            Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                let relevant = event.paths.is_empty()
                    || event.paths.iter().any(|path| {
                        !path
                            .components()
                            .any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))
                            && !path
                                .extension()
                                .is_some_and(|extension| extension.eq_ignore_ascii_case("js"))
                    });
                if relevant {
                    dirty.store(true, Ordering::Release);
                    let _ = signal.try_send(());
                }
            }
            Err(error) => eprintln!("rove-theme: extension watcher: {error}"),
            _ => {}
        })
        .ok()?;
    if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
        eprintln!("rove-theme: extension watcher: {error}");
        return None;
    }
    Some(watcher)
}

pub(super) fn attach(view: &mut Workbench, cx: &mut Context<Workbench>) {
    if !cx.has_global::<Themes>() {
        return;
    }
    apply(view, cx);
    apply_icons(view, cx);
    cx.observe_global::<Themes>(apply).detach();
    cx.observe_global::<AppIcons>(apply_icons).detach();
}

fn apply(view: &mut Workbench, cx: &mut Context<Workbench>) {
    let Some(themes) = cx.global::<Themes>().0.clone() else {
        return;
    };
    view.theme_service
        .set_extension_catalog(themes.colors.clone());
    let service = view.theme_service.clone();
    cx.background_executor()
        .spawn(async move {
            let _ = service.reload();
        })
        .detach();
}

pub(super) fn apply_icons(view: &mut Workbench, cx: &mut Context<Workbench>) {
    let Some(icons) = cx.try_global::<AppIcons>() else {
        return;
    };
    let kind = view.theme_service.current().kind();
    let icons = icons.0.clone();
    view.update_quick_file_icons(icons.clone(), kind, cx);
    view.explorer
        .update(cx, |explorer, cx| explorer.set_file_icons(icons, kind, cx));
    cx.notify();
}

pub(super) fn installed_icon_themes(cx: &App) -> Vec<(Arc<str>, Arc<str>)> {
    let Some(scan) = cx.try_global::<Themes>().and_then(|state| state.0.as_ref()) else {
        return Vec::new();
    };
    scan.icons
        .iter()
        .filter(|entry| entry.id.as_ref() != "vs-seti")
        .map(|entry| (entry.id.clone(), entry.label.clone()))
        .collect()
}

pub(super) fn attach_window(
    view: &mut Workbench,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    reflect_appearance(view, window, cx);
    cx.observe_window_appearance(window, reflect_appearance)
        .detach();
}

fn reflect_appearance(view: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>) {
    let dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    if view.theme_service.set_system_dark(dark)
        && rove_theme::service::AUTO_COLOR_SCHEME
            .read(view.settings.as_ref(), &Scope::default())
            .unwrap_or(false)
    {
        let service = view.theme_service.clone();
        cx.background_executor()
            .spawn(async move {
                let _ = service.reload();
            })
            .detach();
    }
}
