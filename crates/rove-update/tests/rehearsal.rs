//! 「v0.1.0 → v0.1.1」的演练（issue #149 验收第 4 条的可自动化部分）。
//!
//! **不会在 GitHub 上建任何 Release**：演练用的是
//!
//! - 一个只监听 `127.0.0.1` 的临时 HTTP 服务器（本文件里的 [`LocalServer`]，几十行，
//!   收到请求就回一份内存里的字节）；
//! - 测试里现生成的 ed25519 密钥对（签名真的算、真的验），通过只在调试构建里存在的
//!   `Updater::set_test_public_key` 告诉更新端；
//! - 只在调试构建里生效的地址覆盖 `ROVE_UPDATE_TEST_BASE_URL`（见
//!   `crates/rove-update/src/endpoint.rs`）。
//!
//! 走的是**真实的 WinHTTP 传输**（`http://127.0.0.1:<端口>`），所以这条测试同时验证了
//! 网络层本身：解析地址、发请求、读状态码、读响应体。
//!
//! 干净虚拟机上「装 v0.1.0 → 装 v0.1.1 → 会话原样恢复」的那一轮是人工事项
//! （`docs/UPDATE.md` 最后一节），这里覆盖的是「检查 → 下载 → 校验 → 提示 → 重启后清理」。
//!
//! 运行：`cargo test -p rove-update --test rehearsal -j 1`

use ed25519_dalek::{Signer, SigningKey};
use rove_update::{
    component::{self, Installed},
    download::sha256,
    endpoint,
    http::{Transport, WinHttp},
    install,
    mode::UpdateMode,
    paths::UpdatePaths,
    version::Version,
    Outcome, UpdateError, Updater,
};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

// ------------------------------------------------------------ 本地 HTTP 服务器

type Files = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

/// 只监听 127.0.0.1 的最小 HTTP/1.1 服务器：GET 一个已知路径就回对应的字节。
///
/// 先起服务器拿到端口，再 [`LocalServer::publish`] 把产物放上去——清单里的 URL 必须
/// 指回同一台服务器，所以顺序只能是这样。
struct LocalServer {
    base_url: String,
    files: Files,
    stop: Arc<AtomicBool>,
    hits: Arc<Mutex<Vec<String>>>,
}

impl LocalServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("本机端口");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let files: Files = Arc::new(Mutex::new(BTreeMap::new()));
        let (thread_stop, thread_hits, thread_files) = (stop.clone(), hits.clone(), files.clone());
        thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        serve(stream, &thread_files, &thread_hits);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            files,
            stop,
            hits,
        }
    }

    /// 「发布」：把这一版的全部产物放上去。
    fn publish(&self, version: &str, component_version: Option<&str>) -> [u8; 32] {
        let release = build_release(&self.base_url, version, component_version);
        *self.files.lock().unwrap() = release.files;
        release.public_key
    }

    /// 把某个产物换成别的内容（演练「安装包被动过手脚」）。
    fn tamper(&self, suffix: &str, body: &[u8]) {
        let mut files = self.files.lock().unwrap();
        let key = files
            .keys()
            .find(|k| k.ends_with(suffix))
            .cloned()
            .expect("产物在");
        files.insert(key, body.to_vec());
    }

    fn requested(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(mut stream: TcpStream, files: &Files, hits: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    // 把请求头读完，否则客户端可能还在等。
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line.trim().is_empty() => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
    hits.lock().unwrap().push(path.clone());
    let body = files.lock().unwrap().get(&path).cloned();
    let response = match body.as_ref() {
        Some(body) => {
            let mut head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            head.extend_from_slice(body);
            head
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

// ------------------------------------------------------------ 发布侧的替身

/// 测试里的「发布流水线」：造出 latest.json + 签名 + 安装包 + 组件包，
/// 字段形状与 `cargo xtask release` 的产出一致（docs/RELEASE.md §4）。
struct Release {
    files: BTreeMap<String, Vec<u8>>,
    public_key: [u8; 32],
}

fn build_release(base_url: &str, version: &str, component_version: Option<&str>) -> Release {
    let signing = SigningKey::from_bytes(&[42u8; 32]);
    let installer_body = format!("pretend Rove {version} installer").into_bytes();
    let installer_file = format!("Rove-{version}-win-x64-setup.exe");
    let mut components = String::from("[]");
    let mut files = BTreeMap::new();

    if let Some(component_version) = component_version {
        let archive = component_package(component_version);
        let file = format!("rove-indexer-cs-{component_version}-win-x64.zip");
        components = format!(
            r#"[{{"id":"indexer-cs","version":"{component_version}","file":"{file}","url":"{base_url}/{file}","size":{},"sha256":"{}"}}]"#,
            archive.len(),
            sha256(&archive)
        );
        files.insert(format!("/{file}"), archive);
    }

    let manifest = format!(
        r#"{{
  "schema": 1,
  "product": "rove",
  "channel": "stable",
  "version": "{version}",
  "released": "2026-11-14",
  "commit": "0123456789abcdef0123456789abcdef01234567",
  "notes_url": "{base_url}/notes",
  "installer": {{
    "file": "{installer_file}",
    "url": "{base_url}/{installer_file}",
    "size": {},
    "sha256": "{}",
    "silent_args": ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL", "/SP-"]
  }},
  "portable": null,
  "components": {components}
}}
"#,
        installer_body.len(),
        sha256(&installer_body)
    );
    let signature =
        rove_update::signature::signature_line(&signing.sign(manifest.as_bytes()).to_bytes());

    files.insert("/latest.json".into(), manifest.into_bytes());
    files.insert("/latest.json.sig".into(), signature.into_bytes());
    files.insert(format!("/{installer_file}"), installer_body);
    Release {
        files,
        public_key: signing.verifying_key().to_bytes(),
    }
}

/// 一个最小的组件包（zip，只有组件清单与可执行文件）。
fn component_package(version: &str) -> Vec<u8> {
    let manifest = format!(
        r#"{{"component":"indexer-cs","version":"{version}","protocol":1,"protocol_min":1,"index_format":1}}"#
    );
    zip_of(&[
        ("component.json", manifest.as_bytes()),
        ("rove-indexer-cs.exe", b"MZ pretend"),
    ])
}

/// 现造一个「存储」方式的 zip（与 `src/zip.rs` 的测试同一段代码，这里独立一份，
/// 免得测试去依赖别人的私有实现）。
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&[0u8; 4]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_at = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

// ------------------------------------------------------------ 演练

/// 地址覆盖是进程级的环境变量，所以这些测试串行跑（一个互斥量）。
fn endpoint_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn make_updater(
    base_url: &str,
    current: &str,
    mode: UpdateMode,
    paths: UpdatePaths,
    public_key: [u8; 32],
) -> Updater {
    std::env::set_var(endpoint::TEST_BASE_URL_ENV, base_url);
    let transport: Arc<dyn Transport> = Arc::new(WinHttp::new("Rove/test"));
    let mut updater = Updater::with_transport(
        Version::parse(current).unwrap(),
        mode,
        Some(paths),
        transport,
    );
    updater.set_test_public_key(public_key).unwrap();
    updater
}

/// 验收：装着 v0.1.0 → 发布 v0.1.1 → 自动检查、下载、校验、提示 → 重启后是 v0.1.1。
#[test]
fn v0_1_0_updates_to_v0_1_1_over_a_local_server() {
    let _guard = endpoint_lock();
    let temp = tempfile::tempdir().unwrap();
    let paths = UpdatePaths::new(temp.path().join("Rove"));

    // 「发布 v0.1.1」：本机 HTTP 服务器上放一份签名过的清单与安装包。
    let server = LocalServer::start();
    let public_key = server.publish("0.1.1", Some("1.3.0.0"));

    let updater = make_updater(
        &server.base_url,
        "0.1.0",
        UpdateMode::Default,
        paths.clone(),
        public_key,
    );

    // 1. 检查：走真的 WinHTTP，取清单与签名两个文件。
    let outcome = updater.check();
    let Outcome::Available { manifest, required } = &outcome else {
        panic!("应该发现 0.1.1，实际 {outcome:?}");
    };
    assert_eq!(manifest.version, "0.1.1");
    assert!(!required, "没有 min_version 就不是必须更新");
    assert_eq!(
        server.requested(),
        ["/latest.json", "/latest.json.sig"],
        "一次检查只取这两个地址"
    );

    // 2. 后台下载 + 校验，留下待安装记录。
    let pending = updater.download(manifest).unwrap();
    assert_eq!(pending.version, "0.1.1");
    assert!(pending.installer_path(&paths).is_file());
    // 静默参数逐字来自清单（与 #76 对齐）。
    assert_eq!(pending.silent_args[0], "/VERYSILENT");
    assert_eq!(pending.component_version.as_deref(), Some("1.3.0.0"));

    // 3. 「重启以更新」会跑的那一行命令（这里只检查内容，不真的装）。
    let line = install::command_line(
        &pending.installer_path(&paths),
        &pending.silent_args,
        std::path::Path::new(r"C:\Local\Programs\Rove\rove.exe"),
    );
    assert!(line.contains("/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /SP-"));
    assert!(line.contains(r#"start "" "C:\Local\Programs\Rove\rove.exe""#));

    // 4. 没重启之前：下次启动还会再提示一次（issue 第 3 节）。
    let again = updater.take_pending_after_start();
    assert_eq!(again.map(|p| p.version), Some("0.1.1".into()));

    // 5. 重启之后真的是 0.1.1：记录与下载被清掉，不再提示。
    let after = make_updater(
        &server.base_url,
        "0.1.1",
        UpdateMode::Default,
        paths.clone(),
        public_key,
    );
    assert_eq!(after.take_pending_after_start(), None);
    assert_eq!(paths.read_pending(), None);
    // 再检查一次就是「已经最新」。
    assert_eq!(after.check(), Outcome::UpToDate);

    // 6. 日志里每次请求都有一行。
    let log = std::fs::read_to_string(after.log().path().unwrap()).unwrap();
    assert!(log.contains("request GET http://127.0.0.1"), "{log}");
    assert!(log.contains("latest.json.sig"), "{log}");
    std::env::remove_var(endpoint::TEST_BASE_URL_ENV);
}

/// 验收：已安装 C# 索引组件时，组件随更新升级；旧版本在下次启动后清理。
#[test]
fn an_installed_component_is_upgraded_and_the_old_one_is_cleaned_up() {
    let _guard = endpoint_lock();
    let temp = tempfile::tempdir().unwrap();
    let paths = UpdatePaths::new(temp.path().join("Rove"));
    let server = LocalServer::start();
    let public_key = server.publish("0.1.1", Some("1.3.0.0"));

    // 本机装着 1.2.0.0。
    let root = temp.path().join("components").join("indexer-cs");
    std::fs::create_dir_all(root.join("1.2.0.0")).unwrap();
    std::fs::write(root.join("1.2.0.0").join("component.json"), b"{}").unwrap();
    let installed = Installed {
        root: root.clone(),
        version: Some("1.2.0.0".into()),
        manifest_file: "component.json".into(),
    };

    let updater = make_updater(
        &server.base_url,
        "0.1.0",
        UpdateMode::Default,
        paths.clone(),
        public_key,
    );
    let Outcome::Available { manifest, .. } = updater.check() else {
        panic!("应该发现更新");
    };
    let component = manifest.indexer_component().unwrap();
    assert!(matches!(
        component::plan(Some(component), &installed),
        component::Plan::Update(_)
    ));

    let dir = component::install(
        updater.transport().unwrap().as_ref(),
        component,
        &paths.ensure_updates_dir().unwrap(),
        &installed,
    )
    .unwrap();
    assert_eq!(dir, root.join("1.3.0.0"));
    assert!(dir.join("rove-indexer-cs.exe").is_file());
    // 新旧并存，直到下次启动清理旧的。
    assert!(root.join("1.2.0.0").is_dir());
    assert_eq!(component::cleanup_old(&installed, "1.3.0.0"), 1);
    assert!(!root.join("1.2.0.0").exists());

    // 没装组件的机器不下载任何组件。
    let empty = Installed {
        root: temp.path().join("none"),
        version: None,
        manifest_file: "component.json".into(),
    };
    assert_eq!(
        component::plan(Some(component), &empty),
        component::Plan::NotInstalled
    );
    std::env::remove_var(endpoint::TEST_BASE_URL_ENV);
}

/// 验收：安装包被动过手脚（SHA256 不符）→ 不更新，下载被删掉，旧版本不受影响。
#[test]
fn a_tampered_installer_is_refused_and_deleted() {
    let _guard = endpoint_lock();
    let temp = tempfile::tempdir().unwrap();
    let paths = UpdatePaths::new(temp.path().join("Rove"));
    let server = LocalServer::start();
    let public_key = server.publish("0.1.1", None);
    // 服务器上的安装包换成别的内容，清单（已签名）里的 sha256 保持不变。
    server.tamper("setup.exe", b"this is not the installer we signed");

    let updater = make_updater(
        &server.base_url,
        "0.1.0",
        UpdateMode::Default,
        paths.clone(),
        public_key,
    );
    let Outcome::Available { manifest, .. } = updater.check() else {
        panic!("应该发现更新");
    };
    let error = updater.download(&manifest).unwrap_err();
    assert!(matches!(error, UpdateError::Integrity(_)), "{error:?}");
    assert!(!error.is_silent(), "校验失败要发通知");
    // 没有待安装记录，下载目录里什么都没留下。
    assert_eq!(paths.read_pending(), None);
    let left = std::fs::read_dir(paths.updates_dir()).unwrap().count();
    assert_eq!(left, 0, "下载的文件要删掉");
    std::env::remove_var(endpoint::TEST_BASE_URL_ENV);
}

/// 验收：清单签名不对（换了一把钥匙签的）→ 当作没有更新，什么都不下载。
#[test]
fn a_manifest_signed_with_another_key_is_ignored() {
    let _guard = endpoint_lock();
    let temp = tempfile::tempdir().unwrap();
    let paths = UpdatePaths::new(temp.path().join("Rove"));
    let server = LocalServer::start();
    server.publish("0.1.1", None);

    // 用另一把公钥去验：签名必然对不上。
    let other = SigningKey::from_bytes(&[7u8; 32])
        .verifying_key()
        .to_bytes();
    let updater = make_updater(
        &server.base_url,
        "0.1.0",
        UpdateMode::Default,
        paths.clone(),
        other,
    );
    assert_eq!(updater.check(), Outcome::UpToDate);
    assert_eq!(paths.read_pending(), None);
    std::env::remove_var(endpoint::TEST_BASE_URL_ENV);
}

/// 验收：`rove.update.mode = "none"` 时整个会话零连接——服务器一个请求都没收到。
#[test]
fn mode_none_makes_no_connection_at_all() {
    let _guard = endpoint_lock();
    let temp = tempfile::tempdir().unwrap();
    let paths = UpdatePaths::new(temp.path().join("Rove"));
    let server = LocalServer::start();
    server.publish("0.1.1", None);

    std::env::set_var(endpoint::TEST_BASE_URL_ENV, &server.base_url);
    let updater = Updater::with_transport(
        Version::parse("0.1.0").unwrap(),
        UpdateMode::None,
        Some(paths),
        Arc::new(WinHttp::new("Rove/test")),
    );
    assert!(updater.transport().is_none(), "`none` 不该有传输层");
    assert_eq!(updater.check(), Outcome::Disabled);
    assert!(
        server.requested().is_empty(),
        "`none` 模式下服务器收到了请求：{:?}",
        server.requested()
    );
    std::env::remove_var(endpoint::TEST_BASE_URL_ENV);
}
