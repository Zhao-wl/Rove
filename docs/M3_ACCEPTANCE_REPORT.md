# M3 集成验收报告（issue #51）

> 本文如实记录 2026-11-14 在两个真实 Unity 项目上走完 M3 场景清单的结果。
> 写法参照 [M0_UI_SPIKE_REPORT](M0_UI_SPIKE_REPORT.md) §8、[M1_ACCEPTANCE_REPORT](M1_ACCEPTANCE_REPORT.md) 与
> [M2_ACCEPTANCE_REPORT](M2_ACCEPTANCE_REPORT.md)：**未达标的清单比好看的报告有用**。
> 本次没有做的事情一律写进 §8 的人工清单，不臆造结论、不把「应该没问题」记成通过。
>
> **一句话结论先说**：编码往返与外部修改 / 保存冲突两条硬线**达标**；
> **输入法矩阵本次一条都没有在实机上跑**（§2），所以 `PLAN.md` 给 M3 的
> 「输入法场景全部通过」**不能判为通过**；按键到上屏 p99 仍未达 8 ms。详见 §9。

## 1. 环境

| 项 | 值 |
|---|---|
| CPU | Intel Core i7-11700 @ 2.50 GHz，8 核 16 线程 |
| 内存 | 47.7 GB |
| 系统 | Windows 11 专业版 10.0.22631 |
| 显卡 / 显示 | NVIDIA GeForce GTX 1060 6GB，1920×1080 **@ 120 Hz**，缩放 **100%**，**没有混合 DPI 环境** |
| 磁盘 | C: NVMe（系统盘与 `%TEMP%`）、D: HDD（仓库与 `target/`）、E: SATA SSD（两个 Unity 测试项目） |
| 输入法 | 系统只装了**微软拼音**（`zh-Hans-CN`，TIP `{81D4E9C9-…}{FA550B04-…}`）。**没有安装搜狗 / 微信输入法** |
| 机器负载 | 验收期间本机同时开着 Unity 编辑器与另一个构建槽，**负载较高**；所有耗时只作相对参考 |
| 构建 | `rust-toolchain.toml` 的 stable MSVC；本机 release / bench 关掉了 LTO（`CARGO_PROFILE_*_LTO=off`） |

测试项目（路径只来自环境变量 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`，不写进仓库）：

| 槽位 | Unity 版本 | 本次扫描到的文本文件 | 备注 |
|---|---|---:|---|
| 中型 | 2022.3.62f2c1 | 6,000（扫描预算上限） | 规模与 M2 验收当天一致 |
| 大型 | 2022.3.62f2c1 | 6,000（扫描预算上限） | 同上 |

> 「扫描预算」是 `m3_roundtrip` 的上限：深度优先、跳过 `Library` / `Temp` / `obj` / `Logs` / `.git`，
> 每个项目最多读 6,000 个文本文件（`.cs` / `.json` / `.shader` / `.md` …）。
> §3 的「命中数」都是**这 6,000 个文件里**的计数，不是全项目计数。

**C# 语义索引（L2）本次没有重建**：本机没有安装 `rove-indexer-cs` 组件
（`ROVE_INDEXER_CS` 未设置、`%LOCALAPPDATA%\Rove\components\indexer-cs\` 为空），
所以 §6 的「单文件增量更新 < 300 ms」沿用 M2 验收当天的实测值并注明未复测，见 §8 人工事项 11。

## 2. 输入法场景矩阵：**本次一条都没有在实机上跑**

issue #51 把输入法列为「头号风险，也是最容易糊弄过去的一项」。本次验收**没有跑 `spikes/ime_test.ps1`**，
原因如实写明：

- 该脚本会**抢占前台窗口、向前台窗口注入按键、并修改当前输入法的中英文状态**。
  验收期间本机正开着 Unity 编辑器与另外两个构建槽，在别人的桌面上做这件事不安全；
- 判据里的「候选框在插入符正下方」「靠边缘时翻转不被裁切」「不留残片」都要**看截图**才算数，
  本次没有人在屏幕前核对截图。

因此下表**没有一行可以记成通过**。「已自动覆盖的部分」一栏写的是 #47 在
`crates/rove-ui/tests/code_view_edit.rs` 里**驱动 `EntityInputHandler`** 覆盖到的行为
（协议层：组合串、提交、取消、区间换算、候选框坐标），它**不包含**真实 IMM32、
真实候选窗口的位置与真实屏幕边缘翻转。

| # | 场景 | 判据 | 本次结论 | 已自动覆盖的部分（协议层） |
|---|---|---|---|---|
| 1 | 短词输入（`ni'hao`）并提交 | 组合态内嵌带下划线、候选框在插入符正下方、提交内容正确 | **未测（实机）** | `composition_is_inline_and_commits_as_one_undo_step`：组合期间缓冲区不变、无事务、无编辑事件；提交后一次 `Ctrl+Z` 撤销整串；组合串内的插入符有坐标（候选框定位用的那个坐标） |
| 2 | **≥ 20 字整句**输入，候选框全程跟随 | 不偏移（#5 修复的直接验收点） | **未测（实机）** | 无（长串的坐标跟随是真实候选窗口的行为） |
| 3 | 组合中途用鼠标点到另一行 | 提交或取消二选一，不留残片，不崩溃 | **未测（实机）** | `clicking_elsewhere_mid_composition_commits_in_place`：组合串在原位置提交、无残片、插入符落到点击处 |
| 4 | 组合中途切换输入法 | 同上 | **未测（实机）** | `cancelled_composition_leaves_nothing_and_does_not_delete_the_selection`：输入法发空组合串 + 空提交时不留痕迹、不吞选区 |
| 5 | 插入符靠近屏幕右 / 下边缘 | 候选框翻转，不被裁切 | **未测（实机）** | 无 |
| 6 | 全角 / 半角符号、Shift 临时英文 | 字符正确，不吞字 | **未测（实机）** | 无 |
| 7 | CJK 与 emoji 混排长行中部输入 | 插入位置正确，命中测试不漂移 | **未测（实机）** | #47 的 UTF-16 混排用例（emoji 前缀）与 #46 的单词导航在 CJK / emoji / ZWJ 混排行上的一致性测试 |
| 8 | 125% / 150% 缩放下重跑 1、2、5 | 候选框定位正确 | **未测（实机）** | #30 的 `code_view_dpi`（100/125/150/200%）与 `quick_input_overlay` 覆盖的是**绘制几何**，不是候选窗口 |
| 9 | 第三方输入法（搜狗 / 微信） | 条件允许时测 | **未测：本机未安装第三方输入法**（只有微软拼音） | — |

> **结论：issue #51 的「微软拼音全部通过」这一条没有达到**——不是失败，是**没测**。
> 复跑办法与安全阀见 §8 人工事项 1–3。

## 3. 编码往返矩阵（真实文件，字节级）

`cargo test -p rove-buffer --test m3_roundtrip -j 1 -- --nocapture`。做法：
真实文件的**字节复制**到临时目录（**一个字节都不往测试项目里写**）→ `TextFile::load` →
**在中间那一行的行首插入 `RoveM3`（6 B）** → 保存 → 与原字节比较。

判据有三条，全部是硬判定：

1. 没有改动时保存必须是 `NotDirty`，磁盘字节一个都不变；
2. 保存后的字节必须等于「原文本插入那 6 个字符后，按**原编码 / 原 BOM** 重新编码」的结果；
3. 公共前缀 + 公共后缀必须覆盖原文件的**全部**字节（也就是改动处之外一个字节都没动），
   且长度变化恰好等于插入的字节数。

### 3.1 中型项目（扫描 6,000 个文本文件）

| 类别 | 命中 | 选中的样本 | 样本大小 | 往返后长度变化 | 结论 |
|---|---:|---|---:|---:|---|
| UTF-8 无 BOM + LF | 3,658 | `Tools/SkillSystem/global.json` | 68 B | +6 B | 零差异 |
| UTF-8 无 BOM + CRLF | 1,887 | `README.md` | 924 B | +6 B | 零差异 |
| UTF-8 带 BOM | 232 | `AGENTS.md` | 3,589 B | +6 B | 零差异 |
| UTF-16 LE / BE | **0** | — | — | — | 项目里没有，由构造样本覆盖（§3.3） |
| 非 UTF-8（GBK 等） | **0** | — | — | — | 同上 |
| 混合换行符 | 41 | `Docs/四期/调查/2026-11-02-…清单.md` | 18,016 B | +6 B | 零差异，**保存后仍然混合** |
| 末尾无换行 | 954 | `AGENTS.md` | 3,589 B | +6 B | 零差异，**没有被补上换行** |
| 空文件 | 2 | `Packages/com.unity.shadergraph/…/MaterialGraph.uss` | 0 B | +6 B | 零差异 |
| 只有一行的大文件 | 10 | `Docs/一期/…/inputs-unity-bits.json` | 194,708 B | +6 B | 零差异 |

### 3.2 大型项目（扫描 6,000 个文本文件）

| 类别 | 命中 | 选中的样本 | 样本大小 | 往返后长度变化 | 结论 |
|---|---:|---|---:|---:|---|
| UTF-8 无 BOM + LF | 2 | `CLAUDE.md` | 426 B | +6 B | 零差异 |
| UTF-8 无 BOM + CRLF | 5,861 | `.mcp.json` | 119 B | +6 B | 零差异 |
| UTF-8 带 BOM | 81 | `Tools/…/ballpit-death-native-24-pass.xml` | 17,262 B | +6 B | 零差异 |
| **UTF-16 LE / BE** | **1** | `knowledge-base/tasks/pending/t40-r10/endless-commits.txt` | 15,148 B | **+12 B** | 零差异（真实的 UTF-16 文件，一个字符 2 字节） |
| 非 UTF-8（GBK 等） | **0** | — | — | — | 由构造样本覆盖（§3.3） |
| 混合换行符 | 6 | `knowledge-base/…/buff-aggregator-system.md` | 17,869 B | +6 B | 零差异 |
| 末尾无换行 | 240 | `.mcp.json` | 119 B | +6 B | 零差异 |
| 空文件 | 41 | `heu_session.txt` | 0 B | +6 B | 零差异 |
| 只有一行的大文件 | 3 | `Map/WorldScene_1000/A_ServerData/map_npc_refresh_point.json` | 230,289 B | +6 B | 零差异 |

### 3.3 项目里没有的编码：真实内容 + 构造编码

issue 正文说「GBK + CRLF 的 `.cs` 在 Unity 项目里很常见」。**在这两个项目里不是这样**：
扫描到的 12,000 个文本文件里**非 UTF-8 的一个也没有**（与 M1 验收当天的结论一致）。
所以 GBK 一组用**真实文件的内容**（中型项目的 `Tools/ArchivedValidation/ArtV2V5/Tests/Editor/ArtV3AnimationTests.cs`，
3,658 字符，带中文注释）换一种编码写出来再往返：内容是真的，编码是构造的。

| 构造样本 | 编码 / 换行 | 大小 | 往返后长度变化 | 结论 |
|---|---|---:|---:|---|
| `constructed_gbk_crlf.cs` | GBK + CRLF | 3,754 B | +6 B | 零差异 |
| `constructed_utf16le.cs` | UTF-16 LE + BOM + CRLF | 7,484 B | +12 B | 零差异 |
| `constructed_utf16be.cs` | UTF-16 BE + BOM + LF | 7,318 B | +12 B | 零差异 |
| `constructed_mixed_eol.cs` | UTF-8，逐行轮换 LF / CRLF / CR | 3,712 B | +6 B | 零差异，**三种换行符原样保留** |
| `constructed_no_final_newline.cs` | UTF-8，末尾无换行 | 3,683 B | +6 B | 零差异 |
| `constructed_empty.cs` | 空文件 | 0 B | +6 B | 零差异 |
| `constructed_one_big_line.json` | 压成一行，≥ 64 KiB | 69,996 B | +6 B | 零差异 |

### 3.4 GBK 文件里输入 emoji：三个选项各走一遍

`m3_gbk_file_with_emoji_walks_all_three_choices`，内容同样来自那个真实 `.cs`：

| 选项 | 行为 | 磁盘 | 结论 |
|---|---|---|---|
| **取消**（默认 `UnencodableChoice::Ask`） | 返回 `NeedsEncodingDecision { count: 1, encoding: Gbk }`（界面弹模态框） | **一个字节都没写** | 与 `UX_SPEC` §3.5 一致；未保存修改保留 |
| **仍然保存（替换为 `?`）** | `Saved { encoding_changed: false }` | 仍是 GBK，emoji 变成 `?` | 一致 |
| **改用 UTF-8 保存** | `Saved { encoding_changed: true }`，文档编码变成 UTF-8 | emoji 原样保留 | 一致 |

> 界面一侧的同样三条分支由 #49 自己的入口测试覆盖
> （`crates/rove-ui/src/save/tests.rs::unencodable_modal_offers_three_choices`，参数化三个按钮）。

### 3.5 界面一侧的复核

`m3_real_project_file_edits_and_saves_byte_identically`：把中型项目的
`Tools/Performance/ContinuousBall/CurrentExecutionCostTests.cs`（4,984 B）复制进临时工作区，
在**真实 GPUI 窗口**里打开 → 中部行首敲 `//Rove` → `Ctrl+S`，
磁盘字节差异恰好 +6 B，其余字节逐字节相同。**通过**。

## 4. 外部修改风暴与保存冲突

| 场景 | 判据 | 结果 | 验证方式 |
|---|---|---|---|
| 外部连续改写同一文件 **100 次**（有本地修改） | 最终内容与磁盘一致；提示条不堆积 | **通过**：100 次里 `NeedsDecision { first: true }` 恰好 **1 次**；「重新加载」后内容与磁盘逐字符相同 | 本次自动（`m3_real_file_external_storm_and_consecutive_saves`，真实文件副本） |
| 同一件事在**窗口**里 | 提示条 1 条、通知 0 条 | **通过** | 本次自动（`m3_external_change_storm_shows_one_bar_and_no_notifications`，100 次 `FileEvent::Changed`） |
| 有本地修改时外部被改：两个选项各走一遍 | 「重新加载」丢弃本地改动、「保留本地修改」之后保存不再报冲突 | **通过** | 既有自动（`save.rs::dirty_document_asks_once_and_both_choices_work`、`controller.rs::external_change_bar_does_not_stack_and_clean_files_reload_silently`） |
| 无本地修改时外部被改 | 静默重载，滚动位置与光标不变 | **通过** | 既有自动（`edit_tests::edit_save_external_change_reload_through_real_window`：重载后滚动位置差 < 1e-6、文档身份不变、提示条 0 条） |
| 保存冲突 | 通知按钮「比较」隐藏，「覆盖」「放弃本地修改」可用 | **通过** | 既有自动（`save/tests.rs::conflict_notification_hides_compare_and_both_buttons_work`）；核心层在真实文件上复核（`m3_real_file_failure_paths`：`Conflict` → `overwrite_conflict` 后写入成功，本地修改不丢） |
| **被占用**（模拟 Unity 导入期间保存） | 自动重试一次后报告；释放后重试成功 | **通过（模拟）**：同进程持有 `share_mode(0)` 独占句柄 → `Busy { attempts: 2 }`；释放后 `Saved` | 本次自动（`m3_real_file_failure_paths`）。**真实 Unity 导入期间保存没有做** → §8 人工事项 5 |
| 只读属性文件 | 报告 `ReadOnly`；「覆盖」清除属性后写入；**没有任何管理员提权入口** | **通过** | 本次自动（`m3_real_file_failure_paths`）+ 窗口一侧断言通知按钮里不含 `elevat` / `admin`（`m3_read_only_documents_reject_the_save_entry_point`） |
| 文件被外部删除 | 标签标记「已删除」，再保存等同新建 | **通过**：首次 `Deleted { first: true }`、重复报告 `first: false`（风暴不重弹），内容保留，保存把文件重新建出来 | 本次自动 + 既有自动（`edit_tests` 的「已删除」徽标） |
| 连续保存 **20 次**，自己的写入零误报 | 一次都不被当成外部修改 | **通过**：`SelfWrites::is_own` 20/20 命中；即使绕过去重器，`TextFile::disk_changed` 20 次全是 `Unchanged` | 本次自动（真实文件副本） |

## 5. 编辑 → 高亮 / 大纲 / 面包屑 / 索引的联动

`cargo test -p rove-ui m3_ -j 1`，核心是
`m3_edit_then_save_updates_syntax_outline_breadcrumb_and_precision`，逐阶段断言：

| 阶段 | 语法失效区间 | 大纲 / 面包屑 | 查询精确度 | 状态栏 | 通知 |
|---|---|---|---|---|---|
| 打开 | 整层（首次解析） | `Player` / `Update()` | 精确 | — | 0 |
| 改**方法体** | **只有受影响的行**（≤ 4 行） | **不变** | 改动所在行→近似；其余行经位置映射仍→**精确** | — | 0 |
| 改**方法签名**（`Update` → `Tick`） | 同上 | **跟着变**（大纲里 `Update` 消失、`Tick` 出现；面包屑末段变成 `Tick()`） | 近似 | — | 0 |
| 保存后**立刻**查询 | — | — | **仍是近似**（索引还没重建，**这是正确行为**） | 「已保存 Game.cs」（#49 的保存回执） | 0 |
| 索引单文件重绑完成 | — | — | **变成精确** | **不弹任何提示** | 0（同名高亮没有被清空） |

两点要写清楚：

1. **「保存前结果标为近似」的准确口径**。issue 正文那句是简写；真正的口径在
   [LANGUAGE_INTELLIGENCE](LANGUAGE_INTELLIGENCE.md) §3.2：
   「映射成功且该符号所在区域未被修改 → 查 L2（精确）；否则 → L0/L1 回退，标记为近似」。
   也就是**只有被改动的那一行降级**，其余位置经 #38 的位置映射仍然是精确的。
   测试把两侧都钉住了，免得以后有人把任一侧改坏。
2. **「不闪烁」在自动测试里的可检验口径**是：编辑后语法快照**不回退成空**（旧 token 继续画），
   且失效区间不是整层。真正的视觉闪烁要看录屏 → §8 人工事项 6。

另有 `m3_incremental_highlight_matches_a_fresh_parse`：在方法体里插入一整条语句后，
把 #94 增量重解析的逐行记号与**同一份文本从头打开一次**的记号逐条比较，**完全一致**。
这是「高亮正确」在自动测试里唯一能钉死的口径。

### 5.1 本次发现并修复的接缝缺陷

**保存后查询永远变不回精确（两个「磁盘内容哈希」不是同一个算法）。**

- 现象：在窗口里编辑一个 `.cs` 并 `Ctrl+S`，即使 C# 索引随后完成了单文件重绑，
  该文件的导航结果**一直停在近似**，直到把标签关掉重开。
- 成因：`rove-ui` 的保存装配把 `Effect::Saved` 里的 `hash` 直接交给了
  `BufferStates::saved`。但这两个哈希不是同一个算法：
  - `SaveOutcome::Saved { hash }` 是 #49 的 `DiskStamp` 哈希（**FNV-1a**，`rove_buffer::save::content_hash`），
    它的用途是**自写去重**（`SelfWrites`），不能换算法；
  - `BufferStates` 比的是 **C# 索引文件表的 xxHash64**（`rove_lang_cs::router::hash_bytes`），
    打开与重新载入那两条路径（`nav_host::apply_buffer_change`）用的正是它。
  两者永不相等 ⇒ `CsRouter::file_state` 在索引重建后判为 `Diverged` ⇒ **整份文件降级为近似**，
  而且 `rebase` 永远不会发生。
- 修复：`crates/rove-ui/src/skeleton/workbench/edit_host.rs` 的 `apply_save_effects`——
  保存成功后在**后台**把刚写出的字节读回来算一次 `hash_bytes`，并用 FNV 哈希确认读到的
  仍然是我们写的那一份（不是就跳过，交给文件监听走外部修改的路径），再调用
  `BufferStates::saved`。登记口径与打开 / 重新载入完全一致。
- 为什么以前没发现：#35 的 `incr_check` 测的是索引进程与 `rove-lang-cs` 这一层，
  根本不经过窗口的保存装配；#38 / #49 各自的入口测试也只看自己那一层
  （`edit_tests` 只断言 `current_hash.is_some()`，不比较它**等于什么**）。
  只有把「窗口里保存」和「索引换代后再查询」接在一起才会暴露——这正是集成验收该做的事。
- 回归保护：`m3_edit_then_save_updates_syntax_outline_breadcrumb_and_precision` 的最后一段
  （修复前这条断言得到 `Approximate`，修复后得到 `Exact`）。

## 6. 只读强制

| 入口 | `Library/PackageCache/**` | 超大文件（≥ 64 MB，测试里把 `rove.largeFile.threshold` 调小走同一条路径） | 验证方式 |
|---|---|---|---|
| **输入** | 不产生修改，光标处提示带原因（「Unity 包缓存目录，修改会被覆盖。」）并说明可以「在会话中切换只读模式」 | 不产生修改，提示写「超大文件」且**不提**切换只读 | 既有自动（`workbench_readonly_tests::readonly_package_cache_shows_lock_hint_and_toggles` / `readonly_toggle_does_not_apply_to_huge_files`） |
| **替换** | `Ctrl+H` 仍打开查找条，但**不显示替换行**、`can_replace() == false`、执行替换命令后文本不变 | 查找是流式的，**没有替换** | 既有自动（`replace_tests::read_only_documents_have_no_replace_entry_point`、`large_file_tests::streaming_find_on_a_huge_file_has_no_replace`） |
| **保存（本次补的那一条）** | `Ctrl+S` **一个字节都不写**，文件修改时间不变 | 同左 | **本次自动**（`m3_read_only_documents_reject_the_save_entry_point`） |
| 锁形图标 | 标签与文件树都显示 | 显示 | 既有自动 |
| 「在此会话中切换只读」 | **有效**：切换后可以输入、可以保存（测试里切换后保存确实写了盘） | **无效**：切换后仍然只读 | 既有自动 + 本次自动 |
| 管理员提权入口 | **不存在**：只读保存的通知按钮只有「另存为」，按钮 id 里不含 `elevat` / `admin` | 同左 | **本次自动** |

> 只读文档上的 `Ctrl+S` 会给一条**带「另存为」的通知**（#49 的 `SaveOutcome::DocumentReadOnly`）。
> 这是 #49 的既有设计，`UX_SPEC` §6.4 没有单独规定这一条；本次把「**不写盘**」与
> 「**按钮里没有提权入口**」两件事钉成了测试，没有改动它的文案或级别。

## 7. 性能复核

口径：本机**负载较高**（同时开着 Unity 与另一个构建槽），绝对值只作相对参考；
空闲机器复测记在 §8 人工事项 10。本 PR **没有改动任何渲染或编辑的热路径**
（改的是保存后交给 #38 的那个哈希），所以不做「优化前后」对比，只做一次实测与预算对照。

### 7.1 按键到上屏：**未达标**

`cargo bench -p xtask --bench typing`（10 万行 C#、插入符在第 5 万行、带 tree-sitter 着色与 Dark 2026 主题，
进程内经 `Window::dispatch_keystroke` 注入，**采样 400 次**、预热 30 次）：

| 指标 | 均值 | p50 | **p99** | max | 预算 |
|---|---:|---:|---:|---:|---|
| 按键 → 含新字符的帧绘制完成 | 8.515 ms | 8.344 ms | **12.421 ms** | 45.354 ms | **< 8 ms（p99）→ 未达到** |
| 其中**按键处理本身**（事务 + 视图增量更新 + 事件） | 0.354 ms | 0.348 ms | 0.556 ms | 0.668 ms | — |
| 这些帧的代码视图绘制 CPU | 0.357 ms | 0.313 ms | 0.750 ms | 0.842 ms | #19 的 1 ms / 帧 ✅ |

- 与 #47 当时（PR #138）的记录对比：p99 10.80 ms → 本次 **12.421 ms**。
  本次机器负载更高（另一个槽在编译），**两次都没有达到 8 ms**。
- 口径解读（和 #47 的结论一致）：**Rove 自己的处理只有 0.35 ms（p99 0.56 ms）**，
  端到端时间基本等于一个刷新间隔——本机 **120 Hz，一帧 8.33 ms**，注入点落在刚绘制完一帧之后
  （最坏相位），所以 p50 ≈ 1 帧、p99 ≈ 1.5 帧。按这个口径，8 ms 的 p99 在
  「按键 → 帧绘制完成」的定义下**结构上不可能达到**，除非改输入采样相位或改测量口径。
- **没有为了让数字好看去改 `perf/budgets.toml`**（该条目仍是 `status = "recorded"`，数值未动）。
  已开 **#215** 跟进（里程碑「v0.1 首发」）。

### 7.2 其余四项

| 项 | 口径 | 实测 | 预算 / 对照 | 结论 |
|---|---|---:|---|---|
| 输入过程中每帧 CPU | `typing/frame_cpu`（见上表） | 均值 0.357 ms / p99 0.750 ms | #19 的 1 ms / 帧 | **未退化** |
| 滚动每帧 CPU（复跑 #19 / #20 的基准） | `scroll/hl_hit`（生成 C#，300 帧 × 3 行，着色命中） | 均值 **0.410 ms** / p99 0.868 ms | 预算 1 ms；#20 当时记录均值 0.34 ms | **未退化**（差值在本机负载的波动范围内） |
| | `scroll/find_hit`（small.txt + 查找条） | 均值 0.600 ms / p99 1.175 ms | 预算 1 ms（均值口径） | 均值达标 |
| | `scroll/wrap_hit`（small.txt + 软换行） | 均值 0.621 ms / p99 1.292 ms | 预算 1 ms（均值口径） | 均值达标 |
| | `scroll/hl_miss`（每行唯一，400 帧 × 37 行） | 帧间隔 p99 **10.695 ms** | `recorded`：1.5 × 8.33 ≈ 12.5 ms | 达标 |
| 10 万行文件中替换 **5,000 处** | `replace_tests::replace_all_rewrites_five_thousand_matches_in_one_undo_unit`（含重新搜索与界面刷新，**一次撤销全部回退**） | **release 53.9 ms**（debug 563.5 ms） | 无预算条目 | 记录。注意该测试打印的「debug 构建」字样是 #48 写死的文案，本行的 53.9 ms 来自 `--release` 运行 |
| 保存 1 MB / 10 MB | `m3_save_timing_for_one_and_ten_megabytes`（变换 + 编码 + 原子写入，含 `FlushFileBuffers`，5 次取中位数，临时目录在 NVMe） | **1 MB 77.0 ms** / **10 MB 289.1 ms**（release；debug 92.4 / 297.4 ms） | 无预算条目 | 记录。两个构建差别很小 ⇒ 瓶颈是**落盘与刷盘**，不是编码 |
| 单文件索引增量更新 | #35 的 `incr_check --latency` | **本次未复测**（本机没装索引组件） | < 300 ms | 沿用 M2 验收当天的数字：对查询可见 **p50 46 ms / p99 82 ms**（中型项目，稳态）；首次保存含预热 3,842 ms 超预算。见 §8 人工事项 11 |

## 8. 人工事项清单（本次没有做，不算通过）

| # | 人工事项 | 对应场景 | 为什么自动化不了 |
|---|---|---|---|
| 1 | **输入法矩阵第 1–8 项的实机复跑 + 逐项截图**：用 `spikes/ime_test.ps1` 的安全阀（注入按键前校验 `GetForegroundWindow()`，不相等立即中止；结束时还原输入法模式），在一个**真实 Unity 项目的 2000 行 `.cs`** 里滚到中部、靠屏幕边缘整句输入 | §2 全部 | 要抢占前台、向前台注入按键、改系统输入法状态；判据（候选框位置、边缘翻转、无残片）必须看截图 |
| 2 | **第三方输入法（搜狗 / 微信）** | §2 第 9 项 | 本机只装了微软拼音；装完要重跑 1–8 项 |
| 3 | **125% / 150% 缩放下重跑输入法 1、2、5 项** | §2 第 8 项 | 同 1，另需改系统缩放并注销重登 |
| 4 | **真正的双显示器混合 DPI** 下看候选框与浮层 | §2、#30 | 本机两台显示器都是 100%，没有混合 DPI 环境（见 [DPI_CHECKLIST](DPI_CHECKLIST.md)） |
| 5 | **真实 Unity 导入期间保存**（改一个 `.cs` 让 Unity 重新编译，在导入窗口期按 `Ctrl+S`） | §4 | 本次用同进程独占句柄**模拟**占用；真实 Unity 的占用时序、句柄种类可能不同 |
| 6 | **编辑后高亮是否有视觉闪烁、行高是否跳动**：录屏或连续截图 | §5 | 事件序列与失效区间已断言，「有没有一帧白闪」只能看 |
| 7 | **近似 → 精确切换时是否重弹提示 / 闪烁高亮**的录屏确认 | §5 | 同上。状态栏、通知、高亮条数三项已自动断言 |
| 8 | **真正 ≥ 64 MB 的超大文件**上确认替换入口与编辑入口全部不存在 | §6 | 本次把 `rove.largeFile.threshold` 调小走同一条代码路径；真实 64 MB 文件的首屏与索引行为另见 #8 的测试 |
| 9 | **热退出与崩溃恢复（#50）在真实场景下的复核**：强杀进程后重启、确认未保存修改恢复 | — | 本 issue 的场景清单没有列，#50 自己有入口测试；发版前按 `RELEASE.md` 的干净机器清单重跑 |
| 10 | **空闲机器上复测全部性能数字** | §7 | 本次全程机器负载较高；按键到上屏与滚动帧的数字尤其需要在空闲机器上确认 |
| 11 | **装上 `rove-indexer-cs` 组件后复跑 `incr_check --latency`**，并在**大型项目**上补一轮 | §7 最后一行 | 需要 `dotnet build tools/rove-indexer-cs -c Release` + 建索引（大型项目约 2–3 分钟、峰值数 GB 内存） |
| 12 | **保存后索引重绑的端到端复核（修复后的真机确认）**：在真实项目里编辑 → 保存 → 等索引重绑 → `F12`，确认 300 ms 内由「近似」变「精确」 | §5.1 | 本次用合成索引在窗口里断言了状态机；真实索引进程的重绑时序没有在修复后跑过 |

## 9. 对 `PLAN.md` M3 完成标准的结论

| 完成标准 | 结论 | 依据 |
|---|---|---|
| **编码往返零差异** | **达到** | §3：两个真实项目逐类别字节级往返全部零差异（含真实 UTF-16、真实混合换行、末尾无换行、空文件、单行大文件）；GBK / UTF-16 用真实内容 + 构造编码补齐；无法编码字符的三条分支与 `UX_SPEC` §3.5 一致 |
| **输入法场景全部通过** | **不能判为通过**：本次**一条都没有实机跑** | §2。这是 M3 的头号风险项，必须由人补上（§8 人工事项 1–4）后才能结案 |
| **能放心地做局部修改** | **接近，但有两处缺口** | 支持的一面：编码往返零差异、外部修改 100 次不堆积提示、自己保存 20 次零误报、四条失败路径逐条验证且**界面上不存在管理员提权入口**、只读强制的三个入口全部关死、编辑后高亮 / 大纲 / 面包屑正确更新。缺口：① 输入法未实机验收；② 按键到上屏 p99 12.4 ms 未达 8 ms（口径问题，自身处理只有 0.35 ms，见 §7.1 与 #215） |

**本次修复的接缝缺陷**：1 条（§5.1，保存后索引永远变不回精确）。

**本次开出的后续 issue**：

| issue | 标题 | 里程碑 |
|---|---|---|
| #215 | [#51 跟进] 按键到上屏 p99 未达 8 ms：测量口径与输入采样相位 | v0.1 首发 |
| #216 | [#51 跟进] 索引换代后 `BufferStates` 的 rebase 只在查询时发生，换代后立即继续输入会让整份文件降级为近似 | v0.1 首发 |

## 10. 本 issue 新增的测试

| 位置 | 跑法 | 内容 |
|---|---|---|
| `crates/rove-buffer/tests/m3_roundtrip.rs` | `cargo test -p rove-buffer --test m3_roundtrip -j 1 -- --nocapture` | 真实 Unity 项目文件的编码往返矩阵（扫描 + 分类 + 字节级比较）、GBK emoji 的三条分支、外部修改风暴 100 次 / 连续保存 20 次、四条失败路径、1 MB / 10 MB 保存耗时。**环境变量缺失时跳过**（CI 上整组跳过，不失败） |
| `crates/rove-ui/src/skeleton/workbench/m3_acceptance_tests.rs` | `cargo test -p rove-ui m3_ -j 1 -- --nocapture` | 编辑 → 高亮 / 大纲 / 面包屑 / 索引精确度的链路、增量重解析 ≡ 全量解析、只读文档的保存入口、窗口里的外部修改风暴、真实项目文件在窗口里的字节级往返（最后一条需要 `ROVE_TEST_UNITY_MEDIUM`，缺失时跳过） |

合成的那几个测试照常进 CI；需要真实项目的部分缺环境变量时跳过并把原因打到 stderr。
