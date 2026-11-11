//! #10 入口的集成测试：加载主题 → 按 scope 解析 token 样式 → 查颜色键，
//! 以及设置驱动的主题选择、用户覆盖、回退与变更通知。只使用公开接口。

use rove_base::{Error, Events, NoEvents, Result, Subscription, Task, Value, Version};
use rove_config::{Scope, Settings, SettingsChange};
use rove_theme::defaults::{static_defaults, P0_KEYS};
use rove_theme::{parse_hex, Color, Theme, ThemeCatalog, ThemeService, DEFAULT_THEME};
use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type ChangeListener = Arc<dyn Fn(SettingsChange) + Send + Sync>;

/// 可修改的设置替身：只保存用户值，未设置返回 NotFound（与 #13 未登记的键一致）。
#[derive(Default)]
struct UserSettings {
    values: Mutex<BTreeMap<String, Value>>,
    listeners: Mutex<Vec<ChangeListener>>,
}

impl UserSettings {
    fn set(&self, key: &str, value: Value) {
        self.values.lock().unwrap().insert(key.into(), value);
        let change = SettingsChange {
            revision: Version(1),
            keys: Arc::from([Arc::from(key)]),
        };
        let listeners = self.listeners.lock().unwrap().clone();
        for l in listeners {
            l(change.clone());
        }
    }
}

impl Events<SettingsChange> for UserSettings {
    fn subscribe(&self, listener: Arc<dyn Fn(SettingsChange) + Send + Sync>) -> Subscription {
        self.listeners.lock().unwrap().push(listener);
        Subscription::default()
    }
}

impl Settings for UserSettings {
    fn read(&self, key: &str, _: &Scope) -> Result<Value> {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or(Error::NotFound)
    }
    fn write(&self, _: Arc<str>, _: Scope, _: Value, _: rove_base::Cancellation) -> Task<()> {
        Box::pin(async { Err(Error::Unsupported) })
    }
    fn changes(&self) -> &dyn Events<SettingsChange> {
        self
    }
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}
fn obj(entries: &[(&str, Value)]) -> Value {
    Value::Object(Arc::new(
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    ))
}
fn arr(items: Vec<Value>) -> Value {
    Value::Array(items.into())
}
fn c(hex: &str) -> Color {
    parse_hex(hex).unwrap()
}
fn scopes(path: &[&str]) -> Vec<Arc<str>> {
    path.iter().map(|s| Arc::from(*s)).collect()
}
fn fg(theme: &dyn Theme, path: &[&str]) -> Color {
    theme.token_style(&scopes(path)).foreground.unwrap()
}
fn service_with(theme: Option<&str>) -> (Arc<UserSettings>, Arc<ThemeService>) {
    let settings = Arc::new(UserSettings::default());
    if let Some(t) = theme {
        settings
            .values
            .lock()
            .unwrap()
            .insert("workbench.colorTheme".into(), s(t));
    }
    let service = ThemeService::new(settings.clone());
    (settings, service)
}

#[test]
fn load_theme_resolve_token_style_and_query_color_key() {
    let (_, theme) = service_with(None);
    assert_eq!(
        theme.current().id(),
        DEFAULT_THEME,
        "未设置 workbench.colorTheme 时用 Dark 2026"
    );
    // 颜色键：Dark 2026 自己定义的值
    assert_eq!(
        theme.color("editor.background").unwrap(),
        Some(c("#121314"))
    );
    // token：2026-dark.json 的 `keyword`（一段）在 include 链里与 dark_vs 的 `keyword` 同分，后定义者胜。
    assert_eq!(fg(&*theme, &["source.cs", "keyword.cs"]), c("#ff7b72"));
    // 但 dark_plus 的 `keyword.control`（两段）更具体，控制流关键字取它的颜色。
    assert_eq!(
        fg(&*theme, &["source.cs", "keyword.control.flow.return.cs"]),
        c("#C586C0")
    );
    // 父 scope 选择器 + 特异性：include 链里 dark_vs 的 `storage.modifier`（两段）比 2026-dark 的
    // `storage`（一段）更具体，所以 public 取 #569cd6，与 VS Code 一致。
    assert_eq!(
        fg(&*theme, &["source.cs", "storage.modifier.public.cs"]),
        c("#569cd6")
    );
    assert_eq!(fg(&*theme, &["source.cs", "storage.cs"]), c("#ff7b72"));
    // dark_plus 有 C# 专用的 `storage.type.cs`（三段），比 2026 的 `storage.type` 更具体。
    assert_eq!(fg(&*theme, &["source.cs", "storage.type.cs"]), c("#4EC9B0"));
    // 无匹配时是默认前景色
    assert_eq!(fg(&*theme, &["source.cs", "nothing.here"]), c("#bbbebf"));
    // 未登记的键
    assert_eq!(theme.color("no.such.key"), Err(Error::NotFound));
}

#[test]
fn six_builtin_themes_load_with_include_chains() {
    let catalog = ThemeCatalog::builtin();
    for id in [
        "Dark 2026",
        "Light 2026",
        "Dark Modern",
        "Light Modern",
        "Dark+",
        "Light+",
    ] {
        let (_, theme) = service_with(Some(id));
        let current = theme.current();
        assert_eq!(current.id(), id);
        assert!(catalog.get(id).is_some());
        assert!(
            current.diagnostics().is_empty(),
            "{id}: {:?}",
            current.diagnostics()
        );
    }
    // Dark+ 继承 dark_vs：注释色来自 dark_vs，函数色来自 dark_plus 自己。
    let (_, dark_plus) = service_with(Some("Dark+"));
    assert_eq!(
        fg(&*dark_plus, &["source.cs", "comment.line.double-slash.cs"]),
        c("#6A9955")
    );
    assert_eq!(
        fg(&*dark_plus, &["source.cs", "entity.name.function.cs"]),
        c("#DCDCAA")
    );
    assert_eq!(
        dark_plus.color("editor.background").unwrap(),
        Some(c("#1E1E1E"))
    );
    // Light+ 继承 light_vs。
    let (_, light_plus) = service_with(Some("Light+"));
    assert_eq!(
        fg(&*light_plus, &["source.cs", "comment.line.double-slash.cs"]),
        c("#008000")
    );
    assert_eq!(
        fg(&*light_plus, &["source.cs", "entity.name.function.cs"]),
        c("#795E26")
    );
    // Dark Modern → Dark+ → dark_vs 三层：自己的颜色覆盖 dark_vs，规则来自 dark_plus。
    let (_, modern) = service_with(Some("Dark Modern"));
    assert_eq!(
        modern.color("editor.background").unwrap(),
        Some(c("#1F1F1F"))
    );
    assert_eq!(
        fg(&*modern, &["source.cs", "entity.name.function.cs"]),
        c("#DCDCAA")
    );
}

#[test]
fn missing_key_falls_back_to_default_by_theme_type() {
    // Dark+ 没有定义 editorPane.background：默认值引用 editor.background，按主题值求得。
    let (_, dark_plus) = service_with(Some("Dark+"));
    assert_eq!(
        dark_plus.color("editorPane.background").unwrap(),
        Some(c("#1E1E1E"))
    );
    // editorGroup.emptyBackground 默认为 null：明确未设色。
    assert_eq!(
        dark_plus.color("editorGroup.emptyBackground").unwrap(),
        None
    );
    // 主题类型决定默认值表的哪一列：同一个未定义键在深浅主题下取不同默认值。
    let key = "symbolIcon.classForeground";
    let defaults = static_defaults(key).unwrap();
    let (_, light) = service_with(Some("Light+"));
    assert!(!light.current().defines(key) && !dark_plus.current().defines(key));
    assert_eq!(light.color(key).unwrap(), defaults.light);
    assert_eq!(dark_plus.color(key).unwrap(), defaults.dark);
    assert_ne!(defaults.light, defaults.dark);
}

#[test]
fn every_p0_key_in_theme_colors_md_is_registered() {
    let doc = include_str!("../../../docs/compat/THEME_COLORS.md");
    let section = doc
        .split("## 3. 支持（P0）")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    let listed: Vec<&str> = section
        .lines()
        .filter(|l| l.starts_with("**"))
        .flat_map(|l| l.split('`').skip(1).step_by(2))
        .collect();
    assert!(listed.len() >= 403, "清单解析异常：{}", listed.len());
    let (_, theme) = service_with(None);
    let registry = theme.registry();
    for key in &listed {
        let d = registry
            .get(key)
            .unwrap_or_else(|| panic!("P0 键 {key} 未登记"));
        // 每个键都有三套（深/浅/高对比）默认值登记，即使值是明确的“不设色”。
        let _ = (d.dark, d.light, d.high_contrast_dark, d.high_contrast_light);
        assert!(theme.color(key).is_ok(), "{key}");
    }
    assert_eq!(
        registry.entries().count(),
        listed.len(),
        "登记表与文档清单一一对应"
    );
    assert_eq!(P0_KEYS.len(), listed.len());
}

#[test]
fn color_customizations_merge_global_then_theme_specific() {
    let (settings, theme) = service_with(Some("Dark+"));
    settings.values.lock().unwrap().insert(
        "workbench.colorCustomizations".into(),
        obj(&[
            ("editor.background", s("#000001")),
            ("editor.foreground", s("#000002")),
            ("sideBar.background", s("default")),
            ("statusBar.background", s("not-a-color")),
            (
                "[Dark+]",
                obj(&[
                    ("editor.foreground", s("#000003")),
                    ("editor.lineHighlightBackground", s("#000004")),
                ]),
            ),
            ("[Light+]", obj(&[("editor.background", s("#ffffff"))])),
            ("[Dark*]", obj(&[("editorCursor.foreground", s("#000005"))])),
        ]),
    );
    theme.reload().unwrap();
    assert_eq!(
        theme.color("editor.background").unwrap(),
        Some(c("#000001")),
        "全局覆盖"
    );
    assert_eq!(
        theme.color("editor.foreground").unwrap(),
        Some(c("#000003")),
        "按主题优先于全局"
    );
    assert_eq!(
        theme.color("editor.lineHighlightBackground").unwrap(),
        Some(c("#000004"))
    );
    assert_eq!(
        theme.color("editorCursor.foreground").unwrap(),
        Some(c("#000005")),
        "通配符"
    );
    // "default" 忽略主题值（dark_vs 定义了 sideBar.background? 无论如何都取默认值）
    let default_sidebar = static_defaults("sideBar.background").unwrap().dark;
    assert_eq!(theme.color("sideBar.background").unwrap(), default_sidebar);
    // 默认 token 规则跟随覆盖后的 editor.foreground
    assert_eq!(fg(&*theme, &["source.cs", "nothing"]), c("#000003"));
    // 派生默认值也跟随覆盖：editorPane.background → editor.background
    assert_eq!(
        theme.color("editorPane.background").unwrap(),
        Some(c("#000001"))
    );
    let diags: Vec<String> = theme.diagnostics().iter().map(|d| d.to_string()).collect();
    assert!(
        diags
            .iter()
            .any(|d| d.contains("workbench.colorCustomizations[\"statusBar.background\"]")),
        "{diags:?}"
    );
}

#[test]
fn token_color_customizations_shorthand_rules_and_theme_specific() {
    let (settings, theme) = service_with(Some("Dark+"));
    settings.values.lock().unwrap().insert(
        "editor.tokenColorCustomizations".into(),
        obj(&[
            ("comments", s("#00000a")),
            (
                "strings",
                obj(&[("foreground", s("#00000b")), ("fontStyle", s("italic"))]),
            ),
            (
                "textMateRules",
                arr(vec![
                    obj(&[
                        ("scope", s("comment.line.double-slash")),
                        ("settings", obj(&[("foreground", s("#00000c"))])),
                    ]),
                    obj(&[
                        ("scope", s("meta.function entity.name.function")),
                        ("settings", obj(&[("fontStyle", s("bold underline"))])),
                    ]),
                ]),
            ),
            (
                "[Dark+]",
                obj(&[
                    ("keywords", s("#00000d")),
                    (
                        "textMateRules",
                        arr(vec![obj(&[
                            ("scope", s("comment")),
                            (
                                "settings",
                                obj(&[("foreground", s("#00000e")), ("fontStyle", s(""))]),
                            ),
                        ])]),
                    ),
                ]),
            ),
            ("[Light+]", obj(&[("numbers", s("#ffffff"))])),
        ]),
    );
    theme.reload().unwrap();
    // 简写分组
    let string = theme.token_style(&scopes(&["source.cs", "string.quoted.double.cs"]));
    assert_eq!(string.foreground, Some(c("#00000b")));
    assert!(string.italic);
    // 按主题的 `comment`（后定义）覆盖全局简写 `comment`，但更具体的
    // `comment.line.double-slash` 仍然胜出：特异性优先于定义顺序。
    assert_eq!(
        fg(&*theme, &["source.cs", "comment.block.cs"]),
        c("#00000e")
    );
    assert_eq!(
        fg(&*theme, &["source.cs", "comment.line.double-slash.cs"]),
        c("#00000c")
    );
    assert_eq!(
        fg(&*theme, &["source.cs", "keyword.control.cs"]),
        c("#00000d")
    );
    // 不属于当前主题的覆盖不生效
    assert_ne!(
        fg(&*theme, &["source.cs", "constant.numeric.cs"]),
        c("#ffffff")
    );
    // 父 scope 选择器（与 dark_plus 的 `entity.name.function` 同深度、父 scope 更多，所以更具体）
    // 只设置了字体样式，颜色从同节点的 dark_plus 规则继承
    let f = theme.token_style(&scopes(&[
        "source.cs",
        "meta.function.cs",
        "entity.name.function.cs",
    ]));
    assert!(f.bold && f.underline && !f.italic);
    assert_eq!(f.foreground, Some(c("#DCDCAA")));
    let g = theme.token_style(&scopes(&["source.cs", "entity.name.function.cs"]));
    assert!(!g.bold);
}

#[test]
fn unknown_theme_falls_back_to_dark_2026_with_log() {
    let (_, theme) = service_with(Some("Some Extension Theme"));
    assert_eq!(theme.current().id(), "Dark 2026");
    assert_eq!(
        theme.color("editor.background").unwrap(),
        Some(c("#121314"))
    );
    let diags = theme.diagnostics();
    assert!(
        diags.iter().any(|d| d.field == "workbench.colorTheme"
            && d.message.contains("Some Extension Theme")
            && d.message.contains("Dark 2026")),
        "{diags:?}"
    );
}

#[test]
fn settings_change_reloads_and_notifies() {
    let (settings, theme) = service_with(None);
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let _sub = theme
        .changes()
        .subscribe(Arc::new(move |v| tx.lock().unwrap().send(v).unwrap()));
    let before = theme.revision();
    settings.set("workbench.colorTheme", s("Light+"));
    let v = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("theme change event");
    assert!(v > before);
    assert_eq!(theme.current().id(), "Light+");
    assert_eq!(
        theme.color("editor.background").unwrap(),
        Some(c("#FFFFFF"))
    );
    // 无关设置不触发
    settings.set("editor.fontSize", Value::Number(14.0));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

/// 快速连续切换两次主题：先开始、后完成的旧加载不能覆盖后切的主题（#104，PR #99 审核意见）。
#[test]
fn a_slower_earlier_reload_never_overwrites_a_later_one() {
    /// 第一次读取 `workbench.colorTheme` 时停住，直到测试放行，模拟较慢的旧加载。
    struct Gated {
        inner: UserSettings,
        gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    }
    impl Settings for Gated {
        fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
            let value = self.inner.read(key, scope);
            if key == "workbench.colorTheme" {
                let gate = self.gate.lock().unwrap().take();
                if let Some((entered, release)) = gate {
                    entered.send(()).unwrap();
                    release.recv().unwrap();
                }
            }
            value
        }
        fn write(&self, k: Arc<str>, s: Scope, v: Value, c: rove_base::Cancellation) -> Task<()> {
            self.inner.write(k, s, v, c)
        }
        fn changes(&self) -> &dyn Events<SettingsChange> {
            &self.inner
        }
    }
    let settings = Arc::new(Gated {
        inner: UserSettings::default(),
        gate: Mutex::new(None),
    });
    let theme = ThemeService::new(settings.clone());
    let set = |name: &str| {
        settings
            .inner
            .values
            .lock()
            .unwrap()
            .insert("workbench.colorTheme".into(), s(name));
    };
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *settings.gate.lock().unwrap() = Some((entered_tx, release_rx));

    set("Dark+");
    let slow = {
        let theme = theme.clone();
        std::thread::spawn(move || theme.reload())
    };
    entered
        .recv_timeout(Duration::from_secs(10))
        .expect("slow reload started");
    set("Light+");
    theme.reload().expect("later reload applies");
    assert_eq!(theme.current().id(), "Light+");
    release.send(()).unwrap();
    assert_eq!(slow.join().unwrap(), Err(Error::Stale));
    assert_eq!(theme.current().id(), "Light+", "后切的主题必须保留");
}

#[test]
fn snapshot_is_shareable_and_default_settings_work() {
    fn shareable<T: Send + Sync>() {}
    shareable::<ThemeService>();
    shareable::<rove_theme::ResolvedTheme>();
    // DefaultSettings（空登记表）同样可用：全部 NotFound → 默认主题。
    let settings = Arc::new(rove_config::DefaultSettings(Default::default()));
    let theme = ThemeService::new(settings);
    assert_eq!(theme.current().id(), DEFAULT_THEME);
    let _ = NoEvents;
}
