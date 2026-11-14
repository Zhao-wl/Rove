//! 更新流程的状态机（issue #149 第 2、3、5 节）。
//!
//! [`Updater`] 是**同步**的：每个方法做完一件事就返回，由 `rove-ui` 丢到后台执行器上跑
//! （核心层不挑执行器，`AGENTS.md` §2）。这样它既能在真实后台线程上跑，也能在单元测试里
//! 一步步驱动。
//!
//! 一次检查的全过程：
//!
//! ```text
//! check()
//!   ├─ mode = none ───────────────→ Outcome::Disabled（**不构造传输层，不建任何连接**）
//!   ├─ GET latest.json            ─┐ 网络失败 → Outcome::Failed（静默，只写日志）
//!   ├─ GET latest.json.sig        ─┘
//!   ├─ 验签失败 / 版本号非法 / 不比当前新 → Outcome::UpToDate（只写日志）
//!   └─ 有更新 → Outcome::Available{ manifest }
//! download(&manifest)
//!   ├─ size / sha256 任一不符 → 删除下载并报错（调用方发通知，旧版本不受影响）
//!   └─ 通过 → 写 pending.json，Outcome 由调用方转成「重启以更新到 x.y.z」
//! ```

use crate::{
    download::{self, Expect},
    endpoint,
    http::{self, Transport},
    log::{timestamp, UpdateLog},
    manifest::Manifest,
    mode::UpdateMode,
    paths::{Pending, UpdatePaths},
    version::Version,
    Result, UpdateError,
};
use std::{sync::Arc, time::SystemTime};

/// 一次检查的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `rove.update.mode = "none"`：没有联网，也没有任何连接。
    Disabled,
    /// 已经是最新的（包括「清单有问题所以当作没有更新」这几种情况）。
    UpToDate,
    /// 有新版本。`required` 来自清单的 `min_version`。
    Available {
        manifest: Box<Manifest>,
        required: bool,
    },
    /// 检查失败。`silent` 为真时只写日志、不打扰用户。
    Failed { error: UpdateError },
}

impl Outcome {
    pub fn manifest(&self) -> Option<&Manifest> {
        match self {
            Outcome::Available { manifest, .. } => Some(manifest),
            _ => None,
        }
    }
}

/// 更新流程。
pub struct Updater {
    current: Version,
    mode: UpdateMode,
    paths: Option<UpdatePaths>,
    log: Arc<UpdateLog>,
    transport: Option<Arc<dyn Transport>>,
    /// 演练用的验签公钥。**只在调试构建里存在**，见
    /// [`Updater::set_test_public_key`]；发布构建里这个字段根本不存在，
    /// 验签永远用编进二进制的那一把钥匙。
    #[cfg(debug_assertions)]
    test_public_key: Option<ed25519_dalek::VerifyingKey>,
}

impl Updater {
    /// 正常构造：`current` 是本进程的版本（`CARGO_PKG_VERSION`），传输层是真的 WinHTTP。
    ///
    /// **`mode` 为 `none` 时传输层根本不会被构造**：这是「零连接」最直接的保证。
    pub fn new(current: Version, mode: UpdateMode, paths: Option<UpdatePaths>) -> Self {
        let log = Arc::new(UpdateLog::new(paths.as_ref().map(|p| p.logs_dir())));
        let transport: Option<Arc<dyn Transport>> = if mode.may_connect() {
            Some(Arc::new(http::WinHttp::new(&http::user_agent(
                &current.to_string(),
            ))))
        } else {
            None
        };
        Self {
            current,
            mode,
            paths,
            log,
            transport,
            #[cfg(debug_assertions)]
            test_public_key: None,
        }
    }

    /// 自带传输层（集成测试用本机 HTTP 服务器，单元测试用 [`crate::http::StubTransport`]）。
    pub fn with_transport(
        current: Version,
        mode: UpdateMode,
        paths: Option<UpdatePaths>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        let log = Arc::new(UpdateLog::new(paths.as_ref().map(|p| p.logs_dir())));
        Self {
            current,
            mode,
            paths,
            log,
            // `none` 时即使调用方给了传输层也不用它。
            transport: mode.may_connect().then_some(transport),
            #[cfg(debug_assertions)]
            test_public_key: None,
        }
    }

    /// 演练「v0.1.0 → v0.1.1」时换掉验签公钥（本地 HTTP 服务器 + 临时密钥对）。
    ///
    /// 这个方法**只在 `cfg(debug_assertions)` 的构建里存在**，发布的 `rove.exe`
    /// 里既没有它、也没有存放它的字段：没有任何运行期开关能换掉验签的钥匙。
    #[cfg(debug_assertions)]
    pub fn set_test_public_key(&mut self, bytes: [u8; 32]) -> Result<()> {
        self.test_public_key = Some(crate::signature::key_from_bytes(bytes)?);
        Ok(())
    }

    /// 本次验签用的公钥。
    fn verify(&self, bytes: &[u8], signature_file: &str) -> Result<()> {
        #[cfg(debug_assertions)]
        if let Some(key) = &self.test_public_key {
            return crate::signature::verify_with(key, bytes, signature_file);
        }
        crate::signature::verify(bytes, signature_file)
    }

    pub fn current_version(&self) -> &Version {
        &self.current
    }

    pub fn mode(&self) -> UpdateMode {
        self.mode
    }

    pub fn log(&self) -> &Arc<UpdateLog> {
        &self.log
    }

    pub fn paths(&self) -> Option<&UpdatePaths> {
        self.paths.as_ref()
    }

    /// 本次会话实际可用的传输层。`none` 模式下永远是 `None`。
    pub fn transport(&self) -> Option<&Arc<dyn Transport>> {
        self.transport.as_ref()
    }

    fn now(&self) -> String {
        timestamp(SystemTime::now())
    }

    /// 取一个 URL，并在日志里留一行（**每次请求都有记录**）。
    fn get(&self, transport: &dyn Transport, url: &str, limit: u64) -> Result<Vec<u8>> {
        let result = http::get_bytes(transport, url, limit);
        match &result {
            Ok(body) => self
                .log
                .request("GET", url, &format!("200 {} 字节", body.len())),
            Err(error) => self.log.request("GET", url, &error.reason()),
        }
        result
    }

    /// 检查有没有更新。
    pub fn check(&self) -> Outcome {
        let Some(transport) = self.transport.as_ref() else {
            // `none`：连传输层都没有，这里是唯一的出口。
            self.log.line("check skipped: rove.update.mode = none");
            return Outcome::Disabled;
        };
        self.log.line(&format!(
            "check start: 当前 {} / {}",
            self.current,
            self.mode.as_str()
        ));
        match self.check_inner(transport.as_ref()) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.log.line(&format!("check failed: {}", error.reason()));
                Outcome::Failed { error }
            }
        }
    }

    fn check_inner(&self, transport: &dyn Transport) -> Result<Outcome> {
        let manifest_bytes =
            self.get(transport, &endpoint::manifest_url(), http::MANIFEST_LIMIT)?;
        let signature = self.get(transport, &endpoint::signature_url(), http::MANIFEST_LIMIT)?;
        let signature = String::from_utf8(signature)
            .map_err(|_| UpdateError::Signature("签名文件不是 UTF-8".into()))?;

        // 验签 → 解析 → 比版本。任何一步失败都是「没有更新」，只写日志（第 2 节）。
        let manifest = match self
            .verify(&manifest_bytes, &signature)
            .and_then(|()| Manifest::parse(&manifest_bytes))
        {
            Ok(manifest) => manifest,
            Err(error) => {
                self.log
                    .line(&format!("ignored: {}（当作没有更新）", error.reason()));
                return Ok(Outcome::UpToDate);
            }
        };
        let version = manifest.version()?;
        if !version.is_newer_than(&self.current) {
            self.log.line(&format!(
                "up to date: 最新是 {version}，当前 {}",
                self.current
            ));
            return Ok(Outcome::UpToDate);
        }
        let required = manifest.requires_update(&self.current);
        self.log.line(&format!(
            "available: {version}{}",
            if required { "（必须更新）" } else { "" }
        ));
        Ok(Outcome::Available {
            manifest: Box::new(manifest),
            required,
        })
    }

    /// 下载清单里的安装包并校验。成功后写下待安装记录并返回它。
    ///
    /// **同时只保留一个待安装版本**：下载前把下载目录里别的东西清掉（第 5 节）。
    pub fn download(&self, manifest: &Manifest) -> Result<Pending> {
        let transport = self
            .transport
            .as_ref()
            .ok_or(UpdateError::Disabled)?
            .clone();
        let paths = self
            .paths
            .as_ref()
            .ok_or_else(|| UpdateError::Unavailable("找不到 %LOCALAPPDATA%".into()))?;
        let installer = manifest
            .installer
            .as_ref()
            .ok_or_else(|| UpdateError::Unavailable("这个版本没有安装包".into()))?;

        let dir = paths.ensure_updates_dir()?;
        paths.clear_downloads(&[]);
        self.log.line(&format!(
            "download start: {} ({} 字节)",
            installer.file, installer.size
        ));
        let result = download::download_verified(
            transport.as_ref(),
            &installer.url,
            &dir,
            &installer.file,
            &Expect::new(installer.size, installer.sha256.clone()),
        );
        self.log.request(
            "GET",
            &installer.url,
            &match &result {
                Ok(_) => format!("200 {} 字节，校验通过", installer.size),
                Err(error) => error.reason(),
            },
        );
        result?;

        let pending = Pending::from_manifest(manifest, self.now())
            .ok_or_else(|| UpdateError::Unavailable("这个版本没有安装包".into()))?;
        paths.write_pending(&pending)?;
        self.log
            .line(&format!("ready to install: {}", pending.version));
        Ok(pending)
    }

    /// 启动时读出上次留下的待安装版本（还没装、下次启动要再提示一次的那个）。
    ///
    /// 如果它已经不比当前版本新，说明**已经装上了**：把记录与下载清掉，返回 `None`。
    pub fn take_pending_after_start(&self) -> Option<Pending> {
        let paths = self.paths.as_ref()?;
        let pending = paths.read_pending()?;
        if pending.is_newer_than(&self.current) {
            self.log
                .line(&format!("pending on start: {}", pending.version));
            return Some(pending);
        }
        self.log.line(&format!(
            "installed: 当前 {}，清掉 {} 的下载",
            self.current, pending.version
        ));
        paths.discard_pending();
        None
    }

    /// 放弃待安装版本（校验失败、安装失败、用户关掉更新）。
    pub fn discard_pending(&self) {
        if let Some(paths) = &self.paths {
            paths.discard_pending();
        }
        self.log.line("pending discarded");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StubTransport;
    use crate::manifest::tests::sample;
    use crate::signature;

    fn paths() -> (tempfile::TempDir, UpdatePaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path().join("Rove"));
        (dir, paths)
    }

    /// 没有私钥，所以测试里的「验签通过」要绕过签名：用一个只在测试里可见的
    /// 替身传输 + 真实验签会失败。这里专门测**失败路径**；成功路径由
    /// `tests/rehearsal.rs` 用真实签名的清单覆盖。
    fn stub(manifest: &str, signature_line: &str) -> StubTransport {
        StubTransport::new()
            .with(&endpoint::manifest_url(), manifest.as_bytes().to_vec())
            .with(
                &endpoint::signature_url(),
                signature_line.as_bytes().to_vec(),
            )
    }

    fn updater(mode: UpdateMode, transport: StubTransport, paths: UpdatePaths) -> Updater {
        Updater::with_transport(
            Version::parse("0.1.0").unwrap(),
            mode,
            Some(paths),
            Arc::new(transport),
        )
    }

    /// 验收：`none` 时一个连接都不建——连传输层都没有，请求记录为空。
    #[test]
    fn mode_none_never_touches_the_network() {
        let (_dir, paths) = paths();
        let transport = stub(&sample("9.9.9"), "ed25519 AAAA");
        let spy = transport.clone();
        let updater = updater(UpdateMode::None, transport, paths);
        assert!(updater.transport().is_none());
        assert_eq!(updater.check(), Outcome::Disabled);
        assert!(spy.requested().is_empty(), "`none` 模式发了请求");
        // 下载也走不通。
        let manifest = Manifest::parse(sample("9.9.9").as_bytes()).unwrap();
        assert_eq!(updater.download(&manifest), Err(UpdateError::Disabled));
        assert!(spy.requested().is_empty());
    }

    /// 另外三个模式都会去取清单与签名，而且**只取这两个地址**。
    #[test]
    fn the_other_modes_fetch_exactly_the_manifest_and_its_signature() {
        for mode in [UpdateMode::Manual, UpdateMode::Start, UpdateMode::Default] {
            let (_dir, paths) = paths();
            let transport = stub(&sample("0.1.1"), "ed25519 AAAA");
            let spy = transport.clone();
            let updater = updater(mode, transport, paths);
            // 签名对不上 → 当作没有更新（见下一个测试），这里只看请求。
            updater.check();
            assert_eq!(
                spy.requested(),
                [endpoint::manifest_url(), endpoint::signature_url()],
                "{mode:?}"
            );
        }
    }

    /// 验收：签名错误 → 「不更新」，而且旧版本不受影响（没有任何文件被写）。
    #[test]
    fn a_bad_signature_means_no_update() {
        let (_dir, paths) = paths();
        let updater = updater(
            UpdateMode::Manual,
            stub(&sample("0.1.1"), "ed25519 AAAA"),
            paths.clone(),
        );
        assert_eq!(updater.check(), Outcome::UpToDate);
        assert_eq!(paths.read_pending(), None);
    }

    /// 验收：版本号非法 → 「不更新」。
    #[test]
    fn a_malformed_version_means_no_update() {
        let (_dir, paths) = paths();
        let broken = r#"{"schema":1,"product":"rove","version":"not a version"}"#;
        let updater = updater(UpdateMode::Manual, stub(broken, "ed25519 AAAA"), paths);
        assert_eq!(updater.check(), Outcome::UpToDate);
    }

    /// 网络失败是静默的，而且日志里留了一行。
    #[test]
    fn network_failures_are_silent_but_logged() {
        let (_dir, paths) = paths();
        let transport = StubTransport::new().failing(
            &endpoint::manifest_url(),
            UpdateError::Network("连不上".into()),
        );
        let updater = updater(UpdateMode::Default, transport, paths);
        let outcome = updater.check();
        let Outcome::Failed { error } = outcome else {
            panic!("应该是失败，实际 {outcome:?}");
        };
        assert!(error.is_silent());
        let log = std::fs::read_to_string(updater.log().path().unwrap()).unwrap();
        assert!(log.contains("request GET"), "{log}");
        assert!(log.contains("check failed"), "{log}");
    }

    /// GitHub 还没有这个文件（404）也是静默失败。
    #[test]
    fn a_missing_manifest_is_a_silent_failure() {
        let (_dir, paths) = paths();
        let updater = updater(UpdateMode::Default, StubTransport::new(), paths);
        let Outcome::Failed { error } = updater.check() else {
            panic!("应该是失败");
        };
        assert_eq!(error, UpdateError::Http(404));
        assert!(error.is_silent());
    }

    /// 启动时：待安装版本还比当前新 → 再提示一次；已经装上了 → 清掉记录。
    #[test]
    fn a_pending_update_is_offered_again_or_cleaned_up() {
        let (_dir, paths) = paths();
        let manifest = Manifest::parse(sample("0.1.1").as_bytes()).unwrap();
        let pending = Pending::from_manifest(&manifest, "now".into()).unwrap();
        paths.write_pending(&pending).unwrap();
        std::fs::write(pending.installer_path(&paths), b"setup").unwrap();

        let older = updater(UpdateMode::Default, StubTransport::new(), paths.clone());
        assert_eq!(older.take_pending_after_start().as_ref(), Some(&pending));

        // 装上之后（当前版本 = 0.1.1）记录与安装包都被清掉。
        let newer = Updater::with_transport(
            Version::parse("0.1.1").unwrap(),
            UpdateMode::Default,
            Some(paths.clone()),
            Arc::new(StubTransport::new()),
        );
        assert_eq!(newer.take_pending_after_start(), None);
        assert!(!pending.installer_path(&paths).exists());
        assert_eq!(paths.read_pending(), None);
    }

    /// 公钥是编进二进制的那一把（验收材料的一部分）。
    #[test]
    fn the_update_uses_the_embedded_public_key() {
        assert!(signature::verifying_key().is_ok());
        assert_eq!(signature::fingerprint().unwrap().len(), 16);
    }
}
