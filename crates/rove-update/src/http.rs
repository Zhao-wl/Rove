//! **Rove 唯一的网络出口**（issue #149 第 1 节）。
//!
//! 用 Windows 自带的 **WinHTTP**：HTTPS 走系统证书库与系统代理设置，不引入
//! reqwest / hyper / rustls / ring / tokio（#74 刚把这整条栈从依赖图里拿掉，本 issue 不能加回去）。
//!
//! # 请求长什么样
//!
//! 一次检查最多三个 GET，全部**没有请求体、没有自定义请求头**：
//!
//! | 方法 | 地址 | 什么时候 |
//! |---|---|---|
//! | GET | `<base>/latest.json` | 每次检查 |
//! | GET | `<base>/latest.json.sig` | 每次检查 |
//! | GET | 清单里 `installer.url` / `components[].url` | 确认要更新之后 |
//!
//! 请求里**不带任何用户数据**：没有设备标识、没有项目信息、没有 cookie、没有查询参数，
//! User-Agent 固定为 `Rove/<版本>`（[`user_agent`]）。每个请求在 [`crate::log`] 里留一行。
//!
//! # 为什么有 [`Transport`] 这一层
//!
//! 让除了本文件以外的所有逻辑都能脱离网络测试：单元测试用 [`StubTransport`]，
//! 集成测试（`tests/rehearsal.rs`）用**本机 HTTP 服务器** + 真的 [`WinHttp`]。

use crate::{Result, UpdateError};
use std::io::Write;

/// 单次请求的超时（解析 / 连接 / 发送 / 接收），毫秒。更新是后台事情，不值得等很久。
pub const TIMEOUT_MS: i32 = 15_000;
/// 清单与签名的大小上限：它们是小文本，超过就说明拿到的不是清单。
pub const MANIFEST_LIMIT: u64 = 256 * 1024;
/// 安装包与组件包的大小上限（发布包预算是 30 MB，这里留足余量）。
pub const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

/// 固定的 User-Agent。
pub fn user_agent(version: &str) -> String {
    format!("Rove/{version}")
}

/// 一个能发 GET 的东西。**只有 [`WinHttp`] 真的联网**。
pub trait Transport: Send + Sync {
    /// GET `url`，把响应体写进 `sink`，返回写入的字节数。
    /// 超过 `limit` 字节就中止并报 [`UpdateError::Integrity`]。
    fn get(&self, url: &str, sink: &mut dyn Write, limit: u64) -> Result<u64>;
}

/// GET 到内存（清单与签名这种小文本）。
pub fn get_bytes(transport: &dyn Transport, url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    transport.get(url, &mut buffer, limit)?;
    Ok(buffer)
}

/// GET 到字符串。
pub fn get_text(transport: &dyn Transport, url: &str, limit: u64) -> Result<String> {
    let bytes = get_bytes(transport, url, limit)?;
    String::from_utf8(bytes).map_err(|_| UpdateError::Manifest("响应不是 UTF-8".into()))
}

// ---------------------------------------------------------------- URL 解析

/// 拆开的地址。只认 `http` 与 `https`，没有用户名密码、没有片段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// 以 `/` 开头，含查询串。
    pub path: String,
}

impl Url {
    pub fn parse(url: &str) -> Result<Self> {
        let bad = |why: &str| UpdateError::Network(format!("地址无法解析（{why}）"));
        let (scheme, rest) = url.split_once("://").ok_or_else(|| bad("缺少 scheme"))?;
        let secure = match scheme {
            "https" => true,
            "http" => false,
            other => return Err(bad(&format!("不支持 {other}"))),
        };
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };
        if authority.contains('@') {
            return Err(bad("地址里不允许有凭据"));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().map_err(|_| bad("端口不是数字"))?),
            None => (authority, if secure { 443 } else { 80 }),
        };
        if host.is_empty() {
            return Err(bad("主机名是空的"));
        }
        Ok(Self {
            secure,
            host: host.to_string(),
            port,
            path: path.to_string(),
        })
    }
}

// ---------------------------------------------------------------- 测试替身

/// 测试用的传输：按 URL 返回预置内容，**永远不联网**。
#[derive(Clone, Debug, Default)]
pub struct StubTransport {
    responses: std::collections::BTreeMap<String, std::result::Result<Vec<u8>, UpdateError>>,
    /// 记录被请求过的地址，断言「`none` 模式一个请求都没发」。
    requested: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl StubTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, url: &str, body: impl Into<Vec<u8>>) -> Self {
        self.responses.insert(url.to_string(), Ok(body.into()));
        self
    }

    pub fn failing(mut self, url: &str, error: UpdateError) -> Self {
        self.responses.insert(url.to_string(), Err(error));
        self
    }

    /// 被请求过的地址，按顺序。
    pub fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl Transport for StubTransport {
    fn get(&self, url: &str, sink: &mut dyn Write, limit: u64) -> Result<u64> {
        self.requested.lock().unwrap().push(url.to_string());
        match self.responses.get(url) {
            Some(Ok(body)) => {
                if body.len() as u64 > limit {
                    return Err(UpdateError::Integrity(format!("响应超过 {limit} 字节上限")));
                }
                sink.write_all(body)
                    .map_err(|e| UpdateError::Storage(e.to_string()))?;
                Ok(body.len() as u64)
            }
            Some(Err(error)) => Err(error.clone()),
            // 没预置就是 404：和 GitHub 上还没发布这个文件时一样。
            None => Err(UpdateError::Http(404)),
        }
    }
}

// ---------------------------------------------------------------- WinHTTP

#[cfg(windows)]
mod winhttp {
    use super::{Transport, Url, DOWNLOAD_LIMIT, TIMEOUT_MS};
    use crate::{Result, UpdateError};
    use std::io::Write;
    use windows::core::PCWSTR;
    use windows::Win32::Networking::WinHttp::*;

    /// 释放 WinHTTP 句柄的小包装：任何一条错误路径上句柄都会被关掉。
    struct Handle(*mut core::ffi::c_void);

    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: 句柄由 WinHttpOpen / Connect / OpenRequest 产出，只关一次。
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error(what: &str) -> UpdateError {
        UpdateError::Network(format!("{what}：{}", std::io::Error::last_os_error()))
    }

    /// 真·网络出口。
    pub struct WinHttp {
        user_agent: Vec<u16>,
    }

    impl WinHttp {
        pub fn new(user_agent: &str) -> Self {
            Self {
                user_agent: wide(user_agent),
            }
        }
    }

    impl Transport for WinHttp {
        fn get(&self, url: &str, sink: &mut dyn Write, limit: u64) -> Result<u64> {
            let url = Url::parse(url)?;
            let limit = limit.min(DOWNLOAD_LIMIT);
            // SAFETY: 下面每个指针要么来自本函数里活着的局部变量，要么是文档允许的空指针；
            // 三个句柄都由 Handle 负责关闭。
            unsafe {
                // 自动代理 = 跟随系统代理设置（IE / WinINET 的那一套）。
                let session = Handle(WinHttpOpen(
                    PCWSTR(self.user_agent.as_ptr()),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                ));
                if session.0.is_null() {
                    return Err(last_error("WinHttpOpen 失败"));
                }
                WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS)
                    .map_err(|e| UpdateError::Network(format!("设置超时失败：{e}")))?;
                // 跟随重定向，但**绝不从 https 掉到 http**（GitHub 的下载会跳到
                // objects.githubusercontent.com，仍然是 https）。
                let policy = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP.to_ne_bytes();
                let _ = WinHttpSetOption(
                    Some(session.0),
                    WINHTTP_OPTION_REDIRECT_POLICY,
                    Some(&policy),
                );

                let host = wide(&url.host);
                let connect = Handle(WinHttpConnect(
                    session.0,
                    PCWSTR(host.as_ptr()),
                    url.port,
                    0,
                ));
                if connect.0.is_null() {
                    return Err(last_error("WinHttpConnect 失败"));
                }

                let verb = wide("GET");
                let path = wide(&url.path);
                let flags = if url.secure {
                    WINHTTP_FLAG_SECURE
                } else {
                    WINHTTP_OPEN_REQUEST_FLAGS(0)
                };
                let request = Handle(WinHttpOpenRequest(
                    connect.0,
                    PCWSTR(verb.as_ptr()),
                    PCWSTR(path.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    flags,
                ));
                if request.0.is_null() {
                    return Err(last_error("WinHttpOpenRequest 失败"));
                }

                // 不加任何请求头、不带请求体（第 2 节：请求里不带任何用户数据）。
                WinHttpSendRequest(request.0, None, None, 0, 0, 0)
                    .map_err(|e| UpdateError::Network(format!("请求发送失败：{e}")))?;
                WinHttpReceiveResponse(request.0, std::ptr::null_mut())
                    .map_err(|e| UpdateError::Network(format!("没有收到响应：{e}")))?;

                let mut status: u32 = 0;
                let mut size = std::mem::size_of::<u32>() as u32;
                WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    PCWSTR::null(),
                    Some(std::ptr::addr_of_mut!(status).cast()),
                    &mut size,
                    std::ptr::null_mut(),
                )
                .map_err(|e| UpdateError::Network(format!("读不到状态码：{e}")))?;
                if status != 200 {
                    return Err(UpdateError::Http(status));
                }

                let mut buffer = vec![0u8; 64 * 1024];
                let mut total = 0u64;
                loop {
                    let mut read: u32 = 0;
                    WinHttpReadData(
                        request.0,
                        buffer.as_mut_ptr().cast(),
                        buffer.len() as u32,
                        &mut read,
                    )
                    .map_err(|e| UpdateError::Network(format!("读取响应失败：{e}")))?;
                    if read == 0 {
                        break;
                    }
                    total += u64::from(read);
                    if total > limit {
                        return Err(UpdateError::Integrity(format!("响应超过 {limit} 字节上限")));
                    }
                    sink.write_all(&buffer[..read as usize])
                        .map_err(|e| UpdateError::Storage(e.to_string()))?;
                }
                sink.flush()
                    .map_err(|e| UpdateError::Storage(e.to_string()))?;
                Ok(total)
            }
        }
    }
}

#[cfg(windows)]
pub use winhttp::WinHttp;

/// 非 Windows 上没有 WinHTTP：更新功能整体不可用（Rove 是 Windows 优先的产品，
/// 这个分支只为了让核心层在别的平台上也能编过与被测试）。
#[cfg(not(windows))]
pub struct WinHttp;

#[cfg(not(windows))]
impl WinHttp {
    pub fn new(_user_agent: &str) -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl Transport for WinHttp {
    fn get(&self, _url: &str, _sink: &mut dyn Write, _limit: u64) -> Result<u64> {
        Err(UpdateError::Unavailable("只有 Windows 上有 WinHTTP".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_agent_carries_nothing_but_the_version() {
        assert_eq!(user_agent("0.1.0"), "Rove/0.1.0");
    }

    #[test]
    fn urls_split_into_host_port_and_path() {
        let url =
            Url::parse("https://github.com/Zhao-wl/Rove/releases/latest/download/latest.json")
                .unwrap();
        assert!(url.secure);
        assert_eq!(url.host, "github.com");
        assert_eq!(url.port, 443);
        assert_eq!(
            url.path,
            "/Zhao-wl/Rove/releases/latest/download/latest.json"
        );

        let local = Url::parse("http://127.0.0.1:8321/dist/latest.json").unwrap();
        assert!(!local.secure);
        assert_eq!((local.host.as_str(), local.port), ("127.0.0.1", 8321));

        // 没有路径时补 `/`。
        assert_eq!(Url::parse("https://example.invalid").unwrap().path, "/");
    }

    #[test]
    fn hostile_urls_are_rejected() {
        for url in [
            "",
            "github.com/latest.json",
            "ftp://example.invalid/x",
            "file:///C:/x",
            "https://user:pass@example.invalid/x",
            "https://example.invalid:notaport/x",
            "https:///x",
        ] {
            assert!(Url::parse(url).is_err(), "{url:?} 应该被拒绝");
        }
    }

    #[test]
    fn the_stub_records_requests_and_enforces_the_limit() {
        let stub = StubTransport::new().with("https://a/latest.json", b"{}".to_vec());
        assert_eq!(get_text(&stub, "https://a/latest.json", 64).unwrap(), "{}");
        assert_eq!(
            get_bytes(&stub, "https://a/missing", 64).unwrap_err(),
            UpdateError::Http(404)
        );
        assert_eq!(
            stub.requested(),
            ["https://a/latest.json", "https://a/missing"]
        );
        let big = StubTransport::new().with("https://a/x", vec![0u8; 100]);
        assert!(matches!(
            get_bytes(&big, "https://a/x", 10).unwrap_err(),
            UpdateError::Integrity(_)
        ));
    }
}
