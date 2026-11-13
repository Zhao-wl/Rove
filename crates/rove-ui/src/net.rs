//! #74：Rove 运行时不发起任何网络请求（`docs/PROJECT_CHARTER.md` §2.1）。
//!
//! 编译期的保证在 workspace 的 `[patch.crates-io]`：`gpui_http_client` 被换成
//! `crates/rove-gpui-http-stub`，`zed-reqwest` / `hyper` / `rustls` / `ring` / `tokio`
//! 整条栈都不在依赖图里，`cargo xtask check-deps` 在 CI 上守住它。
//!
//! 运行期这里再加一层：gpui 的 `Application::new()` 默认装的是它自己的 `NullHttpClient`
//! （返回 `No HttpClient available`，但**不写日志**）。Rove 启动时显式换成 stub 的
//! [`BlockedHttpClient`]，它把每一次被拒绝的请求连同方法与 URI 写进日志与 stderr。
//! 这样万一哪条路径真的发起了请求，立刻看得见，而不是表现成「图片加载失败」这种含糊的症状。
//!
//! 这不是「运行期不联网」的证明（那属于 #75 的实测），只是让违例无法悄悄发生。

use std::sync::Arc;

use gpui::http_client::{BlockedHttpClient, HttpClient};

/// 应用使用的 HTTP 客户端：拒绝一切请求，并记录被拒绝的方法与 URI。
///
/// 在 `skeleton::launch` 里通过 `Application::with_http_client` 装上。
pub fn blocked_http_client() -> Arc<dyn HttpClient> {
    Arc::new(BlockedHttpClient::new())
}
