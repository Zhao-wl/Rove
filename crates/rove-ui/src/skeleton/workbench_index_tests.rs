//! #36 入口集成测试（界面）：C# 索引状态条目的五种状态（含已禁用与不显示）、悬停内容与点击行为，
//! 失败通知同因只弹一次，三个 `rove.index.*` 命令在命令面板中可见并可执行，以及真实生命周期
//! （假索引进程）从打开到就绪、失败、重试恢复的整条链路。
//! 运行：`cargo test -p rove-ui index_tests`。
use super::*;
use crate::index_status::{
    DETAILS_VIEW, FAILURE_KEY_PREFIX, OPEN_LOG, REBUILD, SHOW_DETAILS, STATUS_ITEM,
};
use crate::shell::feedback::Severity;
use gpui::{TestAppContext, VisualTestContext};
use rove_lang_cs::lifecycle::{
    log::IndexLog, process::BuildRequest, testing::FixtureIndex, BuildFailure, Failure,
    FailureKind, IndexBuilder, IndexInfo, IndexState, Progress,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

const INFERRED: &str =
    "未找到项目文件，正在使用推断的项目结构。在 Unity 中执行 Assets → Open C# Project 可以生成项目文件。";

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext, tempfile::TempDir) {
    let folder = tempfile::tempdir().unwrap();
    let root: PathBuf = folder.path().into();
    let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
    cx.run_until_parked();
    (view, cx, folder)
}

/// 直接推送一个状态（界面只订阅状态，不关心它从哪来）。
fn push(view: &Entity<Workbench>, cx: &mut VisualTestContext, state: IndexState, info: IndexInfo) {
    view.update(cx, |v, cx| {
        let epoch = v.index.attach(None, None);
        let status = IndexStatus {
            state,
            info,
            revision: 1,
        };
        v.index_status_changed(epoch, status, cx);
    });
}

fn entry(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<StatusEntry> {
    view.read_with(cx, |v, _| v.shell.status_bar.get(STATUS_ITEM).cloned())
}

/// 点击状态栏条目：与 `on_click` 相同，执行条目的命令。
fn click(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let command = entry(view, cx).unwrap().command.unwrap();
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            shell::view::run_command(v, &command, window, cx)
        })
    });
    cx.run_until_parked();
}

fn details(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        assert!(
            v.shell.layout.panel_visible,
            "click opens the details panel"
        );
        assert_eq!(v.shell.active_panel_view(), DETAILS_VIEW);
        crate::index_status::details_lines(v.index.status(), &v.strings, 0)
    })
}

fn ready_info() -> IndexInfo {
    IndexInfo {
        cache_dir: Some(PathBuf::from(r"C:\cache\game-0123456789abcdef")),
        cache_bytes: 94 << 20,
        log: Some(PathBuf::from(
            r"C:\cache\game-0123456789abcdef\logs\indexer.log",
        )),
        built_at_ms: Some(0),
        files: 1234,
        symbols: 56789,
        ..Default::default()
    }
}

#[gpui::test]
fn no_csharp_hides_the_item(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    assert!(entry(&view, cx).is_none());
    push(&view, cx, IndexState::Ready, ready_info());
    assert!(entry(&view, cx).is_some());
    push(&view, cx, IndexState::NoCSharp, IndexInfo::default());
    assert!(entry(&view, cx).is_none());
}

#[gpui::test]
fn not_installed_state(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    push(&view, cx, IndexState::NotInstalled, IndexInfo::default());
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(info) C# 基础导航");
    assert_eq!(
        e.tooltip.as_deref(),
        Some("跳转和引用是近似结果。安装语义组件后可以获得精确结果。")
    );
    click(&view, cx);
    let lines = details(&view, cx);
    assert!(lines.iter().any(|l| l.contains("安装方法")), "{lines:?}");
    view.read_with(cx, |v, _| {
        assert!(
            v.shell.feedback.notifications().is_empty(),
            "未安装不是失败"
        );
    });
}

#[gpui::test]
fn building_state(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    let building = IndexState::Building {
        progress: Progress {
            percent: 37,
            files: Some(200),
        },
        eta: Some(Duration::from_secs(150)),
        rebuilding: false,
    };
    push(&view, cx, building, IndexInfo::default());
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(sync~spin) C# 索引 37%");
    assert_eq!(
        e.tooltip.as_deref(),
        Some("已处理 74 / 200 个文件，预计还需 3 分钟。构建期间的结果是近似的。")
    );
    click(&view, cx);
    assert_eq!(details(&view, cx)[0], "C# 索引 37%");
}

#[gpui::test]
fn ready_state(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    push(&view, cx, IndexState::Ready, ready_info());
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(check) C#");
    let tip = e.tooltip.unwrap();
    for part in [
        "上次构建：",
        "文件数：1234，符号数：56789",
        r"缓存：C:\cache\game-0123456789abcdef（94.0 MB）",
    ] {
        assert!(tip.contains(part), "{tip}");
    }
    click(&view, cx);
    let lines = details(&view, cx);
    assert!(
        lines.iter().any(|l| l.starts_with("索引日志：")),
        "{lines:?}"
    );
    view.read_with(cx, |v, _| {
        assert_eq!(
            crate::index_status::details_actions(&v.index.status().state),
            [
                (REBUILD, "index.action.rebuild"),
                (OPEN_LOG, "index.action.openLog")
            ]
        );
    });
}

#[gpui::test]
fn updating_state(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    push(&view, cx, IndexState::Updating { files: 3 }, ready_info());
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(sync~spin) C#");
    assert_eq!(e.tooltip.as_deref(), Some("正在更新 3 个文件"));
    click(&view, cx);
    assert_eq!(details(&view, cx)[0], "C#");
}

#[gpui::test]
fn failed_state_notifies_once_per_reason(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    let failed = |kind| {
        IndexState::Failed(Failure {
            kind,
            detail: "indexer crashed (0xC0000005)".into(),
        })
    };
    // 同一次失败里的状态刷新不再通知；同一原因再次失败只增加次数，不新增通知。
    view.update(cx, |v, cx| {
        let epoch = v.index.attach(None, None);
        for (revision, state) in [
            (1, failed(FailureKind::Crashed)),
            (2, failed(FailureKind::Crashed)),
            (
                3,
                IndexState::Building {
                    progress: Progress::default(),
                    eta: None,
                    rebuilding: false,
                },
            ),
            (4, failed(FailureKind::Crashed)),
        ] {
            v.index_status_changed(
                epoch,
                IndexStatus {
                    state,
                    info: IndexInfo::default(),
                    revision,
                },
                cx,
            );
        }
    });
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(error) C# 索引失败");
    assert_eq!(e.tooltip.as_deref(), Some("C# 索引进程意外退出。"));
    view.read_with(cx, |v, _| {
        let n = v.shell.feedback.notifications();
        assert_eq!(n.len(), 1, "连续失败不刷屏");
        assert_eq!(n[0].count, 2);
        assert_eq!(&*n[0].key, "rove.index.failed.crashed");
        assert!(matches!(n[0].severity, Severity::Error));
        let actions: Vec<_> = n[0].actions.iter().map(|a| a.id.to_string()).collect();
        assert_eq!(actions, [SHOW_DETAILS, REBUILD]);
    });
    click(&view, cx);
    let lines = details(&view, cx);
    assert!(
        lines.contains(&"原因：indexer crashed (0xC0000005)".to_owned()),
        "{lines:?}"
    );
    view.read_with(cx, |v, _| {
        assert_eq!(
            crate::index_status::details_actions(&v.index.status().state)[0],
            (REBUILD, "index.action.retry")
        );
    });
}

#[gpui::test]
fn disabled_state_and_inferred_project_hint(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    push(&view, cx, IndexState::Disabled, IndexInfo::default());
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(circle-slash) C# 基础导航");
    assert_eq!(
        e.tooltip.as_deref(),
        Some("语义索引已在设置中禁用，跳转和引用是近似结果。")
    );
    click(&view, cx);
    assert!(details(&view, cx)
        .iter()
        .any(|l| l.contains("rove.index.enabled")));
    // 按 .asmdef 推断项目结构：悬停提示加上 §2.1 规定的那句话。
    for state in [IndexState::Ready, IndexState::NotInstalled] {
        push(
            &view,
            cx,
            state,
            IndexInfo {
                inferred_project: true,
                ..ready_info()
            },
        );
        assert!(entry(&view, cx)
            .unwrap()
            .tooltip
            .unwrap()
            .ends_with(INFERRED));
    }
}

#[gpui::test]
fn index_commands_are_in_the_palette_and_run(cx: &mut TestAppContext) {
    let (view, cx, _folder) = open(cx);
    cx.update(|_, cx| {
        for id in [REBUILD, SHOW_DETAILS, OPEN_LOG] {
            assert!(crate::keys::is_implemented(id, cx), "{id}");
            assert!(crate::keys::keys(cx).commands.is_registered(id));
        }
    });
    // 键位服务分发的 action（命令面板执行同一个 action）。
    cx.update(|window, cx| window.focus(&view.read(cx).focus));
    cx.dispatch_action(crate::index_status::ShowDetails);
    view.read_with(cx, |v, _| {
        assert!(v.shell.layout.panel_visible);
        assert_eq!(v.shell.active_panel_view(), DETAILS_VIEW);
    });
    // 还没有日志：状态栏临时消息说明，不打开空标签。
    cx.dispatch_action(crate::index_status::OpenLog);
    view.read_with(cx, |v, _| {
        assert_eq!(v.status(), "还没有索引日志。");
        assert!(v.editor.active().current().is_none());
    });
    // 有日志：在编辑器里打开。
    let log = tempfile::tempdir().unwrap();
    let path = log.path().join("indexer.log");
    std::fs::write(&path, "2026-11-11 00:00:00.000Z open project\n").unwrap();
    push(
        &view,
        cx,
        IndexState::Ready,
        IndexInfo {
            log: Some(path.clone()),
            ..ready_info()
        },
    );
    cx.dispatch_action(crate::index_status::OpenLog);
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.editor.find(&Resource::File(path.clone())).is_some());
    });
    // 通过命令 ID 执行（命令面板 / RunCommand 走 run_invocation）。
    view.update(cx, |v, cx| v.command(REBUILD, cx));
}

/// 假索引进程：按需失败，否则按磁盘内容写一份最小索引。
#[derive(Default)]
struct FakeIndexer {
    builds: AtomicUsize,
    failures: Mutex<Vec<BuildFailure>>,
}

impl IndexBuilder for FakeIndexer {
    fn build(
        &self,
        request: &BuildRequest,
        progress: &mut dyn FnMut(Progress),
        _: &IndexLog,
        _: &rove_base::Cancellation,
    ) -> Result<(), BuildFailure> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if let Some(f) = self.failures.lock().unwrap().pop() {
            return Err(f);
        }
        progress(Progress {
            percent: 50,
            files: Some(1),
        });
        FixtureIndex::for_project(&request.project_root, &["Assets/A.cs"], [3; 32])
            .write(&request.output)
            .map_err(|e| BuildFailure::InvalidOutput(e.to_string()))
    }
}

#[gpui::test]
fn lifecycle_drives_the_item_from_open_to_failure_and_retry(cx: &mut TestAppContext) {
    let (view, cx, folder) = open(cx);
    std::fs::create_dir(folder.path().join("Assets")).unwrap();
    std::fs::write(folder.path().join("Assets/A.cs"), "class A {}").unwrap();
    std::fs::write(folder.path().join("Game.csproj"), "<Project/>").unwrap();
    let cache = tempfile::tempdir().unwrap();
    let builder = Arc::new(FakeIndexer::default());
    // 首次构建两次都失败（重试一次后降级）。
    *builder.failures.lock().unwrap() = vec![BuildFailure::ExitCode(2), BuildFailure::ExitCode(2)];
    let config = LifecycleConfig::new(true, Ok(cache.path().to_owned()));
    let lifecycle = IndexLifecycle::start(folder.path().into(), config, builder.clone());
    view.update(cx, |v, cx| v.attach_index(lifecycle, cx));
    let wait =
        |view: &Entity<Workbench>, cx: &mut VisualTestContext, f: fn(&IndexStatus) -> bool| {
            view.read_with(cx, |v, _| {
                v.index
                    .lifecycle()
                    .unwrap()
                    .wait_until(Duration::from_secs(20), f)
            });
            cx.run_until_parked();
        };
    wait(&view, cx, |s| matches!(s.state, IndexState::Failed(_)));
    assert_eq!(entry(&view, cx).unwrap().text, "$(error) C# 索引失败");
    assert_eq!(builder.builds.load(Ordering::SeqCst), 2);
    let key = format!("{FAILURE_KEY_PREFIX}exitCode");
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.notifications().len(), 1);
        assert_eq!(&*v.shell.feedback.notifications()[0].key, key.as_str());
    });
    // 通知上的“重试”：重建成功，状态回到就绪，失败通知收起。
    view.update(cx, |v, _| v.shell.feedback.act(&key, REBUILD).unwrap());
    cx.run_until_parked();
    wait(&view, cx, |s| s.state == IndexState::Ready);
    let e = entry(&view, cx).unwrap();
    assert_eq!(e.text, "$(check) C#");
    assert!(e.tooltip.unwrap().contains("文件数：1，"));
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.notifications().is_empty())
    });
    // 文件变化：经工作区事件转换后标记过期。
    view.read_with(cx, |v, _| {
        v.index_files_changed(&[index_change(&FileEvent::Changed(
            WorkspacePath::new("Assets/A.cs").unwrap(),
        ))]);
        assert!(v.index.lifecycle().unwrap().is_stale("Assets/A.cs"));
    });
    cx.run_until_parked();
    assert!(entry(&view, cx)
        .unwrap()
        .tooltip
        .unwrap()
        .contains("1 个文件在建立索引后被修改"));
}
