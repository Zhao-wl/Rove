//! 拉起与监控索引进程（#36 目标 5）。
//!
//! - 组件位置只在 [`locate_indexer`] 里查找，规则（安装目录、版本目录、兼容判定）在 `component`（#43）。
//! - 进程的 stdout/stderr 逐行写进索引日志，同时解析进度（见 [`Progress::parse`]）。
//! - 取消（关项目、重建）时结束进程；崩溃、非零退出码由调用方（`service`）重试一次再降级。
//! - #35 的常驻进程用同一个组件（`--serve`，见 `resident`）：空闲退出由进程自己决定，`service` 在
//!   有文件变化时重新拉起。
use super::{
    component::{discover, Discovery, Incompatible, Roots},
    log::IndexLog,
    resident::{IncrementalIndexer, ProcessResident},
};
use rove_base::Cancellation;
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

/// 环境变量：指向索引进程（`rove-indexer-cs.exe` 或 `.dll`），优先于开发路径。
pub const INDEXER_ENV: &str = "ROVE_INDEXER_CS";

/// 一个可运行的索引组件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexerComponent {
    pub program: PathBuf,
    /// 放在项目参数之前的参数（用 `dotnet` 运行 `.dll` 时是 dll 路径）。
    pub prefix: Vec<OsString>,
    /// 清单里的组件版本（环境变量覆盖的组件没有清单，为 `None`）。
    pub version: Option<[u16; 4]>,
    /// 按清单协商出的协议版本；`None` 时 `initialize` 用 `PROTOCOL`，以 `hello` 为准（#43）。
    pub protocol: Option<u32>,
}

impl IndexerComponent {
    pub fn from_path(path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        let is_dll = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"));
        Some(if is_dll {
            Self {
                program: PathBuf::from("dotnet"),
                prefix: vec![path.as_os_str().to_owned()],
                version: None,
                protocol: None,
            }
        } else {
            Self {
                program: path.to_owned(),
                prefix: Vec::new(),
                version: None,
                protocol: None,
            }
        })
    }
}

/// 查找索引组件：规则见 `component` 模块（环境变量覆盖 → 用户级安装目录 → `rove.exe` 旁的 `components`，
/// 选版本最高的兼容版本）。没有组件 → `Missing`，有但不兼容 → `Incompatible`。
pub fn locate_indexer() -> Result<IndexerComponent, BuildFailure> {
    locate_in(&Roots::current())
}

/// 同 [`locate_indexer`]，查找位置由调用方给出（测试与 #76 的安装验证用）。
pub fn locate_in(roots: &Roots) -> Result<IndexerComponent, BuildFailure> {
    match discover(roots) {
        Discovery::Found(c) => Ok(c),
        Discovery::NotInstalled => Err(BuildFailure::Missing),
        Discovery::Incompatible(why) => Err(BuildFailure::Incompatible(why)),
    }
}

/// 一次构建要做的事。
#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub project_root: PathBuf,
    /// 新世代索引文件的路径（索引进程先写同目录临时文件再改名）。
    pub output: PathBuf,
}

/// 构建进度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// 0–100。
    pub percent: u8,
    /// 参与索引的源文件总数（索引进程报告后才有）。
    pub files: Option<u32>,
}

/// 索引进程目前在 stderr 上输出阶段行（`[parse_syntax] 1.2s …`）与 `files: N, …`；按阶段
/// 完成的大致耗时比例换算成百分比。索引进程将来若输出 `progress <done> <total>`，直接采用。
const PHASES: &[(&str, u8)] = &[
    ("[parse_csproj]", 5),
    ("[parse_syntax]", 25),
    ("[create_compilations", 40),
    ("[bind_and_collect]", 90),
    ("[write_index]", 99),
];

impl Progress {
    /// 用一行输出更新进度；返回是否有变化。
    pub fn parse(&mut self, line: &str) -> bool {
        let before = *self;
        let text = line.trim();
        if let Some(rest) = text.strip_prefix("progress ") {
            let mut parts = rest.split_whitespace().map(|p| p.parse::<u64>());
            if let (Some(Ok(done)), Some(Ok(total))) = (parts.next(), parts.next()) {
                if let Some(percent) = (done.min(total) * 100).checked_div(total) {
                    self.percent = self.percent.max(percent as u8);
                    self.files = u32::try_from(total).ok();
                }
            }
        }
        for (marker, percent) in PHASES {
            if text.contains(marker) {
                self.percent = self.percent.max(*percent);
            }
        }
        if let Some(rest) = text.split("files: ").nth(1) {
            if let Ok(n) = rest
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("")
                .parse()
            {
                self.files = Some(n);
            }
        }
        *self != before
    }
}

/// 构建失败的原因。`Missing` 表示没有安装语义组件（状态栏显示“基础导航”，不算失败通知）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildFailure {
    Missing,
    /// 组件已安装但版本与本 Rove 不兼容（#43）：提示更新，降级到 L1/L0，不重试。
    Incompatible(Incompatible),
    SpawnFailed(String),
    /// 进程异常终止（Windows NTSTATUS 退出码，例如 0xC0000005）。
    Crashed(u32),
    ExitCode(i32),
    /// 进程报告成功，但输出文件缺失或打不开。
    InvalidOutput(String),
    Cancelled,
}

impl std::fmt::Display for BuildFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "indexer component not found"),
            Self::Incompatible(why) => write!(f, "indexer component incompatible: {why}"),
            Self::SpawnFailed(e) => write!(f, "failed to start indexer: {e}"),
            Self::Crashed(code) => write!(f, "indexer crashed (0x{code:08X})"),
            Self::ExitCode(code) => write!(f, "indexer exited with code {code}"),
            Self::InvalidOutput(e) => write!(f, "indexer output unusable: {e}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// 生成一份完整的 L2 索引。正式实现是 [`ProcessBuilder`]；测试用假实现。
/// 在生命周期的后台线程调用，可以阻塞，但要尽快响应 `cancel`。
pub trait IndexBuilder: Send + Sync {
    fn build(
        &self,
        request: &BuildRequest,
        progress: &mut dyn FnMut(Progress),
        log: &IndexLog,
        cancel: &Cancellation,
    ) -> Result<(), BuildFailure>;

    /// #35：增量更新用的常驻进程。`None`（默认）时源文件变化只标记过期，等下一次全量重建。
    fn incremental(&self) -> Option<Arc<dyn IncrementalIndexer>> {
        None
    }

    /// 已安装组件的版本：缓存索引头部的索引进程版本必须与它一致，否则重建（#43）。
    /// `None`（默认或未知）时按 `EXPECTED_INDEXER_VERSION`。
    fn component_version(&self) -> Option<[u16; 4]> {
        None
    }
}

/// 查找组件的函数：正式装配用 [`locate_indexer`]，测试替换成任意命令。
pub type Locate = Arc<dyn Fn() -> Result<IndexerComponent, BuildFailure> + Send + Sync>;

/// 用索引进程构建。
pub struct ProcessBuilder {
    locate: Locate,
    resident: bool,
}

impl Default for ProcessBuilder {
    /// 正式装配：全量构建与增量更新（常驻模式，#35）都用 [`locate_indexer`] 找到的组件。
    fn default() -> Self {
        Self {
            locate: Arc::new(locate_indexer),
            resident: true,
        }
    }
}

impl ProcessBuilder {
    /// 只做全量构建（测试用任意命令模拟索引进程时，不会被当成常驻进程拉起）。
    pub fn new(locate: impl Fn() -> Option<IndexerComponent> + Send + Sync + 'static) -> Self {
        Self {
            locate: Arc::new(move || locate().ok_or(BuildFailure::Missing)),
            resident: false,
        }
    }

    /// 用任意查找函数（区分“未安装”与“不兼容”）构建，同样只做全量构建。
    pub fn with_locate(
        locate: impl Fn() -> Result<IndexerComponent, BuildFailure> + Send + Sync + 'static,
    ) -> Self {
        Self {
            locate: Arc::new(locate),
            resident: false,
        }
    }

    /// 同一个组件也用于常驻模式（`--serve`）。
    pub fn with_resident(mut self, resident: bool) -> Self {
        self.resident = resident;
        self
    }
}

fn pump(stream: impl Read + Send + 'static, tx: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

impl IndexBuilder for ProcessBuilder {
    fn build(
        &self,
        request: &BuildRequest,
        progress: &mut dyn FnMut(Progress),
        log: &IndexLog,
        cancel: &Cancellation,
    ) -> Result<(), BuildFailure> {
        let component = (self.locate)()?;
        let mut command = Command::new(&component.program);
        command
            .args(&component.prefix)
            .arg(&request.project_root)
            .arg(&request.output)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        log.line(&format!(
            "start: {} {:?} \"{}\" \"{}\"",
            component.program.display(),
            component.prefix,
            request.project_root.display(),
            request.output.display()
        ));
        let started = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|e| BuildFailure::SpawnFailed(e.to_string()))?;
        let (tx, rx) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            pump(out, tx.clone());
        }
        if let Some(err) = child.stderr.take() {
            pump(err, tx);
        }
        let mut current = Progress::default();
        let status = loop {
            while let Ok(line) = rx.try_recv() {
                log.line(&format!("indexer: {line}"));
                if current.parse(&line) {
                    progress(current);
                }
            }
            if cancel.check().is_err() {
                let _ = child.kill();
                let _ = child.wait();
                log.line("cancelled: indexer process terminated");
                return Err(BuildFailure::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    let _ = rx.recv_timeout(Duration::from_millis(50)).map(|line| {
                        log.line(&format!("indexer: {line}"));
                        if current.parse(&line) {
                            progress(current);
                        }
                    });
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(BuildFailure::SpawnFailed(e.to_string()));
                }
            }
        };
        // 进程退出后把剩下的输出读完（管道关闭时读线程结束）。
        while let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) {
            log.line(&format!("indexer: {line}"));
        }
        let elapsed = started.elapsed().as_secs_f64();
        match status.code() {
            Some(0) => {
                log.line(&format!("exit: 0 after {elapsed:.1}s"));
                if request.output.is_file() {
                    Ok(())
                } else {
                    Err(BuildFailure::InvalidOutput("output file missing".into()))
                }
            }
            Some(code) => {
                log.line(&format!(
                    "exit: {code} (0x{:08X}) after {elapsed:.1}s",
                    code as u32
                ));
                // NTSTATUS 错误码（最高两位为 11）是崩溃，例如访问冲突 0xC0000005、栈溢出 0xC00000FD。
                if (code as u32) >= 0xC000_0000 {
                    Err(BuildFailure::Crashed(code as u32))
                } else {
                    Err(BuildFailure::ExitCode(code))
                }
            }
            None => Err(BuildFailure::Crashed(0)),
        }
    }

    fn incremental(&self) -> Option<Arc<dyn IncrementalIndexer>> {
        let locate = self.locate.clone();
        self.resident.then(|| {
            Arc::new(ProcessResident::new(move || locate())) as Arc<dyn IncrementalIndexer>
        })
    }

    fn component_version(&self) -> Option<[u16; 4]> {
        (self.locate)().ok().and_then(|c| c.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_from_indexer_output() {
        let mut p = Progress::default();
        assert!(!p.parse("    0.4s  projects: 3, source entries: 120"));
        assert_eq!(p.percent, 0);
        assert!(p.parse("    0.5s  [parse_csproj] 0.5s  ws=80MB"));
        assert_eq!(p.percent, 5);
        assert!(p.parse("    2.0s  files: 1234, missing: 0"));
        assert_eq!(p.files, Some(1234));
        p.parse("    9.0s  [create_compilations+generators] 3.0s  ws=900MB");
        assert_eq!(p.percent, 40);
        assert!(!p.parse("noise"));
        p.parse("progress 50 200");
        assert_eq!((p.percent, p.files), (40, Some(200)));
        p.parse("progress 190 200");
        assert_eq!(p.percent, 95);
    }

    #[test]
    fn missing_component_and_dll_launcher() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            IndexerComponent::from_path(&dir.path().join("none.exe")),
            None
        );
        let dll = dir.path().join("rove-indexer-cs.dll");
        std::fs::write(&dll, b"").unwrap();
        let c = IndexerComponent::from_path(&dll).unwrap();
        assert_eq!(c.program, PathBuf::from("dotnet"));
        assert_eq!(c.prefix, vec![dll.as_os_str().to_owned()]);
        let log = IndexLog::new(dir.path().join("indexer.log"));
        let request = BuildRequest {
            project_root: dir.path().into(),
            output: dir.path().join("out.rvcsidx"),
        };
        let none = ProcessBuilder::new(|| None);
        assert_eq!(
            none.build(&request, &mut |_| {}, &log, &Cancellation::default()),
            Err(BuildFailure::Missing)
        );
        let bad = ProcessBuilder::new(move || {
            Some(IndexerComponent {
                program: PathBuf::from(r"Z:\no\such\indexer.exe"),
                prefix: vec![],
                version: None,
                protocol: None,
            })
        });
        assert!(matches!(
            bad.build(&request, &mut |_| {}, &log, &Cancellation::default()),
            Err(BuildFailure::SpawnFailed(_))
        ));
        assert!(std::fs::read_to_string(log.path())
            .unwrap()
            .contains("start:"));
    }
}
