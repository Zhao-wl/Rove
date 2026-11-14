# M2 集成验收报告（issue #44）

> 本文如实记录 2026-11-14 在两个真实 Unity 项目上走完 M2 场景清单的结果。
> 写法参照 [M0_UI_SPIKE_REPORT](M0_UI_SPIKE_REPORT.md) §8 与 [M1_ACCEPTANCE_REPORT](M1_ACCEPTANCE_REPORT.md)：
> **未达标的清单比好看的报告有用**。不能自动化、也没有在本次真机上做的事情，一律写进 §7 的人工清单，不臆造结论。

## 1. 环境

| 项 | 值 |
|---|---|
| CPU | Intel Core i7-11700 @ 2.50 GHz，8 核 16 线程 |
| 内存 | 47.7 GB |
| 系统 | Windows 11 专业版 10.0.22631 |
| .NET | 9.0.100；索引进程 `rove-indexer-cs` **1.2.0.0**，索引格式 RVCSIDX **v2** |
| 磁盘 | C: = KXG50ZNV256G **NVMe**（系统盘）、D: = ST2000DM001 **HDD**（仓库与 `target/`）、E: = Samsung 870 EVO **SATA SSD**（两个 Unity 测试项目） |
| 显示 | 两台 1920×1080，缩放 100%，**没有混合 DPI 环境** |
| 机器负载 | 验收期间本机同时开着 Unity 编辑器与另一个构建槽，**负载较高**；所有耗时只作相对参考，空闲机器复测见 §7 |
| 构建 | `rust-toolchain.toml` 的 stable MSVC；本机 release 关掉了 LTO（`CARGO_PROFILE_RELEASE_LTO=off`） |

测试项目（路径只来自环境变量 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`，不写进仓库）：

| 槽位 | Unity | csproj | L2 索引的文件数 | 符号数 | 工作区可见的 `.cs` | 索引体积 |
|---|---|---:|---:|---:|---:|---:|
| 中型 | 2022.3.62f2c1 | 91 | 7,718 | 320,386 | 10,305 | 76,580,040 B（73.0 MiB） |
| 大型 | 2022.3.62f2c1 | 242 | 21,908 | 1,014,054 | 13,742 | 202,114,552 B（192.8 MiB） |

> 两个「文件数」口径不同：L2 按 csproj 的源码条目（含 `Library/PackageCache` 下被引用的包源码），
> 工作区按 Rove 的默认排除规则看到的 `.cs`。两个数都对。
> 大型项目比 #33 当时（21,221 个文件）多了 687 个文件：测试项目在持续开发中。

**测量时的项目状态**：两个项目在本次验收期间都没有在 Unity 里触发导入；
索引是当天用 `cargo run --release -p rove-lang-cs --example nav_check -- --build` 现建的，
`nav_check` 报告「用例所在文件比索引新」的条目为 0。

## 2. 导航准确率测试集

`cargo run --release -p rove-lang-cs --example nav_check -- --build`（精确层）与 `-- --source l1`（近似层）。
完整两份基线已更新进仓库：[`tests/nav-cases/baseline-l2.md`](../tests/nav-cases/baseline-l2.md)、
[`tests/nav-cases/baseline-l1.md`](../tests/nav-cases/baseline-l1.md)（含机器可读的 `.json`）。

### 2.1 精确层（L2）：174 / 200 = 87.0%，**没有达到 100%**

两个项目完全对称，各 87 / 100。过期用例 **0 个**（锚点全部可定位，没有用「过期」掩盖失败）。

| 分类 | medium | large | 结论 |
|---|---:|---:|---|
| var 推断 (`var`) | 6/6 | 6/6 | 100% |
| 链式调用 (`chain`) | 6/6 | 6/6 | 100% |
| 方法重载 (`overload`) | 6/6 | 6/6 | 100% |
| 扩展方法 (`extension`) | 7/7 | 7/7 | 100% |
| 泛型（含 `GetComponent<T>`）(`generic`) | 7/7 | 7/7 | 100% |
| 继承与重写 (`inheritance`) | 6/6 | 6/6 | 100% |
| 接口实现 (`interface`) | 6/6 | 6/6 | 100% |
| partial (`partial`) | 6/6 | 6/6 | 100% |
| Lambda / LINQ (`lambda`) | 6/6 | 6/6 | 100% |
| **条件编译 (`conditional`)** | **3/6** | **3/6** | 生效一侧 6/6；**不生效一侧 0/6**，见下 |
| **外部符号 (`external`)** | **6/6** | **6/6** | #39 当天是 0/12；#34 的元数据视图合并后全部通过 |
| 局部变量与参数 (`local`) | 6/6 | 6/6 | 100% |
| 事件 / 委托 (`event`) | 6/6 | 6/6 | 100% |
| **运算符重载 (`operator`)** | **0/5** | **0/5** | 索引里没有出现位置 → **#212** |
| **索引器 (`indexer`)** | **0/5** | **0/5** | 同上 → **#212** |
| 类型引用（#37 验收）(`type_ref`) | 10/10 | 10/10 | 100% |
| **合计** | **87/100** | **87/100** | **87.0%** |

26 个未通过的用例，逐条结论归成两类（没有「用例本身写错」的）：

| 原因 | 用例数 | 是索引缺陷还是路由缺陷 | 结论 |
|---|---:|---|---|
| `a + b` 的运算符记号、`x[i]` 的元素访问、`operator +` / `this[]` 的声明标识符**都不产生出现位置** | 20 | **索引进程缺陷**（收集范围只有 `SimpleNameSyntax` 与声明标识符） | 已开 **#212**（里程碑 v0.1 首发）。跳转返回空、引用返回 0 处 |
| `#if` 不生效一侧没有语义信息 | 6 | **都不是缺陷** | 不生效代码不参与编译，Roslyn 不可能给出精确结果。#38 的路由对这一侧返回 `Fallback`（`INACTIVE_REASON`）并标为**近似**，由 L0/L1 兜底，符合 `LANGUAGE_INTELLIGENCE` §3.2 与 `UX_SPEC` §2.2。生效一侧 6/6 通过 |

> 口径说明：这 6 个不生效一侧的用例，期望值写的是「读源码时认为应该去的地方」，是给**近似层**的目标。
> `nav_check` 目前对 `--source l2` 也按同一个期望判定，所以它们在精确层必然记为失败。
> 这不是 bug，但表格会一直是 194 的上限；`README.md` 里提到的「改为断言 `precision = approximate`」
> 还没有做（运行器的 `expect.precision` 字段已经支持），留给 #212 一并处理或单独整理。

**结论：issue #44 的「精确层 100%」这一条没有达到。** 实测 87.0%，缺口全部有明确归因，
其中 20 例由 #212 负责，6 例属于设计边界。

### 2.2 近似层（L0/L1）：32 / 200 = 16.0%（基线数字，**预期如此**）

| 分类 | medium | large |
|---|---:|---:|
| 类型引用 (`type_ref`) | 10/10 | 10/10 |
| partial (`partial`) | 2/6 | 4/6 |
| 泛型 (`generic`) | 0/7 | 3/7 |
| 扩展方法 (`extension`) | 1/7 | 0/7 |
| 条件编译 (`conditional`) | 1/6 | 0/6 |
| 事件 / 委托 (`event`) | 0/6 | 1/6 |
| 其余 10 类 | 0 | 0 |
| **合计** | **14/100** | **18/100** |

这 200 个用例是按「**按名称解析一定会错**」挑的（`tests/nav-cases/README.md` 的分类表要求同名符号在项目中不唯一），
所以 L1 这种「只看声明、不做类型推断」的索引得分低是**设计使然**，不是缺陷。它的职责是
**索引构建期间先给出结果**，并由界面标明「近似」。L1 自己的验收口径 `type_ref` 是 20/20。

把这张表当成「索引构建期间体验如何」的基线：**按 `F12` 总能立即得到结果**，类型名一定对，
成员名可能落到同名的另一个符号，界面始终标明是近似结果（§3 的切换链路已固化成测试）。

L1 构建本身：中型 10,305 个 `.cs` / 132,839 个符号 / **1,532 ms**；大型 13,742 个 `.cs` / 276,760 个符号 / **2,593 ms**。

### 2.3 索引质量的旁证

| 项目 | 未解析名字 | 未解析率 |
|---|---:|---:|
| 中型 | 574 | **0.035%** |
| 大型 | 3,161 | **0.060%** |

大型项目有 2 个程序集的元数据镜像因为项目本身的编译错误失败（`CS8773` 的 C# 版本问题、
`CS0234` 的缺失命名空间），索引进程按设计降级为 fallback 引用，没有中断。这是**项目自身**的编译问题，
不是 Rove 的缺陷，但说明元数据视图在这两个程序集上只有签名、没有文档注释。

## 3. 场景清单逐条结果

验证方式一栏：

- **本次自动**＝本 issue 新增的接缝测试
  （`crates/rove-ui/src/skeleton/workbench/m2_acceptance_tests.rs`，`cargo test -p rove-ui m2_ -j 1`）；
- **本次实测**＝本次在真实项目上跑出来的数字（`nav_check` / 索引进程本身 / `index_prefetch_bench` / `incr_check` / `cargo xtask bench`）；
- **既有自动**＝对应 issue 自己的入口测试已经覆盖，本次只复核它仍然通过；
- **人工**＝只能靠人看或靠手工破坏环境的部分，全部列进 §7，**没有一条按「应该没问题」记成通过**。

| # | 场景 | 中型 | 大型 | 结论与验证方式 |
|---|---|---|---|---|
| 1 | 冷启动预读（HDD 与 SSD / NVMe 计时） | 通过 | 通过 | **本次实测**：`index_prefetch_bench` 在三种盘上各跑一遍，补齐了 `M0_LANG_SPIKE_REPORT` §2 标注的 M2 待测项。数字见 §4.1 |
| 2 | 预读完成前 `F12` / `Shift+F12`，近似 → 精确 | 通过 | 通过（同一套代码） | **本次自动**：`m2_approximate_to_exact_switches_silently` 断言「构建中提示一次 → 同符号不重复 → 索引就绪**没有按键**也静默补上语义高亮 → 就绪后无任何标记」，全程通知数为 0、标签数不变；`m2_precision_flag_survives_router_and_peek` 把查询层的 `precision` 与速览标题、结果数逐层对上。**界面闪烁**要靠肉眼 → §7 |
| 3 | 首次全量构建（删缓存后） | 通过 | 通过 | **本次实测**：两个项目的索引都是当天从零重建的，中型 **42.3 s**、大型 **82.1 s**（预算 < 90 s）。状态栏 `$(sync~spin) C# 索引 37%` 的形态与「全程无通知」由**既有自动**（`workbench_index_tests::building_state`）覆盖 |
| 4 | 保存一个文件后的增量更新耗时 | 通过 | 通过 | **本次实测**：`incr_check --latency`，数字见 §4.3。「增量更新中」状态栏不显示百分比由**既有自动**（`workbench_index_tests::updating_state`）覆盖 |
| 5 | 改公开方法签名后查引用 | 部分 | 部分 | **既有自动**：`incr_check` 的随机编辑里包含「改签名」，一致性比较通过（#35）；`router.rs::open_query_edit_query_save_rebuild_query` 覆盖「修改中近似 → 重建后精确且引用数正确」。**真机上看后台队列逐步重建**的过程 → §7 |
| 6 | 未保存修改：上方插入行后查定义 | 通过 | 通过（同一套代码） | **本次自动**：`m2_unsaved_edit_falls_back_to_approximate_and_requests_a_rebuild`（被修改行回退为近似、原因是 `ModifiedRegion`、界面标出来、保存后才请求重建）+ **既有自动** `peek_uses_the_nav_router_and_maps_unsaved_edits`（未修改行精确平移）。发现一处文档与实现不一致 → **#213**，见 §6 |
| 7 | 索引进程崩溃 / 组件未安装 | 通过（组件未安装）/ 人工（崩溃） | 同左 | **本次自动**：`m2_component_missing_degrades_but_navigation_still_works`（状态栏 `$(info) C# 基础导航` + 悬停提示 + **不弹通知** + L1 跨文件跳转仍然可用且标为近似）。崩溃后「自动重试一次 + 只弹一次通知」由**既有自动**（`lifecycle::indexer_missing_failing_or_crashing_degrades_after_one_retry`、`workbench_index_tests::failed_state_notifies_once_per_reason`）覆盖；**手工结束真实进程** → §7 |
| 8 | 版本握手不匹配 | 通过（自动口径） | 同左 | **既有自动**：`component.rs::incompatible_component_degrades_without_retrying`（提示更新组件、降级、**不反复重试**）、`a_compatible_version_is_preferred_over_an_incompatible_newer_one`、`hello_with_disjoint_protocol_range_is_rejected_...`。**真的放一个旧版本组件包** → §7 |
| 9 | `F12` / `Ctrl+点击` / `Ctrl+悬停` / 悬停卡片 / `Alt+←→` / `Ctrl+K Ctrl+Q` | 通过（自动口径） | 同左 | **既有自动**：`nav_tests` 的 12 个测试（真实 GPUI 窗口、键位与鼠标驱动，含跨文件与跨分屏历史）。**真机手感与排版** → §7 |
| 10 | 语义同名高亮（`Init`/`Update`/`Dispose`/`Get`/`Value`） | 通过（自动口径） | 同左 | **既有自动**：`nav_tests::semantic_occurrences_count_only_the_same_symbol` 就是按 `M0_LANG_SPIKE_REPORT` §3 的五个高噪声名称构造的，断言高亮数等于语义引用数而不是按名称命中数。**真实项目上这五个名字的噪声比本次没有单独取数**（M0 §3 已有结论），真机抽查 → §7 |
| 11 | 最重符号上的速览 | 见 §4.4 | 见 §4.4 | **本次实测**：`peek/open_to_first_paint_28k`（预算 100 ms）。`> 100 条只展开第一个文件`、`F4` / `Shift+F4` 遍历由**既有自动**（`peek_tests::peek_expansion_rule_100_expands_all_and_101_only_the_first_file`、`peek_f4_navigates_...`）覆盖。**滚动手感** → §7 |
| 12 | 速览键盘全套 + `editor.stablePeek` / `editor.peekWidgetDefaultFocus` | 通过（自动口径） | 同左 | **既有自动**：`peek_tests` 的 17 个测试逐条对应 `KEYBINDINGS.md` 的导航一节 |
| 13 | 元数据视图三跳 | 通过（部分） | 通过（部分） | **本次实测**：`external` 一类 12/12 通过，其中真实命中 `P:UnityEngine.Transform.position`（两个项目各一例）、`M:UnityEngine.GameObject.GetComponent``1`、`M:System.Math.Sqrt(System.Double)`、`T:System.Version` 等。**`List<T>.Add` 与第三方 DLL（AWSSDK / Firebase 等）的三跳没有单独抽查** → §7。标题 `X [元数据]`、锁形图标、`#region` 头、面包屑四段、标签复用由**既有自动**（`metadata_tests` 的 11 个测试）覆盖 |
| 14 | 元数据视图里 `Shift+F12` 列出工作区用法 | 未记录条数 | 未记录条数 | **既有自动**：`metadata_tests::metadata_peek_and_references_inside_a_view` 覆盖「视图内 `Shift+F12` 的结果与查询层一致」；`m2_precision_flag_survives_router_and_peek` 额外断言速览结果数 == 查询层结果数。issue 要求记录的 **`Transform.position` 实际条数**本次没有取 → §7 |
| 15 | 索引里没有的外部符号 / 未装组件时跳外部符号 | 通过（自动口径） | 同左 | **既有自动**：`metadata_tests::metadata_missing_symbol_shows_a_status_hint_and_no_empty_tab`（状态栏「未找到 `Xxx` 的元数据」、不开空标签） |
| 16 | 状态栏五种状态 + 无 C# 工作区 + 无 `.csproj` | 通过（自动口径） | 同左 | **既有自动**：`workbench_index_tests` 的 9 个测试逐条对照 `UX_SPEC` §2.1，含「未找到项目文件，正在使用推断的项目结构…」整句、点击后的详情、失败通知同因只弹一次。本次新增的 `m2_component_missing_...` 又在**导航可用性**这一侧复核了「未安装」一行 |
| 17 | 近似结果呈现的五种场景 | 通过（自动口径） | 同左 | **既有自动**：`nav_tests::approximate_results_follow_ux_spec` 逐条对应 `UX_SPEC` §2.2 的五行；本次新增的两个链路测试补上了**精确结果没有任何额外标记**与**状态切换**两侧 |
| 18 | 空状态（注释 / 字符串 / 未解析的名称） | 通过（自动口径） | 同左 | **既有自动**：`nav_tests::empty_state_hints_at_the_cursor`（「未找到 `X` 的定义」/ 近似时追加「（语义索引构建中）」）、`peek_tests::peek_empty_results_...` |
| 19 | Source Generators 复核 | 见 §5 | 见 §5 | **本次实测**，结论写在 §5：M0 的第 4 条待办结清 |

另外，issue 目标 2 的「界面手工抽查 ≥ 20 例」这一条，本次做的是**把它自动化的那一半**：
`m2_real_project_ui_matches_the_query_layer` 在真实中型项目上取 20 个真实位置，
把「界面按 `F12` 的落点」与「查询层直接回答的落点」逐点比对，**20 / 20 一致**。
它不能替代人眼看悬停签名的排版与近似标记的观感，那部分在 §7。

## 4. 性能实测与预算对照

口径：`cargo xtask bench --quick --check-budgets`（criterion 采样 20 / 测量 1 s，比默认短，
**数字只作相对参考**）+ 真实索引上的 `index_prefetch_bench` 与 `incr_check`。
**本机负载较高**（同时开着 Unity 与另一个构建槽），空闲机器复测记在 §7。

### 4.1 冷缓存下的索引预读（场景 1，补齐 M0 §2 的待测项）

`cargo run --release -p rove-lang-cs --example index_prefetch_bench -- <index> <scratch>`：
用无缓冲写把索引复制到目标盘上的新文件（页从未进过系统缓存），再测「打开索引 → 立即连续定义跳转」。

用大型项目的真实索引（**192.8 MiB**、21,908 个文件）、200 个定义跳转、每盘 2 轮：

#### 预读整个索引文件的耗时（这就是场景 1 的「后台预读」）

| 盘 | 低优先级 I/O | 普通优先级 | 吞吐 | M0 §2 的估算（300 MB 口径） |
|---|---|---|---|---|
| **HDD**（ST2000DM001） | 1,184 / 1,183 ms | 1,166 / 1,148 ms | 163–168 MB/s | 估 HDD 约 2 s |
| **SATA SSD**（870 EVO） | 452 / 456 ms | 416 / 409 ms | 423–471 MB/s | 估 SATA SSD 约 0.6 s |
| **NVMe**（KXG50ZNV256G） | 169 / 256 ms | 146 / 158 ms | 752–1,320 MB/s | **M0 没测，本次补齐** |

按 300 MB 折算：HDD 约 1.8 s、SATA SSD 约 0.66 s、NVMe 约 0.24 s。
**与 M0 §2 的估算吻合**（HDD 约 2 s、SATA SSD 约 0.6 s），NVMe 这一格是本次新增的。

> 有意思的一点：HDD 上**顺序读**能跑到 165 MB/s，所以 192.8 MB 的索引 1.2 s 就能整体预读完；
> 真正要命的是**随机读**（见下表 `off` 一行）。这正是预读存在的理由。

#### 冷缓存下的前 200 次跳转（四种预读策略，取两轮中较有代表性的一轮）

| 盘 | 策略 | 200 次查询总耗时 | p50 | p99 | max |
|---|---|---:|---:|---:|---:|
| HDD | 不预读 | 3,223 ms | 14.7 ms | 40.3 ms | 69.1 ms |
| HDD | **全速预读（普通优先级）** | **944 ms** | **0.032 ms** | 50.1 ms | 63.7 ms |
| HDD | 全速预读（低优先级 I/O） | 3,445–4,664 ms | 14.8–21.8 ms | 45.1–100.9 ms | 49.8–111.3 ms |
| HDD | 限速 64 MB/s | 3,137 ms | 14.1 ms | 48.2 ms | 69.3 ms |
| SATA SSD | 不预读 | 159 ms | 0.63 ms | 1.5 ms | 2.4 ms |
| SATA SSD | **全速预读（普通优先级）** | 238 ms | **0.011 ms** | 11.7 ms | 12.3 ms |
| NVMe | 不预读 | 172–923 ms | 0.55–0.94 ms | 2.9–27.2 ms | 3.7–31.2 ms |
| NVMe | **全速预读（普通优先级）** | **122–137 ms** | **0.019 ms** | 4.1–4.6 ms | 4.4–5.8 ms |

逐条结论：

- **HDD 上预读把 p50 从 14.7 ms 压到 0.032 ms（约 460 倍）**，总耗时从 3.2 s 降到 0.94 s。
  M0 在 HDD 上测得冷缓存首批跳转 p50 9.0 ms / p99 166 ms / max 2.8 s；本次**预读生效后**
  p50 0.032 ms / p99 50.1 ms / max 63.7 ms，**最差一次从 2.8 s 降到 64 ms**。
  issue 要求的「预读生效后的数字」就是这一行。
- **低优先级 I/O 在 HDD 上几乎没用**（预读只跑到 35–44 MB/s，要 4.4–5.6 s 才读完，期间查询照样等盘），
  限速 64 MB/s 同理。SSD / NVMe 上两者都够快，差别不大。
- **SSD / NVMe 上冷缓存本来就不痛**（不预读也只有 p50 0.6 ms），预读的价值主要在 **p99 / max**：
  NVMe 上不预读的那一轮 p99 到了 27.2 ms，预读后稳定在 4–5 ms。
- 预读期间 p99 反而偏高（HDD 50 ms、SSD 11.7 ms）：预读线程和查询抢同一块盘。
  **这是已知的取舍**，p50 的收益远大于 p99 的代价。

**场景 1 结论：通过。** 三种盘的预读耗时都已实测，`M0_LANG_SPIKE_REPORT` §2 标注的
「SSD / NVMe 冷缓存数据待 M2 补测」这一条**结清**。

### 4.2 查询侧预算（真实索引）

`cargo xtask bench --quick --check-budgets`，`ROVE_PERF_CS_INDEX` 指向大型项目的**真实索引**
（192.8 MiB、21,908 个文件、101 万个符号）。热缓存（索引已在系统文件缓存里），冷缓存见 §4.1。

| 指标 | 预算 | 本次实测（热缓存） | M0 实测 | 判定 |
|---|---|---|---|---|
| 定义跳转（查询侧，p99） | < 16 ms | **1.58 µs** | 3.5 µs | OK |
| 查找引用（查询侧，p99） | < 50 ms | **864 µs** | 1.95 ms（最重符号） | OK |
| 路由开销：未修改文件 | < 100 µs | **811 ns** | — | OK |
| 路由开销：含位置映射 | < 100 µs | **921 ns** | — | OK |
| `F12` → 目标行画面完成（p99，目标文件已打开） | < 50 ms | **36.77 ms** | — | OK（余量 26%） |
| 冷缓存下的首批跳转（HDD，预读生效后） | 记录 | **p50 0.032 ms / p99 50.1 ms / max 63.7 ms** | p50 9.0 ms / p99 166 ms / max 2.8 s | 见 §4.1 |

查询侧比预算快 4 个数量级，这一点与 M0 的结论一致：**瓶颈从来不在查询，而在索引构建与冷读取**。
端到端的 36.77 ms 里绝大部分是界面侧（布局、整形、绘制），不是查询。

### 4.3 单文件增量更新（场景 4）

`cargo run --release -p rove-lang-cs --example incr_check -- --project medium --latency 40 --edits 0`
（在中型项目的**副本**上做，不动原项目；副本 8,586 个文件，复制 16.1 s）。

| 阶段 | p50 | p99 | max | 预算 |
|---|---:|---:|---:|---|
| 索引进程侧重绑单文件 | 23 ms | 60 ms | — | — |
| 往返（发请求 → 收到完成） | 23 ms | 61 ms | — | — |
| **对查询可见**（保存 → `F12` 能看到新结果） | **46 ms** | **82 ms** | — | **< 300 ms ✅** |
| **每个文件的第一次保存**（含该项目的预热） | 31 ms | **3,842 ms** | 3,842 ms | **超 300 ms** |

- **稳态达标，余量很大**：连续 40 次普通保存，端到端可见延迟 p50 46 ms、p99 82 ms，
  只用掉 300 ms 预算的 27%。
- **但「某个项目里的第一次保存」会慢**：实测最差 3.8 s。这是常驻索引进程**第一次为那个
  csproj 建编译**的代价（9 个项目预热完占 526 MB）。用户的体感是：改完第一个文件后，
  状态栏的「增量更新中」会多转几秒。
  中型项目 91 个 csproj，大型 242 个，**每个项目第一次被碰到时都会交一次这个学费**。
  这一条没有写进 `budgets.toml`，`UX_SPEC` §2.1 也允许「增量更新中」不显示百分比，
  所以不算违反预算；但如实记录在这里，供 #150 决定要不要做「打开项目时后台预热常驻进程」。
- 常驻内存：预热 9 个项目后 **526 MB**，空闲回收后 **263 MB**，全量索引峰值 1,392 MB
  （本次副本上的全量索引 39.7 s，与 §1 的 42.3 s 同量级）。

**机器负载**：本次测量时基准与索引任务都已结束，机器相对空闲，但 Unity 仍然开着。

> 大型项目的同一组数字本次**没有跑**（一次检查点约等于一次全量索引，耗时以十分钟计），
> 记在 §7 第 11 条。#35 自己的验收已经在大型项目上做过。

### 4.4 预算逐条对照

`cargo xtask bench --quick --check-budgets`（`ROVE_PERF_CS_INDEX` 指向大型项目的真实索引），
共 87 个用例、39 条预算。完整输出在 `target/bench-44.json`（不进仓库）。

#### M2 自己的预算

| 预算 | 目标 | 实测 | 判定 | 对应场景 |
|---|---|---|---|---|
| `goto_definition`（查询侧，p99） | 16 ms | **1.58 µs** | OK（余量 4 个数量级） | 9 |
| `find_references`（查询侧，p99） | 50 ms | **864 µs** | OK | 9、11 |
| `router_definition_mapped`（路由含位置映射） | 100 µs | **921 ns** | OK | 6 |
| `router_definition_clean`（路由，未修改） | 100 µs | **811 ns** | OK | 2 |
| `goto_definition_e2e`（`F12` → 目标行画面完成，p99） | 50 ms | **36.77 ms** | OK（余量 26%） | 9 |
| `peek_open_to_first_paint_28k`（最重符号速览首绘，p99） | 100 ms | **91.15 ms** | **OK，但只剩 9% 余量** | 11 |
| `peek_tree_scroll_frame_47k`（速览结果树滚动帧，p99） | 8.33 ms | **2.42 ms** | OK | 11 |
| `l1_build_large`（L1 全量） | 5,000 ms | **3.43 s** | OK | 2 |
| `l1_incremental_large`（L1 增量） | 100 ms | **1.84 ms** | OK | 4 |
| `l1_workspace_symbols_large`（p99） | 30 ms | **2.96 ms** | OK | — |
| `nav_end_to_end` | 50 ms | — | `pending`（埋点属 #40，`goto_definition_e2e` 已覆盖同一口径） | 9 |

issue 正文里不在 `budgets.toml` 的三条（属 #32 / #33 的验收口径），本次实测：

| 指标（大型项目） | 目标 | 实测 | 判定 |
|---|---|---|---|
| 首次全量索引耗时 | < 90 s | **82.1 s** | OK，**余量只有 9%**（本机负载较高；#33 空闲机器实测 72–75 s） |
| 索引进程峰值工作集 | < 3 GB | **2,942 MiB** | OK，**余量只有 4%** |
| 索引文件大小 | < 200 MB | **202,114,552 B** = 192.8 MiB = 202.1 MB（十进制） | **按 MiB 算达标，按十进制 MB 算超 1.1%**。M0 的同一口径是 303 MB。文件数比 #33 当时多了 687 个，体积随之上涨；#212 还要再加运算符与索引器的出现位置，**这一条很可能在 #212 之后越线**，需要在那里一并处理 |

中型项目：42.3 s / 1,522 MiB / 76,580,040 B（73.0 MiB）。

#### M1 的界面预算（加入导航装饰后重测，确认没有回退）

| 预算 | 目标 | M1 实测 | 本次实测 | 判定 |
|---|---|---|---|---|
| `quickopen_first_key_large` | 16 ms | 13.18 ms | **10.32 ms** | OK（更快） |
| `quickopen_first_key_25k` / `extend_key_25k` | 16 ms | 4.54 / 2.28 ms | **5.18 / 2.11 ms** | OK（同量级） |
| `search_first_batch_large` | 100 ms | 5.02 ms | **4.04 ms** | OK |
| `search_full_large` | 2,000 ms | 1.61 s | **1.63 s** | OK |
| `csharp_visible_1mb_unity` | 50 ms | 37.35 ms | **34.25 ms** | OK |
| `csharp_incremental_5000_lines` | 2 ms | 175 µs | **185 µs** | OK |
| `breadcrumb_cursor_move_5000_lines` | 1 ms | 156 ns | **153 ns** | OK |
| `find_input_char_100k_lines`（p99） | 16 ms | 5.11 ms | **4.66 ms** | OK |
| `sticky_scroll_frame_extra` | 100 µs | 58 ns | **60 ns** | OK |
| `workspace_symbols_keystroke_large`（p99） | 30 ms | 3.56 ms | **4.01 ms** | OK |
| `edit_insert_char_100k_lines`（p99） | 100 µs | — | **4.73 µs** | OK |
| `cold_start_first_frame`（只记录） | — | 299.8 ms | **312.94 ms** | 记录 |

本次额外补跑了 `scroll` 组（M1 的每帧 CPU 预算；需要先 `cargo xtask gen-testdata --only uniq`）：

| 场景 | 每帧 CPU（mean / p99） | 帧间隔 p99 | M1 对应值 | 判定 |
|---|---|---|---|---|
| `scroll/miss`（`uniq.txt`，整形全未命中） | 0.478 / 0.777 ms | 11.49 ms | 403 µs / 11.58 ms | OK（预算 1 ms；**加了导航装饰层后没有回退**） |
| `scroll/hl_hit`（重复 C#，缓存命中） | 0.497 / 1.138 ms | 12.00 ms | 454 µs | OK |
| `scroll/hl_miss`（每行唯一 C#） | 0.563 / 1.379 ms | 12.58 ms | — | OK |

三个场景的「占位帧」分别是 0 / 400、0 / 300、1 / 400：**滚动时几乎不出现等宽占位**，
与 #132 的结论一致。`wrapped_frame_cpu_cache_hit` 与 `find_visible_frame_cpu_cache_hit`
本轮没有跑到（`--filter scroll/` 只覆盖上面三个场景），记为未测量。

#### 唯一超预算的一条

| 预算 | 目标 | M1 实测 | 本次实测 | 判定 |
|---|---|---|---|---|
| `workspace_full_scan_large` | 1,000 ms | 2.83 s（超 182.7%） | **2.97 s（超 197.0%）** | **超预算**，与 M2 无关，已由 M1 开出的 [#210](https://github.com/Zhao-wl/Rove/issues/210) 跟踪 |

未测量 5 条（`open_1gb_text` 需要 1 GiB 合成数据，本次没有生成；`frame_cpu_cache_miss`、
`highlighted_frame_interval_cache_miss` 只记录不设门槛）、`pending` 4 条
（`cold_start_interactive`、`empty_window_memory`、`key_to_paint`、`nav_end_to_end`，都不属于 M2）。

**结论：M2 的预算全部达标**，其中 `peek_open_to_first_paint_28k`（9% 余量）、
全量索引耗时（9% 余量）、峰值内存（4% 余量）、索引体积（贴线）四项余量很小，
在空闲机器上复测与 #212 落地后都要重新确认（§7 第 11 条）。

## 5. Source Generators 复核（场景 19，结清 M0 第 4 条待办）

`M0_LANG_SPIKE_REPORT` 的「验证中发现的真实问题」第 4 条留了一句话：

> 所有 csproj 都挂了 Unity 的生成器，大型项目还有 37 个项目挂了 Entities 生成器。但两个项目开启后都
> 生成了 0 个文件，未解析率没有变化。**建议默认关闭。** 注意：还没有确认这些生成器是否真的被成功加载
> （也可能是项目里根本没有触发生成的代码）。需要找一个使用了 `ISystem` / `[GeneratePropertyBag]` 的项目复核。

### 5.1 做法

先查两个测试项目里有没有会触发 DOTS 生成器的代码：

| 代码模式 | 中型 | 大型 |
|---|---|---|
| `: ISystem` / `: SystemBase` / `IJobEntity`（DOTS 实体系统） | 没有 | **没有** |
| `[GeneratePropertyBag]` | 没有 | 没有 |
| `[BurstCompile]`（Burst 生成器） | — | **有**（`ScriptCore` 下多处） |
| `com.unity.entities` 包 | — | 有（1.3.0-pre.4，但没有用它的 `ISystem`） |

然后在大型项目上开着生成器重建一次索引：

```powershell
rove-indexer-cs <大型项目> target/generators/large-gen.idx --generators --report target/generators/large-gen.json
```

### 5.2 结果

| 项 | 不开生成器 | **开生成器** | 差值 |
|---|---:|---:|---|
| `generator_issues`（加载失败与诊断） | — | **`[]`（空）** | **没有任何加载失败** |
| `generated_trees`（生成的语法树） | 0 | **17** | +17 |
| 文件数 | 21,908 | 21,925 | +17 |
| 符号数 | 1,014,054 | 1,014,648 | **+594（+0.06%）** |
| 未解析名字 | 3,161 | 3,165 | +4 |
| 未解析率 | 0.0604% | 0.0606% | 基本不变 |
| 索引体积 | 202,114,552 B | 202,265,816 B | +151 KB |
| 生成器阶段的线程秒 | — | **63.42 线程秒** | 约占 `bind_and_collect`（805 线程秒）的 8%，16 线程上折合几秒钟墙钟 |

> 两次运行的总耗时（82.1 s vs 63.2 s）**不能直接比**：开生成器那一轮机器更空闲（基准已经跑完）。
> 生成器本身的代价看 `thread_seconds.generators` 这一项。

### 5.3 结论（M0 第 4 条待办结清）

1. **生成器确实被成功加载了。** `generator_issues` 为空意味着 `AnalyzerFileReference` 没有报
   `AnalyzerLoadFailed`；而且确实**跑出了 17 个生成的语法树**。M0 当时的「0 个文件」不是加载失败，
   而是**项目里当时没有触发生成的代码**——这个疑点可以关掉了。
2. **默认关闭仍然是对的。** 开启后符号只多了 0.06%，未解析率几乎不动，代价是 63 线程秒；
   而且 trim 过的组件包**根本不支持** `--generators`（`Program.cs` 会直接拒绝并说明用未 trim 包）。
3. **留一句给以后**：两个测试项目都**没有**真正使用 DOTS 的 `ISystem` / `IJobEntity`
   （M0 里 `ISystem` 的匹配其实命中的是 `WorkerAISystem` 这类名字的子串）。
   真正重度使用 DOTS 的项目上要不要默认开启，**本次仍然无法回答**，需要另找项目；
   不过现在至少知道「生成器机制本身是通的」，不是加载失败。

## 6. `SKELETON` 审计、接缝缺陷与后续 issue

### 6.1 `SKELETON` 审计

- `grep -rn "SKELETON(" crates xtask tools`：**代码里一个标记都没有**（只有 `docs/` 与 `AGENTS.md` 里
  描述机制的文字）。M2 的键名中只有 `nav`（#40）对应 #2 的既有最简实现，它在 #40 的 PR 里已经删除。
- `docs/ARCHITECTURE.md` 的「最简实现 → 负责替换的 issue」对照表与代码一致，`nav`、`peek` 两行都已标「已完成」。
- **本次补了一行**：`metadata-view`（#34 / #42 新增的文档类型）。它之前只有 §26、§35 的正文，
  没有在对照表里登记，不符合 issue #44 目标 5「新增的速览窗口、元数据视图文档类型已在表中登记」。

### 6.2 本 issue 修复的缺陷

**没有需要改代码的接缝缺陷。** 本次把四种索引状态、索引原子替换、组件缺失、未保存修改四条链路
从头到尾走了一遍，精确度标记在 #38 → #40 → #41 → #42 之间**没有丢**，也没有出现
「替换后视图显示旧内容」「通知刷屏」「标签被重开」这类问题。
改动集中在把这些断言固化成测试（见 §8）与两处文档登记（上面的对照表、`AGENTS.md` §8.10）。

### 6.3 开出的后续 issue

| issue | 标题 | 为什么不在本 PR 里修 |
|---|---|---|
| [#212](https://github.com/Zhao-wl/Rove/issues/212) | 运算符重载与索引器没有出现位置，20 个导航用例必然失败 | 要改 `tools/rove-indexer-cs` 的绑定阶段收集范围（#33 的核心实现），并重测索引体积与全量耗时（大型项目的体积已经贴着 200 MB 的线）。超出「只修接缝性质的小缺陷」的范围 |
| [#213](https://github.com/Zhao-wl/Rove/issues/213) | `LANGUAGE_INTELLIGENCE` §3.2 写「未保存修改也触发重建」，实现只在保存后触发 | 两边各有道理（实现的理由成立：索引进程只读磁盘），该改的是设计文档的口径，需要维护者定。本 issue 只把**实现的现状**写成测试，改哪一边都会红 |

### 6.4 本 PR 没有改 #1 的契约

`crates/rove-lang-cs/src/query.rs`（`Precision`、`QueryResult`、`CsQueries`……）一个字节都没有改动，
也没有新增命令、设置项、颜色键或界面字符串，所以不涉及 #1 的登记机制。

## 7. 人工事项清单（本次没有做，不算通过）

本次验收**没有做任何逐项的真机界面操作**（除了自动化测试打开的 GPUI 测试窗口）。
下面每一条都要在发版前由人补上，不能按「应该没问题」记成通过。

| # | 人工事项 | 对应场景 | 为什么自动化不了 |
|---|---|---|---|
| 1 | **界面手工抽查的另一半**：≥ 20 例逐条看悬停签名是否正确、精确 / 近似标记是否符合 `UX_SPEC` §2.2、排版是否干扰阅读 | 目标 2 | 签名排版与「标注是否克制」是观感判断。落点一致性已自动化（`m2_real_project_ui_matches_the_query_layer`，20/20） |
| 2 | **近似 → 精确切换时界面是否闪烁**：录屏或连续截图 | 2 | 事件序列已断言（提示不重弹、标签不重开、高亮静默替换），但「有没有一帧白闪」只能看 |
| 3 | **手工结束索引进程**（任务管理器杀 `rove-indexer-cs.exe`），确认自动重试一次、失败后状态栏 `$(error) C# 索引失败`、通知只弹一次 | 7 | 需要在真实进程上破坏。建议先把 `rove.index.cacheDir` 指向临时目录，别污染 `%LOCALAPPDATA%\Rove` |
| 4 | **移走 `%LOCALAPPDATA%\Rove\components\indexer-cs\`** 后打开项目，确认状态栏「`$(info) C# 基础导航`」与 L0/L1 导航可用 | 7 | 同上，要动真实安装目录 |
| 5 | **放一个旧版本组件包**，确认提示更新组件、降级到 L1、不崩溃、不反复重试 | 8 | 需要真的打一个旧版本包（`scripts/components/package-indexer.ps1`） |
| 6 | **`List<T>.Add` 与第三方 DLL（AWSSDK / Firebase / Wwise 等）的元数据三跳** | 13 | 本次 `external` 用例覆盖的是 Unity API 与 BCL（`netstandard`）；第三方 DLL 的视图没有单独抽查。注意大型项目有 2 个程序集的元数据镜像因项目自身编译错误降级，只有签名没有文档注释 |
| 7 | **记录 `Transform.position` 在两个项目里的实际引用条数**（元数据视图里 `Shift+F12`） | 14 | issue 要求记录具体数字，本次没有取。「结果数与查询层一致」已自动化 |
| 8 | **最重符号速览的滚动手感**（> 1 万条结果时上下翻） | 11 | 首次绘制时间有预算判定，手感只能看 |
| 9 | **`F12` / `Ctrl+点击` / `Ctrl+悬停` / `Alt+←→` / `Ctrl+K Ctrl+Q` 的真机手感与悬停卡片排版** | 9 | 既有自动测试覆盖的是行为，不是观感 |
| 10 | **签名变更后后台队列逐步重建的过程**：在界面上看受影响文件一个个变回精确 | 5 | 一致性与端到端延迟已自动化（`incr_check`、`router.rs`），「逐步变化的过程」要看 |
| 11 | **空闲机器上复测全部性能数字**，并在**大型项目**上补跑 `incr_check --latency`（本次只在中型上做了） | 3、4、11 | 本次机器大部分时间同时开着 Unity 与另一个构建槽，耗时偏大；余量最小的四项（速览首绘 9%、全量索引 9%、峰值内存 4%、索引体积贴线）尤其需要在空闲机器上确认 |
| 12 | **机械硬盘上的首次全量索引与冷启动整链路**（本次只在 HDD 上测了索引预读，没有把整个项目放到 HDD 上重跑） | 1、3 | 需要把测试项目复制到 D: 盘再跑一轮，耗时以小时计 |
| 13 | **125% / 150% / 混合 DPI 下重看速览窗口与元数据视图** | — | 本机两台显示器都是 100%，没有混合 DPI 环境；见 [DPI_CHECKLIST](DPI_CHECKLIST.md) |
| 14 | **与 VS Code（C# Dev Kit）的并排对照**：同一项目、同一组用例的首次可用时间、跳转 / 引用响应、常驻内存、外部符号落点 | §9 | 需要装 C# Dev Kit 并手工计时；本次只用公开资料与 M0 的数据做了有限对照，结论见 §9 |

## 8. 本 issue 新增的测试

`crates/rove-ui/src/skeleton/workbench/m2_acceptance_tests.rs`，6 个测试，
`cargo test -p rove-ui m2_ -j 1 -- --nocapture`：

| 测试 | 覆盖的场景 | 断言要点 |
|---|---|---|
| `m2_approximate_to_exact_switches_silently` | 2、17 | 构建中提示一次 → 同符号不重复 → **没有按键**索引就绪后语义高亮自己补上 → 就绪后无任何标记；全程通知 0、标签数不变 |
| `m2_precision_flag_survives_router_and_peek` | 2、17 | 同一位置在构建中 / 就绪两种状态下，查询层的 `precision` 与速览标题标记、结果数逐层对上 |
| `m2_index_replacement_refreshes_peek_metadata_and_highlights` | 风险点 2 | 速览 + 元数据视图 + 同名高亮**同时开着**时换索引：元数据标签换进新内容、标签集合不变、速览重开后结果数与精确度不变、不弹通知 |
| `m2_component_missing_degrades_but_navigation_still_works` | 7、16 | 状态栏 `$(info) C# 基础导航` + 悬停提示整句 + 不弹通知 + L1 跨文件跳转可用且标为近似 |
| `m2_unsaved_edit_falls_back_to_approximate_and_requests_a_rebuild` | 6 | 被修改行回退为近似（`ModifiedRegion`）、界面标出来、**未保存不请求重建 / 保存后才请求**（#213） |
| `m2_real_project_ui_matches_the_query_layer` | 目标 2 的自动化部分 | 真实中型项目上 20 个真实位置，界面 `F12` 的落点 == 查询层的回答 |

前五个用合成索引，进 CI；第六个需要 `ROVE_TEST_UNITY_MEDIUM` 与该项目的 L2 索引
（默认 `target/nav-check/medium.idx`，`ROVE_NAV_INDEX_MEDIUM` 可改），缺任何一项都打印原因并跳过。
用法登记在 `AGENTS.md` §8.10。

#2 的端到端测试 `walking_skeleton_opens_csharp_through_real_window` 仍然通过。

## 9. 对 M2 完成标准的结论

`PLAN.md` 的 M2 完成标准：**跳转和引用体验超过 VS Code**（验收口径：导航准确率测试集通过、响应时间指标达标）。

### 9.1 逐项结论

| 口径 | 结果 | 判定 |
|---|---|---|
| 导航准确率测试集通过 | 精确层 **174/200 = 87.0%**，缺口 20 例归 #212、6 例属设计边界 | **未达到**（要求 100%） |
| 响应时间指标达标 | 查询侧、端到端、速览首绘、增量更新见 §4 | 见 §4 的逐条判定 |

### 9.2 与 VS Code 的对照：**本次没有做并排实测**

issue 建议「以同一项目、同一组用例在 VS Code（C# Dev Kit）与 Rove 上对照」。
本次**没有**在本机装 C# Dev Kit 逐项计时，所以下面只写**有数据支撑**的部分，其余列进 §7 第 14 条。

| 维度 | Rove（本次实测） | VS Code / C# Dev Kit | 依据与说明 |
|---|---|---|---|
| 首次可用时间 | 打开即可用：L1 在中型 1.5 s / 大型 2.6 s 内建好，在那之前 L0 兜底，**按 `F12` 永远立即有结果**，界面标明近似 | OmniSharp / Roslyn 语言服务在大型 Unity 项目上要先还原并加载 242 个 csproj，期间跳转不可用 | Rove 侧是本次实测；VS Code 侧**没有本次实测**，只有 `M0_LANG_SPIKE_REPORT` 的背景调研。**不作为结论** |
| 跳转 / 引用的响应时间 | 见 §4.2（微秒级查表） | — | VS Code 侧没有实测 |
| 索引常驻内存 | 查询侧是 mmap，不常驻；索引进程**只在构建时**存在，峰值中型 1.5 GB / 大型 2.9 GB，构建完退出 | — | VS Code 侧没有实测 |
| 外部符号的跳转结果 | 预计算的元数据视图，`external` 用例 12/12 正确落到成员 | C# Dev Kit 用「从元数据」的反编译视图 | 两者形态不同（预计算 vs 反编译），本次没有做质量对照 |

### 9.3 明确结论

**现在还不能说「跳转和引用体验超过 VS Code」。** 理由有三条，按严重程度排：

1. **准确率口径没达到。** 精确层 87.0%，运算符与索引器两类完全不可用（0/20）。
   在大量使用自定义运算符与索引器的 Unity 项目里（两个测试项目都有：`V3`、`Q4`、`ExtendArray`、
   `GridMatrix`、`TeamTactics`……），用户按 `F12` 会得到「什么都没有」，这是**比 VS Code 差**的点。
   修完 #212 后预计 194/200。
2. **「超过 VS Code」这句话本次没有实测支撑。** §9.2 里 Rove 一侧的数字都是真的，
   VS Code 一侧是空的。在补上并排对照之前，这个结论只能是「待证」。
3. **界面观感没有人看过。** §7 的 14 条里有 9 条是观感与真机操作。

**可以确定地说的是**：

- 12 个语义分类（`var`、链式、重载、扩展方法、泛型、继承、接口、partial、Lambda、局部变量、
  事件、类型引用）在两个真实项目上都是 **100%**，这正是「按名称解析一定会错」的那些点；
- **索引构建期间不空窗**：L1 在秒级建好，期间按 `F12` 总有结果且标明近似，索引就绪后**静默**变精确
  （不闪烁、不重弹提示、不重开标签，已固化成测试）；
- 四条降级路径（构建中、组件未安装、版本不匹配、进程失败）都不会让导航变成不可用，
  也不会刷屏通知；
- 精确度标记从 #38 到 #42 的四层传递链**没有丢**，界面不会「看起来正常但其实在骗人」。

**建议**：把 #212 作为 M2 的收尾项排进 v0.1，并在 #150（v0.1 发布候选验收）里补上与 VS Code 的
并排实测，再对「超过 VS Code」这句话下最终结论。
