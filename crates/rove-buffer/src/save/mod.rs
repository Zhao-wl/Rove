//! 保存与外部修改（#49，文档 ID R48）：与界面无关，不依赖 gpui。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`encode`] | 文本 → 磁盘字节：保持编码（含 GBK）与 BOM；无法编码字符的统计与 `?` 替换；UTF-16 自编码 |
//! | [`write`] | 原子写入：同目录临时文件（关闭即删除）→ `ReplaceFileW`；占用自动重试、每次替换前的检查回调、只读属性、故障注入点 |
//! | [`stamp`] | 磁盘标记（长度、修改时间、内容哈希）：冲突检测与自写去重的依据 |
//! | [`transforms`] | 保存时的文本变换：去行尾空白（可跳过字符串 / 正则）、去末尾空行、补末尾换行 |
//! | [`watch`] | 自写事件去重（路径 + 时间窗 + 内容哈希）、外部修改风暴按路径去抖 |
//! | [`settings`] | `files.eol` / `insertFinalNewline` / `trim*` / `saveConflictResolution` / `autoSave*` 的登记与读取 |
//! | [`file`] | [`TextFile`]：编辑会话 + 磁盘身份 + 脏标记；保存（三步）、冲突、还原、外部修改与删除 |
//!
//! 保存路径上**什么都不顺手改**：文本快照保留原始行终止符（混合换行保存后仍混合），编码与 BOM
//! 按打开时的样子写回，没有保存时格式化。

pub mod encode;
pub mod file;
pub mod settings;
pub mod stamp;
pub mod transforms;
pub mod watch;
pub mod write;

pub use encode::{count_unencodable, encode, EncodeError, Unencodable};
pub use file::{
    new_file_encoding, DiskChange, SaveJob, SaveOutcome, SaveRequest, SaveResult, TextFile,
    UnencodableChoice,
};
pub use settings::{AutoSave, AutoSaveTrigger, ConflictResolution, SaveOptions};
pub use stamp::{content_hash, DiskStamp};
pub use watch::{path_key, ChangeCoalescer, SelfWrites};
pub use write::{
    write_atomic, write_atomic_with, FailPoint, ReplaceFailPoint, WriteError, WriteOptions, Written,
};
