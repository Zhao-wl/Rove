//! Markdown 预览（#29）。
pub mod backend;
pub mod page;
pub mod registry;
pub mod render;
#[cfg(windows)]
pub mod webview;

/// #6 报告推荐的 Hwnd 嵌入方式：WebView2 的子 HWND 只有在 GPUI 不用 DirectComposition 时才可见
/// （DirectComposition 的合成目标是 topmost，会盖住子窗口，见报告 §12、§13 的 Baseline 对照）。
/// GPUI 在 `Application::new` 时读取这个环境变量，所以必须在创建应用之前调用；用户已显式设置时不覆盖。
pub fn prepare_hwnd_hosting() {
    const KEY: &str = "GPUI_DISABLE_DIRECT_COMPOSITION";
    if std::env::var_os(KEY).is_none() {
        std::env::set_var(KEY, "1");
    }
}
