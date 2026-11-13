# C# 语义索引文件格式（RVCSIDX v2）

生成方为 tools/rove-indexer-cs，读取方为 crates/rove-lang-cs/src/index.rs。所有整数为小端无符号整数，段按表中顺序写入，起始位置按 8 字节对齐。索引进程先完整写入同目录临时文件并将文件数据刷盘，再原子替换目标。打开的 mmap 对应一份不可变快照。

## 头部（336 字节）

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | 8 字节 | magic，ASCII RVCSIDX 加一个 NUL |
| 8 | u32 | 格式版本，当前为 2 |
| 12 | u32 | 段数，当前为 15 |
| 16 | u64 | 含末尾对齐的文件总字节数 |
| 24 | 4 × u16 | 索引进程的程序集版本，依次为 major、minor、build、revision |
| 32 | 32 字节 | 项目配置 SHA-256 |
| 64 | u64 | 标志位，保留给覆盖层，v2 写 0 |
| 72 | 16 字节 | 基础索引世代标识，保留给覆盖层，v2 写 0 |
| 88 | u64 | 头部 FNV-1a-64，输入为字节区间 [0, 88) 接 [96, 336) |
| 96 | 15 × (u64 offset, u64 count) | 段表，count 单位见下表 |

索引进程版本取自 RoveIndexer.csproj 的显式 Version，目前为 1.2.0（#35 加入常驻模式与声明指纹表；1.1.0 是 #34 的元数据视图），对应头部 1.2.0.0。凡可能改变持久化索引内容、项目解析或缓存判定的改动，都要在合并前递增 Version（兼容修正递增 patch，新能力递增 minor，不兼容的进程接口递增 major）。更新版本时同步更新端到端测试的头部断言。磁盘布局不兼容时还须独立递增格式版本。

后续版本可增加段数。v2 读取端遇到其他格式版本会显式返回版本错误。打开时检查文件总长、头部校验、段对齐、重叠、边界及计数运算，以及独立的字符串偏移表与文件目录。打开过程不读取路径或符号字符串正文，也不建路径 HashMap。字符串 UTF-8 在访问时检查。对打开后仍可能遇到的无效记录索引，查询使用边界检查。

## 段

未注明宽度的 ID、行号、偏移及计数均为 u32，从 0 开始。字段按表中的顺序写入。

| # | 内容 | 元素字节数 | 布局 |
|---|---|---|---|
| 0 | 字符串偏移 | 4 | n+1 个偏移，首项为 0，非递减，末项等于段 14 长度 |
| 1 | 文件 | 24 | path_start、occ_start、occ_count、inactive_start、hash(u64) |
| 2 | 路径正文 | 1 | UTF-8，按文件序号连接，不带 NUL |
| 3 | 符号 | 24 | key、name、display、container、ref_start、kind(u8)、flags(u8)、definition_count(u16) |
| 4 | 局部符号键 | 8 | file、declaration_byte_offset |
| 5 | 紧凑出现位置 | 8 | 每条一字，见下文 |
| 6 | 宽出现位置 | 24 | file、line、col、symbol、len、role |
| 7 | 按符号的出现位置索引 | 1 | count 是字节数，见下文 |
| 8 | 关系（按 from、to 排序） | 12 | from、to、kind |
| 9 | 关系（按 to、from 排序） | 12 | 同段 8，用于查找实现和派生 |
| 10 | 不生效行区间 | 8 | start_line、end_line，左闭右开 |
| 11 | 外部符号签名 | 8 | symbol、text(字符串 ID)，按 symbol 排序 |
| 12 | XML 文档 | 8 | symbol、text(字符串 ID)，按 symbol 排序 |
| 13 | 元数据视图 | 1 | count 是字节数，见下文 |
| 14 | 字符串正文 | 1 | UTF-8，字符串 i 为 offsets[i]..offsets[i+1] |

文件路径用斜杠分隔并相对于项目根目录。Windows 上跨卷文件使用无法相对化的绝对路径。文件表按 UTF-8 字节序排序，路径终点是下一文件的 path_start 或路径段末尾。路径查询使用精确字节比较及二分查找。文件哈希为源文件原始字节的 xxHash64。

启用 --generators 时，Roslyn 的生成树也进入文件表并参与定义和引用收集。虚拟路径为 rove-generated:// 加分别进行 URI 转义的项目相对路径与 Roslyn 树路径，中间用 / 分隔。树路径先统一为斜杠。同一项目中若树路径重复，后续条目附加 #1、#2 等序号。虚拟路径不指向磁盘文件，文件哈希取生成文本无 BOM 的 UTF-8 字节。生命周期层抽查磁盘文件时跳过这些条目。生成树中的局部符号使用正常的文件序号和声明字节偏移。

文件的出现位置连续存放，文件内按 (line, col) 排序。对项目所属宏求值后，索引进程从 Roslyn 的 DisabledTextTrivia 提取不生效行区间。相邻或重叠区间合并。每个文件的区间终点为下一文件的 inactive_start 或段 10 末尾。

符号的 name 和 display 为字符串 ID。container 为符号 ID，u32::MAX 表示无。kind 是 Roslyn SymbolKind，flags 的 bit 0 表示存在源码位置。关系 kind 的值为 1（基类型或接口）、2（实现接口成员）、3（重写成员）。

## 符号键

- 类型和成员使用程序集名加竖线加 Roslyn 文档注释 ID。例如 Football.Core|M:Foo.Bar(System.Int32)。跨项目引用同一个源码符号时键相同。
- 局部变量、参数、类型参数、局部函数、lambda 等拥有可索引声明位置的符号使用段 4 的二进制键。符号记录的 key 最高位为 1，低 31 位为段 4 中的序号。声明偏移基于解码后的 UTF-8 文本，不含 BOM。单参数 lambda 使用箭头起点，避免和参数声明同址。
- 命名实参引用的外部成员参数（方法、索引器、委托的 Invoke）使用所属成员的键加井号和参数序号，例如 UnityEngine.CoreModule|M:UnityEngine.Object.Instantiate(UnityEngine.Object,UnityEngine.Transform)#1。序号为原始定义中的序号：扩展方法以实例形式调用时算上 this 参数，构造泛型按原始定义取。文档注释 ID 里不出现井号，所以不会和成员键相撞。参数所属成员在本次索引的某个项目里有源码声明时不用这种键，仍然回指那个源码声明（见下）。
- 其他 key 最高位为 0，值为字符串 ID。命名空间继续使用 N:全名。无法生成文档注释 ID 且没有可索引声明位置时，沿用以问号开头的种类和显示名后备键。元组元素名（`System.ValueTuple{…}.<名字>`）只存在于源码，元数据里只有 Item1、Item2，所以落在这个后备键上。

## 出现位置与引用索引

紧凑字从低位到高位布局如下。

| 位 | 宽度 | 字段 |
|---|---|---|
| 0–2 | 3 | role，1=引用，2=定义，4=候选标记 |
| 3–7 | 5 | len，UTF-8 字节数 |
| 8–15 | 8 | col，UTF-8 字节列 |
| 16–27 | 12 | line，物理行号 |
| 28–48 | 21 | symbol |
| 49–63 | 15 | file |

字段超出位宽时保留排序位置，低三位写 0，其余位为段 6 中的宽记录序号。宽记录的行、列、长度、ID 均为 u32，role 使用相同位标记。读端二分文件内位置时仅解码参与比较的行列。查找符号列表由符号序号直接定位。

段 7 的每项是段 5 的出现位置序号。出现位置数小于等于 2²⁴ 时每项为 3 字节，否则为 4 字节。ref_start 按项计量。区间终点为下一符号的 ref_start 或出现位置总数。每个符号的定义排在引用前面，两组内分别按出现位置序号升序。definition_count 为区间开头的定义数量，u16::MAX 表示至少 65,535 条，读取端按 role 找定义区间终点。

## 元数据视图（段 11、12、13，#34）

签名与 XML 文档均使用段 14 中的字符串。元数据视图非空时先写 u32 view_count、u32 member_count，再写 view_count 个 16 字节视图记录和 member_count 个 16 字节成员记录。视图记录为 type_symbol、text(字符串 ID)、member_start、member_count，按 type_symbol 排序，同一类型再按程序集标识的显示名排序。成员记录为 member_symbol、line、col、len，每个视图的成员区间内按 (member_symbol, line, col) 排序，可二分查找。成员位置相对于视图正文，列与长度使用 UTF-8 字节。读取端打开时检查段长与两个计数一致、视图有序、成员区间不越界；符号与字符串 ID 在访问时检查。

- **哪些类型有视图**：索引过程中实际被引用到的外部类型与成员（来自不属于本次索引任何项目的 DLL），按最外层类型生成视图；嵌套类型在外层类型的视图里。去重键是程序集标识（名称、版本、文化、公钥标记）加类型的文档注释 ID：多个项目引用同一个 DLL 只生成一份；同名程序集的不同版本各一份，此时同一 type_symbol 有多个视图记录。视图的 DLL 路径在引用同一标识的路径中取 Ordinal 最小者，结果与项目处理顺序无关。
- **视图正文**：前 4 行固定为 `#region 程序集 <程序集显示名>`、`// <DLL 完整路径>`、`#endregion` 和空行，随后是正文用到的 using、文件范围的命名空间声明（`namespace X;`）与类型声明。只有签名，方法、属性访问器和事件都以 `;` 或 `{ get; set; }` 结束，没有方法体。成员包括公共与受保护的字段、属性、索引器、事件、构造函数、析构函数、方法、运算符和嵌套类型，以及源码经 InternalsVisibleTo 实际引用到的非公共成员。签名用 Roslyn 的 SymbolDisplayFormat 渲染。XML 文档注释以 `///` 写在成员上方，`<see cref>`、`<paramref>` 等标签原样保留，只去掉首行缩进与首尾空行。文档来自 DLL 旁的同名 `.xml`；BCL 程序集另查本机 .NET SDK 的参考程序集包。找不到时只有签名。生成侧用 Roslyn 解析每份视图自检，语法错误计入报告。
- **成员记录**：只为符号表中存在的符号写位置，即被引用到的成员、类型自身与类型链上的外层类型，以及命名实参引用到的参数。位置指向声明里的名字：构造函数与析构函数指向类型名，索引器指向 `this`，运算符指向 `operator`，委托的 Invoke 指向委托名，参数指向所属成员签名行上的参数名（与所属成员同一行、不同列）。外部符号的定义位置为视图序号加视图内位置：沿符号的 container 链到最外层类型，在其视图的成员区间里二分查找。
- **段 11（签名）**：有成员记录的符号各一条，文本为视图中该成员声明行去掉缩进。同一符号出现在多个版本的视图中时，取排在前面的视图。
- **段 12（XML 文档）**：#34 不写，保持为空。文档只在视图正文里存一份，读取端从视图中成员上方连续的 `///` 行取出并去掉前缀（`CsIndex::xml_doc`）。在大型项目上，单独存一份约 4 MB。

外部 DLL 不变时视图内容不变。#35 的增量覆盖层只在 DLL 引用或其内容变化时才需要重建视图，普通源码修改只可能让新的外部类型或成员被引用到。

## 项目配置哈希

哈希输入由索引进程序列化，不包含源文件内容。列表先写 i32 项数，字符串以 .NET BinaryWriter 的 7 位编码长度前缀和 UTF-8 正文表示。路径以项目根目录相对路径和斜杠表示，跨卷规则与文件表一致。

1. 写入域标识 rove-cs-project-config-v1，随后写入生成器开关和 Unity 磁盘对齐开关（各一字节 bool）。
2. 写项目数，按项目相对路径的 UTF-16 Ordinal 顺序写入成功解析的 csproj。每项写入相对路径、程序集名、Roslyn LanguageVersion 枚举值(i32)、unsafe 开关。随后依次写入宏集合（去重排序）、实际参与编译的源码路径列表（对齐后排序）、DLL 引用列表和项目引用列表（保留原顺序）、生成器启用时的 analyzer 路径列表（保留原顺序，否则空列表）。
3. 写 Assets 与 Packages 下的 asmdef 和 asmref 文件数。按相对路径的 UTF-16 Ordinal 顺序写相对路径、i32 正文字节数和规范化的 UTF-8 JSON。对象属性按名称 Ordinal 排序，数组保留顺序。空白、注释和尾随逗号不影响结果。无法读取或解析的配置文件保留路径并写零长度正文。

源码路径列表有意保留在哈希中，用于 #36 打开项目时发现离线增删文件或文件的程序集归属变化。仅抽样比对旧索引已有文件的内容哈希，无法发现新文件。运行中普通源码的修改与增删由 #36 转发给常驻索引进程增量更新（#35，见下文「覆盖层」）。项目配置变化仍走全量重建。

同一有效配置的 csproj 排版变化不会触发失效。文件是否需要因源码内容变化而重建，由文件哈希及后续生命周期逻辑判断。DLL 内容变更、缓存目录及预读策略由生命周期层负责。

## 已知待改进

- 元数据视图里的签名中引用到的其他类型没有出现位置记录，在视图里对它们继续跳转需要 #38 / #42 按名称解析或扩展格式。
- 元组元素名（源码里给 `(Renderer component, Bounds bounds)` 起的名字）在元数据里不存在，只有 Item1、Item2，所以没有视图内位置；大型项目上剩余的落空全是这一类，约占外部符号引用的 0.005%。
- 覆盖层段里的新外部符号（基础索引里没有被引用过的 DLL 成员）没有元数据视图位置，下一次全量重建后才有（见下文「覆盖层」的前提）。

## 覆盖层（#35）

常驻索引进程（`rove-indexer-cs --serve`，协议见下一节）每次重绑一批文件，就把结果写成一个独立的**覆盖层段**，追加到与基础索引配套的日志文件 `l2\<世代>.rvcsovl`（与 `<世代>.rvcsidx` 同目录）。基础索引不重写。头部偏移 64 与 72 为覆盖层预留的两个字段没有用上，仍写 0：覆盖层放在独立的日志文件里，用基础索引头部已有的校验和与总长配对（见下表偏移 24），基础索引的字节不需要为覆盖层改变。读取与合并在 `crates/rove-lang-cs/src/overlay.rs`，写入在 `tools/rove-indexer-cs/Overlay.cs`。

### 段布局

所有整数为小端。字符串写成 u32 字节数加 UTF-8 正文。键写成 u8 标记加内容：0 无；1 全局键（字符串，与段 3 的全局键同一文本）；2 局部键（项目相对路径字符串 + u32 声明的 UTF-8 字节偏移，偏移基于该文件的新内容）。

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | 8 字节 | magic，ASCII RVCSOVL 加一个 NUL |
| 8 | u32 | 覆盖层格式版本，当前为 1 |
| 12 | u32 | 保留，写 0 |
| 16 | u64 | 段总字节数（含末尾校验和，8 的倍数） |
| 24 | 16 字节 | 基础索引标识：基础索引头部校验和（偏移 88）接文件总长（偏移 16） |
| 40 | u64 | 段序号，从 1 开始严格递增 |
| 48 | … | 正文 |
| 末尾 8 字节 | u64 | FNV-1a-64，输入为本段之前的全部字节 |

正文依次为：

1. 符号表：u32 个数，每项为键、名称、显示名、种类(u8)、标志(u8，bit 0 有源码位置，bit 1 来自外部 DLL)、容器键（可为无）。出现位置与关系用本段符号表的下标引用符号。
2. 文件：u32 个数，每项为路径、已清空(u8)、内容哈希(u64，源文件原始字节的 xxHash64)、出现位置（u32 个数，每项 line、col、len、symbol 各 u32 与 role u8，文件内按行列排序）、不生效行区间（u32 个数，每项 start、end）、声明指纹（u32 个数，每项 symbol u32 与 fingerprint u64）。删除的文件写一条已清空的记录，没有出现位置。
3. 关系归属：u32 个数，每项 symbol u32 与部分 u8（1 = 符号自己的基类型与重写关系；2 = 类型的成员实现接口成员的关系）。
4. 关系：u32 个数，每项 from、to（符号下标）与 kind(u8，取值同段 8)。只包含本段归属里的关系，且是重算后的完整集合（可以为空）。

正文之后补 0 到总长为 8 的倍数，再写校验和。

### 读取与合并

- 读取端从日志开头逐段校验 magic、版本、总长、校验和、基础索引标识与序号递增，在第一个无效处停止：写到一半的尾部、属于其他世代的日志都会被丢弃。常驻进程打开日志时同样截掉无效尾部再继续追加。
- **文件**：同一路径以最后一个段为准，整份替换基础索引中这个文件的出现位置、不生效区间与内容哈希；已清空的文件不再出现在文件表里，基础索引中它的出现位置也不再返回，不留悬空位置。新文件排在基础文件号之后，结果中按路径序与基础文件交错。
- **符号**：全局键在基础符号表中按键二分（UTF-16 Ordinal，O(log n)）。局部键属于未被替换的文件时按（文件号, 偏移）二分；属于被替换或新建的文件时是覆盖层专用符号。找不到的键分配覆盖层专用序号（从基础符号数往后排）。符号记录（显示名、种类、容器）取最后一份有源码位置的记录，没有时保留基础记录。
- **关系**：每个（符号, 部分）以最后一个声明了该归属的段为准，且只在该段里声明它的文件版本仍是该文件的最新版本时有效（文件后来被改写、不再声明这个符号时旧归属作废）；基础索引里同一部分的关系作废。没有被覆盖层重算、且声明（部分 2 看所在类型的声明）仍在未被替换的基础文件里的关系继续有效；声明已全部被替换掉的符号不再有关系。
- 合并结构在每次读到新段时整体重建一次（与覆盖层规模成正比）。查询时基础索引仍是二分查找，覆盖层是哈希表与文件内二分，复杂度与段数无关（`cargo xtask bench --filter cs_overlay/`：段数 0 → 1024 时单次查询仍在数微秒）。重建合并结构的耗时与覆盖层的出现位置总数成正比，这是设上限的原因。
- 覆盖层的文件数或日志体积超过上限（默认 256 个文件或 32 MiB，`OverlayLimits`）时，生命周期后台全量重建并原子替换，期间旧索引 + 覆盖层照常可查询。新世代就绪后旧日志随旧世代一并删除。
- 文件在索引中的版本（#38 判断缓冲区与索引是否一致）是“覆盖层里的内容哈希，否则基础文件表里的哈希”（`L2View::file_hash`）。

**元数据视图的前提**：DLL 没变时视图不需要重建（见上文「元数据视图」）。覆盖层不带视图；普通源码修改新引用到的外部类型或成员在下一次全量重建前没有视图位置。DLL 引用变化体现在 csproj 上，按项目配置变化全量重建。

### 声明指纹表（`<索引>.decls`）

全量索引同时写出旁路文件 `<索引路径>.decls`（先写临时文件再改名）：magic `RVCSDCL\0`、u32 版本 1、u32 条数、16 字节基础索引标识，随后按键哈希排序的（u64 键的 xxHash64, u64 声明指纹）。它不属于索引格式，读取端不需要；常驻进程用它还原基础索引中声明的旧指纹。缺失或不配套时退回“显示名 + 种类”的弱比较（看不出可见性与修饰符的变化）。

声明指纹是带可访问性、修饰符、返回类型、参数（含名字与默认值）、类型参数约束与基类型列表的完整显示串的 xxHash64；方法体、字段初始值、常量的值不参与。

## 常驻模式与协议（#35）

`rove-indexer-cs --serve [--idle-exit-seconds N] [--trim-seconds N]`。传输是 stdin / stdout 上逐行的 JSON-RPC 2.0（每行一个 UTF-8 JSON 对象，没有 Content-Length 头），stderr 仍是日志。选标准流而不是命名管道：进程本来就是 Rove 的子进程，标准流是这对进程私有的通道，不需要管道名、访问控制与连接握手；Rove 退出时 stdin 关闭，进程随即落盘退出，不会留下孤儿进程。进程不发起任何网络请求（只读写项目文件、基础索引与日志）。

| 方向 | 消息 | 内容 |
|---|---|---|
| 进程 → Rove | `hello`（通知，第一行） | `protocol`（进程能说的最高协议，当前 1）、`protocol_min`（最低协议，当前 1；缺省等于 `protocol`）、`indexer_version`、`pid`。**版本字段在第一条消息里**；协商规则见下文「组件、协议与索引格式的版本」（#43） |
| Rove → 进程 | `initialize` | `protocol`（Rove 按协商规则选定的版本；不在进程支持的区间内则回复错误 -32001 而不是硬跑）、`project`、`base`、`journal`，可选 `idle_exit_seconds`、`trim_seconds`、`background_duty`、`max_warm_files`、`background_batch`。回复已有段数、日志字节数、覆盖层文件数 |
| Rove → 进程 | `changed` | `changes: [{ path, kind }]`（项目相对路径；变更 / 新增 / 删除由进程按磁盘判断，重命名发两条）。前台重绑后回复 `files`、`segment`（序号、偏移、长度）、`impact`（变化的键、新名字、队列长度）、`locked`（被独占、稍后重试的文件）、`rebuild_required`（`.csproj` / `.asmdef` / `.asmref` 变化，不做增量）、`elapsed_ms` |
| Rove → 进程 | `index` | `project`、`output`，可选 `rebase` 与 `journal`：全量索引，期间发 `progress { done, total }` 通知；`rebase` 时完成后切换到新的基础索引 |
| Rove → 进程 | `status` | 回复正在更新的文件、队列长度、被独占的文件数、覆盖层文件数与段数、日志字节数、预热的项目、工作集 |
| Rove → 进程 | `wait_idle` | 后台队列清空后回复（测试与基准用） |
| Rove → 进程 | `set_trim` | 运行中调整 `trim_seconds` / `idle_exit_seconds`（测量空闲内存用） |
| Rove → 进程 | `shutdown` | 日志刷盘后回复并退出；stdin 关闭等同于它 |
| 进程 → Rove | `overlay`（通知） | 后台重绑（影响分析、被独占文件的重试）写了新段，内容同 `changed` 的回复 |
| 进程 → Rove | `status`（通知） | 队列变化时推送 |
| 进程 → Rove | `exiting`（通知） | 空闲退出前发出，`reason: "idle"` |

- 空闲 `trim_seconds`（默认 120 s）后丢掉预热的编译，只保留项目模型与依赖镜像；空闲 `idle_exit_seconds`（Rove 默认 600 s，依据见 `ResidentOptions::idle_exit`）后刷盘退出，下一次文件变化时由 #36 重新拉起。
- 重绑：文件按所属项目的 `DefineConstants` 重新解析（与全量索引同一份解析选项），用 Roslyn 的 `ReplaceSyntaxTree` / `AddSyntaxTrees` / `RemoveSyntaxTrees` 更新该项目的编译，依赖项目以元数据镜像引用（与全量流水线相同）。读文件用共享模式并重试；仍被独占时放进重试列表（退避至多 15 s），不中断进程。
- 影响分析（`Impact.Diff`，规则可单独测试，`--self-test`）：声明的键消失或指纹变化 → 按“基础索引 + 覆盖层”的引用表反查引用它的文件，以及指向它的实现 / 重写关系所在的声明文件；新出现的键 → 在能看到该项目的项目（本项目与传递下游）里按标识符文本查找它的名字（覆盖之前解析不到或绑定到别处的同名标识符）；文件里声明、被其他文件引用的局部键（命名实参指向的参数、partial 类型的类型参数）→ 引用它们的文件。受影响的文件进后台队列：前台请求优先、批之间可取消，线程降为低于正常并按占空比（默认 0.5）休眠。声明变化的项目在下游重绑前重新输出元数据镜像。

## 组件、协议与索引格式的版本（#43）

三个版本号各管一件事，Rove 在三个时刻核对它们：不启动进程时读组件清单、拉起常驻进程时读 `hello`、打开缓存索引时读头部。

| 版本 | 含义 | 来源（单一事实） | Rove 侧 |
|---|---|---|---|
| 组件版本 | 索引进程的发布版本，也是写进索引头部的“索引进程版本” | `RoveIndexer.csproj` 的 `<Version>`（当前 1.2.0） | 缓存索引头部版本必须等于**已安装组件的版本**，否则重建；任意组件版本都可以使用，只要下面两项兼容 |
| 协议版本 | `--serve` 常驻协议的消息约定 | `Server.cs` 的 `Protocol`（最高）与 `ProtocolMin`（最低），当前 1..1 | `resident::PROTOCOL` 与 `PROTOCOL_MIN`，当前 1..1 |
| 索引格式版本 | `.rvcsidx` 的二进制布局 | `IndexWriter.Version`（当前 2） | `validity::EXPECTED_FORMAT`，只读这一个版本 |

对应关系（发版时在这里加一行；`component.json` 由 `rove-indexer-cs --version` 生成，不手写）：

| Rove | 组件 | 协议区间 | 索引格式 |
|---|---|---|---|
| 当前开发版（M2） | 1.2.x | 1..1 | 2 |

### 兼容规则

1. **格式必须相等。** 组件写的索引格式不是 Rove 读得懂的那一个 → 不兼容。格式更低 → 组件太旧；更高 → 组件太新。Rove 不读旧格式，也不猜新格式。
2. **协议区间必须有交集。** Rove 支持 `PROTOCOL_MIN..=PROTOCOL`，组件支持 `protocol_min..=protocol`；取 `min(PROTOCOL, 组件 protocol)` 作为本次使用的协议，它不低于双方的最低值才算兼容，`initialize` 里就发这个版本。组件最高协议低于 Rove 的最低协议 → 组件太旧；组件最低协议高于 Rove 的最高协议 → 组件太新。**两个方向都覆盖**：用户装了新组件、Rove 还是旧的，与反过来的情形，都会得到明确的“更新哪一边”的提示。
3. **组件版本不参与兼容判定**，只用来决定缓存索引是否要重建（头部版本 = 当前组件版本）。所以补丁与小版本更新不需要改 Rove；组件换版本后首次打开项目会重建一次索引。
4. **判定两次**：发现组件时读 `component.json`（`protocol`、`protocol_min`、`index_format`，不启动进程）；拉起常驻进程后再用第一条消息 `hello` 的协议区间核对一次（环境变量覆盖的组件没有清单，只靠这一次）。全量构建不走协议，靠清单与索引头部（格式 + 组件版本）把关。
5. 协议或格式有不兼容改动时：协议 +1（需要时同时抬高 `ProtocolMin`）、`IndexWriter.Version` +1，同一个 PR 里同步 `PROTOCOL` / `PROTOCOL_MIN` / `EXPECTED_FORMAT` 与上面的对应表；向后兼容的扩展（新字段、新方法）不动协议号。

### 不兼容与缺失时的处理

| 情形 | Rove 的状态 | 导航 | 提示 |
|---|---|---|---|
| 没有安装组件 | `NotInstalled`，状态栏 `$(info) C# 基础导航` | L1/L0，近似结果 | 点击打开说明页：含安装方法 |
| 已安装，格式或协议不兼容（组件太旧 / 太新） | `Incompatible`，状态栏 `$(warning) C# 组件需更新`，不重试 | L1/L0 | 悬停与说明页指出更新组件还是更新 Rove，并写出组件版本 |
| 缓存索引有效，但常驻进程 `hello` 协议对不上 | 保持 `Ready`，`component_issue` 记录原因；该进程被放弃 | 索引照常可用，源文件变化只标记过期（近似） | 悬停追加“无法进行增量更新”及更新哪一边 |
| 缓存索引的头部版本 ≠ 当前组件版本 / 格式不是 2 | 重建（有旧索引时重建期间继续使用它） | — | 状态栏显示构建进度 |

不会出现“用不兼容的协议硬跑”：协议对不上的进程不会收到 `initialize`，收到不在区间内的 `initialize` 的进程回复错误 -32001。

## 组件包与安装约定（#43，发布流水线 #76 照此执行）

**包。** `scripts/components/package-indexer.ps1` 产出 `rove-indexer-cs-<版本>-win-x64.zip`（与 `.sha256`）。压缩包顶层目录就是版本号（如 `1.2.0.0`），里面是 `rove-indexer-cs.exe`（self-contained，内置 .NET 9 运行时，**用户不需要安装 .NET**）、运行时与 Roslyn 的 DLL、`component.json`、`THIRD_PARTY_NOTICES.txt`。`component.json` 是 `rove-indexer-cs --version` 的输出原样：`{ component, version, protocol, protocol_min, index_format, trimmed }`。流水线（#76）直接调用该脚本，并把 zip 与 sha256 作为**独立于主安装包的可选组件**发布；主安装包不含它。

**位置与发现规则**（`lifecycle/component.rs`，与 #76 的安装器约定一致）：

```text
%LOCALAPPDATA%\Rove\components\indexer-cs\<版本>\      用户级安装（默认；ROVE_COMPONENTS_DIR 可替换 components 目录）
<rove.exe 所在目录>\components\indexer-cs\<版本>\       便携版 / 随包放置，布局相同
    rove-indexer-cs.exe   component.json   …
```

1. 环境变量 `ROVE_INDEXER_CS`（开发覆盖，指向 `.exe` 或 `.dll`）：设置了就只用它，不读清单；指向的文件不存在 → 视为未安装，不回退到别处（免得悄悄用错组件）。
2. 否则两个目录下的每个版本子目录是一个候选（要有 `rove-indexer-cs.exe` 与 `component.json`），按清单判定兼容；**选版本最高的兼容候选**，版本相同时用户目录优先。
3. 有候选但都不兼容 → “不兼容”（报告其中版本最高者）；没有候选 → “未安装”。
4. 多个版本可以并存；安装新版本就是解压出一个新的版本目录，卸载就是删除目录。安装器的“可选组件”项只需把 zip 解压到 `%LOCALAPPDATA%\Rove\components\indexer-cs\`（用户级，不需要管理员权限，不写 `HKLM`）。#36 里原先的“开发路径”（`tools/.../bin`）已移除；开发时用 `ROVE_INDEXER_CS` 指向构建产物。

**获取与更新。** Rove 运行期不联网、不下载组件，组件由用户从发布页面下载并解压（说明页的“安装方法”就是这一段）。**首版不做自动更新**，这是有意的决定：自动更新需要联网与签名校验，违背“运行期不发起任何网络请求”的硬约束；版本握手保证旧 Rove 配新组件、新 Rove 配旧组件都能得到明确提示。

**与 #76 的对齐结论。** #76 尚未开工，所以这里把约定一次定死，#76 照做：安装位置与发现规则如上；包的形态是 zip（含版本目录）而不是自带安装器；`component.json` 是兼容判定的唯一依据；版本号单一来源是 `RoveIndexer.csproj`。“关于”对话框（#76）显示组件状态时调用 `rove_lang_cs::lifecycle::component::discover(&Roots::current())`：`Found` 里有版本，`Incompatible` 里有方向与组件版本。

## 发布配置：trim 与 ReadyToRun 的结论（#43）

配置写在 `RoveIndexer.csproj`（`SatelliteResourceLanguages=en`、`DebugType=none`、`JsonSerializerIsReflectionEnabledByDefault`、`TrimmerRootAssembly`、`Rove.Trimmed` 标记），发布参数由 `package-indexer.ps1` 传入：`dotnet publish -r win-x64 --self-contained -p:PublishTrimmed=true`。

**结论：默认开启 trim，不开 ReadyToRun。** trim 版不支持 `--generators`（见下），需要生成器时用 `-NoTrim` 打未 trim 的包。

体积（发布目录解压后 / zip 压缩后；机器：Windows 11，.NET SDK 9.0.100）：

| 变体 | 解压后 | zip | 与 60–90 MB 预算 |
|---|---|---|---|
| 不 trim | 83.9 MB | 36.6 MB | 在预算内 |
| 不 trim + ReadyToRun | 101.5 MB | 44.5 MB | 略超 |
| **trim（默认）** | **31.8 MB** | **13.6 MB** | 远低于 |
| trim + ReadyToRun | 54.4 MB | 24.3 MB | 低于 |

构成（trim 版）：Roslyn 的 `Microsoft.CodeAnalysis.CSharp.dll` 6.1 MB + `Microsoft.CodeAnalysis.dll` 1.8 MB，.NET 运行时的原生部分（`coreclr`、`clrjit`、GC、`CoreLib` 等）约 12 MB，其余是被剪过的框架库与 `Microsoft.DiaSymReader.Native` 2.2 MB。不 trim 时多出的部分基本是完整的 BCL。已用 `SatelliteResourceLanguages=en` 去掉 Roslyn 的 13 种界面语言资源；Roslyn 的 VB 支持本来就没有引用，没有更便宜的优化了。

**trim 验证**（两个真实测试项目，默认选项，用 `--report` 的统计逐项对照；“不 trim”是同一次构建的 self-contained 发布）：

| 项目 | 变体 | 耗时 | 峰值工作集 | 名称数 | 未解析名称 | 占比 | 符号 | 出现位置 | 关系 |
|---|---|---|---|---|---|---|---|---|---|
| 中型（7,718 文件） | 不 trim | 46.1 s | 1558 MB | 1,626,282 | 574 | 0.0353% | 320,349 | 1,925,603 | 18,027 |
| 中型 | trim | 44.5 s | 1558 MB | 1,626,282 | 574 | 0.0353% | 320,349 | 1,925,603 | 18,027 |
| 大型（21,677 文件） | 不 trim | 108.8 s | 2753 MB | 5,084,364 | 3,107 | 0.0611% | 992,875 | 6,048,654 | 68,331 |
| 大型 | trim | 92.2 s | 2765 MB | 5,084,364 | 3,107 | 0.0611% | 992,875 | 6,048,654 | 68,331 |

中型项目两种变体产出的索引**逐字节相同**；大型项目的统计逐项相同，文件字节不同（并行绑定时符号的排布顺序不固定，不是内容差异；未另做“同一变体两次运行”的字节比对）。耗时是开着 Unity 等程序的机器上的单次数字，只说明 trim 没有变慢。除全量索引外，`cargo test -p rove-lang-cs --test incremental`（常驻协议、单文件重绑、被独占文件重试、空闲修剪与退出、影响分析、覆盖层上限）用 `ROVE_INDEXER_CS=<trim 版 exe>` 全部通过。

**验证过的反射路径：** Roslyn 的符号显示与文档注释 ID、`MetadataReference` 读取（72 / 212 个程序集镜像）、`System.Text.Json` 的协议消息（`JsonNode`）与报告序列化（匿名类型 / 记录类型）、XML 文档读取、增量重绑（`ReplaceSyntaxTree` 等）。**第一次 trim 实际踩到的坑**是报告序列化：trim 后反射序列化默认关闭，索引写完后写报告时崩溃（退出码 127，索引文件已写出）；已用 `JsonSerializerIsReflectionEnabledByDefault` 与把本程序集整体设为 trim 根修复。这也是验收要求“比对统计”而不只是“能启动”的原因。

**trim 版不支持 `--generators`。** 在大型项目上开启生成器：不 trim 的版本生成 17 个文件（`generated_trees: 17`），trim 版生成 0 个。第三方生成器（netstandard2.0 程序集，如 Unity.Entities 的 `SystemGenerator` / `AspectGenerator`）运行时按名字调用被剪掉的 BCL 成员，日志里是 `MissingMethodException: Stopwatch.Stop()`、`TypeLoadException: Enumerator`；把 Roslyn 程序集设为 trim 根、改用 `TrimMode=partial` 都试过，结果仍是 0。因为 Rove 默认不开生成器（`--generators` 默认关闭，当前没有任何设置项把它传给索引进程），而静默少结果比报错更糟，所以 trim 版进程收到 `--generators` 时直接报错退出（退出码 2；`component.json` 里 `trimmed: true`）。将来要把生成器做成设置项时，那个选项必须同时要求未 trim 的组件包（83.9 MB / zip 36.6 MB，仍在预算内）。

**ReadyToRun 不采用：** 体积 +17.6 MB（不 trim）或 +22.6 MB（trim），而全量索引是分钟级任务，收益只在进程冷启动；常驻进程冷启动加第一次重绑在微型项目上约 1.4 s（`incremental` 测试输出），没有进一步压缩的必要。`-ReadyToRun` 开关保留给以后需要时使用。

## 在干净的 Windows 上验证组件（每次发版重跑）

self-contained 的唯一有效验证是在没有 .NET 的机器上跑（开发机装着 SDK 看不出问题）。用 Windows Sandbox（每次启动都是全新环境；需要在“启用或关闭 Windows 功能”里开启，家庭版没有）或干净的虚拟机：

1. 在主机上打包：`powershell -NoProfile -File scripts\components\package-indexer.ps1`，得到 `target\components\<版本>\` 与 zip。
2. 准备一个中型项目的**副本**（验收标准要求中型项目；用副本是因为冒烟脚本会往一个源文件追加一行注释）。
3. 新建 `indexer.wsb`（路径换成本机的），双击启动。**`Networking` 设为 `Disable`，同时验证“运行期不联网”**：断网下能跑完说明没有硬依赖网络；要抓包验证时把它改成 `Enable`，在主机上用 Wireshark / `pktmon` 观察沙盒的出站流量，应当没有来自 `rove-indexer-cs.exe` 的连接。

```xml
<Configuration>
  <Networking>Disable</Networking>
  <MappedFolders>
    <MappedFolder><HostFolder>D:\path\to\rove\target\components</HostFolder><SandboxFolder>C:\comp</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>D:\path\to\rove\scripts\components</HostFolder><SandboxFolder>C:\scripts</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>D:\path\to\medium-project-copy</HostFolder><SandboxFolder>C:\proj</SandboxFolder></MappedFolder>
    <MappedFolder><HostFolder>D:\path\to\out</HostFolder><SandboxFolder>C:\out</SandboxFolder></MappedFolder>
  </MappedFolders>
  <LogonCommand>
    <Command>powershell -NoProfile -ExecutionPolicy Bypass -Command "&amp; { Get-Command dotnet -ErrorAction SilentlyContinue | Out-String | Set-Content C:\out\dotnet-check.txt; &amp; C:\scripts\smoke-indexer.ps1 -Component C:\comp\1.2.0.0 -Project C:\proj } *&gt; C:\out\smoke.txt"</Command>
  </LogonCommand>
</Configuration>
```

4. 看 `out\smoke.txt`：应依次出现 `version: {…}`、`full index: …`、`hello: protocol=1 min=1 …`、`changed: files=…`、最后一行 `SMOKE OK`；`dotnet-check.txt` 应为空（确认沙盒里没有 .NET）。

`smoke-indexer.ps1` 做三件事：`--version`、全量索引、`--serve` 里读 `hello` → `initialize` → `changed`（改一个源文件）→ `shutdown`。不带 `-Project` 时它自己生成一个最小项目，适合快速冒烟。

**本次验证状态：** 本机没有启用 Windows Sandbox，也没有虚拟机，所以**干净机器与断网验证没有在 #43 的 PR 里实跑**。已经做的：在开发机上用同一个脚本对打包产物（trim 版）跑通了 `SMOKE OK`；两个真实项目的全量索引与 `incremental` 测试套件对打包产物通过；Rove 与组件之间没有任何网络代码（索引进程源码里没有 `System.Net` / `HttpClient` / `Socket`，Rust 侧的组件发现只读本地目录）。干净机器与断网验证需要有 Sandbox 的机器按上面的步骤跑一次。
