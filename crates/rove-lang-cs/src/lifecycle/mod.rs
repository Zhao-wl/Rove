//! C# 索引的生命周期与状态（#36）：缓存目录、有效性检查、后台顺序预读、原子替换、
//! 索引进程的拉起与监控、文件变化标记过期、缓存淘汰，以及给界面的状态机。
//!
//! 本模块不依赖 GPUI；状态栏条目、通知与详情视图在 `rove-ui::index_status`，只订阅
//! [`IndexLifecycle::changes`]。缓存目录布局见 [`layout`] 与 `docs/ARCHITECTURE.md`。
pub mod evict;
pub mod hash;
pub mod inputs;
pub mod layout;
pub mod log;
pub mod meta;
pub mod prefetch;
pub mod process;
pub mod resident;
pub mod service;
pub mod state;
#[doc(hidden)]
pub mod testing;
pub mod validity;

pub use evict::CachePolicy;
pub use layout::ProjectCache;
pub use prefetch::PrefetchMode;
pub use process::{locate_indexer, BuildFailure, IndexBuilder, ProcessBuilder, Progress};
pub use resident::{
    IncrementalIndexer, ProcessResident, ResidentEvent, ResidentOptions, ResidentSession,
    SessionRequest,
};
pub use service::{IndexLifecycle, IndexSnapshot, LifecycleConfig, OverlayLimits, SourceChange};
pub use state::{Failure, FailureKind, IndexInfo, IndexState, IndexStatus, PrefetchState};
pub use validity::SampleRule;
