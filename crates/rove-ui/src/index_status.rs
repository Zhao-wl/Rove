//! C# 索引状态的界面（#36，UX_SPEC §2.1）：状态栏条目、悬停提示、详情面板、失败通知与
//! `rove.index.*` 三个命令。
//!
//! 数据与状态机在 `rove_lang_cs::lifecycle`（不依赖 GPUI）；这里只把 [`IndexStatus`] 投影成
//! 工作台（#16）登记过的状态栏条目与面板视图，失败时经反馈层（`Reason::Global`）报一次通知，
//! 同因去重由反馈层负责。全部文案登记在字符串表里，命令的按键由 #14 的键位服务分发。
use crate::{
    contributions::{Contribution, ContributionRegistry, Placement},
    feedback::FeedbackAction,
    shell::{
        catalog::STATUS_FACTORY,
        feedback::{Reason, Severity},
        icons,
        status::{StatusEntry, StatusKind},
    },
};
use rove_base::{Result, Subscription};
use rove_i18n::{Message, StringTable, Strings};
use rove_lang_cs::lifecycle::{
    Failure, FailureKind, IndexLifecycle, IndexState, IndexStatus, PrefetchState,
};
use std::{path::Path, sync::Arc, time::Duration};

pub const STATUS_ITEM: &str = "rove.status.index";
/// 索引详情（面板视图；“就绪”“失败”等状态的点击目标）。
pub const DETAILS_VIEW: &str = "rove.index.details";
pub const REBUILD: &str = "rove.index.rebuild";
pub const SHOW_DETAILS: &str = "rove.index.showDetails";
pub const OPEN_LOG: &str = "rove.index.openLog";
/// 失败通知的反馈 key 前缀，后接失败类别：同一类别（同一原因）只有一条通知。
pub const FAILURE_KEY_PREFIX: &str = "rove.index.failed.";
/// 状态栏位置：问题计数（50）右侧（UX_SPEC §1.5 左侧“问题计数、C# 索引状态”）。
pub const STATUS_ORDER: i32 = 40;

gpui::actions!(rove_index, [Rebuild, ShowDetails, OpenLog]);

/// 三个命令的 GPUI action（键位服务按命令 ID 分发到这里；见 `crate::keys::action_for`）。
pub fn action_for(id: &str) -> Option<Box<dyn gpui::Action>> {
    match id {
        REBUILD => Some(Box::new(Rebuild)),
        SHOW_DETAILS => Some(Box::new(ShowDetails)),
        OPEN_LOG => Some(Box::new(OpenLog)),
        _ => None,
    }
}

pub fn is_index_command(id: &str) -> bool {
    matches!(id, REBUILD | SHOW_DETAILS | OPEN_LOG)
}

/// 状态栏条目与详情面板。命令本身（标题、分类）已由 #14 的 `rove_commands::builtin` 登记。
pub fn register_contributions(r: &mut ContributionRegistry) -> Result<()> {
    r.register(
        STATUS_ITEM,
        Contribution {
            title: Message::key("index.status.name"),
            placement: Placement::StatusLeft,
            order: STATUS_ORDER,
            icon: None,
            command: Some(SHOW_DETAILS.into()),
            when: None,
            factory_key: STATUS_FACTORY.into(),
        },
    )?;
    r.register(
        DETAILS_VIEW,
        Contribution {
            title: Message::key("index.details.title"),
            placement: Placement::Panel,
            order: 30,
            icon: Some("database".into()),
            command: Some(SHOW_DETAILS.into()),
            when: None,
            factory_key: DETAILS_VIEW.into(),
        },
    )
}

const STRINGS: &[(&str, &str)] = &[
    ("index.status.name", "C# 索引状态"),
    ("index.status.checking", "$(sync~spin) C#"),
    ("index.status.notInstalled", "$(info) C# 基础导航"),
    ("index.status.disabled", "$(circle-slash) C# 基础导航"),
    ("index.status.building", "$(sync~spin) C# 索引 {percent}%"),
    ("index.status.ready", "$(check) C#"),
    ("index.status.updating", "$(sync~spin) C#"),
    ("index.status.failed", "$(error) C# 索引失败"),
    ("index.tooltip.checking", "正在检查 C# 索引缓存…"),
    (
        "index.tooltip.notInstalled",
        "跳转和引用是近似结果。安装语义组件后可以获得精确结果。",
    ),
    (
        "index.tooltip.disabled",
        "语义索引已在设置中禁用，跳转和引用是近似结果。",
    ),
    ("index.tooltip.building", "{progress}，{eta}。构建期间的结果是近似的。"),
    ("index.tooltip.rebuilding", "{progress}，{eta}。重建期间继续使用旧索引。"),
    ("index.progress.files", "已处理 {done} / {total} 个文件"),
    ("index.progress.percent", "已完成 {percent}%"),
    ("index.eta.seconds", "预计还需 {n} 秒"),
    ("index.eta.minutes", "预计还需 {n} 分钟"),
    ("index.eta.unknown", "正在估算剩余时间"),
    (
        "index.tooltip.ready",
        "上次构建：{time}\n文件数：{files}，符号数：{symbols}\n缓存：{dir}（{size}）",
    ),
    ("index.tooltip.updating", "正在更新 {files} 个文件"),
    (
        "index.tooltip.stale",
        "{count} 个文件在建立索引后被修改，它们的结果是近似的。",
    ),
    (
        "index.tooltip.inferred",
        "未找到项目文件，正在使用推断的项目结构。在 Unity 中执行 Assets → Open C# Project 可以生成项目文件。",
    ),
    (
        "index.failure.cacheDir",
        "索引缓存目录不可用，请检查 rove.index.cacheDir 设置与目录权限。",
    ),
    ("index.failure.spawnFailed", "无法启动 C# 索引进程。"),
    ("index.failure.crashed", "C# 索引进程意外退出。"),
    ("index.failure.exitCode", "C# 索引进程未能完成索引。"),
    ("index.failure.invalidOutput", "C# 索引进程生成的索引无法使用。"),
    (
        "index.notification.failed",
        "{reason}跳转和引用暂时使用近似结果。",
    ),
    ("index.action.details", "查看详情"),
    ("index.action.retry", "重试"),
    ("index.action.rebuild", "重建索引"),
    ("index.action.openLog", "打开索引日志"),
    ("index.details.title", "C# 索引"),
    (
        "index.details.notInstalled",
        "没有找到 C# 语义组件（rove-indexer-cs）。跳转和引用来自快速索引与语法分析，是近似结果。\n安装方法：把语义组件放在 rove.exe 同目录的 rove-indexer-cs 文件夹中，或者用环境变量 ROVE_INDEXER_CS 指向 rove-indexer-cs.exe，然后执行“重建 C# 语义索引”。",
    ),
    (
        "index.details.disabled",
        "语义索引已在设置中禁用（rove.index.enabled 为 false），跳转和引用是近似结果。\n在 settings.json 中把 rove.index.enabled 改为 true 即可启用。",
    ),
    ("index.details.reason", "原因：{detail}"),
    ("index.details.builtAt", "上次构建：{time}"),
    ("index.details.counts", "文件数：{files}，符号数：{symbols}"),
    ("index.details.cacheDir", "缓存位置：{dir}"),
    ("index.details.cacheSize", "缓存大小：{size}"),
    ("index.details.log", "索引日志：{path}"),
    ("index.details.prefetchRunning", "预读：正在后台顺序读取索引文件"),
    ("index.details.prefetchDone", "预读：{size}，用时 {ms} 毫秒"),
    ("index.details.prefetchSkipped", "预读：刚构建的索引已在系统缓存中"),
    ("index.details.prefetchCancelled", "预读：已取消"),
    ("index.time.justNow", "刚刚"),
    ("index.time.minutes", "{n} 分钟前"),
    ("index.time.hours", "{n} 小时前"),
    ("index.time.days", "{n} 天前"),
    ("index.time.unknown", "未知"),
    ("index.noLog", "还没有索引日志。"),
];

pub fn register_strings(t: &mut StringTable) -> Result<()> {
    for (key, text) in STRINGS {
        t.register(*key, Arc::from(*text))?;
    }
    Ok(())
}

fn s(strings: &dyn Strings, key: &str, args: &[(&str, String)]) -> String {
    let message = Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
            .collect(),
    };
    strings
        .text(&message)
        .unwrap_or_else(|e| panic!("index string {key}: {e}"))
}

fn size_text(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= 1024.0 * MB {
        format!("{:.1} GB", bytes as f64 / (1024.0 * MB))
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).ceil())
    }
}

fn ago(strings: &dyn Strings, at_ms: Option<u64>, now_ms: u64) -> String {
    let Some(at) = at_ms else {
        return s(strings, "index.time.unknown", &[]);
    };
    let minutes = now_ms.saturating_sub(at) / 60_000;
    match minutes {
        0 => s(strings, "index.time.justNow", &[]),
        1..=59 => s(strings, "index.time.minutes", &[("n", minutes.to_string())]),
        60..=1439 => s(
            strings,
            "index.time.hours",
            &[("n", (minutes / 60).to_string())],
        ),
        _ => s(
            strings,
            "index.time.days",
            &[("n", (minutes / 1440).to_string())],
        ),
    }
}

fn eta_text(strings: &dyn Strings, eta: Option<Duration>) -> String {
    match eta.map(|d| d.as_secs()) {
        None => s(strings, "index.eta.unknown", &[]),
        Some(n) if n < 90 => s(strings, "index.eta.seconds", &[("n", n.max(1).to_string())]),
        Some(n) => s(
            strings,
            "index.eta.minutes",
            &[("n", n.div_ceil(60).to_string())],
        ),
    }
}

/// 失败原因的一句话摘要（悬停提示与通知正文）。
pub fn failure_summary(strings: &dyn Strings, kind: FailureKind) -> String {
    s(strings, &format!("index.failure.{}", kind.id()), &[])
}

fn dir_text(path: Option<&Path>) -> String {
    path.map(|p| p.display().to_string()).unwrap_or_default()
}

/// 状态 → 状态栏条目；`None` 表示条目不显示（工作区里没有 C# 代码）。
pub fn status_entry(
    status: &IndexStatus,
    strings: &dyn Strings,
    now_ms: u64,
) -> Option<StatusEntry> {
    let info = &status.info;
    let (text, mut tooltip) = match &status.state {
        IndexState::NoCSharp => return None,
        IndexState::Checking => (
            s(strings, "index.status.checking", &[]),
            s(strings, "index.tooltip.checking", &[]),
        ),
        IndexState::NotInstalled => (
            s(strings, "index.status.notInstalled", &[]),
            s(strings, "index.tooltip.notInstalled", &[]),
        ),
        IndexState::Disabled => (
            s(strings, "index.status.disabled", &[]),
            s(strings, "index.tooltip.disabled", &[]),
        ),
        IndexState::Building {
            progress,
            eta,
            rebuilding,
        } => {
            let done = progress
                .files
                .map(|total| (u64::from(total) * u64::from(progress.percent) / 100, total));
            let progress_text = match done {
                Some((done, total)) => s(
                    strings,
                    "index.progress.files",
                    &[("done", done.to_string()), ("total", total.to_string())],
                ),
                None => s(
                    strings,
                    "index.progress.percent",
                    &[("percent", progress.percent.to_string())],
                ),
            };
            let key = if *rebuilding {
                "index.tooltip.rebuilding"
            } else {
                "index.tooltip.building"
            };
            (
                s(
                    strings,
                    "index.status.building",
                    &[("percent", progress.percent.to_string())],
                ),
                s(
                    strings,
                    key,
                    &[
                        ("progress", progress_text),
                        ("eta", eta_text(strings, *eta)),
                    ],
                ),
            )
        }
        IndexState::Ready => (
            s(strings, "index.status.ready", &[]),
            s(
                strings,
                "index.tooltip.ready",
                &[
                    ("time", ago(strings, info.built_at_ms, now_ms)),
                    ("files", info.files.to_string()),
                    ("symbols", info.symbols.to_string()),
                    ("dir", dir_text(info.cache_dir.as_deref())),
                    ("size", size_text(info.cache_bytes)),
                ],
            ),
        ),
        IndexState::Updating { files } => (
            s(strings, "index.status.updating", &[]),
            s(
                strings,
                "index.tooltip.updating",
                &[("files", files.to_string())],
            ),
        ),
        IndexState::Failed(failure) => (
            s(strings, "index.status.failed", &[]),
            failure_summary(strings, failure.kind),
        ),
    };
    if info.stale_files > 0 && matches!(status.state, IndexState::Ready) {
        tooltip.push('\n');
        tooltip.push_str(&s(
            strings,
            "index.tooltip.stale",
            &[("count", info.stale_files.to_string())],
        ));
    }
    if info.inferred_project {
        tooltip.push('\n');
        tooltip.push_str(&s(strings, "index.tooltip.inferred", &[]));
    }
    Some(StatusEntry {
        text,
        tooltip: Some(tooltip),
        command: Some(SHOW_DETAILS.into()),
        kind: StatusKind::Standard,
    })
}

/// 详情面板的文字行（纯数据，界面与测试共用）。第一行是状态（去掉图标）。
pub fn details_lines(status: &IndexStatus, strings: &dyn Strings, now_ms: u64) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(entry) = status_entry(status, strings, now_ms) else {
        return lines;
    };
    lines.push(icons::strip_icons(&entry.text).trim().to_owned());
    let info = &status.info;
    match &status.state {
        IndexState::NotInstalled => lines.extend(
            s(strings, "index.details.notInstalled", &[])
                .lines()
                .map(str::to_owned),
        ),
        IndexState::Disabled => {
            lines.extend(
                s(strings, "index.details.disabled", &[])
                    .lines()
                    .map(str::to_owned),
            );
            return lines;
        }
        IndexState::Failed(Failure { kind, detail }) => {
            lines.push(failure_summary(strings, *kind));
            lines.push(s(
                strings,
                "index.details.reason",
                &[("detail", detail.clone())],
            ));
        }
        _ => lines.extend(
            entry
                .tooltip
                .iter()
                .flat_map(|t| t.lines().map(str::to_owned)),
        ),
    }
    if info.cache_dir.is_some() && !matches!(status.state, IndexState::Ready) {
        if info.built_at_ms.is_some() {
            lines.push(s(
                strings,
                "index.details.builtAt",
                &[("time", ago(strings, info.built_at_ms, now_ms))],
            ));
            lines.push(s(
                strings,
                "index.details.counts",
                &[
                    ("files", info.files.to_string()),
                    ("symbols", info.symbols.to_string()),
                ],
            ));
        }
        lines.push(s(
            strings,
            "index.details.cacheDir",
            &[("dir", dir_text(info.cache_dir.as_deref()))],
        ));
        lines.push(s(
            strings,
            "index.details.cacheSize",
            &[("size", size_text(info.cache_bytes))],
        ));
    }
    match info.prefetch {
        PrefetchState::NotStarted => {}
        PrefetchState::Running => lines.push(s(strings, "index.details.prefetchRunning", &[])),
        PrefetchState::Done { bytes, millis } => lines.push(s(
            strings,
            "index.details.prefetchDone",
            &[("size", size_text(bytes)), ("ms", millis.to_string())],
        )),
        PrefetchState::Skipped => lines.push(s(strings, "index.details.prefetchSkipped", &[])),
        PrefetchState::Cancelled => lines.push(s(strings, "index.details.prefetchCancelled", &[])),
    }
    if let Some(log) = &info.log {
        lines.push(s(
            strings,
            "index.details.log",
            &[("path", log.display().to_string())],
        ));
    }
    lines
}

/// 详情面板底部的按钮：(命令, 标签 key)。
pub fn details_actions(state: &IndexState) -> Vec<(&'static str, &'static str)> {
    match state {
        IndexState::Disabled | IndexState::NoCSharp => Vec::new(),
        IndexState::Failed(_) => vec![
            (REBUILD, "index.action.retry"),
            (OPEN_LOG, "index.action.openLog"),
        ],
        _ => vec![
            (REBUILD, "index.action.rebuild"),
            (OPEN_LOG, "index.action.openLog"),
        ],
    }
}

/// 失败通知（反馈层按 key 去重，同一类别只有一条，再次失败只更新次数）。
pub fn failure_notification(
    failure: &Failure,
    strings: &dyn Strings,
) -> (String, Reason, Message, Arc<[FeedbackAction]>) {
    let key = format!("{FAILURE_KEY_PREFIX}{}", failure.kind.id());
    let message = Message {
        key: "index.notification.failed".into(),
        args: Arc::from([(
            Arc::from("reason"),
            Arc::from(failure_summary(strings, failure.kind).as_str()),
        )]),
    };
    let actions: Arc<[FeedbackAction]> = Arc::from([
        FeedbackAction {
            id: SHOW_DETAILS.into(),
            label: Message::key("index.action.details"),
        },
        FeedbackAction {
            id: REBUILD.into(),
            label: Message::key("index.action.retry"),
        },
    ]);
    (key, Reason::Global(Severity::Error), message, actions)
}

/// 全部失败通知的 key（恢复后一起收起）。
pub fn failure_keys() -> impl Iterator<Item = String> {
    [
        FailureKind::CacheDir,
        FailureKind::SpawnFailed,
        FailureKind::Crashed,
        FailureKind::ExitCode,
        FailureKind::InvalidOutput,
    ]
    .into_iter()
    .map(|k| format!("{FAILURE_KEY_PREFIX}{}", k.id()))
}

/// 窗口持有的索引状态：生命周期、最新状态与订阅。
pub struct IndexStatusController {
    lifecycle: Option<IndexLifecycle>,
    status: IndexStatus,
    /// 每换一次生命周期加一；旧生命周期排队中的事件被丢弃。
    epoch: u64,
    _subscription: Option<Subscription>,
}

impl Default for IndexStatusController {
    fn default() -> Self {
        Self {
            lifecycle: None,
            status: IndexStatus {
                state: IndexState::NoCSharp,
                ..Default::default()
            },
            epoch: 0,
            _subscription: None,
        }
    }
}

/// 状态变化后界面要做的事。
#[derive(Debug, PartialEq, Eq)]
pub enum Reaction {
    None,
    /// 进入了新的失败：报一次通知。
    Notify(Failure),
    /// 从失败恢复：收起失败通知。
    Recovered,
}

impl IndexStatusController {
    pub fn status(&self) -> &IndexStatus {
        &self.status
    }
    pub fn lifecycle(&self) -> Option<&IndexLifecycle> {
        self.lifecycle.as_ref()
    }
    /// 换上（或换掉）生命周期；旧的随之关闭（取消预读、结束索引进程）。
    pub fn attach(
        &mut self,
        lifecycle: Option<IndexLifecycle>,
        subscription: Option<Subscription>,
    ) -> u64 {
        self.epoch += 1;
        self._subscription = subscription;
        self.lifecycle = lifecycle;
        self.status = self
            .lifecycle
            .as_ref()
            .map(IndexLifecycle::status)
            .unwrap_or_else(|| IndexStatus {
                state: IndexState::NoCSharp,
                ..Default::default()
            });
        // 新的生命周期从自己的修订号重新计数。
        self.status.revision = 0;
        self.epoch
    }
    /// 应用一次状态变化。只在**进入**失败时要求通知：同一次失败里的状态刷新不再通知。
    pub fn apply(&mut self, epoch: u64, status: IndexStatus) -> Reaction {
        if epoch != self.epoch || status.revision < self.status.revision {
            return Reaction::None;
        }
        let before = std::mem::replace(&mut self.status, status);
        match (&before.state, &self.status.state) {
            (IndexState::Failed(a), IndexState::Failed(b)) if a.kind == b.kind => Reaction::None,
            (_, IndexState::Failed(f)) => Reaction::Notify(f.clone()),
            (IndexState::Failed(_), IndexState::Ready) => Reaction::Recovered,
            _ => Reaction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_lang_cs::lifecycle::{IndexInfo, Progress};

    fn table() -> StringTable {
        let mut t = rove_i18n::chinese().unwrap();
        register_strings(&mut t).unwrap();
        t
    }

    #[test]
    fn every_state_projects_to_the_ux_spec_text() {
        let t = table();
        let status = |state| IndexStatus {
            state,
            info: IndexInfo::default(),
            revision: 1,
        };
        let text = |state| status_entry(&status(state), &t, 0).map(|e| e.text);
        assert_eq!(text(IndexState::NoCSharp), None);
        assert_eq!(
            text(IndexState::NotInstalled).unwrap(),
            "$(info) C# 基础导航"
        );
        assert_eq!(
            text(IndexState::Disabled).unwrap(),
            "$(circle-slash) C# 基础导航"
        );
        assert_eq!(
            text(IndexState::Building {
                progress: Progress {
                    percent: 37,
                    files: Some(200)
                },
                eta: Some(Duration::from_secs(150)),
                rebuilding: false
            })
            .unwrap(),
            "$(sync~spin) C# 索引 37%"
        );
        assert_eq!(text(IndexState::Ready).unwrap(), "$(check) C#");
        assert_eq!(
            text(IndexState::Updating { files: 3 }).unwrap(),
            "$(sync~spin) C#"
        );
        assert_eq!(
            text(IndexState::Failed(Failure {
                kind: FailureKind::Crashed,
                detail: String::new()
            }))
            .unwrap(),
            "$(error) C# 索引失败"
        );
    }

    #[test]
    fn controller_notifies_once_per_failure_episode() {
        let mut c = IndexStatusController::default();
        let failed = |rev, kind| IndexStatus {
            state: IndexState::Failed(Failure {
                kind,
                detail: "x".into(),
            }),
            info: IndexInfo::default(),
            revision: rev,
        };
        assert!(matches!(
            c.apply(0, failed(1, FailureKind::Crashed)),
            Reaction::Notify(_)
        ));
        assert_eq!(c.apply(0, failed(2, FailureKind::Crashed)), Reaction::None);
        // 过期的事件不回退状态。
        assert_eq!(c.apply(0, failed(1, FailureKind::ExitCode)), Reaction::None);
        let ready = IndexStatus {
            state: IndexState::Ready,
            info: IndexInfo::default(),
            revision: 3,
        };
        assert_eq!(c.apply(0, ready), Reaction::Recovered);
        // 换了生命周期后，旧生命周期的事件被丢弃。
        let epoch = c.attach(None, None);
        assert_eq!(
            c.apply(epoch - 1, failed(9, FailureKind::Crashed)),
            Reaction::None
        );
    }
}
