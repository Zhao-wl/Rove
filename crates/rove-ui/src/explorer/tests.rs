//! 资源管理器数据模型的单元测试（真实的 #15 `Folder`，无窗口）。
use super::{
    decorations::{decorate, Badge, DecorationColor, DecorationProvider, ReadOnlyDecorations},
    model::{is_error_id, ExplorerSettings, ExplorerSource, Refresh, SortOrder},
};
use crate::{list_widget::tree::TreeItem, skeleton::appearance};
use rove_base::WorkspacePath;
use rove_workspace::{EmptyWorkspace, FileEvent, Folder, RuleSet, Workspace, WorkspaceConfig};
use std::{path::Path, sync::Arc};

fn touch(root: &Path, rel: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "x").unwrap();
}

fn p(s: &str) -> WorkspacePath {
    WorkspacePath::new(s).unwrap()
}

fn source(root: &Path, settings: ExplorerSettings) -> (Arc<Folder>, ExplorerSource) {
    let folder = Folder::scan_root(root, WorkspaceConfig::default().without_watching()).unwrap();
    let source = ExplorerSource::new(
        folder.clone(),
        settings,
        Arc::new(appearance::strings().unwrap()),
    );
    (folder, source)
}

fn labels(items: &[TreeItem]) -> Vec<String> {
    items.iter().map(|i| i.label.to_string()).collect()
}

#[test]
fn single_child_folders_compress_into_one_node_identified_by_the_first_folder() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "Assets/Scripts/Runtime/Player.cs");
    touch(dir.path(), "Assets/Scripts/Runtime/Core/Bus.cs");
    touch(dir.path(), "Assets/Scripts/Runtime/Core/Bus2.cs");
    touch(dir.path(), "Readme.md");
    let (_folder, source) = source(dir.path(), ExplorerSettings::default());
    let root = source.items(None);
    assert_eq!(labels(&root), ["Assets/Scripts/Runtime", "Readme.md"]);
    assert_eq!(&*root[0].id.0, "Assets");
    assert_eq!(source.chain_of(&p("Assets")).len(), 3);
    // 展开压缩节点：子项是链尾目录的子项。
    let inner = source.items(Some(&p("Assets")));
    assert_eq!(labels(&inner), ["Core", "Player.cs"]);
    assert_eq!(&*inner[0].id.0, "Assets/Scripts/Runtime/Core");
}

#[test]
fn compact_folders_off_shows_every_level() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "A/B/c.cs");
    let settings = ExplorerSettings {
        compact_folders: false,
        ..Default::default()
    };
    let (_folder, source) = source(dir.path(), settings);
    assert_eq!(labels(&source.items(None)), ["A"]);
    assert_eq!(labels(&source.items(Some(&p("A")))), ["B"]);
}

#[test]
fn sort_order_settings_apply() {
    let dir = tempfile::tempdir().unwrap();
    for rel in ["b.txt", "a10.cs", "a2.cs", "Z/x.cs", "m/x.cs"] {
        touch(dir.path(), rel);
    }
    let names = |settings| {
        let (_f, source) = source(dir.path(), settings);
        labels(&source.items(None))
    };
    assert_eq!(
        names(ExplorerSettings::default()),
        ["m", "Z", "a2.cs", "a10.cs", "b.txt"]
    );
    assert_eq!(
        names(ExplorerSettings {
            sort: SortOrder::FilesFirst,
            ..Default::default()
        }),
        ["a2.cs", "a10.cs", "b.txt", "m", "Z"]
    );
    assert_eq!(
        names(ExplorerSettings {
            sort: SortOrder::Mixed,
            ..Default::default()
        }),
        ["a2.cs", "a10.cs", "b.txt", "m", "Z"]
    );
    assert_eq!(
        names(ExplorerSettings {
            reverse: true,
            ..Default::default()
        }),
        ["b.txt", "a10.cs", "a2.cs", "Z", "m"]
    );
}

#[test]
fn reveal_plans_resolve_paths_to_nodes_not_rows() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "Assets/Scripts/Runtime/Player.cs");
    touch(dir.path(), "Assets/Scripts/Runtime/Core/Bus.cs");
    touch(dir.path(), "Assets/Scripts/Runtime/Core/Bus2.cs");
    let (_folder, source) = source(dir.path(), ExplorerSettings::default());
    source.items(None);
    // 文件在压缩节点之下：先展开链首，再选中文件。
    let plan = source
        .reveal_plan(&p("Assets/Scripts/Runtime/Core/Bus.cs"))
        .unwrap();
    let expand: Vec<_> = plan.expand.iter().map(|i| &*i.0).collect();
    assert_eq!(expand, ["Assets", "Assets/Scripts/Runtime/Core"]);
    assert_eq!(&*plan.target.0, "Assets/Scripts/Runtime/Core/Bus.cs");
    // 目标是链中间的目录：选中整条链（链首节点）。
    let plan = source.reveal_plan(&p("Assets/Scripts")).unwrap();
    assert!(plan.expand.is_empty());
    assert_eq!(&*plan.target.0, "Assets");
    // 大小写不同的写法解析到磁盘上的写法（Windows 路径不区分大小写）。
    if cfg!(windows) {
        let plan = source
            .reveal_plan(&p("assets/scripts/runtime/player.cs"))
            .unwrap();
        assert_eq!(&*plan.target.0, "Assets/Scripts/Runtime/Player.cs");
    }
    // 不存在的路径、把文件当目录的路径：不定位。
    assert!(source.reveal_plan(&p("Assets/Nope.cs")).is_none());
    assert!(source
        .reveal_plan(&p("Assets/Scripts/Runtime/Player.cs/x"))
        .is_none());
}

#[test]
fn refresh_plans_target_only_the_nodes_whose_shape_changed() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "Assets/Scripts/Runtime/Player.cs");
    touch(dir.path(), "Other/a.cs");
    let (_folder, source) = source(dir.path(), ExplorerSettings::default());
    source.items(None);
    source.items(Some(&p("Assets")));
    let created = |path: &str| [FileEvent::Created(p(path))];
    // 链尾目录里新增文件：链不变，只刷新链首节点的子项。
    assert_eq!(
        source.plan_refresh(&created("Assets/Scripts/Runtime/New.cs")),
        Refresh::Nodes(vec![Some(p("Assets"))])
    );
    // 根里的变化刷新根；Changed 不触发刷新；溢出要求整棵重建。
    assert_eq!(
        source.plan_refresh(&created("Top.cs")),
        Refresh::Nodes(vec![None])
    );
    assert_eq!(
        source.plan_refresh(&[FileEvent::Changed(p("Other/a.cs"))]),
        Refresh::Nodes(vec![])
    );
    assert_eq!(
        source.plan_refresh(&[FileEvent::RescanRequired]),
        Refresh::Full
    );
}

#[test]
fn an_unreadable_directory_shows_one_line_and_leaves_the_rest_alone() {
    // EmptyWorkspace 对任何子目录都报告 NotFound（目录已不存在或无法读取）。
    let source = ExplorerSource::new(
        Arc::new(EmptyWorkspace),
        ExplorerSettings::default(),
        Arc::new(appearance::strings().unwrap()),
    );
    let items = source.items(Some(&p("Gone")));
    assert_eq!(items.len(), 1);
    assert!(is_error_id(&items[0].id.0));
    assert!(items[0].label.contains("无法读取"));
}

#[test]
fn read_only_files_get_a_lock_and_nothing_comes_from_git() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "Library/PackageCache/pkg/P.cs");
    touch(dir.path(), "Assets/A.cs");
    // 只读规则默认包含 Library/PackageCache；这里让 Library 不被排除。
    let mut config = WorkspaceConfig::default().without_watching();
    config.exclude = RuleSet::empty();
    let workspace: Arc<dyn Workspace> = Folder::scan_root(dir.path(), config).unwrap();
    let readonly: Arc<dyn DecorationProvider> = Arc::new(ReadOnlyDecorations(workspace));
    let providers = [readonly];
    let lock = decorate(&providers, &p("Library/PackageCache/pkg/P.cs"), false).unwrap();
    assert_eq!(lock.badge, Some(Badge::Lock));
    assert_eq!(lock.color, Some(DecorationColor::Deemphasized));
    assert!(decorate(&providers, &p("Assets/A.cs"), false).is_none());
    // 没有任何 Git 来源：装饰只可能来自只读与问题。
    assert!(decorate(&[], &p("Assets/A.cs"), false).is_none());
}
