# R05 内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 4 专家 | ui | 第 1 批 | 无 | R31（【集成验收】M1）、R46（代码视图的编辑交互与输入法）、R73（精简 GPUI 的网络依赖） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

M0 定案使用 GPUI 0.2.2（`M0_UI_SPIKE_REPORT` §7），并附带"固定版本并准备 fork"这条约束。必须立刻 fork 的原因是一个真实缺陷：`gpui-0.2.2/src/platform/windows/window.rs:860` 把 `update_ime_position` 实现成了空函数，注释写着 "There is no such thing on Windows."——这个注释是错的。全仓库只有 `WM_IME_STARTCOMPOSITION` 一个调用点，也就是说**候选框只在组合开始时定位一次，整句输入时不会跟随插入符**（`M0_UI_SPIKE_REPORT` §5.4）。对中文用户这是硬伤。

本 issue 不依赖 R01（【接口先行 A1】架构与接口契约），与所有 issue 并行。R02（【接口先行 A2】行走骨架） 等界面 issue 可以先直接用 crates.io 的 gpui 开工，本 issue 合并后 `[patch]` 自动生效，它们不需要改代码。

## 目标

### 1. 内置 gpui 0.2.2

把 crates.io 上的 `gpui 0.2.2` 源码放进 `vendor/gpui`，在根 `Cargo.toml` 中用 `[patch.crates-io]` 指向它，保留原始 `LICENSE`（Apache-2.0，单一许可，不是 MIT/Apache 双授权）。沿用 M0 原型的 feature 配置：`default-features = false, features = ["windows-manifest"]`（关掉 default 是为了不引入 `font-kit` / `wayland` / `x11`；`windows-manifest` 会嵌入声明 `dpiAwareness = PerMonitorV2` 的清单，**必须显式打开**）。锁定 `Cargo.lock`。

新增 `vendor/gpui/ROVE_PATCHES.md`：逐条记录改了哪个文件、为什么改、原始代码是什么、升级 GPUI 时如何重新应用。

### 2. 实现 Windows 的 `update_ime_position`

- 在插入符区域变化时用 `ImmSetCompositionWindow`（`CFS_POINT`）+ `ImmSetCandidateWindow`（`CFS_CANDIDATEPOS` 或 `CFS_EXCLUDE`）更新位置；
- 在 `WM_IME_COMPOSITION` 中也调用一次定位，而不只是 `WM_IME_STARTCOMPOSITION`；
- 注意 DPI 换算：GPUI 的 `Pixels` 是逻辑像素，IMM32 要的是客户区物理像素，按窗口当前 scale factor 转换（PerMonitorV2 下跨显示器拖动时 scale factor 会变）；
- 位置来源仍然是 `bounds_for_range`（偏移量单位是 **UTF-16**，见 `M0_UI_SPIKE_REPORT` §5.4 的 API 对比表）。

### 3. 验证

沿用 `spikes/ime_test.ps1` 的思路（注入按键前校验前台窗口，不相等立即中止）：

- 连续输入一个超过 20 个字的长句，候选框始终跟随插入符；
- 窗口靠近屏幕右边缘和下边缘时，候选框能正确翻转，不被裁掉；
- 组合中途用鼠标点到别处、组合中途切换输入法，不崩溃、不留下孤立的组合串；
- 125% / 150% 缩放下位置正确（至少在一个非 100% 缩放的配置下测过；本机此前只有 100%，`M0_UI_SPIKE_REPORT` §8 第 1 项把高 DPI 列为未验证）。

## 不包含

- 编辑器本身的输入处理与组合态渲染（R46（代码视图的编辑交互与输入法））。
- 去掉 GPUI 强制链接的 HTTP/TLS 栈（R73（精简 GPUI 的网络依赖），它依赖本 issue 建立的 `vendor/` 目录，必须等本 issue 合并后再开工，否则会在同一目录上冲突）。
- 高 DPI 与多显示器的系统性验证（R30（高 DPI 与多显示器验证与修复））；本 issue 只保证候选框定位在非 100% 缩放下正确。
- 任何 R01（【接口先行 A1】架构与接口契约） 接口的改动：GPUI 的类型不得出现在核心层接口中。

## 验收标准

- [ ] `vendor/gpui` 能编译，workspace 通过 `[patch.crates-io]` 使用它，`cargo tree` 中 gpui 指向本地路径
- [ ] 长句（≥ 20 字）输入时候选框跟随插入符，附截图或录屏
- [ ] 屏幕边缘翻转、组合中途点击他处、组合中途切输入法三个场景通过
- [ ] 微软拼音实测通过；条件允许时再测一款第三方输入法（搜狗或微信输入法），结果如实记录（未测就写未测）
- [ ] `vendor/gpui/ROVE_PATCHES.md` 记录了全部改动与重新应用的步骤
- [ ] `THIRD_PARTY_NOTICES.md` 中声明 gpui 的 Apache-2.0 许可
- [ ] 替换前后的依赖包数、clean build 时间、`rove.exe` 体积对比（基线：411 个包 / 469 s / 6.2 MB）
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试在 patch 生效后继续通过

## 风险与提示

- **这是 fork 的起点，不是终点**。GPUI 在 crates.io 上只有 7 个 0.x 版本，API 不稳定（`M0_UI_SPIKE_REPORT` §6.1）。`ROVE_PATCHES.md` 的质量决定了下次升级的成本，补丁要尽量小、尽量集中，不要顺手重构 GPUI 的其他部分。
- 优先尝试把修复提交给上游（zed-industries/zed），上游合并后我们的补丁就能删掉。注意开发机访问 GitHub 受限（`M0_UI_SPIKE_REPORT` §2.2 坑 2），vendor 的源码取自 crates.io 而不是 git。
- `Pixels` 是 `f32`，大滚动偏移精度不足（1429 万行 × 20 px 时分辨率约 32 px）。这不是本 issue 要解决的问题，但不要在 vendor 里"顺手"改 `Pixels` 的类型——滚动模型用 f64 行号是 R19（代码视图 Element） 的职责。
