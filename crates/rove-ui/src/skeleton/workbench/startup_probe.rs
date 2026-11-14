//! 「可交互」冷启动埋点（#150，预算 `cold_start_interactive`）。
//!
//! `perf/budgets.toml` 对「可交互」的定义是**首帧已呈现 + 第一个输入事件在一帧内得到响应**。
//! 首帧那一半由 #84 的 `FIRSTFRAME_SINCE_PROCESS_MS` 给出；这里补上后一半：首帧画完之后立刻
//! 向窗口注入一个**无副作用的按键**（`right`，插入符右移一列 / 列表里下一项），请求重绘，等
//! 包含这次响应的那一帧画完，打印
//!
//! ```text
//! INTERACTIVE_SINCE_PROCESS_MS <进程启动到这一帧画完的毫秒数> KEY_TO_FRAME_MS <按键到这一帧的毫秒数>
//! ```
//!
//! 采集方由 `cargo xtask startup` 负责（见 `xtask/src/startup.rs`）。
//!
//! **默认关闭**：只有环境变量 `ROVE_STARTUP_PROBE=1` 时才注入按键，正常使用的 `rove.exe`
//! 不会凭空多出一次按键，也不会多打印一行。按键走的是 `Window::dispatch_keystroke`，和 #47
//! 的 `typing/key_to_frame` 基准同一条路径（键位服务 → 命令 / 动作 → 重绘），
//! 不经过系统消息队列，所以这个数字不含 Windows 键盘消息的排队时间——报告里要写明口径。

use std::time::Instant;

use gpui::{App, Keystroke, Window};

/// 注入用的按键：只移动插入符 / 列表选中项，不会改任何文件。
const PROBE_KEY: &str = "right";

/// 打开埋点的环境变量（值必须是 `1`）。
pub const ENV: &str = "ROVE_STARTUP_PROBE";

pub fn enabled() -> bool {
    enabled_for(std::env::var_os(ENV).as_deref())
}

/// 纯函数形式，方便测试：只有值正好是 `1` 才算打开。
fn enabled_for(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| value == "1")
}

/// 在首帧的 `on_next_frame` 回调里调用：注入一个按键，并在下一帧画完时打印标记。
///
/// `started` 是进程启动时刻（`rove-app` 用 `GetProcessTimes` 取的，含加载器时间）。
pub fn after_first_frame(started: Instant, window: &mut Window, cx: &mut App) {
    if !enabled() {
        return;
    }
    let Ok(key) = Keystroke::parse(PROBE_KEY) else {
        return;
    };
    let dispatched = Instant::now();
    window.dispatch_keystroke(key, cx);
    // 按键可能什么都没改（空窗口里没有插入符），这时也要画一帧才能说「响应完成」。
    window.refresh();
    window.on_next_frame(move |_, _| {
        println!(
            "INTERACTIVE_SINCE_PROCESS_MS {:.3} KEY_TO_FRAME_MS {:.3}",
            started.elapsed().as_secs_f64() * 1000.0,
            dispatched.elapsed().as_secs_f64() * 1000.0
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认不开：没有设置、设成别的值、设成空串都不注入按键。
    #[test]
    fn the_probe_is_off_unless_the_environment_variable_is_exactly_one() {
        use std::ffi::OsStr;
        assert!(!enabled_for(None));
        assert!(!enabled_for(Some(OsStr::new(""))));
        assert!(!enabled_for(Some(OsStr::new("0"))));
        assert!(!enabled_for(Some(OsStr::new("true"))));
        assert!(enabled_for(Some(OsStr::new("1"))));
    }

    /// 注入用的按键必须解析得出来，否则埋点会静默失效。
    #[test]
    fn the_probe_key_parses() {
        assert!(Keystroke::parse(PROBE_KEY).is_ok());
    }
}
