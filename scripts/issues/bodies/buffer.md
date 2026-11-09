## 背景

所有视图、编辑与语言功能都建立在文本缓冲之上。本 issue 只做"打开、读取"的数据层：把 [[contracts]] 定义的 `TextSource` 与文档模型实现出来，重点是 Windows 上的编码与换行符现实问题（`PROJECT_CHARTER` §7）。M0 的两个原型一律按 UTF-8 处理、非 UTF-8 走 lossy，**编码处理完全未验证**（`M0_UI_SPIKE_REPORT` §8 第 7 项），这里是第一次真正做。

实现放在 `rove-buffer`，**在 [[contracts]] 定义的接口背后**，本 PR 不修改这些接口；确需修改时按契约变更流程单独提 PR。替换 [[skeleton]] 中的 `SKELETON(buffer)` 标记（"按 UTF-8 读取整个文件"），并更新 `docs/ARCHITECTURE.md` 的对照表。

## 目标

### 1. `Buffer`

基于 `ropey`，实现 [[contracts]] 的 `TextSource`：行数、按行取文本、按字节区间取文本、版本号；`Send + Sync`；不复制全文。文档模型携带语言、编码、换行符、只读原因、大小级别（大小级别的判定与超大文件由 [[largefile]] 接管，本 issue 只预留字段并实现"普通"一级）。

### 2. 编码识别（`UX_SPEC` §3.5）

- **BOM 优先**：UTF-8 BOM、UTF-16 LE/BE BOM。
- **自动识别**：用户没有设置 `files.autoGuessEncoding` 时，Rove **默认开启**自动识别，候选编码默认为 `["gbk"]`（`files.candidateGuessEncodings`）；用户显式设置了这两项时**遵从用户的设置**。这是 `FEATURES` §13 规则 5 的典型案例：Rove 的默认值只在用户未设置时生效，**绝不写回共用的设置文件**。
- 猜不出来时遵循 `files.encoding`（默认 `utf8`），支持语言级覆盖（如 `"[csharp]"`）。
- 用 `chardetng` 推断、`encoding_rs` 解码。
- 记录**是否由自动识别得出**，供状态栏悬停提示注明"自动识别"（界面在 [[statusbar]]）。
- 设置一律通过 [[contracts]] 的设置读取接口获取；[[settings]] 合并前用契约中的空实现与默认值，不要自己读文件。

### 3. BOM

记录是否带 BOM，**原样保留**（保存由 [[save]] 负责，但往返测试在本 issue 完成）。

### 4. 换行符（`UX_SPEC` §3.6）

识别 LF、CRLF、CR 以及混合情况，记录"主换行符"（占多数的那种）和"是否混合"。**混合换行符保持原样，不做静默统一**——这是与 VS Code 的明确差异（VS Code 打开时会 normalize 成多数派，保存后整文件产生差异）。新输入的换行使用主换行符（由 [[edit-core]] 使用）。`files.eol` 只用于新建文件。

### 5. 二进制检测

出现 NUL 字节或解码失败比例异常时，拒绝作为文本打开并返回**明确的错误类型**，以便上层按 `UX_SPEC` §3.5 提示"此文件是二进制文件或使用了不支持的文本编码"并提供"仍然打开"。"仍然打开"的路径也要可用（按字节 + lossy 显示）。

### 6. 位置转换

UTF-8 字节偏移 ↔（行号，UTF-8 列）↔ **UTF-16 列**。C# 索引格式 v1 的列号是 UTF-16 坐标（`docs/CS_INDEX_FORMAT.md`），在 [[idx-format]] 把它改成 UTF-8 列之前，[[router]] 依赖这个转换；代理对与组合字符的边界必须正确。

## 不包含

- 编辑与撤销（[[edit-core]]）、保存与外部修改（[[save]]）。
- 超大文件、mmap、分块行索引、大小分级的判定（[[largefile]]）。
- 文件监听（[[workspace]]）、状态栏与编码切换界面（[[statusbar]]）。
- 搜索时的编码处理（[[search]]，但它会复用本 issue 的解码路径）。

## 验收标准

- [ ] 测试矩阵覆盖：UTF-8（带 / 不带 BOM）、UTF-16 LE/BE、GBK、ASCII、空文件、只有一行的大文件、末尾无换行、纯 CR、混合换行符、含代理对与 emoji、含 NUL 的二进制文件
- [ ] 所有编码"加载 → 原样写回"后字节完全一致（含 BOM 与混合换行符）
- [ ] 混合换行符的文件加载后 `is_mixed = true`，主换行符正确，且内容逐字节未被改写
- [ ] `files.autoGuessEncoding` 未设置时 GBK 文件正确显示；显式设为 `false` 时**不**自动识别（回到 `files.encoding`）；两种情况都有测试
- [ ] UTF-8 列 ↔ UTF-16 列转换在代理对、组合字符、CRLF 行尾上有用例
- [ ] 基准：打开 10 MB 文件 < 50 ms（接入 [[perf]]）
- [ ] `SKELETON(buffer)` 标记已删除，`docs/ARCHITECTURE.md` 的对照表已更新
- [ ] 新增一个走"打开文件 → 读行 → 查编码与换行符"链路的集成测试；[[skeleton]] 的端到端测试继续通过
- [ ] `cargo tree -p rove-buffer -i gpui` 为空

## 风险与提示

- 自动识别一定会有误判（短文件、ASCII 兼容的多编码）。宁可在不确定时回落到 `files.encoding`，也不要给出一个"看起来对"的错误编码——用户能从状态栏手动重新打开（[[statusbar]]），但静默的乱码很难发现。
- 不要在本 issue 里顺手实现"按编码重新打开 / 保存"的命令。命令要通过 [[contracts]] 的命令登记机制注册，由 [[statusbar]] 负责。
- `ropey` 的索引单位是 char，而接口里到处是字节偏移和 UTF-16 列，三套坐标混用是最容易出 bug 的地方。建议在 `rove-base` 的位置类型上做强类型区分（新类型包装），而不是到处传裸 `usize`。
