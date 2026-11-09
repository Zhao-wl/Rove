# C# 语义索引文件格式（RVCSIDX v1，M0 原型）

生成方：`tools/rove-indexer-cs`；读取方：`crates/rove-lang-cs/src/index.rs`。
小端序。每个段的起始位置按 8 字节对齐，读取方 mmap 后可零拷贝转换为结构体切片。

## 头部（160 字节）

| 偏移 | 类型 | 说明 |
|---|---|---|
| 0 | `[u8; 8]` | magic `RVCSIDX\0` |
| 8 | u32 | 版本号（1） |
| 12 | u32 | 保留 |
| 16 | 9 × (u64 offset, u64 count) | 段表 |

## 段

| # | 内容 | 元素大小 | 说明 |
|---|---|---|---|
| 0 | 字符串偏移 | u32 | n+1 个，字符串 i = blob[off[i]..off[i+1]] |
| 1 | 字符串 blob | 1 | UTF-8 |
| 2 | 文件 | 24 | path(str) · occ_start · occ_count · pad · xxhash64 |
| 3 | 符号 | 28 | key(str) · name(str) · display(str) · container(sym 或 u32::MAX) · ref_start · ref_count · kind(u8, Roslyn SymbolKind) · flags(u8, bit0=源码符号) · pad |
| 4 | 出现位置 | 20 | file · line · col(UTF-16) · symbol · len(u16) · role(u16: 1=引用 2=定义 4=候选) |
| 5 | 按符号的出现位置索引 | u32 | 每个符号的区间 [ref_start, ref_start+ref_count)，定义排在前面 |
| 6 | 关系（按 from 排序） | 12 | from · to · kind(1=基类型/接口 2=实现接口成员 3=重写) |
| 7 | 关系（按 to 排序） | 12 | 同上，用于“查找实现/派生” |
| 8 | 保留 | — | 文档注释 |

出现位置按 (file, line, col) 排序，所以每个文件的出现位置是连续区间，定位光标下的符号只需二分查找。

## 符号键

- 类型 / 成员：`程序集名|文档注释 ID`（如 `Football.Core|M:Foo.Bar(System.Int32)`）。同一个源码符号被不同项目引用时，得到的键相同。
- 局部变量 / 参数 / 类型参数 / 局部函数 / lambda：`@文件路径:声明偏移:名称`
- 命名空间：`N:全名`

## 已知待改进（M1/M2）

- 局部符号的键里包含完整路径，导致字符串段偏大。应改为 (文件序号, 偏移) 的二进制键。
- 列号是 UTF-16 坐标，编辑器内部使用 UTF-8 字节偏移，需要在读取侧转换或改用字节列。
- 外部符号（DLL 元数据）还没有存签名和文档，无法显示元数据视图。
- 还不支持增量更新的分段/覆盖层。
