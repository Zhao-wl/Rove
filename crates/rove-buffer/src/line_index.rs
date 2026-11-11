//! 分块行索引（#8）：每 256 行一个 `u64` 锚点，块内各行用 `u32` 相对偏移。
//!
//! M0 原型用 `Vec<u64>` 存每一行的起点，1429 万行要 114 MB；这里每行只占 4 字节
//! （外加每 256 行 8 字节锚点），同样的文件约 57 MB。
//!
//! 存储按“段”组织：每段 65,536 行（256 个块）。写满的段冻结成 `Arc<Segment>`，
//! 之后不再变化，可以在多个快照之间共享——文件只在末尾增长时，新快照复制段指针
//! 与最后一个未写满的段（至多 256 KiB），然后只扫描新增部分，耗时与追加量成正比。
//!
//! 行终止符与 [`RopeText`](crate::RopeText) 一致：LF、CRLF、单独的 CR。

use std::collections::BTreeMap;
use std::sync::Arc;

/// 每个块的行数：块首行的起点存成 `u64` 锚点。
pub(crate) const BLOCK_LINES: usize = 256;
/// 每段的行数（256 个块）。
const SEGMENT_LINES: usize = 256 * BLOCK_LINES;
const SEGMENT_BLOCKS: usize = SEGMENT_LINES / BLOCK_LINES;
/// 块内相对偏移放不下时（单个块跨越 4 GiB 以上）的占位值，真实起点在 `overflow` 里。
const OVERFLOW: u32 = u32::MAX;
/// 先判断子块里有没有 CR：没有（绝大多数 LF 文件）就只用 `memchr` 找 LF。
/// 64 KiB 能留在 L2 缓存里，第二遍扫描不再访问内存。
const SCAN_BLOCK: usize = 64 * 1024;

/// 冻结的段：锚点与相对偏移都按实际长度分配，没有冗余容量。
#[derive(Debug)]
struct Segment {
    anchors: Box<[u64]>,
    relative: Box<[u32]>,
}

/// 行起点的有序集合，外加扫描进度。只由一个构建者写入；读写同步由调用方负责。
#[derive(Debug)]
pub(crate) struct LineIndex {
    frozen: Vec<Arc<Segment>>,
    tail_anchors: Vec<u64>,
    tail_relative: Vec<u32>,
    overflow: BTreeMap<u64, u64>,
    /// 已扫描到的字节位置（不含）。
    pub(crate) scanned: u64,
    /// 文本以单独的 CR 结尾：其后的空行起点（= 文本长度）不入索引，
    /// 追加内容以 LF 开头时这个 CR 与它组成 CRLF。
    pub(crate) trailing_cr: bool,
    /// 已扫描到快照末尾。
    pub(crate) complete: bool,
}

impl Clone for LineIndex {
    fn clone(&self) -> Self {
        let mut tail_anchors = Vec::with_capacity(SEGMENT_BLOCKS);
        tail_anchors.extend_from_slice(&self.tail_anchors);
        let mut tail_relative = Vec::with_capacity(SEGMENT_LINES);
        tail_relative.extend_from_slice(&self.tail_relative);
        Self {
            frozen: self.frozen.clone(),
            tail_anchors,
            tail_relative,
            overflow: self.overflow.clone(),
            scanned: self.scanned,
            trailing_cr: self.trailing_cr,
            complete: self.complete,
        }
    }
}

impl LineIndex {
    /// 只有第 0 行（起点 0）的索引。
    pub(crate) fn new() -> Self {
        let mut index = Self {
            frozen: Vec::new(),
            tail_anchors: Vec::with_capacity(SEGMENT_BLOCKS),
            tail_relative: Vec::with_capacity(SEGMENT_LINES),
            overflow: BTreeMap::new(),
            scanned: 0,
            trailing_cr: false,
            complete: false,
        };
        index.push(0);
        index
    }

    /// 已记录的行起点个数（含第 0 行，不含 `trailing_cr` 的虚拟行）。
    pub(crate) fn starts(&self) -> u64 {
        (self.frozen.len() * SEGMENT_LINES + self.tail_relative.len()) as u64
    }

    fn segment(&self, segment: usize) -> (&[u64], &[u32]) {
        match self.frozen.get(segment) {
            Some(frozen) => (&frozen.anchors, &frozen.relative),
            None => (&self.tail_anchors, &self.tail_relative),
        }
    }

    /// 第 `line` 个行起点；调用方保证 `line < starts()`。
    pub(crate) fn start(&self, line: u64) -> u64 {
        let segment = (line / SEGMENT_LINES as u64) as usize;
        let local = (line % SEGMENT_LINES as u64) as usize;
        let (anchors, relative) = self.segment(segment);
        match relative[local] {
            OVERFLOW => self.overflow[&line],
            delta => anchors[local / BLOCK_LINES] + u64::from(delta),
        }
    }

    /// 追加一个行起点；必须大于上一个起点。
    pub(crate) fn push(&mut self, start: u64) {
        let local = self.tail_relative.len();
        if local.is_multiple_of(BLOCK_LINES) {
            self.tail_anchors.push(start);
            self.tail_relative.push(0);
        } else {
            let anchor = self.tail_anchors[local / BLOCK_LINES];
            match u32::try_from(start - anchor) {
                Ok(delta) if delta != OVERFLOW => self.tail_relative.push(delta),
                _ => {
                    self.overflow.insert(self.starts(), start);
                    self.tail_relative.push(OVERFLOW);
                }
            }
        }
        if self.tail_relative.len() == SEGMENT_LINES {
            let anchors =
                std::mem::replace(&mut self.tail_anchors, Vec::with_capacity(SEGMENT_BLOCKS));
            let relative =
                std::mem::replace(&mut self.tail_relative, Vec::with_capacity(SEGMENT_LINES));
            self.frozen.push(Arc::new(Segment {
                anchors: anchors.into_boxed_slice(),
                relative: relative.into_boxed_slice(),
            }));
        }
    }

    /// 起点不大于 `offset` 的最后一行（`offset` 不小于 0，所以总有结果）。
    pub(crate) fn line_at(&self, offset: u64) -> u64 {
        // 段：首行起点不大于 offset 的最后一段。
        let segments = self.frozen.len() + usize::from(!self.tail_relative.is_empty());
        let segment = partition(segments, |s| self.segment(s).0[0] <= offset);
        let (anchors, relative) = self.segment(segment);
        let block = partition(anchors.len(), |b| anchors[b] <= offset);
        let first = segment * SEGMENT_LINES + block * BLOCK_LINES;
        let count = (relative.len() - block * BLOCK_LINES).min(BLOCK_LINES);
        let local = partition(count, |l| self.start((first + l) as u64) <= offset);
        (first + local) as u64
    }

    /// 索引本身占用的堆内存（字节）：段、未写满的段（按容量）与溢出表。
    pub(crate) fn heap_bytes(&self) -> usize {
        let frozen: usize = self
            .frozen
            .iter()
            .map(|segment| segment.anchors.len() * 8 + segment.relative.len() * 4)
            .sum();
        frozen
            + self.frozen.capacity() * std::mem::size_of::<Arc<Segment>>()
            + self.tail_anchors.capacity() * 8
            + self.tail_relative.capacity() * 4
            + self.overflow.len() * 32
    }

    /// 扫描内存中的 `data[from..to)`（`data` 是整个快照），把行起点写进索引。
    pub(crate) fn scan(&mut self, data: &[u8], from: u64, to: u64) {
        let mut starts = Vec::new();
        let window_start = from.saturating_sub(1);
        let window_end = to.saturating_add(1).min(data.len() as u64);
        let window = &data[window_start as usize..window_end as usize];
        let trailing_cr = scan_chunk(
            window,
            window_start,
            from,
            to,
            data.len() as u64,
            &mut starts,
        );
        self.publish(&starts, to, trailing_cr);
    }

    /// 写入一块的扫描结果：块必须按顺序发布，`to` 是这一块的末尾。
    pub(crate) fn publish(&mut self, starts: &[u64], to: u64, trailing_cr: bool) {
        for &start in starts {
            self.push(start);
        }
        self.scanned = to;
        self.trailing_cr = trailing_cr;
    }

    /// 从已完成的旧快照延续：文件在末尾增长后，旧快照末尾单独的 CR 可能与新内容
    /// 开头的 LF 组成 CRLF。返回继续扫描的位置（总是旧快照的末尾：那个 LF 由
    /// [`scan_chunk`] 归给前面的 CR，不会再算一次）。
    pub(crate) fn resume_after_append(&mut self, data: &[u8]) -> u64 {
        let position = self.scanned;
        if self.trailing_cr {
            let crlf = data.get(position as usize) == Some(&b'\n');
            self.push(position + u64::from(crlf));
            self.trailing_cr = false;
        }
        self.complete = false;
        position
    }
}

/// 满足谓词的前缀长度减一（谓词对前缀为真、之后为假），至少为 0。
fn partition(len: usize, mut predicate: impl FnMut(usize) -> bool) -> usize {
    let (mut low, mut high) = (0usize, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if predicate(middle) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low.saturating_sub(1)
}

/// 扫描一块：只认领**首字节**落在 `[from, to)` 里的行终止符，把下一行的起点放进 `starts`。
///
/// 块与块互不依赖，可以并行扫描后按顺序发布：
/// - `window` 是快照 `[window_start, window_start + window.len())` 的内容，必须包含
///   `from - 1`（`from > 0` 时）与 `to`（`to < len` 时），用来判断跨块的 CRLF；
/// - CRLF 由 CR 认领（起点在 LF 之后）；块首的 LF 若紧跟在 CR 后面则跳过；
/// - CR 恰好是快照最后一个字节（`len`）时不产生起点，返回 `true`（`trailing_cr`）。
///
/// 窗口比预期短（文件在外部被改写）时只扫描已有的字节，不会越界。
pub(crate) fn scan_chunk(
    window: &[u8],
    window_start: u64,
    from: u64,
    to: u64,
    len: u64,
    starts: &mut Vec<u64>,
) -> bool {
    let local = |offset: u64| (offset - window_start) as usize;
    let lo = local(from).min(window.len());
    let hi = local(to).min(window.len());
    let after_cr = |index: usize| index > 0 && window[index - 1] == b'\r';
    let mut trailing_cr = false;
    let mut position = lo;
    while position < hi {
        let block_end = (position + SCAN_BLOCK).min(hi);
        let block = &window[position..block_end];
        if memchr::memchr(b'\r', block).is_none() {
            // 只有 LF：子块首字节之前的 CR 是唯一可能的例外。
            let skip_first = after_cr(position);
            starts.extend(
                memchr::memchr_iter(b'\n', block)
                    .filter(|&i| !(i == 0 && skip_first))
                    .map(|i| window_start + (position + i) as u64 + 1),
            );
        } else {
            for i in memchr::memchr2_iter(b'\n', b'\r', block) {
                let index = position + i;
                let offset = window_start + index as u64;
                if window[index] == b'\n' {
                    if !after_cr(index) {
                        starts.push(offset + 1);
                    }
                } else if offset + 1 >= len {
                    trailing_cr = true;
                } else if window.get(index + 1) == Some(&b'\n') {
                    starts.push(offset + 2);
                } else {
                    starts.push(offset + 1);
                }
            }
        }
        position = block_end;
    }
    trailing_cr
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(text: &[u8]) -> LineIndex {
        let mut index = LineIndex::new();
        index.scan(text, 0, text.len() as u64);
        index.complete = true;
        index
    }

    fn starts(index: &LineIndex) -> Vec<u64> {
        (0..index.starts()).map(|line| index.start(line)).collect()
    }

    #[test]
    fn records_every_terminator_form() {
        let index = build(b"a\r\nb\nc\rd");
        assert_eq!(starts(&index), [0, 3, 5, 7]);
        assert!(!index.trailing_cr);
        let index = build(b"a\n");
        assert_eq!(starts(&index), [0, 2]);
        let index = build(b"a\r");
        assert_eq!(starts(&index), [0]);
        assert!(index.trailing_cr);
        let index = build(b"\r\n\r\n");
        assert_eq!(starts(&index), [0, 2, 4]);
    }

    #[test]
    fn crlf_across_scan_ranges_counts_once() {
        let text = b"ab\r\ncd\r\nef";
        // 每个切分点（含 CR 与 LF 之间）分两块扫描，结果都与整体扫描一致。
        for split in 0..=text.len() as u64 {
            let mut index = LineIndex::new();
            index.scan(text, 0, split);
            index.scan(text, split, text.len() as u64);
            assert_eq!(starts(&index), [0, 4, 8], "切分点 {split}");
        }
    }

    #[test]
    fn many_lines_cross_blocks_and_segments() {
        let lines = SEGMENT_LINES * 2 + 1000;
        let mut text = Vec::new();
        let mut expected = vec![0u64];
        for line in 0..lines {
            text.extend_from_slice(format!("{line}").as_bytes());
            text.extend_from_slice(if line % 3 == 0 { b"\r\n" } else { b"\n" });
            expected.push(text.len() as u64);
        }
        let index = build(&text);
        assert_eq!(index.starts() as usize, lines + 1);
        assert_eq!(starts(&index), expected);
        for (line, &start) in expected.iter().enumerate() {
            assert_eq!(index.line_at(start), line as u64);
            if start > 0 {
                assert_eq!(index.line_at(start - 1), line as u64 - 1);
            }
        }
        assert_eq!(index.line_at(u64::MAX), lines as u64);
    }

    #[test]
    fn overflowing_relative_offsets_fall_back_to_absolute() {
        let mut index = LineIndex::new();
        index.push(10);
        index.push(u64::from(u32::MAX) + 5);
        index.push(u64::from(u32::MAX) * 3);
        assert_eq!(
            starts(&index),
            [0, 10, u64::from(u32::MAX) + 5, u64::from(u32::MAX) * 3]
        );
        assert_eq!(index.line_at(u64::from(u32::MAX) + 4), 1);
        assert_eq!(index.line_at(u64::from(u32::MAX) + 5), 2);
    }

    #[test]
    fn resume_joins_a_trailing_cr_with_an_appended_lf() {
        let mut index = build(b"a\r");
        let data = b"a\r\nb";
        let next = index.resume_after_append(data);
        assert_eq!(next, 2);
        index.scan(data, next, data.len() as u64);
        assert_eq!(starts(&index), [0, 3]);

        let mut index = build(b"a\r");
        let data = b"a\rb";
        let next = index.resume_after_append(data);
        assert_eq!(next, 2);
        index.scan(data, next, data.len() as u64);
        assert_eq!(starts(&index), [0, 2]);
    }

    #[test]
    fn cloned_index_shares_frozen_segments() {
        let text = "x\n".repeat(SEGMENT_LINES + 10);
        let index = build(text.as_bytes());
        let copy = index.clone();
        assert!(Arc::ptr_eq(&index.frozen[0], &copy.frozen[0]));
        assert_eq!(starts(&index), starts(&copy));
    }

    #[test]
    fn memory_per_line_is_about_four_bytes() {
        let text = "x\n".repeat(SEGMENT_LINES * 4);
        let index = build(text.as_bytes());
        let per_line = index.heap_bytes() as f64 / index.starts() as f64;
        assert!(per_line < 5.2, "每行 {per_line} 字节");
    }
}
