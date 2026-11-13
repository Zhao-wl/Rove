//! #35 常驻索引进程：拉起 `rove-indexer-cs --serve`，转发文件变化，把它的回复与通知交给生命周期。
//!
//! 协议是 stdin / stdout 上逐行的 JSON-RPC 2.0（说明见 `docs/CS_INDEX_FORMAT.md`「常驻模式与覆盖层」）。
//! 本模块只管“消息内容”：何时拉起、何时关闭由 `service`（#36）决定。版本协商（#43）：第一条消息
//! `hello` 带组件的协议区间，按 `component::negotiate_protocol` 取交集；交集为空就报告
//! [`ResidentEvent::Incompatible`] 并放弃这个进程，不硬跑。
//!
//! 重绑结果不经过管道：进程把覆盖层段追加到与基础索引配套的日志里，回复只说明段已写入，
//! 生命周期再从日志读出新段、合并、发布新的快照。
use super::{
    component::{negotiate_protocol, Incompatible},
    log::IndexLog,
    process::{BuildFailure, IndexerComponent},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

/// 本端能说的最高协议版本（`tools/rove-indexer-cs/Server.cs` 的 `Protocol` 与它同步演进）。
pub const PROTOCOL: u32 = 1;
/// 本端还能说的最低协议版本。
pub const PROTOCOL_MIN: u32 = 1;

/// 进程第一条消息 `hello` 的内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub protocol: u32,
    pub protocol_min: u32,
    pub indexer_version: String,
}

/// 常驻进程的策略参数。
#[derive(Clone, Debug, PartialEq)]
pub struct ResidentOptions {
    /// 空闲多久后自动退出（退出前把覆盖层日志刷盘）。
    ///
    /// 默认 10 分钟：M0 实测中型项目 1 小时改动 790 个文件，改动是成批到来的（保存、AI 批量修改、切分支），
    /// 批与批之间常有几分钟的空档；10 分钟覆盖绝大多数空档，避免反复冷启动（冷启动要重新加载项目模型、
    /// 预热编译，大型项目的第一次重绑要多花数秒）。空闲时进程只占项目模型与依赖镜像，内存远低于全量索引
    /// 峰值，所以多留几分钟的代价很小；更久则没有必要——下一次改动会自动重新拉起。
    pub idle_exit: Duration,
    /// 空闲多久后丢掉预热的编译（只保留项目模型与依赖镜像）。
    pub trim: Duration,
    /// 后台重绑的 CPU 占空比（0–1）。
    pub background_duty: f64,
}

impl Default for ResidentOptions {
    fn default() -> Self {
        Self {
            idle_exit: Duration::from_secs(600),
            trim: Duration::from_secs(120),
            background_duty: 0.5,
        }
    }
}

/// 拉起常驻进程所需的信息。
#[derive(Clone, Debug)]
pub struct SessionRequest {
    pub project_root: PathBuf,
    /// 当前世代的基础索引。
    pub base: PathBuf,
    /// 与它配套的覆盖层日志。
    pub journal: PathBuf,
    pub options: ResidentOptions,
}

/// 常驻进程发给生命周期的事件。
#[derive(Clone, Debug, PartialEq)]
pub enum ResidentEvent {
    /// `initialize` 成功。
    Ready { indexer_version: String },
    /// `hello` 里的协议区间与本端没有交集（#43）：组件太旧或太新，进程已被放弃。
    Incompatible(Incompatible),
    /// 一次 `changed` 请求处理完：`files` 已写入覆盖层；`locked` 暂时读不到（被独占），稍后由后台重试；
    /// `rebuild_required`：项目结构文件变了，需要全量重建。
    Applied {
        id: u64,
        files: Vec<String>,
        locked: Vec<String>,
        rebuild_required: bool,
        queued: u32,
        elapsed_ms: f64,
    },
    /// 后台重绑（影响分析、被独占文件的重试）写了新段。
    Overlay { files: Vec<String>, queued: u32 },
    /// 请求失败（协议错误、重绑异常）。
    Failed { id: Option<u64>, message: String },
    /// 进程退出（空闲退出时 `idle` 为 true）。
    Exited { idle: bool },
}

/// 一个运行中的常驻进程。
pub trait ResidentSession: Send {
    /// 通知文件已变更 / 新增 / 删除（项目相对路径）。返回 false 表示进程已不可用。
    fn changed(&mut self, id: u64, paths: &[String]) -> bool;
    /// 关闭：请求进程落盘退出，稍等后强制结束。
    fn shutdown(&mut self);
}

/// 拉起常驻进程。正式实现是 [`ProcessResident`]；测试用假实现。
pub trait IncrementalIndexer: Send + Sync {
    fn start(
        &self,
        request: &SessionRequest,
        events: Box<dyn Fn(ResidentEvent) + Send + Sync>,
        log: Arc<IndexLog>,
    ) -> Result<Box<dyn ResidentSession>, BuildFailure>;
}

/// 用索引进程的 `--serve` 模式。
pub struct ProcessResident {
    locate: Box<dyn Fn() -> Result<IndexerComponent, BuildFailure> + Send + Sync>,
}

impl ProcessResident {
    pub fn new(
        locate: impl Fn() -> Result<IndexerComponent, BuildFailure> + Send + Sync + 'static,
    ) -> Self {
        Self {
            locate: Box::new(locate),
        }
    }
}

struct ProcessSession {
    child: Child,
    stdin: Option<ChildStdin>,
    log: Arc<IndexLog>,
}

fn send(stdin: &mut ChildStdin, message: &Value) -> bool {
    let mut line = message.to_string();
    line.push('\n');
    stdin.write_all(line.as_bytes()).is_ok() && stdin.flush().is_ok()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// 解析一行进程输出成事件。`initialize` 的 id 固定为 0。
pub fn parse_message(line: &str, hello: &mut Option<Hello>) -> Option<ResidentEvent> {
    let msg: Value = serde_json::from_str(line).ok()?;
    if let Some(method) = msg.get("method").and_then(Value::as_str) {
        let params = &msg["params"];
        return match method {
            "hello" => {
                let protocol = params["protocol"]
                    .as_u64()
                    .and_then(|p| u32::try_from(p).ok())
                    .unwrap_or(0);
                *hello = Some(Hello {
                    protocol,
                    protocol_min: params["protocol_min"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(protocol),
                    indexer_version: params["indexer_version"].as_str().unwrap_or("").to_owned(),
                });
                None
            }
            "overlay" => Some(ResidentEvent::Overlay {
                files: strings(&params["files"]),
                queued: params["impact"]["queued"].as_u64().unwrap_or(0) as u32,
            }),
            "exiting" => Some(ResidentEvent::Exited {
                idle: params["reason"] == "idle",
            }),
            _ => None,
        };
    }
    let id = msg.get("id").and_then(Value::as_u64);
    if let Some(error) = msg.get("error") {
        return Some(ResidentEvent::Failed {
            id,
            message: error["message"].as_str().unwrap_or("error").to_owned(),
        });
    }
    let result = msg.get("result")?;
    match id? {
        0 => Some(ResidentEvent::Ready {
            indexer_version: result["indexer_version"].as_str().unwrap_or("").to_owned(),
        }),
        id if result.get("files").is_some() => Some(ResidentEvent::Applied {
            id,
            files: strings(&result["files"]),
            locked: strings(&result["locked"]),
            rebuild_required: result["rebuild_required"].as_bool().unwrap_or(false),
            queued: result["impact"]["queued"].as_u64().unwrap_or(0) as u32,
            elapsed_ms: result["elapsed_ms"].as_f64().unwrap_or(0.0),
        }),
        _ => None,
    }
}

impl IncrementalIndexer for ProcessResident {
    fn start(
        &self,
        request: &SessionRequest,
        events: Box<dyn Fn(ResidentEvent) + Send + Sync>,
        log: Arc<IndexLog>,
    ) -> Result<Box<dyn ResidentSession>, BuildFailure> {
        let component = (self.locate)()?;
        let mut command = Command::new(&component.program);
        command
            .args(&component.prefix)
            .arg("--serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // 正常优先级：前台重绑要快；后台队列由进程自己降低线程优先级并按占空比休眠。
            command.creation_flags(CREATE_NO_WINDOW);
        }
        log.line(&format!(
            "resident: start {} {:?} --serve",
            component.program.display(),
            component.prefix
        ));
        let mut child = command
            .spawn()
            .map_err(|e| BuildFailure::SpawnFailed(e.to_string()))?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let err_log = log.clone();
        std::thread::Builder::new()
            .name("rove-resident-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    let Ok(line) = line else { break };
                    err_log.line(&format!("resident: {line}"));
                }
            })
            .map_err(|e| BuildFailure::SpawnFailed(e.to_string()))?;
        let out_log = log.clone();
        std::thread::Builder::new()
            .name("rove-resident-stdout".into())
            .spawn(move || {
                let mut hello = None;
                let mut idle = false;
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let had_hello = hello.is_some();
                    let event = parse_message(&line, &mut hello);
                    if !had_hello {
                        if let Some(h) = &hello {
                            if let Err(mut why) = negotiate_protocol(h.protocol, h.protocol_min) {
                                why.component_version = Some(h.indexer_version.clone());
                                out_log.line(&format!("resident: {why}"));
                                events(ResidentEvent::Incompatible(why));
                                break;
                            }
                        }
                    }
                    if let Some(event) = event {
                        if let ResidentEvent::Exited { idle: i } = event {
                            idle = i;
                            continue;
                        }
                        events(event);
                    }
                }
                events(ResidentEvent::Exited { idle });
            })
            .map_err(|e| BuildFailure::SpawnFailed(e.to_string()))?;
        let init = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocol": component.protocol.unwrap_or(PROTOCOL),
                "project": request.project_root,
                "base": request.base,
                "journal": request.journal,
                "idle_exit_seconds": request.options.idle_exit.as_secs_f64(),
                "trim_seconds": request.options.trim.as_secs_f64(),
                "background_duty": request.options.background_duty,
            }
        });
        if !send(&mut stdin, &init) {
            let _ = child.kill();
            return Err(BuildFailure::SpawnFailed("cannot write to indexer".into()));
        }
        Ok(Box::new(ProcessSession {
            child,
            stdin: Some(stdin),
            log,
        }))
    }
}

impl ResidentSession for ProcessSession {
    fn changed(&mut self, id: u64, paths: &[String]) -> bool {
        let Some(stdin) = &mut self.stdin else {
            return false;
        };
        let changes: Vec<Value> = paths.iter().map(|p| json!({ "path": p })).collect();
        send(
            stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": "changed", "params": { "changes": changes } }),
        )
    }

    fn shutdown(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = send(
                &mut stdin,
                &json!({ "jsonrpc": "2.0", "id": u64::MAX, "method": "shutdown" }),
            );
            // 关闭 stdin：进程即使正在处理请求，也会在读到 EOF 后退出。
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                self.log.line("resident: exited");
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.log.line("resident: terminated");
    }
}

impl Drop for ProcessSession {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            self.shutdown();
        }
    }
}

/// 生命周期记录的未完成请求：路径 → 最新请求号。
#[derive(Default, Debug)]
pub struct Pending {
    latest: HashMap<String, u64>,
}

impl Pending {
    pub fn request(&mut self, id: u64, paths: &[String]) {
        for p in paths {
            self.latest.insert(p.clone(), id);
        }
    }
    /// 请求 `id` 已处理：返回不再有更新请求在途的路径（可以清除过期标记）。
    pub fn applied(&mut self, id: u64, files: &[String]) -> Vec<String> {
        let mut done = Vec::new();
        for f in files {
            if self.latest.get(f).is_some_and(|&latest| latest <= id) {
                self.latest.remove(f);
                done.push(f.clone());
            }
        }
        done
    }
    /// 后台写入的段（被独占文件的重试）：这些路径的在途请求都已满足。
    pub fn completed(&mut self, files: &[String]) -> Vec<String> {
        files
            .iter()
            .filter(|f| self.latest.remove(*f).is_some())
            .cloned()
            .collect()
    }
    pub fn len(&self) -> usize {
        self.latest.len()
    }
    pub fn is_empty(&self) -> bool {
        self.latest.is_empty()
    }
    pub fn clear(&mut self) {
        self.latest.clear();
    }
}

/// 测试辅助：解析一组输出行。
#[doc(hidden)]
pub fn parse_lines(lines: &[&str]) -> Vec<ResidentEvent> {
    let mut hello = None;
    lines
        .iter()
        .filter_map(|l| parse_message(l, &mut hello))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_from_the_indexer() {
        let events = parse_lines(&[
            r#"{"jsonrpc":"2.0","method":"hello","params":{"protocol":1,"protocol_min":1,"indexer_version":"1.2.0.0"}}"#,
            r#"{"jsonrpc":"2.0","id":0,"result":{"protocol":1,"indexer_version":"1.2.0.0"}}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"files":["Assets/A.cs"],"locked":["Assets/L.cs"],"rebuild_required":false,"impact":{"queued":3},"elapsed_ms":12.5}}"#,
            r#"{"jsonrpc":"2.0","method":"overlay","params":{"files":["Assets/B.cs"],"impact":{"queued":2}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"error":{"code":-32004,"message":"boom"}}"#,
            r#"{"jsonrpc":"2.0","method":"exiting","params":{"reason":"idle"}}"#,
            "not json",
        ]);
        assert_eq!(
            events,
            vec![
                ResidentEvent::Ready {
                    indexer_version: "1.2.0.0".into()
                },
                ResidentEvent::Applied {
                    id: 7,
                    files: vec!["Assets/A.cs".into()],
                    locked: vec!["Assets/L.cs".into()],
                    rebuild_required: false,
                    queued: 3,
                    elapsed_ms: 12.5,
                },
                ResidentEvent::Overlay {
                    files: vec!["Assets/B.cs".into()],
                    queued: 2
                },
                ResidentEvent::Failed {
                    id: Some(8),
                    message: "boom".into()
                },
                ResidentEvent::Exited { idle: true },
            ]
        );
    }

    #[test]
    fn pending_requests_clear_only_when_the_latest_is_applied() {
        let mut p = Pending::default();
        p.request(1, &["A".into(), "B".into()]);
        p.request(2, &["A".into()]);
        assert_eq!(
            p.applied(1, &["A".into(), "B".into()]),
            vec!["B".to_owned()]
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p.applied(2, &["A".into()]), vec!["A".to_owned()]);
        assert!(p.is_empty());
        p.request(3, &["L".into()]);
        assert!(p.applied(3, &[]).is_empty());
        assert_eq!(p.completed(&["L".into(), "X".into()]), vec!["L".to_owned()]);
    }
}
