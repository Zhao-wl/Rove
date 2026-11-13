//! 快速打开与命令面板（#24）：`Ctrl+P`、`Ctrl+Shift+P` / `F1`、`Ctrl+G`，以及 `:` `@` `#` `>` 四个前缀。
//!
//! 打分、排序与高亮全部来自核心层 `rove-fuzzy`（VS Code `fuzzyScorer` 的逐条移植）。浮层、列表与键盘导航
//! 复用 #18 的 [`QuickInputView`](crate::list_widget::quick_input_view::QuickInputView)；这里只负责
//! “输入文本 → 候选列表”：
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`parse`] | 输入文本的前缀判定（文件 / `>` / `:` / `@` / `#`）与 `文件:行:列` 后缀 | 否 |
//! | [`files`] | 文件候选（工作区文件列表的增量索引）、按键间前缀剪枝、历史文件加权 | 否 |
//! | [`commands`] | 命令面板：已实现的命令、最近使用、VS Code 的 `WORD_FILTER` | 否 |
//! | [`lines`] | 转到行/列的文本解析与提示 | 否 |
//! | [`matcher`] | 把 `scoreFuzzy2` 包成 #18 的 [`Matcher`](crate::list_widget::matcher::Matcher)（`@` 符号与 `#` 工作区符号的高亮用） | 否 |
//! | [`registry`] | 命令（VS Code ID）、界面字符串、GPUI action | 仅 action |
//!
//! 宿主（骨架 `Workbench`，见 `skeleton/workbench/quick_access_host.rs`）负责：取工作区文件列表与事件、
//! 创建浮层、按设置（`search.quickOpen.*`、`workbench.quickOpen.*`）决定行为、经 #17 的
//! `OpenDocuments`（`OpenSource::QuickOpen` / `GoToLine`）打开与定位。
pub mod commands;
pub mod files;
pub mod lines;
pub mod matcher;
pub mod parse;
pub mod registry;

use crate::{
    list_widget::{quick_input::RowDecor, StaticList},
    lists::Row,
};
use rove_base::Version;
use std::sync::Arc;

/// 一次计算得到的列表：行、每行的高亮与右侧文本、分组分隔行（起点下标, 标签）。
#[derive(Default)]
pub struct Picks {
    pub rows: Vec<Row>,
    pub decor: Vec<RowDecor>,
    pub separators: Vec<(usize, Option<Arc<str>>)>,
}

impl Picks {
    pub fn model(&self, revision: u64) -> StaticList {
        StaticList::new(self.rows.clone(), Version(revision))
    }

    pub fn push(&mut self, row: Row, decor: RowDecor) {
        self.rows.push(row);
        self.decor.push(decor);
    }

    pub fn separator(&mut self, label: Arc<str>) {
        self.separators.push((self.rows.len(), Some(label)));
    }
}

#[cfg(test)]
mod tests;
