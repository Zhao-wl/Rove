//! 超长行的只读临时标签内容（#22，`UX_SPEC` §3.3）。
//!
//! 临时标签是一份**只读的内存文档**，经 #17 的打开文档服务创建（`rove-long-line:<文档>/<行>`）。
//! 生命周期照搬 VS Code 对临时编辑器的做法：
//!
//! - **同一行重复打开复用同一个标签**（资源名相同 → #17 的标签复用语义）；
//! - 关闭源文件不会连带关掉临时标签：它已经是一份独立的内存文档，内容在准备时就固定了
//!   （源文档释放后仍能继续看）；
//! - 文档被新快照替换时，重新打开会按新内容重新准备。
//!
//! 软换行是 #57 的范围；在它合并之前这里**按固定宽度插入换行符**（临时实现，见
//! [`super::long_line::wrap_fixed`]）。
//!
//! 本文件不依赖 GPUI。

use super::long_line;
use crate::skeleton::text::{VirtualContent, VirtualSource};
use rove_base::{DocumentId, Error, Line, Result};
use rove_buffer::{ReadOnlyReason, TextSource};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// 准备好的一行（内容已固定，不随源文档变化）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedLine {
    /// 已按固定宽度断行的内容。
    pub text: String,
    /// 这一行原本有多少个字符。
    pub chars: u64,
    /// 断行宽度（字符数）。
    pub columns: usize,
    /// 源文档里的行号（0 起始）。
    pub line: u64,
    /// 语言 ID（沿用源文档的，截断标记里的 JSON 仍按 JSON 显示）。
    pub language: Arc<str>,
}

/// 临时标签的内容来源。可以串在另一个 [`VirtualSource`] 前面（#42 的元数据视图）。
pub struct LongLineSource {
    lines: Mutex<BTreeMap<Arc<str>, Arc<PreparedLine>>>,
    next: Option<Arc<dyn VirtualSource>>,
}

impl LongLineSource {
    pub fn new() -> Self {
        Self {
            lines: Mutex::new(BTreeMap::new()),
            next: None,
        }
    }

    /// 不是本模块的资源时交给 `next`（装配时把 #42 的加载器串在后面）。
    pub fn chained(next: Arc<dyn VirtualSource>) -> Self {
        Self {
            lines: Mutex::new(BTreeMap::new()),
            next: Some(next),
        }
    }

    /// 读出源文档的一行、按固定宽度断行并登记成临时标签的内容，返回资源名。
    ///
    /// 同一行重复调用会重新准备（源文档可能换了快照），但资源名不变，所以 #17 复用同一个标签。
    pub fn prepare(
        &self,
        document: DocumentId,
        text: &dyn TextSource,
        line: u64,
        columns: usize,
        language: Arc<str>,
    ) -> Result<Arc<str>> {
        let chars = long_line::line_chars(text, Line(line))?;
        let wrapped = long_line::wrapped_line(text, Line(line), columns)?;
        let name = long_line::resource_name(document, line);
        let prepared = Arc::new(PreparedLine {
            text: wrapped,
            chars,
            columns,
            line,
            language,
        });
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.clone(), prepared);
        Ok(name)
    }

    /// 已准备好的内容（标签标题与说明文字用）。
    pub fn get(&self, name: &str) -> Option<Arc<PreparedLine>> {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
    }

    /// 丢掉某个文档的全部临时行（源文档彻底关闭且没有临时标签引用时由宿主调用）。
    pub fn forget_document(&self, document: DocumentId) {
        let prefix = format!("{}{}/", long_line::SCHEME, document.0);
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|name, _| !name.starts_with(&prefix));
    }

    pub fn len(&self) -> usize {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for LongLineSource {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualSource for LongLineSource {
    fn load(&self, name: &str) -> Option<Result<VirtualContent>> {
        if long_line::parse_resource(name).is_none() {
            return self.next.as_ref().and_then(|next| next.load(name));
        }
        Some(match self.get(name) {
            Some(prepared) => Ok(VirtualContent {
                text: prepared.text.clone(),
                language: prepared.language.clone(),
                // UX_SPEC §3.3：只读的临时标签。会话内也不能切换成可写。
                read_only: vec![ReadOnlyReason::Metadata],
            }),
            // 还没准备过（例如从布局恢复出来的旧标签）：按“没有内容”处理，不造假数据。
            None => Err(Error::NotFound),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_buffer::RopeText;

    fn source() -> (LongLineSource, RopeText) {
        let long = "0123456789".repeat(50);
        let text = RopeText::from_text(&format!("short\n{long}\n"));
        (LongLineSource::new(), text)
    }

    #[test]
    fn prepares_a_wrapped_read_only_document() {
        let (src, text) = source();
        let name = src
            .prepare(DocumentId(4), &text, 1, 64, "json".into())
            .unwrap();
        assert_eq!(&*name, "rove-long-line:4/1");
        let content = src.load(&name).unwrap().unwrap();
        assert_eq!(content.read_only, vec![ReadOnlyReason::Metadata]);
        assert_eq!(&*content.language, "json");
        assert_eq!(content.text.lines().count(), 500usize.div_ceil(64));
        assert_eq!(content.text.replace('\n', "").len(), 500);
        let prepared = src.get(&name).unwrap();
        assert_eq!(prepared.chars, 500);
        assert_eq!(prepared.line, 1);
    }

    #[test]
    fn the_same_line_reuses_the_same_resource() {
        let (src, text) = source();
        let a = src
            .prepare(DocumentId(4), &text, 1, 64, "json".into())
            .unwrap();
        let b = src
            .prepare(DocumentId(4), &text, 1, 32, "json".into())
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(src.len(), 1, "复用同一个标签，不会越开越多");
        // 重新准备后按新的断行宽度。
        assert_eq!(src.get(&b).unwrap().columns, 32);
    }

    #[test]
    fn unknown_resources_fall_through_to_the_next_source() {
        struct Other;
        impl VirtualSource for Other {
            fn load(&self, name: &str) -> Option<Result<VirtualContent>> {
                (name == "rove-metadata:X").then(|| {
                    Ok(VirtualContent {
                        text: "class X {}".into(),
                        language: "csharp".into(),
                        read_only: vec![ReadOnlyReason::Metadata],
                    })
                })
            }
        }
        let src = LongLineSource::chained(Arc::new(Other));
        assert!(src.load("rove-metadata:X").is_some());
        assert!(src.load("rove-other:Y").is_none());
        // 本模块的资源没准备过时报 NotFound，不会落到下一个来源。
        assert!(matches!(
            src.load("rove-long-line:1/2"),
            Some(Err(Error::NotFound))
        ));
    }

    #[test]
    fn closing_a_document_forgets_only_its_lines() {
        let (src, text) = source();
        src.prepare(DocumentId(1), &text, 1, 64, "json".into())
            .unwrap();
        src.prepare(DocumentId(2), &text, 1, 64, "json".into())
            .unwrap();
        src.forget_document(DocumentId(1));
        assert!(src.get("rove-long-line:1/1").is_none());
        assert!(src.get("rove-long-line:2/1").is_some());
    }
}
