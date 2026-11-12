//! 位置映射：纯函数，不碰索引、缓冲区和锁。
//!
//! 输入是“索引时的文本 → 当前文本”之间累积的编辑（按编辑记录累积，不做整文件 diff），输出是
//! 映射后的位置，或者“落在修改区内”。
//!
//! 表示：一组按位置排序、两两不相邻的修改块（[`Hunk`]），每块同时记录它在旧文本（索引版本）
//! 和新文本（当前缓冲区）里的半开区间。块之间是未修改的文本，所以块外的位置只需要按前一块的
//! 终点平移：与前一块终点同一行的平移列，其余只平移行。查询是一次二分查找，O(log 块数)。
//!
//! **“区域是否被修改”的判定粒度是行级**（[`EditMap::line_modified_old`]）：旧文本里被任何
//! 修改块触及的行都算修改过，该行上的符号回退为近似。整行插入或整行删除（块的起止都在行首）
//! 不触及相邻行，所以“在文件开头插入 10 行”之后，原来的每一行仍然是未修改的。
//! 选行级而不是语句级：改了一个标识符所在行的其他部分（类型、修饰符、实参）同样可能改变绑定，
//! 行级能把它们都算进去；语句级需要语法树，且跨行语句会把很远的行一起降级。
//! 代价：其他行的修改（例如新增一个遮蔽字段的局部变量）可能改变本行的绑定而不被发现——这是
//! `LANGUAGE_INTELLIGENCE` §4 代价第 2 条接受的范围，保存并重建后恢复精确。
use rove_base::{Line, Position, Utf8Column};

/// 旧文本与新文本中对应的一段修改，两侧都是半开区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: Position,
    pub old_end: Position,
    pub new_start: Position,
    pub new_end: Position,
}

impl Hunk {
    /// 起止都在行首：整行替换（含纯插入整行、纯删除整行）。
    fn whole_lines(&self) -> bool {
        self.old_start.column.0 == 0
            && self.old_end.column.0 == 0
            && self.new_start.column.0 == 0
            && self.new_end.column.0 == 0
    }

    /// 旧文本中被触及的行（左闭右开）。
    fn old_lines(&self) -> (u64, u64) {
        if self.whole_lines() {
            (self.old_start.line.0, self.old_end.line.0)
        } else {
            (self.old_start.line.0, self.old_end.line.0 + 1)
        }
    }

    /// 新文本中被触及的行（左闭右开）。
    fn new_lines(&self) -> (u64, u64) {
        if self.whole_lines() {
            (self.new_start.line.0, self.new_end.line.0)
        } else {
            (self.new_start.line.0, self.new_end.line.0 + 1)
        }
    }
}

/// 一次编辑：把当前文本的 `[start, end)` 换成一段文本，插入文本用“换行数 + 最后一行的字节数”描述。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub start: Position,
    pub end: Position,
    /// 插入文本中的换行数（`\r\n`、`\r`、`\n` 各算一个）。
    pub inserted_breaks: u64,
    /// 插入文本最后一行的 UTF-8 字节数（没有换行时就是整段的字节数）。
    pub inserted_tail: u64,
}

impl LineEdit {
    /// 由插入文本构造。
    pub fn new(start: Position, end: Position, inserted: &str) -> Self {
        let (breaks, tail) = count_breaks(inserted);
        Self {
            start,
            end,
            inserted_breaks: breaks,
            inserted_tail: tail,
        }
    }

    /// 插入后新文本中被替换部分的终点。
    fn inserted_end(&self) -> Position {
        if self.inserted_breaks == 0 {
            pos(self.start.line.0, self.start.column.0 + self.inserted_tail)
        } else {
            pos(self.start.line.0 + self.inserted_breaks, self.inserted_tail)
        }
    }

    /// 编辑之后的位置（`p` 不在 `[start, end)` 之内）。
    fn shift(&self, p: Position) -> Position {
        if p < self.end {
            return p;
        }
        let ins_end = self.inserted_end();
        if p.line == self.end.line {
            pos(
                ins_end.line.0,
                ins_end.column.0 + (p.column.0 - self.end.column.0),
            )
        } else {
            pos(p.line.0 + ins_end.line.0 - self.end.line.0, p.column.0)
        }
    }
}

/// 换行数与最后一行的字节数。`\r\n` 算一个换行。
pub fn count_breaks(text: &str) -> (u64, u64) {
    let bytes = text.as_bytes();
    let mut breaks = 0;
    let mut tail_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                breaks += 1;
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                tail_start = i + 1;
            }
            b'\n' => {
                breaks += 1;
                tail_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    (breaks, (bytes.len() - tail_start) as u64)
}

/// 映射结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapped {
    /// 未修改的文本，映射到另一侧的这个位置。
    At(Position),
    /// 位置落在修改块里，另一侧没有对应位置。
    Modified,
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

/// `p` 位于块 `(from_end → to_end)` 之后的未修改区时，平移到另一侧。
fn shift_after(p: Position, from_end: Position, to_end: Position) -> Position {
    if p.line == from_end.line {
        pos(
            to_end.line.0,
            to_end.column.0 + (p.column.0 - from_end.column.0),
        )
    } else {
        pos(p.line.0 - from_end.line.0 + to_end.line.0, p.column.0)
    }
}

/// 索引版本（旧）与当前缓冲区（新）之间的修改块表。空表表示两侧相同。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditMap {
    hunks: Vec<Hunk>,
}

impl EditMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    /// 并入一次编辑（坐标是**编辑发生前**的当前文本）。与已有块重叠或相接时合并成一块。
    /// O(块数)：编辑点之后的块整体平移。
    pub fn push(&mut self, edit: LineEdit) {
        debug_assert!(edit.start <= edit.end);
        // 与编辑重叠或相接的块（新文本坐标）。
        let first = self.hunks.partition_point(|h| h.new_end < edit.start);
        let mut last = first;
        while last < self.hunks.len() && self.hunks[last].new_start <= edit.end {
            last += 1;
        }
        let (old_start, new_start) = match self.hunks.get(first).filter(|_| first < last) {
            Some(h) if h.new_start <= edit.start => (h.old_start, h.new_start),
            _ => (self.unchanged_new_to_old(first, edit.start), edit.start),
        };
        let (old_end, new_end_before) = match last.checked_sub(1).filter(|&l| l >= first) {
            Some(l) if self.hunks[l].new_end >= edit.end => {
                (self.hunks[l].old_end, self.hunks[l].new_end)
            }
            _ => (self.unchanged_new_to_old(last, edit.end), edit.end),
        };
        let merged = Hunk {
            old_start,
            old_end,
            new_start,
            new_end: edit.shift(new_end_before),
        };
        for h in &mut self.hunks[last..] {
            h.new_start = edit.shift(h.new_start);
            h.new_end = edit.shift(h.new_end);
        }
        self.hunks.splice(first..last, [merged]);
    }

    /// 新文本中位于未修改区的位置 `p`（第 `next` 块之前）对应的旧位置。
    fn unchanged_new_to_old(&self, next: usize, p: Position) -> Position {
        match next.checked_sub(1).map(|i| &self.hunks[i]) {
            Some(h) => shift_after(p, h.new_end, h.old_end),
            None => p,
        }
    }

    /// 旧（索引）位置 → 新（当前缓冲区）位置。
    pub fn old_to_new(&self, p: Position) -> Mapped {
        let i = self.hunks.partition_point(|h| h.old_start <= p);
        match i.checked_sub(1).map(|i| &self.hunks[i]) {
            None => Mapped::At(p),
            Some(h) if p < h.old_end => Mapped::Modified,
            Some(h) => Mapped::At(shift_after(p, h.old_end, h.new_end)),
        }
    }

    /// 旧位置 → 新位置；落在修改块里时取该块在新文本中的起点（尽力而为，调用方须标为近似）。
    pub fn old_to_new_nearest(&self, p: Position) -> Position {
        let i = self.hunks.partition_point(|h| h.old_start <= p);
        match i.checked_sub(1).map(|i| &self.hunks[i]) {
            None => p,
            Some(h) if p < h.old_end => h.new_start,
            Some(h) => shift_after(p, h.old_end, h.new_end),
        }
    }

    /// 新（当前缓冲区）位置 → 旧（索引）位置。
    pub fn new_to_old(&self, p: Position) -> Mapped {
        let i = self.hunks.partition_point(|h| h.new_start <= p);
        match i.checked_sub(1).map(|i| &self.hunks[i]) {
            None => Mapped::At(p),
            Some(h) if p < h.new_end => Mapped::Modified,
            Some(h) => Mapped::At(shift_after(p, h.new_end, h.old_end)),
        }
    }

    /// 旧文本的第 `line` 行是否被修改过（行级判定，见模块注释）。
    pub fn line_modified_old(&self, line: u64) -> bool {
        Self::line_touched(
            &self.hunks,
            line,
            |h| (h.old_start, h.old_end),
            Hunk::old_lines,
        )
    }

    /// 新文本的第 `line` 行是否被修改过。
    pub fn line_modified_new(&self, line: u64) -> bool {
        Self::line_touched(
            &self.hunks,
            line,
            |h| (h.new_start, h.new_end),
            Hunk::new_lines,
        )
    }

    fn line_touched(
        hunks: &[Hunk],
        line: u64,
        side: impl Fn(&Hunk) -> (Position, Position),
        lines: impl Fn(&Hunk) -> (u64, u64),
    ) -> bool {
        // 块按位置排序且互不重叠，终点单调递增：从起始行 ≤ line 的最后一块往前，
        // 遇到终点行在 line 之前的块就可以停。
        let i = hunks.partition_point(|h| side(h).0.line.0 <= line);
        hunks[..i]
            .iter()
            .rev()
            .take_while(|h| side(h).1.line.0 >= line)
            .map(lines)
            .any(|(start, end)| start <= line && line < end)
    }

    /// 映射一个旧位置，并且要求它所在的行未被修改。
    pub fn old_to_new_clean(&self, p: Position) -> Mapped {
        if self.line_modified_old(p.line.0) {
            return Mapped::Modified;
        }
        self.old_to_new(p)
    }

    /// 映射一个新位置，并且要求它所在的行未被修改。
    pub fn new_to_old_clean(&self, p: Position) -> Mapped {
        if self.line_modified_new(p.line.0) {
            return Mapped::Modified;
        }
        self.new_to_old(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: u64, col: u64) -> Position {
        pos(line, col)
    }

    /// 测试用文本模型：按 `\n` 分行。
    fn position_of(text: &str, offset: usize) -> Position {
        let before = &text[..offset];
        let line = before.matches('\n').count() as u64;
        let col = before.rfind('\n').map_or(offset, |i| offset - i - 1) as u64;
        p(line, col)
    }

    fn offset_of(text: &str, at: Position) -> Option<usize> {
        let mut start = 0;
        for _ in 0..at.line.0 {
            start += text[start..].find('\n')? + 1;
        }
        let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        let off = start + at.column.0 as usize;
        (off <= line_end).then_some(off)
    }

    fn line(text: &str, n: u64) -> Option<&str> {
        text.split('\n').nth(n as usize)
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// 在 `text` 上做一次编辑，同时并入 map。
    fn apply(text: &mut String, map: &mut EditMap, start: usize, end: usize, inserted: &str) {
        let edit = LineEdit::new(position_of(text, start), position_of(text, end), inserted);
        map.push(edit);
        text.replace_range(start..end, inserted);
    }

    #[test]
    fn insert_ten_lines_at_top_shifts_everything_untouched() {
        let old = "class A {\n  int x;\n  void F() { x = 1; }\n}\n";
        let mut text = old.to_owned();
        let mut map = EditMap::new();
        apply(&mut text, &mut map, 0, 0, &"// new\n".repeat(10));
        for l in 0..4 {
            assert!(!map.line_modified_old(l), "line {l}");
        }
        assert_eq!(map.old_to_new_clean(p(2, 13)), Mapped::At(p(12, 13)));
        assert_eq!(map.new_to_old_clean(p(12, 13)), Mapped::At(p(2, 13)));
        assert_eq!(map.new_to_old(p(3, 0)), Mapped::Modified);
    }

    #[test]
    fn editing_a_line_marks_only_that_line() {
        let mut text = "a\nbcd\ne\n".to_owned();
        let mut map = EditMap::new();
        apply(&mut text, &mut map, 3, 4, "XYZ"); // b[c]d -> bXYZd
        assert!(!map.line_modified_old(0));
        assert!(map.line_modified_old(1));
        assert!(!map.line_modified_old(2));
        assert_eq!(map.old_to_new_clean(p(1, 0)), Mapped::Modified);
        assert_eq!(map.old_to_new_clean(p(2, 0)), Mapped::At(p(2, 0)));
        // 同一行编辑点之后的位置在非“clean”映射下仍能平移。
        assert_eq!(map.old_to_new(p(1, 2)), Mapped::At(p(1, 4)));
    }

    #[test]
    fn deleting_whole_lines_does_not_touch_neighbours() {
        let mut text = "a\nb\nc\nd\n".to_owned();
        let mut map = EditMap::new();
        apply(&mut text, &mut map, 2, 6, ""); // 删除 "b\nc\n"
        assert!(!map.line_modified_old(0));
        assert!(map.line_modified_old(1));
        assert!(map.line_modified_old(2));
        assert!(!map.line_modified_old(3));
        assert_eq!(map.old_to_new_clean(p(3, 0)), Mapped::At(p(1, 0)));
    }

    #[test]
    fn crlf_and_cr_count_as_one_break() {
        assert_eq!(count_breaks("a\r\nbc"), (1, 2));
        assert_eq!(count_breaks("a\rb\nc\r\n"), (3, 0));
        assert_eq!(count_breaks("xyz"), (0, 3));
    }

    /// 属性测试：随机文本上随机编辑若干次后，
    /// 1. 每个未修改的旧行，映射到的新行内容与旧行完全相同；
    /// 2. 旧行上的每个字节位置映射后指向同一个字节；
    /// 3. 新 → 旧是旧 → 新的逆映射；
    /// 4. 新文本中未修改的行都能找到来源（不漏报修改）。
    #[test]
    fn random_edits_preserve_untouched_lines() {
        let alphabet = b"ab \n\nxy{}";
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for round in 0..400 {
            let len = 1 + rng.below(80);
            let old: String = (0..len)
                .map(|_| alphabet[rng.below(alphabet.len())] as char)
                .collect();
            let mut text = old.clone();
            let mut map = EditMap::new();
            for _ in 0..1 + rng.below(6) {
                let a = rng.below(text.len() + 1);
                let b = (a + rng.below(6)).min(text.len());
                let ins: String = (0..rng.below(5))
                    .map(|_| alphabet[rng.below(alphabet.len())] as char)
                    .collect();
                apply(&mut text, &mut map, a, b, &ins);
            }
            let old_lines = old.split('\n').count() as u64;
            for l in 0..old_lines {
                if map.line_modified_old(l) {
                    continue;
                }
                let src = line(&old, l).unwrap();
                let Mapped::At(start) = map.old_to_new(p(l, 0)) else {
                    panic!("round {round}: untouched line {l} start maps into a hunk: {map:?}");
                };
                assert_eq!(start.column.0, 0, "round {round} line {l}");
                assert_eq!(
                    line(&text, start.line.0),
                    Some(src),
                    "round {round} line {l}: old={old:?} new={text:?} {map:?}"
                );
                for c in 0..=src.len() as u64 {
                    let at = p(l, c);
                    let Mapped::At(to) = map.old_to_new_clean(at) else {
                        panic!("round {round}: {at:?}");
                    };
                    assert_eq!(to, p(start.line.0, c));
                    assert_eq!(map.new_to_old_clean(to), Mapped::At(at), "round {round}");
                    let (o, n) = (offset_of(&old, at).unwrap(), offset_of(&text, to).unwrap());
                    assert_eq!(old.as_bytes().get(o), text.as_bytes().get(n));
                }
            }
            // 新文本里未修改的行：映射回旧文本后内容相同。
            for l in 0..text.split('\n').count() as u64 {
                if map.line_modified_new(l) {
                    continue;
                }
                let Mapped::At(from) = map.new_to_old(p(l, 0)) else {
                    panic!("round {round}: new line {l} {map:?}");
                };
                assert_eq!(
                    line(&old, from.line.0),
                    line(&text, l),
                    "round {round} {map:?}"
                );
                assert!(!map.line_modified_old(from.line.0), "round {round}");
            }
        }
    }
}
