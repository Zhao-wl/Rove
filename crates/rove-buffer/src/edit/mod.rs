//! 编辑核心模型（#46，文档 ID R45）：与界面无关，不依赖 gpui。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`transaction`] | 多区间事务、[`EditBuffer`]、tree-sitter 的 [`InputEdit`]、按行失效的 [`LineChanges`] |
//! | [`history`] | 撤销分组规则（[`EditKind`]）与光标撤销 |
//! | [`selection`] | 多选区集合、合并 / 截断、期望列、按字素簇与可见列移动 |
//! | [`word`] | VS Code `wordOperations` / `wordPartOperations` 的单词边界 |
//! | [`clipboard`] | 复制 / 剪切 / 粘贴的文本语义 |
//! | [`rewrite`] | 缩进转换与换行符统一 |
//! | [`session`] | [`EditSession`]：上面这些的组合，编辑类 issue 改文本的唯一入口 |
//! | [`settings`] | 读取的 `editor.*` 设置项 |
//!
//! 超大文件与带只读原因的文档：建立 [`EditBuffer`] 或执行事务时返回
//! [`rove_base::Error::Unsupported`]。

pub mod clipboard;
pub mod history;
mod lines;
pub mod rewrite;
pub mod selection;
pub mod session;
pub mod settings;
pub mod transaction;
pub mod word;

pub use clipboard::{ClipboardData, MultiCursorPaste};
pub use history::{EditKind, UndoSnapshot, UndoStep, UndoUnit, UNDO_GROUP_TIMEOUT};
pub use selection::{CursorPlacement, Selection, Selections};
pub use session::{CursorEdit, EditSession};
pub use settings::EditOptions;
pub use transaction::{
    AppliedEdit, Bias, EditBuffer, InputEdit, LineChange, LineChanges, NewlineMode, Replacement,
    Transaction,
};
pub use word::{DeleteWord, MoveWord, WordClassifier};
