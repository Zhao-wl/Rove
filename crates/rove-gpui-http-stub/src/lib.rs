//! `gpui_http_client` 的本地替身（issue #74，路线 B）。
//!
//! Rove 在运行时**不发起任何网络请求**（`docs/PROJECT_CHARTER.md` §2.1，唯一例外是自动更新
//! 模块所在的 crate）。但 `gpui` 对 `gpui_http_client` 的依赖不是 optional，它会把
//! `zed-reqwest` → `hyper` / `hyper-rustls` → `rustls` → `ring` / `tokio` 整条 HTTP + TLS 栈
//! 拉进依赖图。这些代码最终被链接器剔除，**不是体积问题**，但它是多余的审计面与供应链面。
//!
//! 本 crate 是 **Rove 自己的代码**（MIT，与仓库一致），不是 gpui / Zed 的移植：它只保留
//! `gpui` 编译所需的公开 API 形状，内部没有任何网络实现。workspace 的
//! `[patch.crates-io] gpui_http_client = { path = "crates/rove-gpui-http-stub" }` 让整个依赖图
//! 都用它，`vendor/gpui` 的源码因此**零改动**。
//!
//! # 语义：调用即失败，并且看得见
//!
//! 所有真正会「发出去」的路径都返回错误，并在**构造 future 的那一刻**（不是 await 时）
//! 写一条 `log::error!` 与一行 stderr。绝不静默返回空响应：万一哪天 gpui 或 Rove 真的调用了
//! HTTP 客户端，必须立刻在日志里看得到，而不是表现成「图片加载失败」这种含糊的症状。
//!
//! # 与上游 `gpui_http_client` 0.2.2 的差异（全部是删减，没有新增语义）
//!
//! - 删除 `github` / `github_download` 模块：只服务 Zed 的「下载 GitHub Release」，
//!   会把 `async-tar` / `async-compression` / `sha2` / `tempfile` 拖进来，Rove 用不到。
//! - 删除 `HttpClient::send_multipart_form`：它的签名里直接出现 `reqwest::multipart::Form`，
//!   保留就等于保留 reqwest 依赖。gpui 自身从不调用它。
//! - 删除 `impl http_body::Body for AsyncBody` 与 `TryFrom<reqwest::Body>`：前者只为 hyper 服务，
//!   后者需要 reqwest。gpui 读响应体走的是 `futures::AsyncRead`，仍然保留。
//! - 删除 `HttpClientWithUrl::build_zed_api_url` / `build_zed_cloud_url` / `build_zed_llm_url`：
//!   只是把 `zed.dev` 系列域名拼进 URL，对一个不联网的阅读器没有意义，也不该把这些域名
//!   留在二进制里。`base_url` / `set_base_url` / `build_url` 保留。
//!
//! 升级 gpui 时如何重新应用见 `vendor/gpui/ROVE_PATCHES.md`。

mod async_body;

pub use anyhow::{anyhow, Result};
pub use async_body::{AsyncBody, Inner};
pub use http::{self, request::Builder, Method, Request, Response, StatusCode, Uri};
pub use url::Url;

use futures::future::BoxFuture;
use http::HeaderValue;
use parking_lot::Mutex;
use std::any::type_name;
use std::sync::Arc;

/// 重定向策略。保留上游形状；本 crate 不会真的发请求，所以它只是被原样记录在请求扩展里。
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum RedirectPolicy {
    /// 不跟随重定向。
    #[default]
    NoFollow,
    /// 最多跟随 N 次。
    FollowLimit(u32),
    /// 一直跟随。
    FollowAll,
}

/// 上游的标记类型，保留以维持 API 形状。
pub struct FollowRedirects(pub bool);

/// 给 `http::request::Builder` 加的链式扩展（上游 API）。
pub trait HttpRequestExt {
    /// 条件成立时用闭包改写自身。
    fn when(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized,
    {
        if condition {
            then(self)
        } else {
            self
        }
    }

    /// `Option` 是 `Some` 时用闭包改写自身。
    fn when_some<T>(self, option: Option<T>, then: impl FnOnce(Self, T) -> Self) -> Self
    where
        Self: Sized,
    {
        match option {
            Some(value) => then(self, value),
            None => self,
        }
    }

    /// 设置重定向策略。
    fn follow_redirects(self, follow: RedirectPolicy) -> Self;
}

impl HttpRequestExt for Builder {
    fn follow_redirects(self, follow: RedirectPolicy) -> Self {
        self.extension(follow)
    }
}

/// 被拒绝时写进日志与错误里的固定前缀，方便在日志里直接搜。
pub const BLOCKED_MESSAGE: &str =
    "ROVE_NO_NETWORK: Rove 不发起网络请求，gpui 的 HTTP 客户端已被 stub 替换（issue #74）";

/// 记录并构造「请求被拒绝」的错误。
///
/// 在**构造 future 时**调用，而不是在 future 被 poll 时：调用方即使把 future 丢掉，
/// 这次尝试也已经留在日志里了。
fn reject(client: &'static str, method: &Method, uri: &Uri) -> anyhow::Error {
    let detail = format!("{BLOCKED_MESSAGE}；被拒绝的请求：{method} {uri}（客户端 {client}）");
    log::error!("{detail}");
    // Rove 目前没有初始化 `log` 的后端，stderr 是唯一一定看得见的通道；
    // 这是错误路径，正常运行一次都不会触发。
    eprintln!("{detail}");
    anyhow!(detail)
}

/// 返回一个立刻就绪的失败 future，并在此刻记录日志。
fn rejected_response(
    client: &'static str,
    req: &Request<AsyncBody>,
) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
    let err = reject(client, req.method(), req.uri());
    Box::pin(async move { Err(err) })
}

/// HTTP 客户端接口（上游形状）。本 crate 提供的所有非测试实现都「调用即失败」。
pub trait HttpClient: 'static + Send + Sync {
    /// 实现类型名，出现在日志里。
    fn type_name(&self) -> &'static str;

    /// User-Agent（本 crate 的实现都是 `None`）。
    fn user_agent(&self) -> Option<&HeaderValue>;

    /// 发送请求。
    fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>>;

    /// GET。
    fn get(
        &self,
        uri: &str,
        body: AsyncBody,
        follow_redirects: bool,
    ) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        let request = Builder::new()
            .uri(uri)
            .follow_redirects(if follow_redirects {
                RedirectPolicy::FollowAll
            } else {
                RedirectPolicy::NoFollow
            })
            .body(body);

        match request {
            Ok(request) => self.send(request),
            Err(e) => Box::pin(async move { Err(e.into()) }),
        }
    }

    /// POST JSON。
    fn post_json(
        &self,
        uri: &str,
        body: AsyncBody,
    ) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        let request = Builder::new()
            .uri(uri)
            .method(Method::POST)
            .header("Content-Type", "application/json")
            .body(body);

        match request {
            Ok(request) => self.send(request),
            Err(e) => Box::pin(async move { Err(e.into()) }),
        }
    }

    /// 代理设置（本 crate 的实现都是 `None`）。
    fn proxy(&self) -> Option<&Url>;

    /// 仅测试：向下转型到 `FakeHttpClient`。
    #[cfg(feature = "test-support")]
    fn as_fake(&self) -> &FakeHttpClient {
        panic!("called as_fake on {}", type_name::<Self>())
    }
}

/// 带代理信息的包装（上游 API，gpui 的 `FakeHttpClient` 会用到）。
pub struct HttpClientWithProxy {
    client: Arc<dyn HttpClient>,
    proxy: Option<Url>,
}

impl std::ops::Deref for HttpClientWithProxy {
    type Target = Arc<dyn HttpClient>;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl HttpClientWithProxy {
    /// 用字符串形式的代理地址构造；解析失败时回落到环境变量。
    pub fn new(client: Arc<dyn HttpClient>, proxy_url: Option<String>) -> Self {
        let proxy_url = proxy_url
            .and_then(|proxy| proxy.parse().ok())
            .or_else(read_proxy_from_env);

        Self::new_url(client, proxy_url)
    }

    /// 用已解析的代理地址构造。
    pub fn new_url(client: Arc<dyn HttpClient>, proxy_url: Option<Url>) -> Self {
        Self {
            client,
            proxy: proxy_url,
        }
    }
}

impl HttpClient for HttpClientWithProxy {
    fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        self.client.send(req)
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        self.client.user_agent()
    }

    fn proxy(&self) -> Option<&Url> {
        self.proxy.as_ref()
    }

    fn type_name(&self) -> &'static str {
        self.client.type_name()
    }

    #[cfg(feature = "test-support")]
    fn as_fake(&self) -> &FakeHttpClient {
        self.client.as_fake()
    }
}

/// 带基地址的包装（上游 API）。
pub struct HttpClientWithUrl {
    base_url: Mutex<String>,
    client: HttpClientWithProxy,
}

impl std::ops::Deref for HttpClientWithUrl {
    type Target = HttpClientWithProxy;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl HttpClientWithUrl {
    /// 用字符串形式的代理地址构造。
    pub fn new(
        client: Arc<dyn HttpClient>,
        base_url: impl Into<String>,
        proxy_url: Option<String>,
    ) -> Self {
        let client = HttpClientWithProxy::new(client, proxy_url);

        Self {
            base_url: Mutex::new(base_url.into()),
            client,
        }
    }

    /// 用已解析的代理地址构造。
    pub fn new_url(
        client: Arc<dyn HttpClient>,
        base_url: impl Into<String>,
        proxy_url: Option<Url>,
    ) -> Self {
        let client = HttpClientWithProxy::new_url(client, proxy_url);

        Self {
            base_url: Mutex::new(base_url.into()),
            client,
        }
    }

    /// 当前基地址。
    pub fn base_url(&self) -> String {
        self.base_url.lock().clone()
    }

    /// 设置基地址。
    pub fn set_base_url(&self, base_url: impl Into<String>) {
        let base_url = base_url.into();
        *self.base_url.lock() = base_url;
    }

    /// 基地址 + 路径。
    pub fn build_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url(), path)
    }
}

impl HttpClient for HttpClientWithUrl {
    fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        self.client.send(req)
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        self.client.user_agent()
    }

    fn proxy(&self) -> Option<&Url> {
        self.client.proxy.as_ref()
    }

    fn type_name(&self) -> &'static str {
        self.client.type_name()
    }

    #[cfg(feature = "test-support")]
    fn as_fake(&self) -> &FakeHttpClient {
        self.client.as_fake()
    }
}

/// 从环境变量读代理地址（上游 API；本 crate 不会使用它发请求）。
pub fn read_proxy_from_env() -> Option<Url> {
    const ENV_VARS: &[&str] = &[
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ];

    ENV_VARS
        .iter()
        .find_map(|var| std::env::var(var).ok())
        .and_then(|env| env.parse().ok())
}

/// 从环境变量读 `NO_PROXY`（上游 API）。
pub fn read_no_proxy_from_env() -> Option<String> {
    const ENV_VARS: &[&str] = &["NO_PROXY", "no_proxy"];

    ENV_VARS.iter().find_map(|var| std::env::var(var).ok())
}

/// 唯一的非测试实现：拒绝一切请求，并把被拒绝的请求写进日志。
#[derive(Default)]
pub struct BlockedHttpClient;

impl BlockedHttpClient {
    /// 新建。
    pub fn new() -> Self {
        BlockedHttpClient
    }
}

impl HttpClient for BlockedHttpClient {
    fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        rejected_response(type_name::<Self>(), &req)
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    #[cfg(feature = "test-support")]
    fn as_fake(&self) -> &FakeHttpClient {
        panic!("called as_fake on {}", type_name::<Self>())
    }
}

#[cfg(feature = "test-support")]
type FakeHttpHandler = Arc<
    dyn Fn(Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>>
        + Send
        + Sync
        + 'static,
>;

/// 仅测试：把请求交给闭包处理，不碰网络。gpui 的 `TestAppContext` 需要它。
#[cfg(feature = "test-support")]
pub struct FakeHttpClient {
    handler: Mutex<Option<FakeHttpHandler>>,
    user_agent: HeaderValue,
}

#[cfg(feature = "test-support")]
impl FakeHttpClient {
    /// 用闭包处理请求。
    pub fn create<Fut, F>(handler: F) -> Arc<HttpClientWithUrl>
    where
        Fut: futures::Future<Output = Result<Response<AsyncBody>>> + Send + 'static,
        F: Fn(Request<AsyncBody>) -> Fut + Send + Sync + 'static,
    {
        Arc::new(HttpClientWithUrl {
            base_url: Mutex::new("http://test.example".into()),
            client: HttpClientWithProxy {
                client: Arc::new(Self {
                    handler: Mutex::new(Some(Arc::new(move |req| Box::pin(handler(req))))),
                    user_agent: HeaderValue::from_static(type_name::<Self>()),
                }),
                proxy: None,
            },
        })
    }

    /// 所有请求都返回 404。
    pub fn with_404_response() -> Arc<HttpClientWithUrl> {
        Self::create(|_| async move {
            Ok(Response::builder()
                .status(404)
                .body(Default::default())
                .unwrap())
        })
    }

    /// 所有请求都返回 200。
    pub fn with_200_response() -> Arc<HttpClientWithUrl> {
        Self::create(|_| async move {
            Ok(Response::builder()
                .status(200)
                .body(Default::default())
                .unwrap())
        })
    }

    /// 在旧处理函数外面再包一层。
    pub fn replace_handler<Fut, F>(&self, new_handler: F)
    where
        Fut: futures::Future<Output = Result<Response<AsyncBody>>> + Send + 'static,
        F: Fn(FakeHttpHandler, Request<AsyncBody>) -> Fut + Send + Sync + 'static,
    {
        let mut handler = self.handler.lock();
        let old_handler = handler.take().unwrap();
        *handler = Some(Arc::new(move |req| {
            Box::pin(new_handler(old_handler.clone(), req))
        }));
    }
}

#[cfg(feature = "test-support")]
impl std::fmt::Debug for FakeHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeHttpClient").finish()
    }
}

#[cfg(feature = "test-support")]
impl HttpClient for FakeHttpClient {
    fn send(&self, req: Request<AsyncBody>) -> BoxFuture<'static, Result<Response<AsyncBody>>> {
        ((self.handler.lock().as_ref().unwrap())(req)) as _
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    fn as_fake(&self) -> &FakeHttpClient {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex as StdMutex, OnceLock};

    static RECORDS: StdMutex<Vec<String>> = StdMutex::new(Vec::new());

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

    /// 装上记录型 logger（整个进程只装一次）并返回当前已有的日志条数。
    fn start_recording() -> usize {
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

    /// 验收：调用即失败，不是静默成功；错误里带上方法与 URI。
    #[test]
    fn get_is_rejected_with_an_explicit_error() {
        let client = BlockedHttpClient::new();
        let err = futures::executor::block_on(client.get(
            "https://example.com/logo.png",
            ().into(),
            true,
        ))
        .map(|_| ())
        .expect_err("stub 必须返回错误，不能返回空响应");
        let message = err.to_string();
        assert!(message.contains(BLOCKED_MESSAGE), "{message}");
        assert!(
            message.contains("https://example.com/logo.png"),
            "{message}"
        );
        assert!(message.contains("GET"), "{message}");
    }

    /// POST 同样被拒绝。
    #[test]
    fn post_json_is_rejected_with_an_explicit_error() {
        let client = BlockedHttpClient::new();
        let err = futures::executor::block_on(
            client.post_json("https://example.com/api", "{}".to_string().into()),
        )
        .map(|_| ())
        .expect_err("stub 必须返回错误");
        assert!(err.to_string().contains("POST"), "{err}");
    }

    /// 验收：被拒绝的请求必须记进日志，而且是在**构造 future** 时就记（future 丢掉也算数）。
    #[test]
    fn a_rejected_request_is_logged_even_if_the_future_is_dropped() {
        let start = start_recording();
        let client = BlockedHttpClient::new();
        let request = Builder::new()
            .uri("https://example.com/dropped")
            .body(AsyncBody::empty())
            .unwrap();
        drop(client.send(request));

        let records = logged_since(start);
        assert!(
            records
                .iter()
                .any(|r| r.starts_with("ERROR") && r.contains("https://example.com/dropped")),
            "被拒绝的请求没有写进日志：{records:?}"
        );
    }

    /// 包装类型（gpui 的 `FakeHttpClient` 走的就是这两层）不会把错误吞掉。
    #[test]
    fn the_wrappers_do_not_swallow_the_rejection() {
        let client = HttpClientWithUrl::new(
            Arc::new(BlockedHttpClient::new()),
            "https://example.com",
            None,
        );
        assert_eq!(client.build_url("/a"), "https://example.com/a");
        let err =
            futures::executor::block_on(client.get(&client.build_url("/a"), ().into(), false))
                .map(|_| ())
                .expect_err("包装层必须把错误透传出来");
        assert!(err.to_string().contains("https://example.com/a"), "{err}");
    }

    /// 空体读出来是 0 字节，不是「悄悄成功的空响应」——它只是体，不代表请求成功。
    #[test]
    fn async_body_round_trips_bytes() {
        use futures::AsyncReadExt as _;
        let mut body = AsyncBody::from("rove".to_string());
        let mut out = String::new();
        futures::executor::block_on(body.read_to_string(&mut out)).unwrap();
        assert_eq!(out, "rove");
    }
}
