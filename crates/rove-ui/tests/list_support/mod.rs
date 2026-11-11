//! Shared fixtures for the #18 list / tree / quick input integration tests.
#![allow(dead_code)]
use rove_i18n::{chinese, StringTable, Strings};
use rove_theme::{ColorRegistry, DefaultTheme, Theme, ThemeKind};
use rove_ui::list_widget::registry;
use std::sync::Arc;

/// Theme and strings assembled exactly like the app does: through the #1 registries.
pub fn theme_and_strings() -> (Arc<dyn Theme>, Arc<dyn Strings>) {
    let mut colors = ColorRegistry::default();
    registry::register_colors(&mut colors).unwrap();
    let mut strings: StringTable = chinese().unwrap();
    registry::register_strings(&mut strings).unwrap();
    for (k, v) in [
        ("test.empty", "尚未打开文件夹。"),
        ("test.open", "打开文件夹"),
        ("test.scanning", "正在加载文件列表…"),
        ("test.pick", "选择编码"),
    ] {
        strings.register(k, Arc::from(v)).unwrap();
    }
    (
        Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        Arc::new(strings),
    )
}
