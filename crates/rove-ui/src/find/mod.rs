//! 当前文件查找条（#21）与当前文件替换（#48）。
pub mod engine;
pub mod host;
pub mod registry;
// #48：替换串解析、保留大小写与“全部替换 = 一次事务”。
pub mod replace;
pub mod state;
