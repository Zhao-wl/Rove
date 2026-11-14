//! 更新清单的地址（issue #149 第 2 节）。
//!
//! 地址**写死**在这里，**不允许由设置或环境变量改写**：
//! 能被改写的更新地址等于把签名校验绕过去半步，所以发布构建里它是一个常量。
//!
//! 唯一的例外是**测试覆盖**，它受两道闸同时控制：
//!
//! 1. 只在 `cfg(debug_assertions)`（含 `cargo test`）的构建里编译出读环境变量的那段；
//!    发布的 `rove.exe` 是 release 构建，那段代码根本不存在。
//! 2. 还要显式设置 [`TEST_BASE_URL_ENV`]。
//!
//! 所以「用本地 HTTP 服务器演练 v0.1.0 → v0.1.1」可以做（`tests/rehearsal.rs` 就是这么跑的），
//! 而用户手上的那个 `rove.exe` 永远只会去 GitHub 取清单。

/// 清单与产物的固定目录（GitHub 的「最新 Release」别名）。
pub const BASE_URL: &str = "https://github.com/Zhao-wl/Rove/releases/latest/download";
/// 清单文件名。
pub const MANIFEST_FILE: &str = "latest.json";
/// 分离签名文件名（#76 产出）。
pub const SIGNATURE_FILE: &str = "latest.json.sig";
/// 发布页（便携版「检查更新」时请用户自己去下载的地方）。
pub const RELEASES_PAGE: &str = "https://github.com/Zhao-wl/Rove/releases/latest";

/// 测试专用的地址覆盖。**只在调试构建里被读**，见模块文档。
pub const TEST_BASE_URL_ENV: &str = "ROVE_UPDATE_TEST_BASE_URL";

/// 本次检查要用的目录地址。
pub fn base_url() -> String {
    resolve(override_from_env())
}

/// 覆盖值的来源：发布构建里永远是 `None`（那段代码不参与编译）。
fn override_from_env() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var(TEST_BASE_URL_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// 纯函数，方便两种情况都测到。
fn resolve(override_value: Option<String>) -> String {
    match override_value {
        Some(value) => value.trim_end_matches('/').to_string(),
        None => BASE_URL.to_string(),
    }
}

/// 清单地址。
pub fn manifest_url() -> String {
    format!("{}/{MANIFEST_FILE}", base_url())
}

/// 签名地址。
pub fn signature_url() -> String {
    format!("{}/{SIGNATURE_FILE}", base_url())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有覆盖时就是写死的那个地址（这是发布构建里唯一可能的结果）。
    #[test]
    fn without_an_override_the_address_is_the_fixed_one() {
        assert_eq!(resolve(None), BASE_URL);
        assert!(BASE_URL.starts_with("https://github.com/"));
        assert!(!BASE_URL.ends_with('/'));
    }

    /// 覆盖只影响目录部分，文件名仍然是清单与签名两个固定名字。
    #[test]
    fn an_override_only_replaces_the_directory() {
        let base = resolve(Some("http://127.0.0.1:8080/dist/".into()));
        assert_eq!(base, "http://127.0.0.1:8080/dist");
        assert_eq!(
            format!("{base}/{MANIFEST_FILE}"),
            "http://127.0.0.1:8080/dist/latest.json"
        );
    }

    /// 一次检查只会访问这两个地址（加上清单里写明的产物 URL）。
    #[test]
    fn a_check_touches_exactly_two_urls() {
        assert_eq!(manifest_url(), format!("{}/latest.json", base_url()));
        assert_eq!(signature_url(), format!("{}/latest.json.sig", base_url()));
    }
}
