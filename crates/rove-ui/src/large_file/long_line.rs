//! 超长行：截断标记与“在只读临时标签中查看这一行”（#22，`UX_SPEC` §3.3）。
//!
//! 渲染上限沿用 VS Code 的隐藏设置 `editor.stopRenderingLineAfter`（默认 10000 字符），
//! 不另外定义 `rove.*` 替代项；截断本身由 #19 的
//! [`DisplayLine`](crate::code_view::columns::DisplayLine) 完成，这里只负责行尾那条灰色标记
//! 与临时标签的内容。
//!
//! 本文件不依赖 GPUI。

use rove_base::{Line, Result, Version};
use rove_buffer::TextSource;
use std::collections::HashMap;
use std::sync::Arc;

/// Rove 专有命令（`docs/compat/KEYBINDINGS.md` §5，无默认键位）；
/// 命令标题已由 #14 的 `rove_commands::builtin` 登记，这里只引用 ID。
pub const OPEN_LONG_LINE: &str = "rove.editor.openLongLine";

/// 截断标记的文案 key：“…（已截断，共 {chars} 个字符）”。
pub const MARK_KEY: &str = "largeFile.longLine.truncated";
/// 临时标签的标题文案 key。
pub const TAB_TITLE_KEY: &str = "largeFile.longLine.tabTitle";
/// 临时标签顶部的说明（临时实现：#57 的软换行合并前按固定宽度断行）。
pub const TAB_NOTE_KEY: &str = "largeFile.longLine.note";
/// 没有超长行时的光标处提示。
pub const NO_LONG_LINE_KEY: &str = "largeFile.longLine.none";

/// 标记使用的颜色键（`THEME_COLORS` 中已有，不新造）。
pub const MARK_COLOR: &str = "descriptionForeground";

/// 临时标签的资源名前缀：`rove-long-line:<文档 id>/<行号>`。
/// 同一行重复打开复用同一个资源（照搬 VS Code 对临时编辑器的做法）。
pub const SCHEME: &str = "rove-long-line:";

/// 临时标签按固定宽度断行的列数（#57 软换行合并前的临时实现，见本 issue 的 PR 说明）。
pub const FALLBACK_WRAP_COLUMNS: usize = 120;

/// 一个超长行的临时标签资源名。
pub fn resource_name(document: rove_base::DocumentId, line: u64) -> Arc<str> {
    Arc::from(format!("{SCHEME}{}/{}", document.0, line))
}

/// 解析临时标签的资源名；不是本模块的资源时为 `None`。
pub fn parse_resource(name: &str) -> Option<(u64, u64)> {
    let rest = name.strip_prefix(SCHEME)?;
    let (document, line) = rest.split_once('/')?;
    Some((document.parse().ok()?, line.parse().ok()?))
}

/// 统计一行的**字符数**（Unicode 标量值个数），不把整行读进内存。
///
/// 50 MB 的压缩 JSON 只有一行：`visit_chunks` 按块回调，这里只数 UTF-8 首字节。
pub fn line_chars(text: &dyn TextSource, line: Line) -> Result<u64> {
    let range = text.line_range(line)?;
    let mut chars = 0u64;
    text.visit_chunks(range, &mut |chunk| {
        chars += chunk
            .as_bytes()
            .iter()
            .filter(|b| (**b as i8) >= -0x40)
            .count() as u64;
    })?;
    Ok(chars)
}

/// 每行字符数的缓存：按 (文本版本, 行号) 键控，文本换快照后自动失效。
///
/// 统计一行要扫完整行（50 MB 约 10 ms），所以只在这一行真的被截断时算一次。
#[derive(Default)]
pub struct CharCounts {
    version: Version,
    counts: HashMap<u64, u64>,
}

impl CharCounts {
    /// 取（必要时计算）第 `line` 行的字符数。文本版本变化时整表失效。
    pub fn get(&mut self, text: &dyn TextSource, line: u64) -> Option<u64> {
        let version = text.version();
        if self.version != version {
            self.version = version;
            self.counts.clear();
        }
        if let Some(count) = self.counts.get(&line) {
            return Some(*count);
        }
        let count = line_chars(text, Line(line)).ok()?;
        self.counts.insert(line, count);
        Some(count)
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    pub fn clear(&mut self) {
        self.counts.clear();
    }
}

/// 把一行按固定宽度（字符数）断成多行，供临时标签显示。
///
/// **临时实现**：#57（软换行）合并后换成真正的软换行；在那之前临时标签里是真的插入了换行符，
/// 所以复制出来的文本会带换行。断点落在字符边界上（不拆 UTF-8）。
pub fn wrap_fixed(line: &str, columns: usize) -> String {
    let columns = columns.max(1);
    let mut out = String::with_capacity(line.len() + line.len() / columns + 1);
    let mut in_row = 0usize;
    for ch in line.chars() {
        if in_row == columns {
            out.push('\n');
            in_row = 0;
        }
        out.push(ch);
        in_row += 1;
    }
    out
}

/// 读一行并按固定宽度断行（临时标签的内容）。
pub fn wrapped_line(text: &dyn TextSource, line: Line, columns: usize) -> Result<String> {
    let range = text.line_range(line)?;
    let mut out = String::new();
    let mut in_row = 0usize;
    let columns = columns.max(1);
    text.visit_chunks(range, &mut |chunk| {
        for ch in chunk.chars() {
            if in_row == columns {
                out.push('\n');
                in_row = 0;
            }
            out.push(ch);
            in_row += 1;
        }
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::DocumentId;
    use rove_buffer::RopeText;

    #[test]
    fn counts_characters_not_bytes() {
        let text = RopeText::from_text("a你好🙂\nxyz");
        // 'a' + 2 个汉字 + 1 个表情 = 4 个字符（字节数是 1+3+3+4 = 11）。
        assert_eq!(line_chars(&text, Line(0)).unwrap(), 4);
        assert_eq!(line_chars(&text, Line(1)).unwrap(), 3);
    }

    #[test]
    fn counts_are_cached_per_version() {
        let text = RopeText::from_text(&"x".repeat(5000));
        let mut counts = CharCounts::default();
        assert_eq!(counts.get(&text, 0), Some(5000));
        assert_eq!(counts.len(), 1);
        assert_eq!(counts.get(&text, 0), Some(5000));
        assert_eq!(counts.len(), 1);
        // 换一个快照（不同版本）：缓存清空。
        let other = RopeText::from_text("ab");
        assert_eq!(counts.get(&other, 0), Some(2));
        assert_eq!(counts.len(), 1);
    }

    #[test]
    fn fixed_width_wrapping_breaks_on_character_boundaries() {
        assert_eq!(wrap_fixed("abcdef", 2), "ab\ncd\nef");
        assert_eq!(wrap_fixed("你好世界", 2), "你好\n世界");
        assert_eq!(wrap_fixed("abc", 10), "abc");
        assert_eq!(wrap_fixed("", 10), "");
        // 宽度 0 当作 1，不死循环。
        assert_eq!(wrap_fixed("ab", 0), "a\nb");
    }

    #[test]
    fn wrapping_from_a_text_source_matches_wrap_fixed() {
        let line = "0123456789".repeat(37);
        let text = RopeText::from_text(&format!("short\n{line}\n"));
        let wrapped = wrapped_line(&text, Line(1), 64).unwrap();
        assert_eq!(wrapped, wrap_fixed(&line, 64));
        assert_eq!(wrapped.lines().count(), 370usize.div_ceil(64));
    }

    #[test]
    fn temporary_tab_resources_round_trip_and_are_reused() {
        let a = resource_name(DocumentId(7), 3);
        let b = resource_name(DocumentId(7), 3);
        assert_eq!(a, b, "同一行重复打开复用同一个资源");
        assert_eq!(parse_resource(&a), Some((7, 3)));
        assert_eq!(parse_resource("file:///x"), None);
        assert_ne!(a, resource_name(DocumentId(7), 4));
    }
}
