# v0.1 发布候选集成验收报告（issue #150）

> 本文如实记录 2026-11-14 在本机与两个真实 Unity 项目上做的 v0.1 发布候选验收。
> 写法沿用 [M1](M1_ACCEPTANCE_REPORT.md) / [M2](M2_ACCEPTANCE_REPORT.md) / [M3](M3_ACCEPTANCE_REPORT.md)
> 三份验收报告：**没做的事一律写进 §8 的人工清单，不臆造结论、不把「应该没问题」记成通过**。
>
> **一句话结论先说**：**现在还不能判「可以发布 v0.1.0」**。
> 代码侧的发布门槛 1、6 已经达成，门槛 4 的 `none` 一半有实测；
> 门槛 3（干净虚拟机安装 → 自动更新 → 卸载）**一条都没做**，而且里程碑「v0.1 首发」里
> 还有 8 个跟进 issue 没关闭（§9）。阻塞项清单见 §9，人工事项见 §8。
>
> 本次**真正落地的东西**是：`perf/budgets.toml` 的 4 条 `pending` 全部清零（两条补了埋点与实测、
> 两条是重复占位，合并到已有条目），新增「可交互」与「空窗口内存」的自动采集、
> 一次会话的网络监视命令，以及 8 条叠加场景的接缝测试（§2、§5、§6）。

## 1. 环境

| 项 | 值 |
|---|---|
| CPU | Intel Core i7-11700 @ 2.50 GHz，8 核 16 线程 |
| 内存 | 47.7 GB |
| 系统 | Windows 11 专业版 10.0.22631 |
| 显卡 / 显示 | NVIDIA GeForce GTX 1060 6GB，1920×1080 @ 120 Hz，缩放 100%，**没有混合 DPI 环境** |
| 机器负载 | 验收期间本机同时开着 Unity 编辑器与另外的构建槽，**负载较高**；所有耗时与内存只作相对参考，空闲机器复测见 §8 |
| 构建 | stable MSVC（`rust-toolchain.toml`）；本机 release 构建**关掉了 LTO**（`CARGO_PROFILE_RELEASE_LTO=off`，内存限制），所以体积数字不是发布口径（§6.4） |
| 工作区版本 | `Cargo.toml` 的 `[workspace.package] version = "0.0.1"`——**打 `v0.1.0` 之前必须先改成 `0.1.0`**，否则 `cargo xtask check-version --tag v0.1.0` 会失败（§9 第 7 条） |
| 测试项目 | 中型 / 大型两个真实 Unity 项目，路径只来自环境变量 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`，**不写进仓库** |
| C# 索引组件 | 本机**没有安装**（`ROVE_INDEXER_CS` 未设置、`%LOCALAPPDATA%\Rove\components\indexer-cs\` 为空）：涉及索引进程的实测沿用 M2 / M3 当天的数字并注明未复测 |

## 2. 功能覆盖逐行核对（COVERAGE.md 中「v0.1」= 是）

一共 **57 行**，逐行结论都是**已完成**，没有「改判」。「验证方式」里写的是这次真的跑过、
或在 M1–M3 验收报告里已经记过的测试；本次新增的测试用 **本次新增** 标出。

> 这张表**不是手抄的**：`cargo xtask check-release-gate`（新加，见 §7）会把
> COVERAGE.md 中「v0.1 = 是」的每一行和本节的表格逐行对照，漏一行、或者结论不是
> 「已完成 / 改判」就以非零码退出；`cargo test -p xtask` 里也有同一条判定，
> 以后 COVERAGE 改了而报告没跟上，CI 直接会红。

| 章节 | 功能 | 负责 issue | 验证方式 | 结论 |
|---|---|---|---|---|
| §2 | 标题栏（自定义标题栏 + 命令中心） | R16 工作台（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §2 | 活动栏 | R16 工作台（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §2 | 主侧边栏 | R16 工作台（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §2 | 编辑区（编辑器组网格） | R17 编辑器组与标签（M1） | `cargo test -p rove-ui editor_tests` | 已完成 |
| §2 | 面板（底部） | R16 工作台（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §2 | 状态栏 | R16 工作台（M1）、R27 状态栏条目（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §2 | 通知 | R16 工作台（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §3 | 语法高亮（tokenization） | R09 tree-sitter 集成（M1）、R20 代码视图（M1） | `cargo test -p rove-syntax`（含 `--test golden_colors` 着色黄金测试） | 已完成 |
| §3 | 行号、当前行高亮、空白字符显示、缩进参考线 | R19 代码视图 Element（M1）、R20 代码视图（M1） | `cargo test -p rove-ui --test code_view`、`--test code_view_dpi` | 已完成 |
| §3 | 括号匹配（bracketMatching） | R20 代码视图（M1） | `cargo test -p rove-ui --test code_view_features` | 已完成 |
| §3 | 代码折叠（folding） | R20 代码视图（M1） | `cargo test -p rove-ui --test code_view_features` | 已完成 |
| §3 | 粘性滚动（stickyScroll） | R54 粘性滚动（M4） | `cargo test -p rove-ui --test code_view_sticky`；本次新增 `v01_sticky_scroll_agrees_with_the_breadcrumb_while_soft_wrap_is_on` 与两条分级关闭 | 已完成 |
| §3 | 面包屑 | R26 大纲（M1） | `cargo test -p rove-ui outline_tests`；本次新增的「粘性区 = 面包屑符号链」断言 | 已完成 |
| §3 | 同名高亮（wordHighlighter） | R20 代码视图（M1）、R40 C# 导航交互（M2） | `cargo test -p rove-ui --test code_view_features` | 已完成 |
| §3 | 悬停（hover） | R40 C# 导航交互（M2） | `cargo test -p rove-ui nav_tests`、`nav_cases_tests`；[M2 验收报告](M2_ACCEPTANCE_REPORT.md) §3 | 已完成 |
| §3 | 软换行（word wrap） | R57 软换行（M4） | `cargo test -p rove-ui --test code_view_wrap`；本次新增 `v01_*`（软换行 + 粘性滚动 + 编辑叠加） | 已完成 |
| §3 | 缩放 | R16 工作台（M1）、R19 代码视图 Element（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §3 | 长行处理（longLinesHelper） | R22 超大文件与超长行的界面（M1） | `cargo test -p rove-ui large_file_tests`；本次新增 `v01_a_huge_document_switches_off_sticky_scroll_and_soft_wrap_together` | 已完成 |
| §3 | 只读提示（readOnlyMessage） | R20 代码视图（M1） | `cargo test -p rove-ui --test code_view_features` | 已完成 |
| §4 | 基本输入、IME、剪贴板 | R46 代码视图的编辑交互与输入法（M3）、R19 代码视图 Element（M1） | `cargo test -p rove-ui --test code_view_edit`（输入法走协议层）；实机输入法矩阵见 §8 | 已完成 |
| §4 | 撤销 / 重做、光标撤销（cursorUndo） | R45 编辑核心模型（M3）、R46 代码视图的编辑交互与输入法（M3） | `cargo test -p rove-buffer`（编辑核心模型与撤销栈） | 已完成 |
| §4 | 多光标（multicursor） | R51 多光标与行操作（M4） | `cargo test -p rove-ui --test code_view_multicursor`；本次新增 `v01_multicursor_line_edit_with_wrap_on_undoes_as_one_unit` | 已完成 |
| §4 | 行操作（linesOperations） | R51 多光标与行操作（M4） | `cargo test -p rove-ui --test code_view_multicursor`；本次新增 `v01_multicursor_line_edit_with_wrap_on_undoes_as_one_unit` | 已完成 |
| §4 | 单词操作（wordOperations、wordPartOperations） | R45 编辑核心模型（M3）、R46 代码视图的编辑交互与输入法（M3） | `cargo test -p rove-buffer`（编辑核心模型与撤销栈） | 已完成 |
| §4 | 注释切换（comment） | R52 注释切换、自动缩进、自动闭合、包围、回车规则（M4） | `cargo test -p rove-ui --test code_view_autoedit`；本次新增 `v01_comment_toggle_on_a_gbk_file_saves_with_no_extra_difference` | 已完成 |
| §4 | 自动缩进、缩进转换（indentation） | R52 注释切换、自动缩进、自动闭合、包围、回车规则（M4）、R46 代码视图的编辑交互与输入法（M3） | `cargo test -p rove-ui --test code_view_autoedit`；本次新增 `v01_comment_toggle_on_a_gbk_file_saves_with_no_extra_difference` | 已完成 |
| §4 | 自动闭合括号与引号 | R52 注释切换、自动缩进、自动闭合、包围、回车规则（M4） | `cargo test -p rove-ui --test code_view_autoedit`；本次新增 `v01_comment_toggle_on_a_gbk_file_saves_with_no_extra_difference` | 已完成 |
| §4 | 末尾自动加换行、去除行尾空格 | R48 保存与外部修改（M3） | `cargo test -p rove-buffer --test m3_roundtrip`、`cargo test -p rove-ui m3_`；[M3 验收报告](M3_ACCEPTANCE_REPORT.md) §3–§5 | 已完成 |
| §4 | 查找 / 替换（find） | R21 当前文件查找条（M1）、R47 当前文件替换（M3） | `cargo test -p rove-ui find_tests` | 已完成 |
| §5 | 转到定义 / 声明 / 类型定义（gotoSymbol） | R40 C# 导航交互（M2）、R59 转到类型定义、工作区符号、侧边引用视图（M4） | `cargo test -p rove-ui nav_tests`、`nav_cases_tests`；[M2 验收报告](M2_ACCEPTANCE_REPORT.md) §3 | 已完成 |
| §5 | 查找所有引用 | R41 速览窗口（M2）、R59 转到类型定义、工作区符号、侧边引用视图（M4） | `cargo test -p rove-ui peek_tests`；[M2 验收报告](M2_ACCEPTANCE_REPORT.md) §4 | 已完成 |
| §5 | 速览（peekView：速览定义、速览引用、速览实现） | R41 速览窗口（M2） | `cargo test -p rove-ui peek_tests`；[M2 验收报告](M2_ACCEPTANCE_REPORT.md) §4 | 已完成 |
| §5 | 转到实现 | R40 C# 导航交互（M2） | `cargo test -p rove-ui nav_tests`、`nav_cases_tests`；[M2 验收报告](M2_ACCEPTANCE_REPORT.md) §3 | 已完成 |
| §5 | 转到文件中的符号（`Ctrl+Shift+O`） | R26 大纲（M1） | `cargo test -p rove-ui outline_tests`；本次新增的「粘性区 = 面包屑符号链」断言 | 已完成 |
| §5 | 转到工作区中的符号（`Ctrl+T`） | R59 转到类型定义、工作区符号、侧边引用视图（M4） | `cargo test -p rove-ui nav_extra_tests`、`references_tests`（`Ctrl+T` 与快速打开 `#` 逐条断言同一份数据） | 已完成 |
| §5 | 转到行（`Ctrl+G`） | R24 快速打开与命令面板（M1） | `cargo test -p rove-ui quick_access_tests` | 已完成 |
| §5 | 导航历史（前进、后退、上次编辑位置） | R17 编辑器组与标签（M1）、R40 C# 导航交互（M2） | `cargo test -p rove-ui editor_tests` | 已完成 |
| §5 | 大纲视图（outline） | R26 大纲（M1） | `cargo test -p rove-ui outline_tests`；本次新增的「粘性区 = 面包屑符号链」断言 | 已完成 |
| §5 | 语言状态（languageStatus） | R27 状态栏条目（M1）、R36 索引生命周期与状态（M2） | `cargo test -p rove-ui status_tests` | 已完成 |
| §6 | 资源管理器（files） | R23 资源管理器（M1） | `cargo test -p rove-ui explorer_tests` | 已完成 |
| §6 | 文件排除（`files.exclude`） | R15 工作区（M1） | `cargo test -p rove-workspace`；[M1 验收报告](M1_ACCEPTANCE_REPORT.md) §3 | 已完成 |
| §6 | 文件监听与自动重新加载 | R15 工作区（M1）、R48 保存与外部修改（M3） | `cargo test -p rove-workspace`；[M1 验收报告](M1_ACCEPTANCE_REPORT.md) §3 | 已完成 |
| §6 | 快速打开（quickaccess：`Ctrl+P`） | R24 快速打开与命令面板（M1） | `cargo test -p rove-ui quick_access_tests` | 已完成 |
| §6 | 命令面板（`Ctrl+Shift+P`） | R24 快速打开与命令面板（M1） | `cargo test -p rove-ui quick_access_tests` | 已完成 |
| §6 | 全局搜索（search） | R25 全局搜索（M1） | `cargo test -p rove-search`、`cargo test -p rove-ui search_tests` | 已完成 |
| §6 | 欢迎页 / 最近打开 | R28 欢迎页、最近打开、启动参数打开，以及首选项入口（M1） | `cargo test -p rove-ui welcome_tests` | 已完成 |
| §6 | 热退出（关闭时保留未保存内容，下次恢复） | R49 热退出与崩溃恢复（M3） | `cargo test -p rove-ui hotexit_tests` | 已完成 |
| §7 | 颜色主题（themes） | R10 主题引擎（M1）、R68 只读加载 VS Code 扩展主题、Seti 文件图标主题（M4） | `cargo test -p rove-theme`、`cargo test -p rove-syntax --test golden_colors`。扩展主题与 Seti 文件图标属 M4（v0.2），v0.1 只做内置主题 | 已完成 |
| §7 | 产品图标（codicons） | R16 工作台（M1） | `cargo test -p rove-ui shell::` 与各 `*_host` 装配测试；骨架端到端 `cargo test -p rove-app` | 已完成 |
| §7 | 设置（`settings.json`） | R13 设置层（M1） | `cargo test -p rove-config` | 已完成 |
| §7 | 键位（keybindings） | R14 键位与命令层（M1） | `cargo test -p rove-commands`；`python scripts/compat/gen_default_keybindings.py --check` | 已完成 |
| §7 | 自动更新（update） | R76 自动更新（M5） | `cargo test -p rove-update`、`cargo test -p rove-update --test rehearsal`；本次 `cargo xtask netwatch --mode none`（见 §4） | 已完成 |
| §9 | Markdown 预览（侧边预览、滚动同步） | R29 Markdown 预览（M1） | `cargo test -p rove-ui markdown_tests`；真实 WebView2 由 `markdown_probe` 验收（§8 人工事项） | 已完成 |
| §9 | Markdown 大纲、链接跳转 | R26 大纲（M1）、R71 链接检测与打开（M4）、R29 Markdown 预览（M1） | `cargo test -p rove-ui outline_tests`；本次新增的「粘性区 = 面包屑符号链」断言。链接检测属 M4（v0.2） | 已完成 |
| §9 | JSON：高亮、折叠、校验、格式化 | R09 tree-sitter 集成（M1）、R60 JSON/YAML/XML（M4） | `cargo test -p rove-syntax`（含 `--test golden_colors` 着色黄金测试）。JSON/YAML/XML 的校验、格式化与结构树属 M4（v0.2），v0.1 只做高亮与折叠 | 已完成 |
| §9 | YAML、XML：高亮、折叠、校验 | R09 tree-sitter 集成（M1）、R60 JSON/YAML/XML（M4） | `cargo test -p rove-syntax`（含 `--test golden_colors` 着色黄金测试）。JSON/YAML/XML 的校验、格式化与结构树属 M4（v0.2），v0.1 只做高亮与折叠 | 已完成 |
| §9 | 首批编程语言：C#（完整的语言智能） | R09 tree-sitter 集成（M1）、R37 L1 快速声明索引（M2）、R38 查询路由与未保存修改的位置映射（M2）、R40 C# 导航交互（M2）、R41 速览窗口（M2） | `cargo test -p rove-syntax`（含 `--test golden_colors` 着色黄金测试） | 已完成 |

几点说明（不改变上面的结论）：

- 带两个负责 issue 的行里，第二个是 M4（v0.2）的参与方（扩展主题、链接检测、JSON/YAML/XML 的校验与
  格式化、语义高亮）：v0.1 的范围是**主负责 issue 的那一部分**，这一点与 `COVERAGE.md` 表头的口径一致。
- 「基本输入、IME、剪贴板」这一行的自动覆盖只到**输入法协议层**（`EntityInputHandler`：组合串、提交、
  取消、区间换算、候选框坐标）。真实 IMM32、候选框位置与屏幕边缘翻转**本次仍未在实机上跑**
  （M3 验收也没有跑），见 §8 人工事项 1。
- 「自动更新」这一行的自动覆盖是 `rove-update` 的单元测试与**本机 HTTP 服务器**上的演练
  （`--test rehearsal`），不是真实 GitHub Releases；真机升级见 §3。

## 3. 第 2 节的六个场景（两个 Unity 项目各走一遍）

能自动化的部分**本次都固化成了测试**，放在
`crates/rove-ui/src/skeleton/workbench/v01_acceptance_tests.rs`（用例名以 `v01_` 开头，
`cargo test -p rove-ui v01_ -j 1 -- --nocapture`）。其中两条真实项目用例分别跑中型与大型项目，
**只读取**真实 `.cs` 的字节、复制进临时工作区后操作，**一个字节都不往测试项目里写**。

| # | 场景 | 自动覆盖（本次） | 结论 | 仍需人工 |
|---|---|---|---|---|
| 1 | 多光标与行操作 | `v01_multicursor_line_edit_with_wrap_on_undoes_as_one_unit`：软换行开着时 `ctrl+d` → `ctrl+shift+l` → 一次输入改掉全部 → **一次 `ctrl+z` 整体回退**；逐键与 VS Code 的一致性由 #52 的 `code_view_multicursor`（含 `alt+点击`、`ctrl+alt+↑↓`、移动 / 复制 / 删除行）覆盖 | **通过**（自动部分） | 多光标下的**实机输入法**组合（§8 人工事项 1） |
| 2 | 注释切换 / 自动缩进 / 自动闭合与包围 | `v01_comment_toggle_on_a_gbk_file_saves_with_no_extra_difference`：GBK 文件上注释切换 → 保存，差异**恰好是 `// `**，再切回来与原文件**逐字节相同**；四种语言（C# / JSON / YAML / XML）的行为来自 `language-configuration.json`，由 #53 的 `code_view_autoedit` 覆盖 | **通过** | 四种语言的逐键手感对照（录屏，§8 人工事项 2） |
| 3 | 粘性滚动 | `v01_sticky_scroll_agrees_with_the_breadcrumb_while_soft_wrap_is_on`（粘性区级数与名字 = 面包屑符号链）、`v01_a_large_document_only_switches_off_sticky_scroll`、`v01_a_huge_document_switches_off_sticky_scroll_and_soft_wrap_together`，以及两个真实项目上的同一条断言 | **通过** | 点击粘性行跳转的视觉效果（录屏） |
| 4 | 软换行 | 上面三条里软换行全程开着；**坐标系**这条接缝单独钉住了：`set_scroll_position` 的 top 是屏幕行，`visual_line_of` 换算后才和文档行对得上（测试里断言两者确实不同）。列号 / 选区 / 插入符仍按文档坐标，查找命中与 F12 落点由 #58、#40 的用例覆盖 | **通过** | `alt+z` 的视觉效果与超长行横向滚动的手感（录屏） |
| 5 | 转到类型定义、`Ctrl+T`、侧边引用视图 | #60 的 `nav_extra_tests` / `references_tests`：类型定义覆盖 `var` / 字段 / 参数 / 属性 / 泛型参数五种写法，`Ctrl+T` 与快速打开 `#` **逐条断言是同一份数据与同一套排序**，结果都来自 M2 的查询路由 | **通过**（沿用 #60 的入口测试，本次未新增） | 大型项目上的人工抽查（§8 人工事项 6） |
| 6 | 1–5 与 M1–M3 叠加 | `v01_editing_with_sticky_and_wrap_on_goes_approximate_then_exact`：粘性滚动 + 软换行开着时编辑 → 查询标为**近似** → 保存 + 索引重绑 → **静默**回到精确、全程 0 通知、面包屑不乱、两项功能仍开着；两个真实项目上另有「粘性 + 软换行 + 编辑 + 保存，除改动处外逐字节不变」 | **通过**（功能接缝） | 「每帧 CPU 与按键到上屏仍在预算内」**本次没有在叠加状态下测**（§6.3、§8 人工事项 4） |

本次 8 条 `v01_` 用例全部通过（含中型与大型项目各一条）：

```text
running 8 tests ... test result: ok. 8 passed; 0 failed
```

## 4. 干净机器上的安装与自动更新：**本次一条都没做**

issue #150 §3 要求在干净的 Windows 10 / 11 虚拟机上走安装 → 使用 → 自动更新到 v0.1.1 → 卸载。
**本次没有做，一条都不能记成通过**，原因如实写明：

- 本机没有干净的 Win10 / Win11 快照虚拟机，也不具备「不装 .NET / WebView2 / VS Code」的环境；
- 演练需要发布一个 v0.1.1（哪怕是测试仓库），而本 issue 的约束是**不打 tag、不创建 Release**；
  `rove-update` 自己的演练口径也是「一律用本机 HTTP 服务器」（[UPDATE.md](UPDATE.md) §8.8）；
- SmartScreen 放行步骤、未保存修改跨更新保留、索引组件跟随升级这三条都要**看屏幕**才算数。

已自动覆盖的部分（不等于上面那几条）：`cargo test -p rove-update --test rehearsal`
在**本机 HTTP 服务器**上走完 v0.1.0 → v0.1.1 的检查、下载、验签、重启安装；
热退出（#50）保住未保存修改的能力由 `hotexit_tests` 覆盖。
两者加起来**仍然不能替代**干净机器上的一次真实升级。清单见 §8 人工事项 7–9。

## 5. 网络监视

### 5.1 `rove.update.mode = "none"`：**零出站连接**（本次实测）

新加的 `cargo xtask netwatch`（§7）在临时用户目录里写好 `rove.update.mode`，启动
release 版 `rove.exe` 打开一个真实 Unity 项目，每 500 ms 用 `netstat -ano` 采一次**整棵进程树**
的连接：

| 项目 | 时长 | 采样次数 | 进程树 | 连接表条目 | **出站连接** |
|---|---|---:|---|---:|---:|
| 中型 | 45 s | 39 | `rove.exe` | 0 | **0** |
| 大型 | 45 s | 35 | `rove.exe` | 0 | **0** |

- 口径：**连接表采样，不是 OS 级抓包**。两次采样之间建立又关闭的连接可能漏掉；
  需要更强的证据要用 `pktmon`（管理员权限），记在 §8 人工事项 10。
- 这次会话是「启动 → 打开项目 → 空闲」，**没有走完 §3 的六个场景**（那需要有人在屏幕前操作）。
- 本机没有安装 C# 索引组件，所以进程树里**只有 `rove.exe`**：索引进程与 WebView2 进程树的零连接
  **本次没有覆盖**（§8 人工事项 10）。

### 5.2 `rove.update.mode = "default"`：**本次没有测**

`default` 模式会在首帧之后真的去 `github.com` 检查更新。本次验收**不主动发起对外请求**，
所以这一半留给人工：`cargo xtask netwatch --mode default --seconds 90 --project <项目>`
跑一次，核对出现的远端地址只有 GitHub 的 HTTPS，且只在检查 / 下载时出现（§8 人工事项 11）。

### 5.3 编译期检查仍然生效

`cargo xtask check-deps`：依赖图里没有 `reqwest` / `hyper` / `rustls` / `ring` / `tokio`；
`cargo test -p xtask` 里的 `only_the_update_crate_opens_a_network_feature` 保证除 `rove-update`
外没有第二个 crate 打开 `Win32_Networking_*` feature。两条本次都跑过，**通过**。
`cargo test -p rove-ui --test no_network` 也照常通过。

## 6. 性能预算

### 6.1 `pending` 条目清零（发布门槛 5 的前半截）

| 原条目 | 原状态 | 本次处理 | 依据 |
|---|---|---|---|
| `cold_start_interactive`（#16，400 ms） | pending：「可交互」没有埋点 | **补埋点 + 实测 → enforced**，`bench = startup/interactive` | §6.2 |
| `empty_window_memory`（#16，60 MB） | pending：等工作台落地 | **补采集 + 实测 → enforced**，`bench = startup/empty_window_working_set` | §6.2 |
| `key_to_paint`（#46，8 ms） | pending：等 M3 的编辑交互 | **删除占位**：同一指标已经是 `keystroke_to_paint`（#47，8 ms、p99、`typing/key_to_frame`） | 数值未动 |
| `nav_end_to_end`（#40，50 ms） | pending：等导航交互与速览 | **删除占位**：同一指标已经是 `goto_definition_e2e`（#40，50 ms、p99、`nav/f12_to_paint`） | 数值未动 |

**没有为了让检查通过改任何预算数值**。两条删除的是「同一个指标的占位条目」，
budgets.toml 里原地留了注释说明来龙去脉。另外给 `csharp_incremental_5000_lines` 补了缺失的 `reason`
（只补文字，数值与 bench 未动）。

现在 `cargo test -p xtask` 里有三条守卫：没有 `pending`、`enforced` 必须指得出测量来源、id 不重复。

### 6.2 新测到的两个数字（本机，负载较高）

`cargo xtask startup --startup-tries 5`（release 版 `rove.exe`，临时用户目录里 `rove.update.mode = none`）：

| 指标 | 实测（中位数） | 范围 | 预算 | 结论 |
|---|---:|---|---|---|
| 冷启动到首帧（`startup/first_frame`） | **309.5 ms** | 304.0 – 403.7 ms（n=5） | 只记录 | 记录（M0 当年 396 ms） |
| 冷启动到**可交互**（`startup/interactive`） | **323.9 ms** | 316.5 – 419.3 ms（n=5） | **< 400 ms** | **达标** |
| 空窗口常驻内存：**工作集** | **54.9 MB** | 54.7 – 56.7 MB（n=3） | **< 60 MB** | **达标** |
| 空窗口常驻内存：私有提交 | 121.6 MB | 115.7 – 121.6 MB | 只记录 | **见下面的提醒** |

- 「可交互」的口径：首帧画完后立刻经 `Window::dispatch_keystroke` 注入一个**无副作用的按键**
  （`right`），等包含这次响应的那一帧画完。**不含 Windows 键盘消息的排队时间**（与 #47 的
  `typing/key_to_frame` 同口径）。埋点默认关闭，只有 `ROVE_STARTUP_PROBE=1` 时才注入，
  正常使用的 `rove.exe` 既不会多一次按键，也不会多打印一行。
- 「可交互 − 首帧」= **14.4 ms**（中位数差），也就是一次按键派发加一帧绘制；
  本机 120 Hz 一帧 8.33 ms，这个数量级是合理的。
- **本次改动不碰任何热路径**：埋点默认关闭，首帧回调里只多一次 `env::var_os` 判断，
  其余改动都在 `xtask` 里。首帧中位数 309.5 ms 与 #149 当天记录的 299.3 ms（同一台机器、同样负载较高）
  在同一量级，没有「优化前后」可比的对象，因此不做前后对比表。
- **提醒**：CHARTER §6 只写「空窗口常驻内存 < 60 MB」，没有写是工作集还是私有提交。
  这里按**工作集**判定（M0 的 47.7 MB 也是工作集，才能比较），私有提交 121.6 MB 另行记录。
  私有提交比工作集高一倍多，口径该怎么定已开跟进 issue #219（§7.2）。

`cargo xtask bench --from <startup 结果> --check-budgets` 的输出：`pending 0 条`，
两条新判定都是 OK，没有超预算项（其余 28 条本次未测量，不算失败）。

### 6.3 其余预算：**本次没有在空闲机器上重跑完整基准**

完整的 `cargo xtask bench --check-budgets` 需要几十分钟、几 GB 内存，本机负载较高，
跑出来的绝对值没有判定价值；按本次的约束记为人工事项（§8 人工事项 3）。
已知的两处**没有被本次改动影响**、但仍然挂在里程碑上的性能问题：

- **#210**：大型 Unity 项目全量扫描 2.83 s，超 `workspace_full_scan_large` 预算 182%——
  这是一条 `enforced` 预算的**实测超限**，门槛 5 的「每一条都达标」因此**还没有满足**；
- **#215**：按键到上屏 p99 12.4 ms（预算 8 ms，`recorded`）。M3 验收的结论是
  「Rove 自己只花 0.35 ms，其余是刷新间隔，按现口径结构上到不了 8 ms」，要么改口径、要么改采样相位。

### 6.4 体积：本机构建**超出发布包预算 4.3%**，但口径不对

`cargo xtask check-size`：`rove.exe` 29.83 MiB、发布包 29.85 MiB，**超过 30,000,000 B 的预算 4.34%**
（命令以 1 退出）。**但本机的 release 构建关掉了 LTO**（内存限制，见 §1），
发布口径的构建是开 LTO 的，体积会明显小于这个数。
所以这一条**既不能判通过、也不能判超限**，必须在开 LTO 的构建上复测（§8 人工事项 5）。

## 7. 本次改了什么（接缝与工具）

| 改动 | 位置 | 为什么 |
|---|---|---|
| 「可交互」埋点 | `crates/rove-ui/src/skeleton/workbench/startup_probe.rs`（+ `workbench.rs` 首帧回调里一行调用） | `cold_start_interactive` 缺的就是这半截；默认关闭，`ROVE_STARTUP_PROBE=1` 才生效 |
| 冷启动 / 空窗口内存采集 | `xtask/src/startup.rs`、新模块 `xtask/src/procwatch.rs` | 两条 pending 预算要有可重复的测量路径；内存走 `Get-CimInstance Win32_Process` 的进程树合计 |
| 预算判定支持非时间指标 | `xtask/src/budgets.rs`、`xtask/src/results.rs`（新增 `metrics` 表） | 内存类预算以前「用 bench 判定」会直接报错，只能挂着 pending |
| 一次会话的网络监视 | 新命令 `cargo xtask netwatch`（`xtask/src/netwatch.rs`） | R77 §4 的 `none` 零连接要有可重复的采集；[UPDATE.md](UPDATE.md) §9 把这件事点名留给了本 issue |
| 发布门槛自检 | 新命令 `cargo xtask check-release-gate`（`xtask/src/release_gate.rs`）+ `cargo test -p xtask` 里的同一条判定 | 让「COVERAGE 的每一行都在报告里有结论」「预算没有 pending」变成机器可查，而不是人工抄表 |
| 叠加场景的接缝测试 | `crates/rove-ui/src/skeleton/workbench/v01_acceptance_tests.rs`（8 条） | §3 的场景 1–4、6 里可自动化的部分 |

### 7.1 验收中发现的一处不一致（已开 issue，本 PR 不改行为）

把插入符放在**方法声明行的行首缩进**里时，面包屑不显示这个方法（只到类），
而粘性滚动已经把这一行钉上去了——两者对「光标在哪个作用域」的判定不是同一套
（面包屑按位置含列判定，粘性区按行判定）。
写测试时绕开的办法是把插入符放到声明行的**第一个非空白字符**上，两边就完全一致。
这属于 #26 与 #55 之间的口径问题，改动会影响面包屑的既有用例，**不在本 PR 里顺手改**，
已开 **#218**（里程碑「v0.1 首发」）。

### 7.2 本次开出的跟进 issue

| issue | 内容 | 为什么不在本 PR 里做 |
|---|---|---|
| **#218** | 面包屑与粘性滚动对「光标在哪个作用域」的判定不一致（声明行行首缩进） | 改动会动到 #26 面包屑的既有用例，属于口径决定（§7.1） |
| **#219** | 空窗口私有提交 121.6 MB 远高于工作集 54.9 MB：确认「常驻内存」的判定口径 | CHARTER §6 没写是哪一个口径，要走章程流程确认（§6.2） |

## 8. 人工事项清单（本次没有做，不算通过）

1. **实机输入法矩阵**：`spikes/ime_test.ps1` 的全部场景（含多光标下的组合输入），判据要看截图。
   M3 验收也没有跑，这一条从 M3 一直挂到现在。
2. **逐键对照 VS Code**：场景 1 / 2 的手感与键位（录屏或截图），四种语言各一例。
3. **空闲机器上的完整基准**：`cargo xtask bench --check-budgets` 与 `cargo xtask startup`，
   与 M1–M3 的基线对照。本机负载较高，绝对值不作判定。
4. **叠加状态下的帧预算**：粘性滚动 + 软换行 + 编辑 + 索引增量更新同时进行时的每帧 CPU 与
   按键到上屏（场景 6 的性能判据），本次只验了功能接缝。
5. **开 LTO 的构建上复测体积**：`cargo xtask check-size`（§6.4）。
6. **大型项目上的导航抽查**：转到类型定义 / `Ctrl+T` / 引用视图在真实大项目上的人工抽查，
   以及 `nav_check` 的两条基线复跑（需要先建索引）。
7. **干净 Windows 10 / 11 虚拟机上的安装**：SmartScreen 放行步骤、首次启动、打开 Unity 项目、
   读代码 / 跳转 / 改代码 / 保存，并**留一处未保存的修改**。
8. **真实自动更新演练**：发布一个演练用 v0.1.1（测试仓库）→ 自动检查、下载、提示 → 重启更新 →
   版本号、未保存的修改与会话原样恢复、C# 索引组件同步升级。
9. **卸载**：程序目录、快捷方式、注册表项清除；用户数据按选项保留或删除。
10. **网络监视补全**：走完六个场景的完整会话；装上 C# 索引组件后覆盖索引进程与 WebView2 进程树；
    必要时用 `pktmon` 做 OS 级抓包。
11. **`rove.update.mode = "default"` 的监视**：`cargo xtask netwatch --mode default`，
    核对只出现到 GitHub 的 HTTPS 连接。
12. **多显示器与高 DPI**：[DPI_CHECKLIST.md](DPI_CHECKLIST.md) 的清单（本机没有混合 DPI 环境）。
13. **真实 WebView2 的 Markdown 预览**：`cargo run -p rove-ui --example markdown_probe`（会弹窗约 45 秒）。

## 9. 对发布门槛的逐条结论（[MILESTONE_v0.1.md](MILESTONE_v0.1.md) §5）

| # | 门槛 | 结论 |
|---|---|---|
| 1 | COVERAGE「v0.1 = 是」的行全部完成或改判 | **达成**：57 行全部「已完成」（§2），并有机器可查的对照 |
| 2 | 两个 Unity 项目上的端到端场景通过 | **部分达成**：可自动化的部分全部通过（§3，8 条用例、两个项目各一条）；逐键手感、录屏与实机输入法仍是人工事项 |
| 3 | 干净 Win10 / Win11 上安装 → 更新到 v0.1.1 → 卸载 | **未达成**（§4）：本次一条都没做 |
| 4 | `none` 零连接、`default` 只有 GitHub | **一半达成**：`none` 两个项目各 45 s 实测 0 条出站连接（§5.1）；`default` 未测（§5.2） |
| 5 | `perf/budgets.toml` 没有 pending、`--check-budgets` 通过 | **部分达成**：pending 已清零、本次测到的两条达标（§6.1、§6.2）；但 **#210 是一条 enforced 预算的实测超限**，空闲机器上的完整基准也还没跑（§6.3） |
| 6 | `cargo test --workspace` 全绿 | **达成**：`cargo test -p rove-ui -j 1`（744 + 各集成测试，0 失败）与 `cargo test --workspace --exclude rove-ui -j 1`（64 个测试二进制，0 失败）都通过，#2 的骨架端到端测试继续通过；`cargo fmt --all --check`、`cargo clippy -p rove-ui / -p xtask --all-targets -- -D warnings`、`cargo xtask check-deps`、`check-eol.ps1`、`check-core-crates.ps1` 也都通过 |

**阻塞项**（这些不解决就不能打 `v0.1.0`）：

1. 里程碑「v0.1 首发」里还有 **8 个跟进 issue 未关闭**：#209、#210、#212、#213、#215、#216，
   外加本次开出的 #218、#219。其中 **#210 直接让门槛 5 不成立**。
   （issue #150 的前提是「开工前里程碑中的跟进 issue 都已关闭」，这个前提**本次并不成立**。）
2. 门槛 3 的干净机器安装、自动更新、卸载（§4、§8 人工事项 7–9）。
3. 门槛 4 的 `default` 模式监视（§5.2）。
4. 场景 6 的性能判据（§8 人工事项 4）与空闲机器上的完整基准（§8 人工事项 3）。
5. 实机输入法矩阵（§8 人工事项 1）——从 M3 一直挂到现在。
6. 开 LTO 的构建上复测发布包体积（§6.4）。
7. 发布前把 `Cargo.toml` 的 `[workspace.package] version` 从 `0.0.1` 改成 `0.1.0`
   （`cargo xtask check-version --tag v0.1.0` 会卡住不一致）。

上面 7 条逐条清掉之后，这份报告的结论才能改写成「**可以发布 v0.1.0**」。
