//! #74 的入口测试：Rove 装给 gpui 的 HTTP 客户端「调用即失败并记日志」，不是静默成功。
//!
//! 运行：`cargo test -p rove-ui --test no_network`
//!
//! 编译期那一半（依赖图里没有 reqwest / hyper / rustls / ring / tokio）由
//! `cargo xtask check-deps` 判定，并已纳入 CI。

use gpui::http_client::BLOCKED_MESSAGE;
use gpui::TestAppContext;
use rove_ui::net::blocked_http_client;
use std::sync::Mutex;

static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Recorder;

impl log::Log for Recorder {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        RECORDS
            .lock()
            .unwrap()
            .push(format!("{} {}", record.level(), record.args()));
    }

    fn flush(&self) {}
}

/// 装上记录型 logger（整个进程只装一次），返回当前日志条数。
fn start_recording() -> usize {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = log::set_logger(&Recorder);
        log::set_max_level(log::LevelFilter::Trace);
    });
    RECORDS.lock().unwrap().len()
}

fn logged_since(start: usize) -> Vec<String> {
    RECORDS.lock().unwrap()[start..].to_vec()
}

/// 直接用 Rove 装给应用的那个客户端：返回明确错误，并写进日志。
#[test]
fn the_client_rove_installs_rejects_every_request_and_logs_it() {
    let start = start_recording();
    let client = blocked_http_client();
    let error = futures_lite::future::block_on(client.get(
        "https://example.com/remote.png",
        ().into(),
        true,
    ))
    .map(|_| ())
    .expect_err("Rove 的 HTTP 客户端必须返回错误，不能静默成功");

    let message = error.to_string();
    assert!(message.contains(BLOCKED_MESSAGE), "{message}");
    assert!(
        message.contains("https://example.com/remote.png"),
        "{message}"
    );

    let records = logged_since(start);
    assert!(
        records
            .iter()
            .any(|r| r.starts_with("ERROR") && r.contains("https://example.com/remote.png")),
        "被拒绝的请求必须写进日志：{records:?}"
    );
}

/// 走 gpui 自己的 `cx.http_client()`：这是 gpui 内部（例如远程图片资源）真正用的那条路径。
#[gpui::test]
fn gpui_sees_the_blocked_client_through_cx_http_client(cx: &mut TestAppContext) {
    let start = start_recording();
    cx.update(|cx| cx.set_http_client(blocked_http_client()));
    let client = cx.update(|cx| cx.http_client());

    let error = futures_lite::future::block_on(client.get(
        "https://example.com/via-gpui.svg",
        ().into(),
        false,
    ))
    .map(|_| ())
    .expect_err("gpui 取到的客户端同样必须拒绝请求");
    assert!(error.to_string().contains(BLOCKED_MESSAGE), "{error}");

    let records = logged_since(start);
    assert!(
        records
            .iter()
            .any(|r| r.starts_with("ERROR") && r.contains("https://example.com/via-gpui.svg")),
        "被拒绝的请求必须写进日志：{records:?}"
    );
}
