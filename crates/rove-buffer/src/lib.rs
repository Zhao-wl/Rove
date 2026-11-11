//! rove-buffer：不可变文本快照、文档元信息与打开文件的加载路径。
//!
//! 冻结的跨 issue 接口在 [`contracts`]（#1）；本 crate 提供它们的正式实现：
//!
//! - [`RopeText`]：基于 `ropey` 的 [`TextSource`]，负责行与 UTF-8/UTF-16 坐标转换；
//! - [`open_path`] / [`open_bytes`]：编码识别（BOM、自动识别与候选编码）、解码、
//!   换行符统计与元信息；
//! - [`settings`]：`files.encoding` 等三个设置键的登记与默认值读取。
//!
//! - [`open_file`]（#8）：按文件大小分级（[`size`]）；超大文件走 [`MappedText`]
//!   （内存映射 + 分块行索引 + 追加内容的增量索引）。
//!
//! 不包含编辑、保存、文件监听（#15）与界面（#22、#27）。

pub mod contracts;
pub use contracts::*;

mod decode;
mod open;
pub mod settings;
mod text;

pub use decode::Detection;
pub use open::{open_bytes, open_path, LoadOptions, LoadedText};
pub use text::RopeText;

mod line_index;
mod mapped;
pub mod size;

pub use mapped::{MappedText, Reload, INDEX_CHUNK_BYTES, SYNC_INDEX_BYTES};
pub use open::{open_file, FileText, OpenedFile};
pub use size::{disabled_features, session_toggle_allowed, ReducedFeature, SizePolicy};
