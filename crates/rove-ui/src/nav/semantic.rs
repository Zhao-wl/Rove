//! #38 的装配（ARCHITECTURE §25 “装配”一段）：
//!
//! - [`LiveSemantic`]：可替换的语义来源。窗口按设置重启 #36 的索引生命周期时只换里面的来源，
//!   `CsRouter` 只建一次；没有生命周期时（测试入口、`rove.index.enabled = false` 之前）报告 `NoCSharp`，
//!   查询走 L0/L1 回退。
//! - [`OpenBuffers`]：比较“打开中的 C# 文档”前后两次的集合，得出 `BufferStates` 的打开 / 重新载入 /
//!   关闭调用。编辑（M3 编辑事务）与保存（#49）由各自的装配处直接调用 `BufferStates::edited` /
//!   `saved`（`Workbench::cs_buffers` 给出同一个实例）。

use rove_base::{DocumentId, Version, WorkspacePath};
use rove_buffer::TextSource;
use rove_lang_cs::router::{L2Snapshot, SemanticSource, SemanticStatus};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// 可替换的 [`SemanticSource`]。
#[derive(Default)]
pub struct LiveSemantic {
    current: RwLock<Option<Arc<dyn SemanticSource>>>,
}

impl LiveSemantic {
    pub fn set(&self, source: Option<Arc<dyn SemanticSource>>) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = source;
    }

    fn get(&self) -> Option<Arc<dyn SemanticSource>> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl SemanticSource for LiveSemantic {
    fn snapshot(&self) -> Option<L2Snapshot> {
        self.get()?.snapshot()
    }
    fn status(&self) -> SemanticStatus {
        self.get().map_or(SemanticStatus::NoCSharp, |s| s.status())
    }
    fn is_stale(&self, path: &str) -> bool {
        self.get().is_some_and(|s| s.is_stale(path))
    }
    fn has_stale(&self) -> bool {
        self.get().is_some_and(|s| s.has_stale())
    }
    fn request_update(&self, path: &str) {
        if let Some(s) = self.get() {
            s.request_update(path);
        }
    }
}

/// 一份打开中的 C# 文档（项目相对路径、文档身份、文本版本与快照）。
#[derive(Clone)]
pub struct OpenBuffer {
    pub path: WorkspacePath,
    pub document: DocumentId,
    pub version: Version,
    pub text: Arc<dyn TextSource>,
}

/// 需要对 `BufferStates` 做的调用。
#[derive(Clone)]
pub enum BufferChange {
    /// 新打开：后台读盘算原始字节的哈希后调用 `opened`。
    Opened(OpenBuffer),
    /// 同一文档的新快照（外部修改后重新载入）：`reloaded`。
    Reloaded(OpenBuffer),
    /// 最后一个视图关闭：`closed`。
    Closed(WorkspacePath),
}

/// 上一次已同步给 `BufferStates` 的文档集合。
#[derive(Default)]
pub struct OpenBuffers {
    known: BTreeMap<String, (DocumentId, Version)>,
}

impl OpenBuffers {
    /// 与当前集合比较，返回需要的调用（按路径排序，确定性）。每帧调用也只是比较几十个条目。
    pub fn diff(&mut self, current: Vec<OpenBuffer>) -> Vec<BufferChange> {
        let mut out = Vec::new();
        let mut next = BTreeMap::new();
        for buffer in current {
            let key = buffer.path.as_str().to_owned();
            if next.contains_key(&key) {
                continue;
            }
            next.insert(key.clone(), (buffer.document, buffer.version));
            match self.known.get(&key) {
                None => out.push(BufferChange::Opened(buffer)),
                Some((doc, _)) if *doc != buffer.document => out.push(BufferChange::Opened(buffer)),
                Some((_, version)) if *version != buffer.version => {
                    out.push(BufferChange::Reloaded(buffer))
                }
                Some(_) => {}
            }
        }
        for key in self.known.keys() {
            if !next.contains_key(key) {
                if let Ok(path) = WorkspacePath::new(key) {
                    out.push(BufferChange::Closed(path));
                }
            }
        }
        self.known = next;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_lang_cs::router::ManualSemantic;

    fn text(s: &str) -> Arc<dyn TextSource> {
        Arc::new(rove_buffer::RopeText::from_text(s))
    }

    fn buf(p: &str, doc: u64, v: u64) -> OpenBuffer {
        OpenBuffer {
            path: WorkspacePath::new(p).unwrap(),
            document: DocumentId(doc),
            version: Version(v),
            text: text("x"),
        }
    }

    fn kinds(c: &[BufferChange]) -> Vec<String> {
        c.iter()
            .map(|c| match c {
                BufferChange::Opened(b) => format!("open {}", b.path.as_str()),
                BufferChange::Reloaded(b) => format!("reload {}", b.path.as_str()),
                BufferChange::Closed(p) => format!("close {}", p.as_str()),
            })
            .collect()
    }

    #[test]
    fn open_reload_and_close_are_derived_from_the_open_set() {
        let mut b = OpenBuffers::default();
        assert_eq!(
            kinds(&b.diff(vec![
                buf("A.cs", 1, 1),
                buf("B.cs", 2, 1),
                buf("A.cs", 1, 1)
            ])),
            ["open A.cs", "open B.cs"]
        );
        assert!(b
            .diff(vec![buf("A.cs", 1, 1), buf("B.cs", 2, 1)])
            .is_empty());
        assert_eq!(
            kinds(&b.diff(vec![buf("A.cs", 1, 2)])),
            ["reload A.cs", "close B.cs"]
        );
        // 关闭后用新的文档身份重新打开：重新登记。
        assert_eq!(kinds(&b.diff(vec![buf("A.cs", 9, 1)])), ["open A.cs"]);
    }

    #[test]
    fn live_semantic_switches_sources() {
        let live = LiveSemantic::default();
        assert_eq!(live.status(), SemanticStatus::NoCSharp);
        assert!(live.snapshot().is_none());
        let manual = Arc::new(ManualSemantic::unavailable(SemanticStatus::Building {
            percent: Some(40),
        }));
        manual.mark_stale("A.cs");
        live.set(Some(manual.clone()));
        assert_eq!(
            live.status(),
            SemanticStatus::Building { percent: Some(40) }
        );
        assert!(live.is_stale("A.cs") && live.has_stale());
        live.request_update("B.cs");
        assert_eq!(manual.requested(), ["B.cs"]);
        live.set(None);
        assert!(!live.has_stale());
    }
}
