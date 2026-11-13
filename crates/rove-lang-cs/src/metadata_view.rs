//! #42 元数据视图的内容与位置（核心层，不依赖 gpui）。
//!
//! 视图正文由 #34 在建索引时生成（段 13，`CsIndex::metadata_view_text`）。本模块把它变成界面可用的“一份普通
//! 文档”所需的全部数据：
//!
//! - **文档标识**：`rove-metadata:<最外层类型的全局符号键>`（[`VIEW_SCHEME`]）。键取自索引的符号键
//!   （`程序集|T:命名空间.类型`），不用显示标题，所以 `List<T>` 与 `List<String>` 不会被混为一谈，嵌套类型总是
//!   落在外层类型的视图里；同一个类型因此只会有一个标签（#17 的打开文档服务按资源复用标签）。
//! - **`rove.metadataView.showDocs`**：为 false 时正文去掉 `///` 行。显示行号与索引里成员位置的行号因此不同，
//!   行号换算（[`MetadataViews::to_display`]、[`original_line`]）全部在这里，界面与查询路由共用同一套。
//! - **位置 → 符号**：视图内的查询点（显示坐标）翻译回索引坐标，再按成员位置记录或按名字解析
//!   （`router/metadata.rs`）。

use crate::index::{CsIndex, SymbolKey};
use crate::router::{L2Snapshot, SemanticSource};
use rove_base::Resource;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

/// 元数据视图文档的资源方案（`Resource::Virtual("rove-metadata:<类型键>")`）。
pub const VIEW_SCHEME: &str = "rove-metadata:";

/// 视图文档的语言（与普通 `.cs` 文件相同的语法与主题，不另配色）。
pub const VIEW_LANGUAGE: &str = "csharp";

/// 视图资源里的类型键。
pub fn view_key(resource: &Resource) -> Option<&str> {
    match resource {
        Resource::Virtual(uri) => uri.strip_prefix(VIEW_SCHEME),
        _ => None,
    }
}

/// 类型键 → 视图资源。
pub fn view_resource(type_key: &str) -> Resource {
    Resource::Virtual(format!("{VIEW_SCHEME}{type_key}").into())
}

/// 类型键里的程序集名（`程序集|T:…` 的前半）。
pub fn assembly_of(type_key: &str) -> &str {
    type_key.split_once('|').map_or(type_key, |(a, _)| a)
}

/// 为什么没有内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// 索引没有就绪（构建中、组件未安装、已禁用）。
    NotReady,
    /// 索引里没有这个符号，或者它没有元数据视图。
    NotFound,
}

/// 一份视图的显示内容。
#[derive(Clone, Debug)]
pub struct ViewContent {
    /// 显示的正文（`showDocs = false` 时已去掉文档注释）。
    pub text: String,
    /// 标签标题里的类型名，泛型带类型参数，如 `List<T>`。
    pub title: String,
    /// 文件头 `#region` 里的程序集显示名（含版本）。
    pub assembly: String,
    /// DLL 完整路径（标签悬停提示）。
    pub dll_path: Option<String>,
    /// 取内容时的索引快照代次（索引原子替换后据此刷新）。
    pub generation: u64,
}

/// 一个成员在视图里的位置（显示坐标）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewLocation {
    /// 视图的类型键（最外层类型）。
    pub type_key: String,
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

/// 元数据视图的内容来源：语义来源（#36 的快照）+ `rove.metadataView.showDocs`。
pub struct MetadataViews {
    semantic: Arc<dyn SemanticSource>,
    show_docs: AtomicBool,
    /// 按快照代次缓存的“类型名 → 有视图的外部类型符号”（视图内按名字解析类型用）。
    names: Mutex<Option<(u64, Arc<HashMap<String, Vec<u32>>>)>>,
}

impl MetadataViews {
    pub fn new(semantic: Arc<dyn SemanticSource>, show_docs: bool) -> Self {
        Self {
            semantic,
            show_docs: AtomicBool::new(show_docs),
            names: Mutex::new(None),
        }
    }

    pub fn show_docs(&self) -> bool {
        self.show_docs.load(Ordering::Relaxed)
    }

    /// 返回值表示是否真的变了。
    pub fn set_show_docs(&self, show: bool) -> bool {
        self.show_docs.swap(show, Ordering::Relaxed) != show
    }

    /// 当前索引快照代次（没有快照为 `None`）。
    pub fn generation(&self) -> Option<u64> {
        self.semantic.snapshot().map(|s| s.generation)
    }

    fn snapshot(&self) -> Result<L2Snapshot, ViewError> {
        self.semantic.snapshot().ok_or(ViewError::NotReady)
    }

    /// 符号键（成员或类型）→ 它在视图里的位置。键里的符号沿容器找到最外层类型，视图用外层类型的键标识。
    pub fn locate(&self, symbol_key: &str) -> Result<ViewLocation, ViewError> {
        let snap = self.snapshot()?;
        let index = snap.holder.index();
        let symbol = index
            .find_symbol(SymbolKey::Global(symbol_key))
            .ok_or(ViewError::NotFound)?;
        let location = *index
            .metadata_definitions(symbol)
            .first()
            .ok_or(ViewError::NotFound)?;
        let outer = outermost(index, symbol);
        let type_key = global_key(index, outer).ok_or(ViewError::NotFound)?;
        let text = index
            .metadata_view_text(location.view)
            .map_err(|_| ViewError::NotFound)?;
        Ok(ViewLocation {
            type_key: type_key.to_owned(),
            line: to_display(text, location.line, self.show_docs()),
            col: location.col.0 as u32,
            len: location.len,
        })
    }

    /// 类型键 → 视图内容。
    pub fn content(&self, type_key: &str) -> Result<ViewContent, ViewError> {
        let snap = self.snapshot()?;
        let index = snap.holder.index();
        let symbol = index
            .find_symbol(SymbolKey::Global(type_key))
            .ok_or(ViewError::NotFound)?;
        let view = first_view(index, symbol).ok_or(ViewError::NotFound)?;
        let original = index
            .metadata_view_text(view)
            .map_err(|_| ViewError::NotFound)?;
        let mut lines = original.lines();
        let assembly = lines
            .next()
            .and_then(|l| l.strip_prefix("#region"))
            .map(|l| l.trim().trim_start_matches("程序集").trim().to_owned())
            .unwrap_or_else(|| assembly_of(type_key).to_owned());
        let dll_path = lines
            .next()
            .and_then(|l| l.strip_prefix("//"))
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty());
        let title = title_of(index, view, symbol, original);
        Ok(ViewContent {
            text: display_text(original, self.show_docs()),
            title,
            assembly,
            dll_path,
            generation: snap.generation,
        })
    }

    /// 视图内按名字解析类型：有视图的外部类型里名字相同的符号（按快照代次缓存）。
    pub(crate) fn types_named(&self, snap: &L2Snapshot, name: &str) -> Vec<u32> {
        let map = {
            let mut cache = self.names.lock().unwrap_or_else(|e| e.into_inner());
            match &*cache {
                Some((generation, map)) if *generation == snap.generation => map.clone(),
                _ => {
                    let index = snap.holder.index();
                    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
                    for record in index.metadata_view_records() {
                        let symbol = record.type_symbol;
                        let Some(rec) = index.symbols().get(symbol as usize) else {
                            continue;
                        };
                        if let Ok(n) = index.str(rec.name) {
                            let list = map.entry(n.to_owned()).or_default();
                            if !list.contains(&symbol) {
                                list.push(symbol);
                            }
                        }
                    }
                    let map = Arc::new(map);
                    *cache = Some((snap.generation, map.clone()));
                    map
                }
            }
        };
        map.get(name).cloned().unwrap_or_default()
    }

    /// 视图的原文（含文档注释，索引坐标）；行号换算用。
    pub fn original_text(&self, type_key: &str) -> Option<String> {
        let snap = self.semantic.snapshot()?;
        let index = snap.holder.index();
        let symbol = index.find_symbol(SymbolKey::Global(type_key))?;
        let view = first_view(index, symbol)?;
        index.metadata_view_text(view).ok().map(str::to_owned)
    }

    /// 原始行号（索引坐标）→ 当前显示的行号。
    pub fn to_display(&self, original_text: &str, line: u32) -> u32 {
        to_display(original_text, line, self.show_docs())
    }
}

/// 沿 container 链找到最外层类型。
pub(crate) fn outermost(index: &CsIndex, symbol: u32) -> u32 {
    let mut outer = symbol;
    // 嵌套层数有限；上限只防御损坏数据中的环。
    for _ in 0..64 {
        match index.container(outer) {
            Some(parent) if parent != outer => outer = parent,
            _ => break,
        }
    }
    outer
}

fn global_key(index: &CsIndex, symbol: u32) -> Option<&str> {
    match index.symbol_key(symbol).ok()? {
        SymbolKey::Global(g) => Some(g),
        SymbolKey::Local { .. } => None,
    }
}

/// 类型的第一个视图（同名程序集有多个版本时取排在前面的，与 `metadata_definitions` 的第一个位置一致）。
pub(crate) fn first_view(index: &CsIndex, type_symbol: u32) -> Option<u32> {
    let views = index.metadata_views_of(type_symbol);
    (views.start < views.end).then_some(views.start)
}

fn is_doc_line(line: &str) -> bool {
    line.trim_start().starts_with("///")
}

/// 去掉文档注释后的正文。
pub fn display_text(original: &str, show_docs: bool) -> String {
    if show_docs {
        return original.to_owned();
    }
    let mut out = String::with_capacity(original.len());
    for line in original.split_inclusive('\n') {
        if !is_doc_line(line) {
            out.push_str(line);
        }
    }
    out
}

/// 原始行号 → 显示行号（`show_docs` 为 false 时去掉它之前的文档注释行；文档行本身落到下一个保留行）。
pub fn to_display(original: &str, line: u32, show_docs: bool) -> u32 {
    if show_docs {
        return line;
    }
    let hidden = original
        .lines()
        .take(line as usize)
        .filter(|l| is_doc_line(l))
        .count();
    line - hidden as u32
}

/// 显示行号 → 原始行号。
pub fn original_line(original: &str, line: u32, show_docs: bool) -> u32 {
    if show_docs {
        return line;
    }
    let mut shown = 0u32;
    for (i, l) in original.lines().enumerate() {
        if is_doc_line(l) {
            continue;
        }
        if shown == line {
            return i as u32;
        }
        shown += 1;
    }
    original.lines().count() as u32
}

/// 标签标题：类型名 + 泛型参数（取自视图里类型声明的名字位置，如 `List<T>`）。
fn title_of(index: &CsIndex, view: u32, symbol: u32, text: &str) -> String {
    let fallback = || {
        index
            .symbols()
            .get(symbol as usize)
            .and_then(|r| index.str(r.name).ok())
            .unwrap_or_default()
            .to_owned()
    };
    let Some(member) = index
        .metadata_view_members(view)
        .iter()
        .find(|m| m.symbol == symbol)
    else {
        return fallback();
    };
    let Some(line) = text.lines().nth(member.line as usize) else {
        return fallback();
    };
    let start = member.col as usize;
    let end = start + member.len as usize;
    let Some(name) = line.get(start..end) else {
        return fallback();
    };
    let mut title = name.to_owned();
    if line[end..].starts_with('<') {
        let mut depth = 0;
        for (i, c) in line[end..].char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        title.push_str(&line[end..end + i + 1]);
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "#region a\n// p\n#endregion\n\n/// <summary>x</summary>\n/// more\npublic int P { get; }\n";

    #[test]
    fn docs_hide_and_line_numbers_convert_both_ways() {
        assert_eq!(display_text(TEXT, true), TEXT);
        let hidden = display_text(TEXT, false);
        assert!(!hidden.contains("///"));
        assert_eq!(hidden.lines().count(), 5);
        // 成员行在原文第 6 行（0 起算），去掉 2 行文档后是第 4 行。
        assert_eq!(to_display(TEXT, 6, false), 4);
        assert_eq!(original_line(TEXT, 4, false), 6);
        assert_eq!(to_display(TEXT, 6, true), 6);
        assert_eq!(original_line(TEXT, 6, true), 6);
        // 文档行本身落到下一个保留行。
        assert_eq!(to_display(TEXT, 4, false), 4);
    }

    #[test]
    fn resource_keys_round_trip() {
        let r = view_resource("UnityEngine.CoreModule|T:UnityEngine.Transform");
        assert_eq!(
            view_key(&r),
            Some("UnityEngine.CoreModule|T:UnityEngine.Transform")
        );
        assert_eq!(view_key(&Resource::Virtual("x:y".into())), None);
        assert_eq!(assembly_of("A.B|T:A.B.C"), "A.B");
    }
}
