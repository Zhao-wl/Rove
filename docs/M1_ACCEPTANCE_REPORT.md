# M1 集成验收报告（issue #31）

> 本文如实记录 2026-11-14 在两个真实 Unity 项目上走完 M1 场景清单的结果。
> 写法参照 [M0_UI_SPIKE_REPORT](M0_UI_SPIKE_REPORT.md) §8：**未达标的清单比好看的报告有用**。
> 不能自动化、也没有在本次真机上做的事情，一律写进 §7 的人工清单，不臆造结论。

## 1. 环境

| 项 | 值 |
|---|---|
| CPU | Intel Core i7-11700 @ 2.50 GHz，8 核 16 线程 |
| 内存 | 47.7 GB |
| 系统 | Windows 11 专业版 10.0.22631 |
| 显示 | 两台 1920×1080，缩放 100%（`AppliedDPI = 96`），**没有混合 DPI 环境** |
| 机器负载 | 验收期间本机同时开着 Unity 编辑器等程序，**负载较高**；所有耗时只作相对参考 |
| 构建 | `rustc` 按 `rust-toolchain.toml`（stable MSVC）；本机构建关掉了 LTO（`CARGO_PROFILE_RELEASE_LTO=off`），所以 release 体积数字不代表发布配置 |
| VS Code | 已安装（`D:\Tools\...\bin\code.cmd`），共用 `settings.json` / `keybindings.json` 都存在 |

测试项目（路径只来自环境变量，不写进仓库）：

| 槽位 | 环境变量 | Unity 版本 | 规模（扫描口径） |
|---|---|---|---|
| 中型 | `ROVE_TEST_UNITY_MEDIUM` | 2022.3.62f2c1 | 工作区可见文件 **137,128** 个；非排除目录下 `.cs` **10,305** 个；根目录有 `.gitignore` 与 106 个 `.csproj` |
| 大型 | `ROVE_TEST_UNITY_LARGE` | 2022.3.62f2c1 | 工作区可见文件 **159,066** 个；非排除目录下 `.cs` **13,742** 个；根目录有 `.gitignore` 与 244 个 `.csproj` |

> 注：issue 正文里写的大型项目「约 2.1 万个 `.cs`」是按包含 `Library/PackageCache` 的口径算的；
> 按 Rove 的默认排除规则（`Library`、`Temp`、`obj`、`Logs`）看到的是 13,742 个。两个数都对，口径不同。

## 2. 场景清单逐条结果

验证方式一栏里：

- **自动**＝本 issue 新增的真实项目验收测试
  （`crates/rove-ui/src/skeleton/workbench/acceptance_tests.rs`，`cargo test -p rove-ui acceptance_ -- --nocapture`）；
- **既有自动**＝对应 issue 自己的入口测试（合成数据）已经覆盖，本次只复核它仍然通过
  （`cargo test --workspace -j 1` 全绿，79 个测试二进制、0 失败）；
- **人工**＝只能靠人看的部分。**本次验收没有做任何逐项的真机界面操作**（除了 `xtask startup`
  真的启动了 10 次 `rove.exe`），所有「人工」项一律列进 §7，没有一条按「应该没问题」记成通过。

| # | 场景 | 中型 | 大型 | 结论与验证方式 |
|---|---|---|---|---|
| 1 | `rove.exe <文件夹>` / 欢迎页最近打开 | 通过 | 通过 | **自动**：`xtask startup --project <两个项目>` 各启动 release `rove.exe <文件夹>` 5 次，首帧 median 308 ms / 348 ms（§3.1）+ 既有自动（#28 `welcome_tests`、#16 布局持久化）。欢迎页「最近打开」是既有自动，不是真机点击 |
| 2 | 文件树：大目录、紧凑文件夹、定位、过滤 | 通过 | 通过 | **自动**：`Library/`、`Temp/`、`obj/`、`Logs/` 一条都没漏；`.gitignore` 忽略的 106 / 244 个 `.csproj` 仍然可见（Rove 不读 `.gitignore`）。展开 / 紧凑文件夹 / 过滤由 #23 既有自动覆盖 |
| 3 | `Ctrl+P`、`:` `@` `>` `#`、命令中心 | 通过（1 处记录） | 通过 | **自动**（中型）+ 既有自动（#24）。逐键有结果、回车打开并定位。按键预算见 §3；`#` 前缀现在是**可用**的（#60 接上工作区符号），issue 正文里「优雅提示不可用」的口径已过期。打开 `.png` 的行为见 §5 的记录 1 |
| 4 | `Ctrl+Shift+F`、三个开关、include/exclude、`F4` | 通过 | 通过 | **自动**（中型：`GetComponent` 3,187 行结果；空状态含「已排除 Unity 生成的目录。」）+ 既有自动（#25 的开关、`F4`、include/exclude）。引擎侧预算见 §3 |
| 5 | `.cs` 高亮、折叠、括号、同名高亮、参考线、字体缩放 | 通过（自动口径） | 通过（自动口径） | 既有自动（#20 `code_view_features`、#11 着色黄金测试：按字符比颜色，不比 scope）。**与 VS Code 的截图对照本次没有做**，所以「无明显差异」这句话目前没有截图证据 → §7 第 1 条 |
| 6 | GBK 中文注释，其他编码重开再切回 | 通过（构造数据） | — | **自动**：两个项目里**没有**非 UTF-8 的源码文件（各扫前 2 万个文件），所以取真实项目里带中文注释的 `.cs`，把内容转成 GBK 写进临时工作区再打开：识别为 GBK、悬停提示含「自动识别」、解码结果与原文逐字相同。「通过编码重新打开再切回」由 #27 既有自动覆盖 |
| 7 | 混合换行符，浏览后关闭 | 通过 | — | **自动**：中型项目里真实的混合换行符文件（`Docs/…/*.md`）状态栏显示「混合」，浏览前后磁盘字节完全一致 |
| 8 | ≥ 1 GB 日志：滚动到尾、`Ctrl+G`、`Ctrl+F` | 通过（合成 1 GiB） | — | **自动**（`huge_file` 基准 + #8 / #22 既有自动）。真实 `Editor.log` 没有 1 GB 量级的样本，用 `cargo xtask gen-testdata` 的 `big1g.txt`（1 GiB / 1,429 万行）。数字见 §3；**真机滚动手感**见 §7 |
| 9 | 压缩成一行的 JSON | 通过 | — | 既有自动（#22 的超长行截断标记与「格式化后查看」）。两个项目里没有找到单行超长 JSON 的真实样本，用的是合成数据 |
| 10 | `.md` 默认预览、切源码、并排、Mermaid / 公式、断网 | 通过 | — | **自动**：真实 `.md` 从文件树打开是预览，同一个文件从**搜索结果**打开是源码（来源语义没丢）。Mermaid / 公式 / 断网与真实 WebView2 由 #29 的 `markdown_probe` 覆盖（本次未重跑，见 §7） |
| 11 | `Library/PackageCache/**` 只读 | 通过 | — | **自动**：真实包缓存文件打开后状态栏「只读」，原因「Unity 包缓存目录，修改会被覆盖。」 |
| 12 | 大纲与面包屑（五种类型） | 通过 | — | **自动**：真实的 C# / JSON / YAML / XML / Markdown 文件各取一个，大纲都是 `Ready` 且有节点（5 / 3 / 59 / 222 / 6），面包屑都有段。不支持类型的空状态由 #26 既有自动覆盖。面包屑更新预算见 §3 |
| 13 | 外部改共用 `settings.json`（主题 / 字号 / `files.exclude`） | 通过 | — | **自动**：改文件 → 主题 Dark→Light、`editor.fontSize` 22 生效、`files.exclude` 加 `**/Assets` 后文件树里 `Assets` 消失。**真实文件监听**（不是手动 `reload()`）由 #13 的 `settings_service` 集成测试覆盖；用真实 VS Code 改真实用户文件的那一跳见 §7 |
| 14 | `keybindings.json` 覆盖默认键位 | 通过 | — | 既有自动（#14：`keybindings_json_keys_dispatch_commands` 覆盖 `-command` 移除、未知命令忽略、和弦、未知 `when`） |
| 15 | 多标签、预览标签、网格分屏、拖动、固定 | 通过 | — | 既有自动（#17 `editor_tests`：同一文件在两个分屏里共用一个 `TextSource`，预览 / 固定 / 拖动 / 最大化） |
| 16 | 命令面板只列已实现命令；菜单无未实现项 | 通过（1 处清理） | — | **自动**：真实项目上完整装配的窗口里，命令面板列出 368 行，33 条「已登记但未实现」的命令（M2–M4 的功能）一条都没出现；菜单由 #16 的 `visible_items` 既有自动覆盖。本次登记审计还发现一条**过期登记**并清掉，见 §5 的修复 1 |
| 17 | 反馈分级：临时消息、提示条、通知、勿扰 | 通过（自动口径） | — | 既有自动（#16 的五级反馈测试覆盖四级的显示、去重与勿扰）。**真机观感没有抽查** → §7 第 9 条 |
| 18 | 125% / 150% / 混合 DPI 下重跑 1、3、4、10 | **未验证** | **未验证** | 本机两台显示器都是 100%，**没有混合 DPI 环境**；参数化回归测试（#30 的 `code_view_dpi`，100/125/150/200%）通过，但那是模拟缩放。真机见 §7 与 [DPI_CHECKLIST](DPI_CHECKLIST.md) |

## 3. 性能实测与预算对照

口径：`cargo xtask bench --quick --check-budgets`（criterion 采样 20 / 测量 1 s，比默认短，**数字只作相对参考**），
`perf/budgets.toml` 判定 `status = "enforced"` 的条目。**本机负载较高**（同时开着 Unity 与另一个构建槽），
空闲机器复测记在 §7。

### 3.1 冷启动（场景 1 的实测）

`xtask startup` 启动 release 版 `rove.exe <文件夹>`，读它打印的 `FIRSTFRAME_SINCE_PROCESS_MS`，各 5 次：

| 项目 | median | min | max |
|---|---|---|---|
| 合成小工程（`target/perf-project`，bench 默认） | 299.8 ms | — | — |
| **中型 Unity 项目** | **308.2 ms** | 306.1 ms | 383.8 ms |
| **大型 Unity 项目** | **348.0 ms** | 322.3 ms | 434.8 ms |

- 预算 `cold_start_interactive`（400 ms）考核的是「**可交互**」，埋点属于 #16、目前还是 `pending`；
  上表是**首帧**口径，`cold_start_first_frame` 只记录不设门槛。
- 首帧 median 在两个真实项目上都在 400 ms 以内，但**大型项目的最差一次 434.8 ms 超过了 400 ms**。
- 本机的 release 构建关掉了 LTO（见 §1），发布配置只会更快；这个数字偏保守。

### 3.2 预算逐条对照

`cargo xtask bench --quick --check-budgets`（共 108 个用例、39 条预算）。只列与 M1 场景相关的条目，
全文见 `target/bench-31.log` / `target/bench-31.json`（都在 `target/` 下，不进仓库）。

| 预算 | 目标 | 实测 | 判定 | 对应场景 |
|---|---|---|---|---|
| `cold_start_interactive` | 400 ms | —（埋点未做，`pending`，负责 #16） | 未判定 | 1 |
| `cold_start_first_frame` | 只记录 | 299.8 ms（合成）/ 308.2 ms（中型）/ 348.0 ms（大型） | 记录 | 1 |
| `empty_window_memory` | 60 MB | —（`pending`，负责 #16） | 未判定 | 1 |
| `workspace_full_scan_large` | 1,000 ms | **2.83 s** | **超预算 182.7%** → #210 | 2 |
| `quickopen_first_key_large` | 16 ms | 13.18 ms | OK | 3 |
| `quickopen_first_key_25k` / `extend_key_25k` | 16 ms | 4.54 ms / 2.28 ms | OK | 3 |
| `search_first_batch_large` | 100 ms | 5.02 ms | OK | 4 |
| `search_full_large` | 2,000 ms | 1.61 s | OK | 4 |
| `frame_cpu_cache_hit` | 1 ms | 403 µs | OK | 5 |
| `highlighted_frame_cpu_cache_hit` | 1 ms | 454 µs | OK | 5 |
| `wrapped_frame_cpu_cache_hit` | 1 ms | 727 µs | OK | 5 |
| `find_visible_frame_cpu_cache_hit` | 1 ms | 547 µs | OK | 5 |
| `frame_cpu_cache_miss`（帧间隔 p99） | 12.5 ms（只记录） | 11.58 ms | 记录 | 5 |
| `highlighted_frame_interval_cache_miss`（p99） | 12.5 ms（只记录） | 11.35 ms | 记录 | 5 |
| `csharp_visible_1mb_unity` | 50 ms | 37.35 ms | OK | 5 |
| `csharp_incremental_5000_lines` | 2 ms | 175 µs | OK | 5 |
| `open_10mb_text` | 只记录（#7 验收 50 ms） | 19.71 ms | 记录 | 6、7 |
| `open_1gb_text`（到首屏） | 1,000 ms | **17.05 ms** | OK | 8 |
| （同组）`huge/index_1gb_full` | #8 验收 1 s | 305.11 ms | 参考 | 8 |
| （同组）`huge/goto_line_1gb` | — | 443 ns | 参考 | 8 |
| `find_input_char_100k_lines`（p99） | 16 ms | 5.11 ms | OK | 8 |
| `breadcrumb_cursor_move_5000_lines` | 1 ms | **156 ns** | OK | 12 |
| `sticky_scroll_frame_extra` | 100 µs | 58 ns | OK | （M4 提前项） |
| `l1_build_large` | 5,000 ms | 3.50 s | OK | （M2 索引，顺带） |
| `workspace_symbols_keystroke_large`（p99） | 30 ms | 3.56 ms | OK | 3（`#` 前缀） |

未测量 2 条（`goto_definition`、`find_references`：需要 `--cs-index`，属于 M2 的语义索引），
`pending` 4 条（`cold_start_interactive`、`empty_window_memory`、`key_to_paint`、`nav_end_to_end`）。

### 3.3 结论

- **除 `workspace_full_scan_large` 外，M1 相关的 `enforced` 预算全部达标**，其中几条有很大余量
  （1 GB 文件到首屏 17 ms / 预算 1 s；面包屑 156 ns / 预算 1 ms；搜索首批 5 ms / 预算 100 ms）。
- 唯一超标的一条已复测确认不是噪声（背靠背 [2.83 s, 2.87 s, 2.90 s]），口径差异与优化方向写进 **#210**。
- 采样用的是 `--quick`（样本 20、测量 1 s），**数字只作相对参考**；机器负载较高。
  空闲机器上的完整采样复测列在 §7。

## 4. `SKELETON` 标记审计

- 仓库里 `*.rs` / `*.cs` / `*.toml` / `*.ps1` / `*.py` **没有任何 `SKELETON(...)` 标记**
  （唯一命中的 `scripts/issues/plan.py:56` 是 issue 正文模板里的说明文字，不是代码标记）。
- `docs/ARCHITECTURE.md` §10 的「最简实现 → 负责替换的 issue」对照表逐行核对了一遍，与代码一致。
  本次把两行的状态补齐：`syntax`（#9 已关闭）与 `workspace`（#15 已关闭）补上「（已完成）」，
  其余行原本就已标注。
- 结论：M1 的键名（`workspace`、`explorer`、`groups`、`buffer`、`syntax`、`theme`、`codeview`、
  `statusbar`、`quickopen`、`keys` 等）**全部已被替换**，没有残留。

## 5. 本 issue 修复的缺陷 / 记录的现象

**修复 1：清掉一条过期的界面字符串登记。**
`crates/rove-ui/src/shell/catalog.rs` 里仍登记着 `commandCenter.notImplemented` = 「快速打开尚未实现」，
这是 #24 落地前的占位文案，全仓库再没有任何地方引用它。留着它迟早会被人当成「还有未实现的入口」。
本 PR 删除该登记（纯删除，不涉及任何显示路径）。

**记录 1（不在本 PR 修）：二进制文件的错误页缺少「仍然打开」。**
真实项目里 `Ctrl+P` 会列出 `.png`（VS Code 也列）。打开后标签是 `Failed(Unsupported)`，错误页显示
「无法打开编辑器：文件格式或编码不受支持。」，操作只有「重试」与「在资源管理器中显示」。
`UX_SPEC` §3.5 要求提供「仍然打开」。`rove-buffer` 已经有 `LoadOptions::force_open`，但 #1 契约的
`Documents::open` / `OpenOptions` 没有对应入口，补这条路径属于契约变更，必须单独提 PR。
→ 开了 **#209**（里程碑「v0.1 首发」）。验收测试把现状记录下来（场景 3）。

**核对过、确认不是缺陷的两处：**

1. `Folder::rules()` 对**被排除**的路径返回空的只读原因（`excluded = true`、`read_only = []`），
   而同一个包缓存文件真正打开时是只读的。看起来不一致，其实是按设计：树里看不到的文件没有装饰可言，
   只读规则在 `files.exclude` 把 `Library` 放出来时才需要（那时 `excluded = false`，规则正常求值）。
   打开路径另走 `read_only_rules`，所以场景 11 的端到端行为是对的。
2. 场景 3 里 `#` 前缀已经可用（#60 的工作区符号），不再是 issue 正文写的「优雅提示不可用」。
   这是正文写于 M1 拆分时的口径，不是缺陷。

**开出的后续 issue（都在里程碑「v0.1 首发」）：**

| issue | 来源 | 一句话 |
|---|---|---|
| [#209](https://github.com/Zhao-wl/Rove/issues/209) | 场景 3 | 二进制文件的错误页缺少「仍然打开」（UX_SPEC §3.5）；要先走契约变更给打开路径加「强制打开」入口 |
| [#210](https://github.com/Zhao-wl/Rove/issues/210) | §3 性能 | 大型 Unity 项目全量扫描 2.83 s，超 `workspace_full_scan_large` 预算 182.7% |

## 6. 本次新增的自动化

`crates/rove-ui/src/skeleton/workbench/acceptance_tests.rs`（8 个测试，环境变量缺失时整组跳过，
所以 CI 上 `cargo test --workspace` 不受影响）：

| 测试 | 覆盖的场景 | 项目 |
|---|---|---|
| `acceptance_scenario_2_workspace_excludes_ignore_gitignore` | 2、11（规则侧） | 中型 + 大型 |
| `acceptance_scenario_3_quick_open_on_real_project` | 3（含二进制文件的现状记录） | 中型 |
| `acceptance_scenario_4_search_on_real_project` | 4 | 中型 |
| `acceptance_scenario_6_real_chinese_source_as_gbk` | 6 | 中型（内容真实、编码构造） |
| `acceptance_scenario_7_real_mixed_line_endings` | 7 | 中型 |
| `acceptance_scenario_10_11_markdown_source_semantics_and_package_cache` | 10（来源语义）、11（界面侧） | 中型 |
| `acceptance_scenario_12_outline_for_five_real_file_types` | 12 | 中型 |
| `acceptance_scenario_16_command_palette_lists_only_implemented` | 16 | 中型 |

这些测试只读测试项目：窗口根目录指向项目本身，共用 `settings.json` 放在临时用户目录里，
涉及写入的场景（GBK）在临时工作区里造文件；每个会改到文件的断言后面都核对了磁盘字节。

## 7. 人工事项（**本次没有做，不能用自动化代替**）

1. **与 VS Code 的截图对照**（场景 5）：同一主题、同一文件，逐像素比对高亮、参考线、折叠标记。
2. **真实输入法**（搜狗 / 微软拼音）在查找框、搜索框、快速打开里的候选框跟随（#5 的专项风险）。
3. **真正的双显示器混合 DPI**（场景 18）：125% + 100% 两台屏之间拖窗口，重跑场景 1、3、4、10，
   判据见 [DPI_CHECKLIST](DPI_CHECKLIST.md)。本机两台屏都是 100%，**这一条没有验证**。
4. **1 GB 日志的真机手感**（场景 8）：用合成的 `big1g.txt` 跑了基准，但「滚动是否顺滑」要人眼看。
5. **真实 WebView2 的 Markdown 预览**（场景 10）：`cargo run -p rove-ui --example markdown_probe`
   （含 Mermaid、公式、断网、空闲回收），本次未重跑。
6. **用真实 VS Code 改真实共用文件**（场景 13、14）：自动化覆盖的是「文件变了 → Rove 生效」，
   「VS Code 写文件」这一跳没有真机验证；反向（Rove 改主题后 VS Code 侧注释与格式不变）也没验。
7. **空闲机器上复测全部性能数字**：本次全程机器负载较高，§3 的数字只能作相对比较。
8. **干净 Windows（Sandbox）上的首次启动**：属于 #76 / #43 的发布验收，这里只做记录。
9. **反馈分级的真机观感**（场景 17）：临时消息 / 提示条 / 通知 / 勿扰四级在真实窗口里的停留时间与
   叠放次序，自动化只能验状态，不能验「看起来对不对」。
10. **逐项真机走查**：本次 18 个场景里凡标「既有自动」的，都是复核测试通过，没有人真的点一遍。
   发布前建议按本表再走一遍真机，重点是场景 5、15、17 这类「手感」项。

## 8. 对 `PLAN.md` M1 完成标准的结论

M1 的完成标准是：**「在两个 Unity 测试项目中，日常「看代码」不需要再打开 VS Code；性能指标达标」**。

**结论：基本达成，但有两条明确缺口，不能说「完全达成」。**

达成的部分（有据可查）：

- 18 个场景里 **17 个有明确结论且通过**（场景 18 没有环境可验），其中 8 条结论来自本次新增的、
  跑在真实 Unity 项目上的自动化测试，其余来自各 issue 既有的入口测试；
- 最容易丢的「来源语义」（同一个 `.md`：文件树 → 预览、搜索结果 → 源码）在真实项目上验证通过；
- 「不读 `.gitignore`」这条硬约束在两个项目上用真实数据验证：被 `.gitignore` 忽略的 106 / 244 个
  `.csproj` 仍然可见，而 `Library/`、`Temp/`、`obj/`、`Logs/` 一条都没漏；
- 性能上，M1 相关的 `enforced` 预算除一条外全部达标，1 GB 文件、搜索、快速打开、面包屑都有明显余量。

缺口（**未达成**，各有去处）：

1. **场景 18（高 DPI / 混合 DPI）完全没有真机验证**：本机两台显示器都是 100% 缩放。#30 的参数化回归
   测试覆盖 100/125/150/200% 的**模拟**缩放，但跨屏拖动、浮层定位、命中测试的真机判据
   （[DPI_CHECKLIST](DPI_CHECKLIST.md)）这次一条都没走。这是 M1 完成标准里**唯一一条拿不出证据**的场景。
2. **`workspace_full_scan_large` 超预算 182.7%**（2.83 s / 1 s）。它不影响「能不能看代码」
   （扫描是流式的，根目录立即可见），但「性能指标达标」这半句话现在不成立 → #210。

另外三条不算「未达成」、但要记下来：

- 二进制文件（Unity 项目里到处都是的 `.png`）打不开也没有「仍然打开」，图片预览本身是 M4 的 #63；
  按 M1 的范围这不算未达标，但「日常看代码不用开 VS Code」在看图这件事上确实还做不到 → #209。
- 与 VS Code 的**截图对照**（场景 5）没做：现在「高亮没有明显差异」的依据只有着色黄金测试
  （#11，用 VS Code 的 colorize 数据按字符比颜色），没有截图证据。
- 标「既有自动」的场景本次只复核了测试，**没有逐项真机点一遍**（§7 第 10 条）。

一句话：**「日常看代码不需要再打开 VS Code」在两个真实 Unity 项目上基本成立；「性能指标达标」还差
一条扫描预算；高 DPI 这一项缺真机证据，必须在发布前补上（#30 的清单 + §7 的人工事项）。**

## 9. 怎么复跑这份验收

```powershell
# 真实项目上的验收测试（环境变量缺失时整组跳过）
$env:ROVE_TEST_UNITY_MEDIUM = "<中型 Unity 项目>"
$env:ROVE_TEST_UNITY_LARGE  = "<大型 Unity 项目>"
cargo test -p rove-ui acceptance_ -j 1 -- --nocapture

# 性能：先生成测试数据（含 1 GiB 文件）与 release 构建，再跑基准与预算
cargo xtask gen-testdata
cargo build --release -p rove-app
cargo xtask bench --quick --check-budgets --save target/bench-31.json

# 冷启动（场景 1）：直接对真实项目测首帧
cargo xtask startup --project "<中型 Unity 项目>" --startup-tries 5

# 全量
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

跑完对照 §2 的表逐条看结论，§7 的人工事项要人去做，不要因为自动化全绿就当它们通过。
