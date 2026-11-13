//! 撤销分组与光标撤销（#46）。
//!
//! 分组规则照搬 VS Code（`cursorTypeEditOperations.ts` 的 `shouldPushStackElementBetween`、
//! `cursorDeleteOperations.ts` 与 `cursor.ts` 的 `_executeEditOperation`）：
//!
//! - 连续输入合并为一个撤销单元；输入第一个空格时断开（`abc| ` → 新单元），空格后紧跟的
//!   输入不断开（`abc |d` 仍在同一单元），连续多个空格之后再输入时断开；
//! - 输入与删除之间、向左删与向右删之间断开；连续同方向删除合并；
//! - 其他编辑（粘贴、剪切、替换、缩进转换……）前后都断开：一次多区间事务永远是一个撤销单元；
//! - 显式移动光标（点击、方向键、按单词移动）断开（VS Code 的命令在移动时 `pushStackElement`）；
//! - Rove 额外规则：同一单元内两次编辑间隔超过 [`UNDO_GROUP_TIMEOUT`] 时断开（VS Code 没有）。
//!
//! 撤销栈只保存“反向事务 + 正向事务 + 编辑前后的选区”，不保存整份文本的快照。
//!
//! 光标撤销（`cursorUndo` / `cursorRedo`，移植 `cursorUndo.ts`）是独立的栈：只记录非编辑引起的
//! 选区变化，最多 50 条；文本一旦变化就清空（与 VS Code 相同）。

use std::time::{Duration, Instant};

use super::selection::Selections;
use super::transaction::Transaction;
use crate::contracts::LineEndings;

/// 同一撤销单元内两次编辑的最大间隔（Rove 的差异规则）。
pub const UNDO_GROUP_TIMEOUT: Duration = Duration::from_secs(1);

/// 光标撤销栈的上限（VS Code 相同）。
pub const CURSOR_UNDO_LIMIT: usize = 50;

/// 编辑类型（VS Code `EditOperationType`），决定与上一次编辑之间是否断开撤销单元。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditKind {
    /// 粘贴、剪切、替换、缩进转换、多区间批量编辑等：前后都断开。
    #[default]
    Other,
    DeletingLeft,
    DeletingRight,
    TypingOther,
    TypingFirstSpace,
    TypingConsecutiveSpace,
}

impl EditKind {
    /// `getTypingOperation`：输入 `text` 时的编辑类型。
    pub fn typing(text: &str, previous: EditKind) -> EditKind {
        if text == " " {
            if matches!(
                previous,
                EditKind::TypingFirstSpace | EditKind::TypingConsecutiveSpace
            ) {
                EditKind::TypingConsecutiveSpace
            } else {
                EditKind::TypingFirstSpace
            }
        } else {
            EditKind::TypingOther
        }
    }

    pub fn is_typing(self) -> bool {
        matches!(
            self,
            EditKind::TypingOther | EditKind::TypingFirstSpace | EditKind::TypingConsecutiveSpace
        )
    }

    fn normalized(self) -> EditKind {
        match self {
            EditKind::TypingConsecutiveSpace => EditKind::TypingFirstSpace,
            other => other,
        }
    }

    /// 在 `previous` 之后执行 `self` 前是否断开撤销单元。
    pub fn breaks_after(self, previous: EditKind) -> bool {
        match self {
            EditKind::Other => true,
            EditKind::DeletingLeft => previous != EditKind::DeletingLeft,
            EditKind::DeletingRight => previous != EditKind::DeletingRight,
            typing => {
                if previous.is_typing() != typing.is_typing() {
                    // 从非输入切到输入；输入切到非输入在上面的分支处理。
                    return true;
                }
                if previous == EditKind::TypingFirstSpace {
                    return false;
                }
                previous.normalized() != typing.normalized()
            }
        }
    }
}

/// 一次事务在撤销栈中的记录。
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub forward: Transaction,
    pub inverse: Transaction,
    pub line_endings_before: LineEndings,
    pub line_endings_after: LineEndings,
}

/// 一个撤销单元。
#[derive(Clone, Debug)]
pub(crate) struct UndoGroup {
    /// 单元的身份（#49 的脏标记：撤销 / 重做回到保存时的单元即为“未修改”）。从 1 开始，同一
    /// 历史内不重复；向单元里追加编辑不改变身份。
    pub id: u64,
    pub steps: Vec<Step>,
    pub selections_before: Selections,
    pub selections_after: Selections,
    pub last_edit: Instant,
}

/// 文本撤销 / 重做栈。
#[derive(Clone, Debug, Default)]
pub(crate) struct UndoHistory {
    pub undo: Vec<UndoGroup>,
    pub redo: Vec<UndoGroup>,
    /// 栈顶单元是否还能继续合并。
    pub open: bool,
    /// 上一个分配出去的单元身份。
    pub last_id: u64,
}

impl UndoHistory {
    /// 记录一次编辑。`start_new` 为真时开启新单元。
    pub fn record(
        &mut self,
        step: Step,
        start_new: bool,
        selections_before: &Selections,
        selections_after: &Selections,
        now: Instant,
    ) {
        self.redo.clear();
        match self.undo.last_mut() {
            Some(group) if self.open && !start_new => {
                group.steps.push(step);
                group.selections_after = selections_after.clone();
                group.last_edit = now;
            }
            _ => {
                self.last_id += 1;
                self.undo.push(UndoGroup {
                    id: self.last_id,
                    steps: vec![step],
                    selections_before: selections_before.clone(),
                    selections_after: selections_after.clone(),
                    last_edit: now,
                })
            }
        }
        self.open = true;
    }

    /// 当前文本对应的撤销位置：栈顶单元的身份，空栈为 0（#49）。
    pub fn marker(&self) -> u64 {
        self.undo.last().map_or(0, |group| group.id)
    }

    /// 栈顶单元在 `now` 时是否因超时而不能再合并。
    pub fn timed_out(&self, now: Instant, timeout: Duration) -> bool {
        self.undo
            .last()
            .is_some_and(|group| now.saturating_duration_since(group.last_edit) > timeout)
    }

    /// `pushStackElement`：关闭栈顶单元。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// #50：撤销栈的可持久化快照，最多保留最近 `limit` 个撤销单元与 `limit` 个重做单元
    /// （超限时丢掉最旧的，不报错）。
    pub fn snapshot(&self, limit: usize) -> UndoSnapshot {
        let tail = |groups: &[UndoGroup]| {
            groups[groups.len().saturating_sub(limit)..]
                .iter()
                .map(UndoUnit::from_group)
                .collect()
        };
        UndoSnapshot {
            undo: tail(&self.undo),
            redo: tail(&self.redo),
            truncated: self.undo.len() > limit || self.redo.len() > limit,
        }
    }

    /// #50：按快照重建撤销栈。单元身份沿用快照里的值，下一个新单元的身份比其中最大的大 1，
    /// 所以恢复后的脏标记判定与崩溃前一致。栈顶单元不再接受合并。
    pub fn restore(&mut self, snapshot: UndoSnapshot) {
        let to_group = |unit: UndoUnit| UndoGroup {
            id: unit.id,
            steps: unit.steps.into_iter().map(UndoStep::into_step).collect(),
            selections_before: unit.selections_before,
            selections_after: unit.selections_after,
            // 恢复后的单元不参与合并，时间戳只需要是过去的某个时刻。
            last_edit: Instant::now(),
        };
        self.undo = snapshot.undo.into_iter().map(to_group).collect();
        self.redo = snapshot.redo.into_iter().map(to_group).collect();
        self.last_id = self
            .undo
            .iter()
            .chain(&self.redo)
            .map(|group| group.id)
            .max()
            .unwrap_or(0);
        self.open = false;
    }
}

/// 撤销 / 重做栈的可持久化快照（#50 热退出：`files.restoreUndoStack`）。
///
/// 只含事务与选区，不含文本快照；恢复时按事务回放，所以与备份里的缓冲内容是同一套坐标。
#[derive(Clone, Debug, Default)]
pub struct UndoSnapshot {
    /// 从旧到新；最后一个是栈顶（当前文本对应的单元）。
    pub undo: Vec<UndoUnit>,
    pub redo: Vec<UndoUnit>,
    /// 取快照时按上限丢掉过最旧的单元。
    pub truncated: bool,
}

impl UndoSnapshot {
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    /// 栈顶单元的身份（脏标记用；空栈为 0）。
    pub fn marker(&self) -> u64 {
        self.undo.last().map_or(0, |unit| unit.id)
    }

    /// 在这份快照上撤销 / 重做能不能回到 `marker` 这个保存点。
    ///
    /// 截断掉最旧的单元之后“撤到底”得到的是空栈（标记 0），不再是原来的保存点——#50 据此
    /// 判断恢复出来的文档必须一直显示为“已修改”。
    pub fn reaches(&self, marker: u64) -> bool {
        if self
            .undo
            .iter()
            .chain(&self.redo)
            .any(|unit| unit.id == marker)
        {
            return true;
        }
        marker == 0 && !self.truncated
    }
}

/// 快照中的一个撤销单元。
#[derive(Clone, Debug)]
pub struct UndoUnit {
    pub id: u64,
    pub steps: Vec<UndoStep>,
    pub selections_before: Selections,
    pub selections_after: Selections,
}

impl UndoUnit {
    fn from_group(group: &UndoGroup) -> Self {
        Self {
            id: group.id,
            steps: group.steps.iter().map(UndoStep::from_step).collect(),
            selections_before: group.selections_before.clone(),
            selections_after: group.selections_after.clone(),
        }
    }
}

/// 快照中的一次事务（正向 + 反向 + 前后的换行符状态）。
#[derive(Clone, Debug)]
pub struct UndoStep {
    pub forward: Transaction,
    pub inverse: Transaction,
    pub line_endings_before: LineEndings,
    pub line_endings_after: LineEndings,
}

impl UndoStep {
    fn from_step(step: &Step) -> Self {
        Self {
            forward: step.forward.clone(),
            inverse: step.inverse.clone(),
            line_endings_before: step.line_endings_before.clone(),
            line_endings_after: step.line_endings_after.clone(),
        }
    }

    fn into_step(self) -> Step {
        Step {
            forward: self.forward,
            inverse: self.inverse,
            line_endings_before: self.line_endings_before,
            line_endings_after: self.line_endings_after,
        }
    }
}

/// 光标撤销 / 重做栈：只记录选区。
#[derive(Clone, Debug, Default)]
pub(crate) struct CursorHistory {
    undo: Vec<Selections>,
    redo: Vec<Selections>,
}

impl CursorHistory {
    /// 非编辑引起的选区变化：记录变化前的选区（与栈顶相同则不重复记录）。
    pub fn record(&mut self, previous: &Selections) {
        if self
            .undo
            .last()
            .is_some_and(|top| top.same_ranges(previous))
        {
            return;
        }
        self.undo.push(previous.clone());
        self.redo.clear();
        if self.undo.len() > CURSOR_UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self, current: &Selections) -> Option<Selections> {
        let target = self.undo.pop()?;
        self.redo.push(current.clone());
        Some(target)
    }

    pub fn redo(&mut self, current: &Selections) -> Option<Selections> {
        let target = self.redo.pop()?;
        self.undo.push(current.clone());
        Some(target)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
