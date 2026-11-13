//! 编辑事务与可编辑缓冲（#46）。
//!
//! - [`Transaction`]：一组按位置排序、互不重叠的 [`Replacement`]，原子执行；所有区间都以
//!   **事务开始前**的字节偏移给出，执行时从后往前应用，后面的区间不需要调用方自己平移。
//! - [`EditBuffer`]：持有当前文本（写时复制的 rope），每次成功的事务产出新的不可变快照
//!   [`RopeText`]（新 [`Version`]）以及 [`AppliedEdit`]：给 tree-sitter 的 [`InputEdit`]、
//!   给视图的按行失效通知 [`LineChanges`]、以及撤销用的反向事务。
//!
//! 坐标：事务内部只用 [`ByteOffset`]（解码后的 UTF-8），对外的行列一律是 [`Position`]
//! （[`Line`] + [`Utf8Column`]）；UTF-16 列只在 [`TextSource`] 的边界转换，这里不出现。
//!
//! 换行符：默认（[`NewlineMode::Normalize`]）把插入文本里的 LF / CRLF / CR 统一换成文档的
//! 主换行符；不在编辑区间内的行尾一个字节都不动，所以混合换行符的文档不会因为一次编辑被统一
//! （`UX_SPEC.md` §3.6）。反向事务与“统一换行符”用 [`NewlineMode::Verbatim`] 原样写入。

use std::borrow::Cow;
use std::sync::Arc;

use ropey::Rope;
use rove_base::*;

use crate::contracts::{DocumentMetadata, Eol, LineEndings, ReadOnlyReason, SizeClass, TextSource};
use crate::open::LoadedText;
use crate::text::RopeText;

/// 事务中的一次替换：把 `range`（事务开始前的坐标）换成 `text`。纯插入用空区间，纯删除用空文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    pub range: ByteRange,
    pub text: Arc<str>,
}

impl Replacement {
    pub fn new(range: ByteRange, text: impl Into<Arc<str>>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }

    pub fn insert(at: ByteOffset, text: impl Into<Arc<str>>) -> Self {
        Self::new(Span::new(at, at).expect("空区间总是合法"), text)
    }

    pub fn delete(range: ByteRange) -> Self {
        Self::new(range, "")
    }
}

/// 插入文本中的换行符如何处理。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewlineMode {
    /// 统一换成文档的主换行符（输入、粘贴等用户编辑）。
    #[default]
    Normalize,
    /// 原样写入（撤销 / 重做的反向事务、换行符统一）。
    Verbatim,
}

/// 多区间编辑事务：一次事务永远是一个撤销单元。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    edits: Vec<Replacement>,
    newlines: NewlineMode,
}

impl Transaction {
    /// 按 (起点, 终点) 稳定排序后校验：区间重叠时返回 [`Error::InvalidInput`]，不会悄悄合并。
    /// 首尾相接的区间与同一位置上的多个插入是允许的（同一位置的插入按给出的先后顺序排列）。
    pub fn new(mut edits: Vec<Replacement>) -> Result<Self> {
        edits.sort_by_key(|edit| (edit.range.start(), edit.range.end()));
        if edits
            .windows(2)
            .any(|pair| pair[1].range.start() < pair[0].range.end())
        {
            return Err(Error::InvalidInput);
        }
        Ok(Self {
            edits,
            newlines: NewlineMode::Normalize,
        })
    }

    /// 只有一个区间的事务。
    pub fn single(range: ByteRange, text: impl Into<Arc<str>>) -> Self {
        Self {
            edits: vec![Replacement::new(range, text)],
            newlines: NewlineMode::Normalize,
        }
    }

    /// 改为原样写入换行符。
    pub fn verbatim(mut self) -> Self {
        self.newlines = NewlineMode::Verbatim;
        self
    }

    /// 已排序的区间。
    pub fn edits(&self) -> &[Replacement] {
        &self.edits
    }

    pub fn newlines(&self) -> NewlineMode {
        self.newlines
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

/// tree-sitter `InputEdit` 需要的六个值（R09 的增量解析直接转换）。
///
/// 同一事务的多个 [`InputEdit`] 按**应用顺序**（从后往前）给出：每一个都以“前面几个已经应用”
/// 的文本为坐标系，可以依次喂给 `Tree::edit`。行号按本项目的行模型（LF / CRLF / 单独 CR
/// 都是行终止符）；tree-sitter 只认 LF，单独 CR 的文档由 R09 自行换算行号（它本来就按字节定位）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEdit {
    pub start_byte: ByteOffset,
    pub old_end_byte: ByteOffset,
    pub new_end_byte: ByteOffset,
    pub start_position: Position,
    pub old_end_position: Position,
    pub new_end_position: Position,
}

/// 一处行级变化：旧文本中的 `old` 行区间被新文本中的 `new` 行区间取代（半开区间）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineChange {
    pub old: LineRange,
    pub new: LineRange,
}

/// 变更通知：哪些行变了（按位置升序、互不重叠）、总行数变化多少。视图据此按行失效。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineChanges {
    pub ranges: Vec<LineChange>,
    pub line_delta: i64,
}

/// 偏移落在被替换区间的边界或内部时，映射到新文本的哪一侧。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    /// 映射到插入文本之前（区间起点）。
    Before,
    /// 映射到插入文本之后（区间终点）。
    After,
}

/// 一次成功事务的全部产出。
#[derive(Clone, Debug)]
pub struct AppliedEdit {
    pub before: Version,
    pub after: Version,
    /// 给 tree-sitter 的增量编辑，按应用顺序（从后往前）。
    pub input_edits: Vec<InputEdit>,
    /// 与 `input_edits` 一一对应的 [`TextEdit`]（版本为 `before`），可以逐个交给 R09。
    pub text_edits: Vec<TextEdit>,
    /// 按行失效的通知。
    pub changes: LineChanges,
    old_ranges: Vec<ByteRange>,
    new_ranges: Vec<ByteRange>,
    forward: Transaction,
    inverse: Transaction,
}

impl AppliedEdit {
    /// 每个替换在新文本中占据的区间（升序，与事务的区间一一对应）。
    pub fn inserted_ranges(&self) -> &[ByteRange] {
        &self.new_ranges
    }

    /// 实际写入的事务（换行符已规范化，原样模式），重做时重新应用它。
    pub fn forward(&self) -> &Transaction {
        &self.forward
    }

    /// 反向事务（新文本坐标，原样模式）：应用后文本逐字节回到事务之前。
    pub fn inverse(&self) -> &Transaction {
        &self.inverse
    }

    /// 把事务前的偏移映射到事务后的文本。
    ///
    /// 同一个偏移可能被多个区间碰到（同一位置的多个插入、替换的终点上又接一个插入）：
    /// [`Bias::Before`] 取第一个区间之前，[`Bias::After`] 取**最后一个**区间之后，这样
    /// 在同一位置连续插入 `X`、`Y` 时光标停在 `Y` 之后而不是夹在两段之间。
    pub fn map_offset(&self, offset: ByteOffset, bias: Bias) -> ByteOffset {
        let mut delta: i64 = 0;
        let mut after: Option<ByteOffset> = None;
        for (old, new) in self.old_ranges.iter().zip(&self.new_ranges) {
            if offset < old.start() {
                break;
            }
            if offset <= old.end() {
                match bias {
                    Bias::Before => return new.start(),
                    // 继续向后看与该偏移相接的区间，循环结束或下一个区间不再相接时才定下来。
                    Bias::After => after = Some(new.end()),
                }
            }
            delta = new.end().0 as i64 - old.end().0 as i64;
        }
        after.unwrap_or(ByteOffset((offset.0 as i64 + delta) as u64))
    }

    /// 把整个事务合并成一个 [`TextEdit`]（覆盖第一个区间起点到最后一个区间终点），
    /// 给只接受单个编辑的消费方（R09 的 `DocumentHighlighter::analyze`）。`after` 必须是
    /// 本次事务产出的快照。没有区间时返回 `None`。
    pub fn coalesced(&self, after: &dyn TextSource) -> Result<Option<TextEdit>> {
        if after.version() != self.after {
            return Err(Error::Stale);
        }
        let (Some(first_old), Some(last_old)) = (self.old_ranges.first(), self.old_ranges.last())
        else {
            return Ok(None);
        };
        let first_new = self.new_ranges[0].start();
        let last_new = self.new_ranges[self.new_ranges.len() - 1].end();
        let mut inserted = String::with_capacity((last_new.0 - first_new.0) as usize);
        after.visit_chunks(Span::new(first_new, last_new)?, &mut |chunk| {
            inserted.push_str(chunk)
        })?;
        Ok(Some(TextEdit {
            version: self.before,
            range: Span::new(first_old.start(), last_old.end())?,
            inserted: inserted.into(),
        }))
    }
}

/// 换行符对应的字节序列。
pub fn eol_str(eol: Eol) -> &'static str {
    match eol {
        Eol::Lf => "\n",
        Eol::CrLf => "\r\n",
        Eol::Cr => "\r",
    }
}

/// 把文本中的 LF / CRLF / CR 统一换成 `eol`；本来就一致时不分配。
pub fn normalize_newlines(text: &str, eol: Eol) -> Cow<'_, str> {
    let target = eol_str(eol);
    let bytes = text.as_bytes();
    let mut needs_change = false;
    let mut i = 0;
    while let Some(found) = memchr::memchr2(b'\r', b'\n', &bytes[i..]) {
        let at = i + found;
        let len = if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
            2
        } else {
            1
        };
        if &text[at..at + len] != target {
            needs_change = true;
            break;
        }
        i = at + len;
    }
    if !needs_change {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut last = 0;
    let mut i = 0;
    while let Some(found) = memchr::memchr2(b'\r', b'\n', &bytes[i..]) {
        let at = i + found;
        let len = if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
            2
        } else {
            1
        };
        out.push_str(&text[last..at]);
        out.push_str(target);
        last = at + len;
        i = last;
    }
    out.push_str(&text[last..]);
    Cow::Owned(out)
}

/// 可编辑的文本：当前快照 + 主换行符 + 只读原因。
///
/// 只读原因非空或文档是超大文件时，编辑接口返回 [`Error::Unsupported`]（超大文件一律只读，
/// `UX_SPEC.md` §3.1）。`Clone` 是 O(1)：快照共享。
#[derive(Clone, Debug)]
pub struct EditBuffer {
    text: Arc<RopeText>,
    line_endings: LineEndings,
    read_only: Arc<[ReadOnlyReason]>,
}

impl EditBuffer {
    /// 可编辑的新文本（不带只读原因）。`line_endings` 决定新输入的换行符。
    pub fn new(text: &str, line_endings: LineEndings) -> Self {
        Self {
            text: Arc::new(RopeText::from_text(text)),
            line_endings,
            read_only: Arc::from([]),
        }
    }

    /// 从打开结果建立，共享 rope（O(1)）。超大文件返回 [`Error::Unsupported`]；
    /// 元信息里的只读原因原样保留（默认的会话只读由 R46 解除）。
    pub fn from_loaded(loaded: &LoadedText) -> Result<Self> {
        if loaded.metadata.size == SizeClass::Huge {
            return Err(Error::Unsupported);
        }
        Ok(Self {
            text: loaded.text.clone(),
            line_endings: loaded.metadata.line_endings.clone(),
            read_only: loaded.metadata.read_only.clone(),
        })
    }

    /// 从任意文本源建立（复制一次全文）。超大文件 / 没有索引完的文本源返回错误。
    pub fn from_source(text: &dyn TextSource, metadata: &DocumentMetadata) -> Result<Self> {
        if metadata.size == SizeClass::Huge {
            return Err(Error::Unsupported);
        }
        if !text.lines().complete {
            return Err(Error::NotReady);
        }
        let mut builder = ropey::RopeBuilder::new();
        text.visit_chunks(
            Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?,
            &mut |chunk| builder.append(chunk),
        )?;
        Ok(Self {
            text: Arc::new(RopeText::from_rope(builder.finish())),
            line_endings: metadata.line_endings.clone(),
            read_only: metadata.read_only.clone(),
        })
    }

    /// 当前快照（O(1)）。快照不可变，版本与 [`EditBuffer::version`] 相同。
    pub fn snapshot(&self) -> Arc<RopeText> {
        self.text.clone()
    }

    /// 当前快照的借用。
    pub fn text(&self) -> &RopeText {
        &self.text
    }

    pub fn version(&self) -> Version {
        self.text.version()
    }

    /// 新输入的换行符：统一时就是它，混合时取多数派（R07 识别）。
    pub fn eol(&self) -> Eol {
        match self.line_endings {
            LineEndings::Uniform(eol) | LineEndings::Mixed { preferred: eol } => eol,
        }
    }

    pub fn line_endings(&self) -> &LineEndings {
        &self.line_endings
    }

    pub(crate) fn set_line_endings(&mut self, line_endings: LineEndings) {
        self.line_endings = line_endings;
    }

    pub fn read_only(&self) -> &[ReadOnlyReason] {
        &self.read_only
    }

    /// 替换只读原因（R46 解除会话只读、R48 判定权限时调用）。
    pub fn set_read_only(&mut self, reasons: Arc<[ReadOnlyReason]>) {
        self.read_only = reasons;
    }

    /// 全文（测试与保存路径使用；复制一次）。
    pub fn contents(&self) -> String {
        self.text.rope().to_string()
    }

    /// 把落在 `\r\n` 中间的偏移规整到换行之后；其余偏移原样返回。
    ///
    /// 一次编辑可能把原本分开的 `\r` 与 `\n` 并成一个换行（例如 `\rX\n` 删掉 `X`），
    /// 光标正好停在中间，下一次普通输入会被 [`EditBuffer::apply`] 判为 [`Error::OutOfBounds`]。
    /// 规整到换行之后（而不是之前）让光标留在被编辑的那一行上。
    pub fn normalize_crlf(&self, offset: ByteOffset) -> ByteOffset {
        match to_usize(offset) {
            Ok(byte) if inside_crlf(self.text.rope(), byte) => ByteOffset(offset.0 + 1),
            _ => offset,
        }
    }

    /// 原子执行一个事务。区间越界、不在 Unicode scalar 边界、或（规范化模式下）落在 CRLF
    /// 中间时返回 [`Error::OutOfBounds`]，文本保持不变。
    pub fn apply(&mut self, transaction: &Transaction) -> Result<AppliedEdit> {
        if !self.read_only.is_empty() {
            return Err(Error::Unsupported);
        }
        let old = self.text.rope();
        let len = old.len_bytes();
        let mut spans = Vec::with_capacity(transaction.edits.len());
        for edit in &transaction.edits {
            let start = to_usize(edit.range.start())?;
            let end = to_usize(edit.range.end())?;
            if end > len || !is_boundary(old, start) || !is_boundary(old, end) {
                return Err(Error::OutOfBounds);
            }
            if transaction.newlines == NewlineMode::Normalize
                && (inside_crlf(old, start) || inside_crlf(old, end))
            {
                return Err(Error::OutOfBounds);
            }
            spans.push((start, end));
        }
        let eol = self.eol();
        let texts: Vec<Cow<'_, str>> = transaction
            .edits
            .iter()
            .map(|edit| match transaction.newlines {
                NewlineMode::Normalize => normalize_newlines(&edit.text, eol),
                NewlineMode::Verbatim => Cow::Borrowed(&*edit.text),
            })
            .collect();

        let before = self.text.version();
        let mut rope = old.clone();
        let count = spans.len();
        let mut removed: Vec<String> = vec![String::new(); count];
        let mut input_edits = Vec::with_capacity(count);
        let mut text_edits = Vec::with_capacity(count);
        // 每个区间应用后的 (起点行, 旧终点行, 新终点行)，坐标系是“前面的区间尚未应用”。
        let mut lines = vec![(0u64, 0u64, 0u64); count];
        for index in (0..count).rev() {
            let (start, end) = spans[index];
            let inserted = &texts[index];
            let start_position = position_in(&rope, start);
            let old_end_position = position_in(&rope, end);
            if start < end {
                removed[index] = rope.byte_slice(start..end).to_string();
                let start_char = rope.byte_to_char(start);
                let end_char = rope.byte_to_char(end);
                rope.remove(start_char..end_char);
            }
            if !inserted.is_empty() {
                rope.insert(rope.byte_to_char(start), inserted);
            }
            let new_end = start + inserted.len();
            let new_end_position = position_in(&rope, new_end);
            lines[index] = (
                start_position.line.0,
                old_end_position.line.0,
                new_end_position.line.0,
            );
            input_edits.push(InputEdit {
                start_byte: ByteOffset(start as u64),
                old_end_byte: ByteOffset(end as u64),
                new_end_byte: ByteOffset(new_end as u64),
                start_position,
                old_end_position,
                new_end_position,
            });
            text_edits.push(TextEdit {
                version: before,
                range: transaction.edits[index].range.clone(),
                inserted: Arc::from(&**inserted),
            });
        }

        let mut old_ranges = Vec::with_capacity(count);
        let mut new_ranges = Vec::with_capacity(count);
        let mut forward = Vec::with_capacity(count);
        let mut inverse = Vec::with_capacity(count);
        let mut changes: Vec<LineChange> = Vec::with_capacity(count);
        let mut byte_delta: i64 = 0;
        let mut line_delta: i64 = 0;
        for index in 0..count {
            let (start, end) = spans[index];
            let inserted_len = texts[index].len();
            let new_start = (start as i64 + byte_delta) as u64;
            let new_range = Span::new(
                ByteOffset(new_start),
                ByteOffset(new_start + inserted_len as u64),
            )?;
            let (start_line, old_end_line, new_end_line) = lines[index];
            let change = LineChange {
                old: Span::new(Line(start_line), Line(old_end_line + 1))?,
                new: Span::new(
                    Line((start_line as i64 + line_delta) as u64),
                    Line((new_end_line as i64 + line_delta) as u64 + 1),
                )?,
            };
            match changes.last_mut() {
                // 同一行上的多个区间合并成一处变化。
                Some(previous) if change.old.start() < previous.old.end() => {
                    previous.old = Span::new(previous.old.start(), change.old.end())?;
                    previous.new = Span::new(previous.new.start(), change.new.end())?;
                }
                _ => changes.push(change),
            }
            line_delta += new_end_line as i64 - old_end_line as i64;
            byte_delta += inserted_len as i64 - (end - start) as i64;
            old_ranges.push(transaction.edits[index].range.clone());
            forward.push(Replacement::new(
                transaction.edits[index].range.clone(),
                &*texts[index],
            ));
            inverse.push(Replacement::new(
                new_range.clone(),
                std::mem::take(&mut removed[index]),
            ));
            new_ranges.push(new_range);
        }

        self.text = Arc::new(RopeText::from_rope(rope));
        Ok(AppliedEdit {
            before,
            after: self.text.version(),
            input_edits,
            text_edits,
            changes: LineChanges {
                ranges: changes,
                line_delta,
            },
            old_ranges,
            new_ranges,
            forward: Transaction {
                edits: forward,
                newlines: NewlineMode::Verbatim,
            },
            inverse: Transaction {
                edits: inverse,
                newlines: NewlineMode::Verbatim,
            },
        })
    }
}

fn to_usize(offset: ByteOffset) -> Result<usize> {
    usize::try_from(offset.0).map_err(|_| Error::OutOfBounds)
}

fn is_boundary(rope: &Rope, byte: usize) -> bool {
    rope.get_byte_slice(byte..byte).is_some()
}

/// `byte` 恰好落在 `\r\n` 的两个字节之间。
fn inside_crlf(rope: &Rope, byte: usize) -> bool {
    byte > 0 && byte < rope.len_bytes() && rope.byte(byte - 1) == b'\r' && rope.byte(byte) == b'\n'
}

/// 字节偏移 → 行列。落在行终止符内部时列会超过行内容长度（只用于 [`InputEdit`]）。
fn position_in(rope: &Rope, byte: usize) -> Position {
    let line = rope.byte_to_line(byte);
    Position {
        line: Line(line as u64),
        column: Utf8Column((byte - rope.line_to_byte(line)) as u64),
    }
}
