//! 资源管理器的数据模型（#23，无 GPUI）：设置、排序、紧凑文件夹、树数据源、刷新与定位计划。
use crate::{
    list_widget::tree::{ChildCount, Children, TreeItem, TreeSource},
    lists::ItemId,
};
use rove_base::{Error, Value, WorkspacePath};
use rove_config::{Scope, Settings};
use rove_i18n::{Message, Strings};
use rove_workspace::{fold_key, Entry, EntryKind, FileEvent, RuleSet, Workspace};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
};

/// 读取目录失败时的提示行：ID 是目录路径加这个后缀，打开 / 装饰都要认得它。
pub const ERROR_MARK: &str = "\u{1}error";
/// 链最长深度（防止异常的符号链接环或极深的目录）。
const MAX_CHAIN: usize = 256;
/// 子项达到这个数量时并行探测紧凑文件夹链。
const PARALLEL_PROBE: usize = 512;

/// 这一行是不是“读取失败”提示行。
pub fn is_error_id(id: &str) -> bool {
    id.ends_with(ERROR_MARK)
}

/// `explorer.sortOrder`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    /// 文件夹在前，按名称。
    Default,
    /// 文件与文件夹混排。
    Mixed,
    FilesFirst,
    /// 按扩展名分组，再按名称。
    Type,
    /// 需要修改时间，#15 不提供，按 `Default` 处理。
    Modified,
    /// 文件嵌套属于 P2，按 `Default` 处理。
    FoldersNestsFiles,
}

/// `explorer.sortOrderLexicographicOptions`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lexicographic {
    Default,
    Upper,
    Lower,
    Unicode,
}

/// `explorer.autoReveal`：true / false / `focusNoScroll`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoReveal {
    Off,
    On,
    FocusNoScroll,
}

/// 资源管理器读取的全部设置（每个设置版本读一次，绘制时不再读设置）。
#[derive(Clone, Debug)]
pub struct ExplorerSettings {
    pub compact_folders: bool,
    pub sort: SortOrder,
    pub lexicographic: Lexicographic,
    pub reverse: bool,
    pub auto_reveal: AutoReveal,
    pub auto_reveal_exclude: RuleSet,
    /// `workbench.editor.decorations.badges` / `.colors`。
    pub badges: bool,
    pub colors: bool,
}

impl Default for ExplorerSettings {
    fn default() -> Self {
        Self {
            compact_folders: true,
            sort: SortOrder::Default,
            lexicographic: Lexicographic::Default,
            reverse: false,
            auto_reveal: AutoReveal::On,
            auto_reveal_exclude: RuleSet::empty(),
            badges: true,
            colors: true,
        }
    }
}

/// 设置键（变化时宿主据此决定是否刷新资源管理器）。
pub const SETTING_KEYS: &[&str] = &[
    "explorer.compactFolders",
    "explorer.sortOrder",
    "explorer.sortOrderLexicographicOptions",
    "explorer.sortOrderReverse",
    "explorer.autoReveal",
    "explorer.autoRevealExclude",
    "workbench.editor.decorations.badges",
    "workbench.editor.decorations.colors",
];

impl ExplorerSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        let get = |key: &str| settings.read(key, &Scope::default()).ok();
        let flag = |key: &str, default: bool| match get(key) {
            Some(Value::Bool(b)) => b,
            _ => default,
        };
        let text = |key: &str| match get(key) {
            Some(Value::String(s)) => Some(s),
            _ => None,
        };
        Self {
            compact_folders: flag("explorer.compactFolders", true),
            sort: match text("explorer.sortOrder").as_deref() {
                Some("mixed") => SortOrder::Mixed,
                Some("filesFirst") => SortOrder::FilesFirst,
                Some("type") => SortOrder::Type,
                Some("modified") => SortOrder::Modified,
                Some("foldersNestsFiles") => SortOrder::FoldersNestsFiles,
                _ => SortOrder::Default,
            },
            lexicographic: match text("explorer.sortOrderLexicographicOptions").as_deref() {
                Some("upper") => Lexicographic::Upper,
                Some("lower") => Lexicographic::Lower,
                Some("unicode") => Lexicographic::Unicode,
                _ => Lexicographic::Default,
            },
            reverse: flag("explorer.sortOrderReverse", false),
            auto_reveal: match get("explorer.autoReveal") {
                Some(Value::Bool(false)) => AutoReveal::Off,
                Some(Value::String(s)) if &*s == "focusNoScroll" => AutoReveal::FocusNoScroll,
                _ => AutoReveal::On,
            },
            auto_reveal_exclude: get("explorer.autoRevealExclude")
                .and_then(|v| RuleSet::from_value(&v).ok())
                .unwrap_or_default(),
            badges: flag("workbench.editor.decorations.badges", true),
            colors: flag("workbench.editor.decorations.colors", true),
        }
    }
}

// ------------------------------------------------------------------ 排序

/// 数字按数值比较、字母不分大小写的“自然”比较（VS Code `compareFileNames` 的行为）。
fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (run(&mut x), run(&mut y));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(p), Some(q)) => {
                x.next();
                y.next();
                let order = p.to_lowercase().cmp(q.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// 名称比较；大小写只在其余都相同时决定顺序。
pub fn compare_names(a: &str, b: &str, lexicographic: Lexicographic) -> Ordering {
    if lexicographic == Lexicographic::Unicode {
        return a.cmp(b);
    }
    natural(a, b).then_with(|| {
        for (p, q) in a.chars().zip(b.chars()) {
            if p != q {
                let lower_first = lexicographic != Lexicographic::Upper;
                return if p.is_lowercase() == lower_first {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
        }
        a.len().cmp(&b.len())
    })
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(0) | None => "",
        Some(i) => &name[i + 1..],
    }
}

fn is_dir(entry: &Entry) -> bool {
    matches!(entry.kind, EntryKind::Directory)
}

/// 按设置排序一个目录的子项（就地）。
pub fn sort_entries(entries: &mut [Entry], settings: &ExplorerSettings) {
    let lex = settings.lexicographic;
    entries.sort_by(|a, b| {
        let (na, nb) = (file_name(a.path.as_str()), file_name(b.path.as_str()));
        let by_name = || compare_names(na, nb, lex);
        let group = match settings.sort {
            SortOrder::Mixed => Ordering::Equal,
            SortOrder::FilesFirst => is_dir(a).cmp(&is_dir(b)),
            _ => is_dir(b).cmp(&is_dir(a)),
        };
        let order = group.then_with(|| match settings.sort {
            SortOrder::Type if !is_dir(a) && !is_dir(b) => {
                compare_names(extension(na), extension(nb), lex).then_with(by_name)
            }
            _ => by_name(),
        });
        if settings.reverse {
            order.reverse()
        } else {
            order
        }
    });
}

// ------------------------------------------------------------------ 路径辅助

/// 父目录路径；顶层返回 `None`（根）。
pub fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

/// 紧凑文件夹行的各段路径：`id` 是链首目录，`label` 是 `a/b/c`。普通行只有一段。
pub fn chain_paths(id: &str, label: &str) -> Vec<String> {
    let first = file_name(id);
    let prefix = &id[..id.len() - first.len()];
    let mut out = Vec::new();
    let mut acc = String::from(prefix);
    for (i, name) in label.split('/').enumerate() {
        if i > 0 {
            acc.push('/');
        }
        acc.push_str(name);
        out.push(acc.clone());
    }
    out
}

fn path(value: &str) -> Option<WorkspacePath> {
    WorkspacePath::new(value).ok()
}

// ------------------------------------------------------------------ 树数据源

#[derive(Default)]
struct Shared {
    /// 每个已显示的目录节点（链首的 key）→ 链上各目录。
    chains: HashMap<String, Arc<[WorkspacePath]>>,
    /// 链上每个目录（含链首）的 key → 链首的 key。
    member_of: HashMap<String, String>,
    /// 子节点已经加载过的节点（链首的 key）。
    loaded: HashSet<String>,
}

/// 一次文件事件批次需要刷新的节点。
#[derive(Debug, PartialEq, Eq)]
pub enum Refresh {
    /// 整棵树按新数据重新装填（展开状态仍按 ID 保留）。
    Full,
    /// 这些节点（`None` = 根）的子节点需要重新装填。
    Nodes(Vec<Option<WorkspacePath>>),
}

/// 定位当前文件的计划：按顺序展开 `expand`，再选中 `target`。
#[derive(Debug, PartialEq, Eq)]
pub struct RevealPlan {
    pub expand: Vec<ItemId>,
    pub target: ItemId,
}

/// 资源管理器树的数据源：目录树来自 #15，展开时才向它要子节点；排序与紧凑文件夹在这里完成。
pub struct ExplorerSource {
    workspace: Arc<dyn Workspace>,
    settings: ExplorerSettings,
    strings: Arc<dyn Strings>,
    shared: Mutex<Shared>,
}

impl ExplorerSource {
    pub fn new(
        workspace: Arc<dyn Workspace>,
        settings: ExplorerSettings,
        strings: Arc<dyn Strings>,
    ) -> Self {
        Self {
            workspace,
            settings,
            strings,
            shared: Mutex::new(Shared::default()),
        }
    }

    pub fn workspace(&self) -> &Arc<dyn Workspace> {
        &self.workspace
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn list(&self, dir: Option<&WorkspacePath>) -> Result<Vec<Entry>, Error> {
        let page = self.workspace.children(dir, 0..usize::MAX)?;
        Ok(page.items.to_vec())
    }

    /// 从 `top` 起沿“只有一个子目录”的链向下的所有目录（含 `top`）。
    fn chain_from(&self, top: &WorkspacePath) -> Vec<WorkspacePath> {
        let mut chain = vec![top.clone()];
        if !self.settings.compact_folders {
            return chain;
        }
        while chain.len() < MAX_CHAIN {
            let last = chain.last().expect("chain is never empty");
            let Ok(page) = self.workspace.children(Some(last), 0..2) else {
                break;
            };
            match page.items.as_ref() {
                [only] if is_dir(only) => chain.push(only.path.clone()),
                _ => break,
            }
        }
        chain
    }

    fn error_item(&self, dir: Option<&WorkspacePath>, error: &Error) -> TreeItem {
        let key = match error {
            Error::Io { kind, .. } if *kind == std::io::ErrorKind::PermissionDenied => {
                "explorer.readFailed.permission"
            }
            Error::Io { kind, .. } if *kind == std::io::ErrorKind::NotFound => {
                "explorer.readFailed.missing"
            }
            Error::NotFound => "explorer.readFailed.missing",
            Error::Io { message, .. } if message.contains("too long") => {
                "explorer.readFailed.tooLong"
            }
            _ => "explorer.readFailed.other",
        };
        let label = self
            .strings
            .text(&Message::key(key))
            .unwrap_or_else(|_| key.to_owned());
        let owner = dir.map_or("", WorkspacePath::as_str);
        TreeItem {
            id: ItemId(format!("{owner}{ERROR_MARK}").into()),
            label: label.into(),
            detail: None,
            children: ChildCount::None,
        }
    }

    /// 某个节点（`None` = 根）的子项。
    pub fn items(&self, node: Option<&WorkspacePath>) -> Arc<[TreeItem]> {
        // 紧凑文件夹节点的子项是链尾目录的子项。
        let listing = node.map(|top| {
            let known = self.shared().chains.get(&fold_key(top.as_str())).cloned();
            known
                .and_then(|chain| chain.last().cloned())
                .unwrap_or_else(|| top.clone())
        });
        let mut entries = match self.list(listing.as_ref()) {
            Ok(entries) => entries,
            Err(error) => {
                return Arc::from([self.error_item(listing.as_ref(), &error)]);
            }
        };
        sort_entries(&mut entries, &self.settings);
        // 每个子目录要探测一次“单子文件夹链”：目录很多时并行（1 万个子目录是最坏情况）。
        let probes: Vec<Option<Vec<WorkspacePath>>> = if entries.len() >= PARALLEL_PROBE {
            use rayon::prelude::*;
            entries
                .par_iter()
                .map(|e| is_dir(e).then(|| self.chain_from(&e.path)))
                .collect()
        } else {
            entries
                .iter()
                .map(|e| is_dir(e).then(|| self.chain_from(&e.path)))
                .collect()
        };
        let mut chains = Vec::new();
        let items: Arc<[TreeItem]> = entries
            .into_iter()
            .zip(probes)
            .map(|(entry, probe)| {
                let name = file_name(entry.path.as_str());
                if !is_dir(&entry) {
                    return TreeItem {
                        id: ItemId(entry.path.as_str().into()),
                        label: name.into(),
                        detail: None,
                        children: ChildCount::None,
                    };
                }
                let chain = probe.unwrap_or_else(|| vec![entry.path.clone()]);
                let label = if chain.len() == 1 {
                    name.to_owned()
                } else {
                    let mut label = name.to_owned();
                    for member in &chain[1..] {
                        label.push('/');
                        label.push_str(file_name(member.as_str()));
                    }
                    label
                };
                let item = TreeItem {
                    id: ItemId(entry.path.as_str().into()),
                    label: label.into(),
                    detail: None,
                    children: ChildCount::Unknown,
                };
                chains.push((entry.path, chain));
                item
            })
            .collect();
        let mut shared = self.shared();
        if let Some(top) = node {
            shared.loaded.insert(fold_key(top.as_str()));
        }
        for (top, chain) in chains {
            let key = fold_key(top.as_str());
            for member in &chain {
                shared
                    .member_of
                    .insert(fold_key(member.as_str()), key.clone());
            }
            shared.chains.insert(key, Arc::from(chain));
        }
        items
    }

    /// 这个节点的子节点是否已经加载过。
    pub fn is_loaded(&self, node: &WorkspacePath) -> bool {
        self.shared().loaded.contains(&fold_key(node.as_str()))
    }

    /// 节点当前显示的链（链首的路径 → 各目录）；普通目录只有一项。
    pub fn chain_of(&self, top: &WorkspacePath) -> Vec<WorkspacePath> {
        self.shared()
            .chains
            .get(&fold_key(top.as_str()))
            .map(|chain| chain.to_vec())
            .unwrap_or_else(|| vec![top.clone()])
    }

    /// 列出 `top` 的节点所在的父节点：根返回 `Some(None)`；父目录不属于任何已显示的节点返回 `None`。
    fn listing_node(&self, top: &WorkspacePath) -> Option<Option<WorkspacePath>> {
        let Some(parent) = parent_path(top.as_str()) else {
            return Some(None);
        };
        let shared = self.shared();
        let owner = shared.member_of.get(&fold_key(parent))?;
        let chain = shared.chains.get(owner)?;
        Some(chain.first().cloned())
    }

    /// 一批文件事件需要刷新哪些节点。只处理已经显示或已经加载的部分：
    /// 目录自己的链变了（单子目录变多或变少）刷新它所在的节点，否则只刷新它的子项。
    pub fn plan_refresh(&self, events: &[FileEvent]) -> Refresh {
        let mut dirs: BTreeSet<Option<String>> = BTreeSet::new();
        for event in events {
            match event {
                FileEvent::RescanRequired => return Refresh::Full,
                FileEvent::Created(p) | FileEvent::Removed(p) => {
                    dirs.insert(parent_path(p.as_str()).map(str::to_owned));
                }
                FileEvent::Renamed { from, to } => {
                    dirs.insert(parent_path(from.as_str()).map(str::to_owned));
                    dirs.insert(parent_path(to.as_str()).map(str::to_owned));
                }
                FileEvent::Changed(_) => {}
            }
        }
        let mut targets: BTreeSet<Option<String>> = BTreeSet::new();
        for dir in dirs {
            let Some(dir) = dir else {
                targets.insert(None);
                continue;
            };
            let key = fold_key(&dir);
            let top_key = self.shared().member_of.get(&key).cloned();
            let Some(top_key) = top_key else { continue };
            let Some(old) = self.shared().chains.get(&top_key).cloned() else {
                continue;
            };
            let Some(top) = old.first().cloned() else {
                continue;
            };
            let fresh = self.chain_from(&top);
            let same = fresh.len() == old.len()
                && fresh
                    .iter()
                    .zip(old.iter())
                    .all(|(a, b)| a.as_str() == b.as_str());
            if !same {
                if let Some(owner) = self.listing_node(&top) {
                    targets.insert(owner.map(|p| p.as_str().to_owned()));
                }
            } else if old
                .last()
                .is_some_and(|last| fold_key(last.as_str()) == key)
                && self.is_loaded(&top)
            {
                targets.insert(Some(top.as_str().to_owned()));
            }
        }
        Refresh::Nodes(
            targets
                .into_iter()
                .map(|t| t.and_then(|p| path(&p)))
                .collect(),
        )
    }

    /// 在 `parent` 下找名称（不分大小写）等于 `name` 的子项，返回磁盘上的写法。
    fn find_child(&self, parent: Option<&WorkspacePath>, wanted: &str) -> Option<Entry> {
        let key = fold_key(wanted);
        let page = self.workspace.children(parent, 0..usize::MAX).ok()?;
        page.items
            .iter()
            .find(|entry| {
                let name = file_name(entry.path.as_str());
                name.len() == file_name(wanted).len() && fold_key(entry.path.as_str()) == key
            })
            .cloned()
    }

    /// 定位 `target`：沿路径逐级展开（紧凑文件夹一次跨多级），最后选中它所在的节点。
    /// 路径不在工作区（被排除、外部文件）返回 `None`。
    pub fn reveal_plan(&self, target: &WorkspacePath) -> Option<RevealPlan> {
        let segments: Vec<&str> = target.as_str().split('/').collect();
        // all[k] = 前 k+1 段的规范路径（磁盘上的大小写）。
        let mut all: Vec<WorkspacePath> = Vec::with_capacity(segments.len());
        let mut parent: Option<WorkspacePath> = None;
        for i in 0..segments.len() {
            let wanted = segments[..=i].join("/");
            let entry = self.find_child(parent.as_ref(), &wanted)?;
            if i + 1 < segments.len() && !is_dir(&entry) {
                return None;
            }
            parent = Some(entry.path.clone());
            all.push(entry.path);
        }
        let mut expand = Vec::new();
        let mut k = 0;
        while k < all.len() {
            let top = &all[k];
            let last = k + 1 == all.len();
            if last {
                return Some(RevealPlan {
                    expand,
                    target: ItemId(top.as_str().into()),
                });
            }
            let chain = self.chain_from(top);
            let end = (k + chain.len()).min(all.len());
            // 目标落在链内（含链尾本身）：选中整条链。
            if end >= all.len() {
                return Some(RevealPlan {
                    expand,
                    target: ItemId(top.as_str().into()),
                });
            }
            expand.push(ItemId(top.as_str().into()));
            k += chain.len();
        }
        None
    }
}

impl TreeSource for ExplorerSource {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        let node = parent.and_then(|id| path(&id.0));
        if parent.is_some() && node.is_none() {
            return Children::Ready(Arc::from([]));
        }
        Children::Ready(self.items(node.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_naturally_and_case_decides_last() {
        let lex = Lexicographic::Default;
        assert_eq!(compare_names("a2", "a10", lex), Ordering::Less);
        assert_eq!(compare_names("File2.cs", "file10.cs", lex), Ordering::Less);
        assert_eq!(compare_names("b", "A", lex), Ordering::Greater);
        assert_eq!(compare_names("a", "A", lex), Ordering::Less);
        assert_eq!(
            compare_names("a", "A", Lexicographic::Upper),
            Ordering::Greater
        );
        assert_eq!(
            compare_names("B", "a", Lexicographic::Unicode),
            Ordering::Less
        );
    }

    #[test]
    fn chain_paths_follow_the_label() {
        assert_eq!(chain_paths("Assets", "Assets"), ["Assets"]);
        assert_eq!(
            chain_paths("Assets/Scripts", "Scripts/Runtime/Core"),
            [
                "Assets/Scripts",
                "Assets/Scripts/Runtime",
                "Assets/Scripts/Runtime/Core"
            ]
        );
    }

    #[test]
    fn parent_of_top_level_is_the_root() {
        assert_eq!(parent_path("a"), None);
        assert_eq!(parent_path("a/b"), Some("a"));
        assert!(is_error_id("a/b\u{1}error"));
    }
}
