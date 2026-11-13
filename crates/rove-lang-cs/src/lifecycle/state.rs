//! 索引状态（UX_SPEC §2.1）。界面只订阅 [`IndexStatus`] 的变化；所有迁移集中在 `service` 的
//! 后台线程里一处完成。
use super::{
    component::Incompatible,
    process::{BuildFailure, Progress},
};
use std::{path::PathBuf, time::Duration};

/// 显式状态机。“未安装 / 首次构建中 / 就绪 / 增量更新中 / 失败”是 §2.1 的五种状态，另有
/// “已禁用”“没有 C# 代码（条目不显示）”与打开时短暂的“检查中”。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexState {
    /// 打开项目：扫描项目输入、检查缓存。
    Checking,
    /// 工作区里没有 C# 代码：状态栏条目不显示，不创建缓存目录。
    NoCSharp,
    /// `rove.index.enabled = false`：不启动索引进程、不创建缓存目录。
    Disabled,
    /// 没有安装语义组件：C# 导航停留在 L0/L1。
    NotInstalled,
    /// 组件已安装，但版本与本 Rove 不兼容（#43）：提示更新组件或 Rove，C# 导航停留在 L0/L1。
    Incompatible(Incompatible),
    /// 构建中。`rebuilding` 为 true 时旧索引仍然可以查询。
    Building {
        progress: Progress,
        eta: Option<Duration>,
        rebuilding: bool,
    },
    Ready,
    /// 增量更新中（#35 合并后由常驻索引进程驱动）。
    Updating {
        files: u32,
    },
    Failed(Failure),
}

/// 失败的类别与日志用的诊断文字。类别决定通知的去重键：同一类别不重复弹出。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailureKind {
    /// 缓存目录不可用（路径无效、无法创建、没有写权限）。
    CacheDir,
    SpawnFailed,
    Crashed,
    ExitCode,
    InvalidOutput,
}

impl FailureKind {
    /// 稳定的类别名（通知去重键与界面字符串 key 的后缀）。
    pub fn id(self) -> &'static str {
        match self {
            Self::CacheDir => "cacheDir",
            Self::SpawnFailed => "spawnFailed",
            Self::Crashed => "crashed",
            Self::ExitCode => "exitCode",
            Self::InvalidOutput => "invalidOutput",
        }
    }
}

impl Failure {
    pub fn from_build(failure: &BuildFailure) -> Self {
        let kind = match failure {
            BuildFailure::Crashed(_) => FailureKind::Crashed,
            BuildFailure::ExitCode(_) => FailureKind::ExitCode,
            BuildFailure::InvalidOutput(_) => FailureKind::InvalidOutput,
            _ => FailureKind::SpawnFailed,
        };
        Self {
            kind,
            detail: failure.to_string(),
        }
    }
}

/// 预读进度（详情页显示）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrefetchState {
    #[default]
    NotStarted,
    Running,
    Done {
        bytes: u64,
        millis: u64,
    },
    Cancelled,
    /// 刚构建完的索引已经在系统缓存里，不需要预读。
    Skipped,
}

/// 悬停提示与详情页的数据。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexInfo {
    pub cache_dir: Option<PathBuf>,
    pub cache_bytes: u64,
    pub log: Option<PathBuf>,
    /// 当前索引的构建时间（Unix 毫秒）。
    pub built_at_ms: Option<u64>,
    pub files: u64,
    pub symbols: u64,
    pub generation: u64,
    /// 已标记为过期的文件数（#38 据此把这些文件的结果降级为近似）。
    pub stale_files: usize,
    pub prefetch: PrefetchState,
    /// 没有 csproj，按 asmdef 推断项目结构。
    pub inferred_project: bool,
    /// 索引仍然可用，但组件无法使用（#43）：常驻进程握手失败，源文件变化只标记过期，
    /// 全量重建也不会进行。界面在提示里说明要更新哪一边。
    pub component_issue: Option<Incompatible>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexStatus {
    pub state: IndexState,
    pub info: IndexInfo,
    /// 每次变化递增。
    pub revision: u64,
}

impl Default for IndexStatus {
    fn default() -> Self {
        Self {
            state: IndexState::Checking,
            info: IndexInfo::default(),
            revision: 0,
        }
    }
}
