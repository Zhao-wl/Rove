# R69 Zen 模式、全屏、居中布局

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 1 简单 | ui | 第 4 批 | R16（工作台） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

`FEATURES.md` §2 把「全屏 / Zen 模式 / 居中布局」判定为照搬、P1，理由只有一句：**阅读场景中有用**。Rove 的定位就是读代码，所以这三件事的优先级比在 VS Code 里更高一些，但实现量很小——它们都是对 R16（工作台） 已有布局的重新摆放，不引入任何新数据。

`docs/compat/KEYBINDINGS.md`「布局」一节已经把三条键位判定为 P1：`ctrl+k z`（`workbench.action.toggleZenMode`，when `!isAuxiliaryWindowFocusedContext`）、`escape escape`（`workbench.action.exitZenMode`，when `inZenMode`）、`f11`（`workbench.action.toggleFullScreen`，when `!isIOS`），另外 §7 附录「P1 · 布局」中有 `workbench.action.toggleCenteredLayout`（无默认键位）。R16（工作台） 与 R17（编辑器组与标签） 都已明确把这几条留给本 issue。

必读：`FEATURES.md` §2（全屏 / Zen / 居中布局一行）；`UX_SPEC.md` §1.1（整体布局）、§6.1（通知与勿扰模式）；`docs/compat/KEYBINDINGS.md`「布局」一节与 §7 附录「P1 · 布局」；`docs/compat/SETTINGS.md`「布局」一节（8 个 `zenMode.*`）；`docs/M0_UI_SPIKE_REPORT.md` §5（窗口与 DPI 相关的已知坑）。

## 目标

### 1. 全屏（`workbench.action.toggleFullScreen`，`f11`）

- 通过 gpui 的窗口接口切换无边框全屏；自绘标题栏（R16（工作台））在全屏下按 VS Code 的行为隐藏，鼠标移到顶部时不自动浮出（VS Code 在 Windows 上也不浮出）。
- 退出全屏后**恢复进入前的窗口位置与大小**（包括最大化状态）。
- 多显示器：在窗口当前所在的那块屏上全屏；跨不同缩放比的显示器时按 R30（高 DPI 与多显示器验证与修复） 已有的处理重新布局，本 issue 不自己换算 DPI。

### 2. Zen 模式（`workbench.action.toggleZenMode`，`ctrl+k z`）

进入时按 8 个设置逐项处理，默认值与 VS Code 一致：

| 设置 | 默认 | 行为 |
|---|---|---|
| `zenMode.fullScreen` | `true` | 进入 Zen 时同时进入全屏 |
| `zenMode.centerLayout` | `true` | 同时开启居中布局 |
| `zenMode.hideActivityBar` | `true` | 隐藏活动栏 |
| `zenMode.hideStatusBar` | `true` | 隐藏状态栏 |
| `zenMode.hideLineNumbers` | `true` | 隐藏编辑器行号（覆盖 `editor.lineNumbers`，退出后还原） |
| `zenMode.showTabs` | `"multiple"` | 多标签 / 单标签 / 完全隐藏编辑器标题区 |
| `zenMode.silentNotifications` | `true` | 等效于开启勿扰模式：只弹错误级通知（`UX_SPEC.md` §6.1） |
| `zenMode.restore` | `true` | 在 Zen 模式下退出的窗口，下次启动恢复 Zen 模式 |

- 侧边栏与面板在进入 Zen 时隐藏，**退出后还原到进入前的状态**（宽度、可见性、活动视图）——这份快照由本 issue 保存，不去改 R16（工作台） 的布局持久化。
- `escape escape`（`workbench.action.exitZenMode`）退出；上下文键 **`inZenMode`** 通过 R01（【接口先行 A1】架构与接口契约） 的上下文键机制登记，使 `keybindings.json` 里带 `inZenMode` 的自定义键位能生效。
- `zenMode.restore` 的状态随 R16（工作台） 的布局持久化一起按工作区保存（往它已有的登记项里加一个字段，不新开存储）。

### 3. 居中布局（`workbench.action.toggleCenteredLayout`）

- 编辑器组整体居中，两侧留白，留白宽度与 VS Code 一致，并随窗口宽度变化；留白区域使用 `editorGroup` 相关的已登记颜色键，不自造颜色。
- 与分屏的配合照搬 VS Code：多个编辑器组时居中区域包含整个网格，不是每组各自居中。
- 独立于 Zen 模式：可以单独开关，也可以由 `zenMode.centerLayout` 带起来；两者同时开启时退出 Zen 只还原由 Zen 带起的那一次。

### 4. 登记与替换

四条命令、上下文键 `inZenMode`、8 个设置项的读取、界面字符串全部通过 R01（【接口先行 A1】架构与接口契约） 与 R16（工作台） 的登记机制加入，不得写死。R02（【接口先行 A2】行走骨架） 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增自己的一行。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口；布局区域的显示 / 隐藏通过 R16（工作台） 已有的接口调用，不在 R16（工作台） 的代码里加分支。

## 不包含

- 工作台各区域本身与布局持久化（R16（工作台））；编辑器组网格与标签（R17（编辑器组与标签））。
- P0 的单项显隐命令：`workbench.action.toggleSidebarVisibility`（`ctrl+b`）、`togglePanel`（`ctrl+j`）、`toggleActivityBarVisibility`、`toggleStatusbarVisibility`、`toggleMenuBar`（全部属于 R16（工作台））。
- 辅助侧边栏与视图拖动重排（P2）；`workbench.action.toggleMaximizedPanel`（R16（工作台））。
- 高 DPI 与多显示器的系统性验证（R30（高 DPI 与多显示器验证与修复））；本 issue 只保证全屏切换后布局正确。
- 勿扰模式本身（`notifications.toggleDoNotDisturbMode`，R16（工作台））；本 issue 只在 Zen 模式下调用它。

## 验收标准

- [ ] `f11` 进入 / 退出全屏；退出后窗口位置、大小、最大化状态与进入前完全一致
- [ ] 在副显示器上（与主屏不同缩放比）全屏时占满该屏，布局与命中测试正确（截图）
- [ ] `ctrl+k z` 进入 Zen 模式：活动栏、状态栏、侧边栏、面板、行号按默认值全部隐藏，编辑器居中并全屏
- [ ] 8 个 `zenMode.*` 设置逐项验证：每项分别设为非默认值后行为符合该项说明（8 条断言）
- [ ] `escape escape` 退出 Zen；`inZenMode` 为 false 时 `escape escape` 不触发（证明 `when` 生效）
- [ ] 退出 Zen 后侧边栏宽度、可见性、活动视图、面板高度、行号设置全部还原
- [ ] `zenMode.silentNotifications=true` 时信息级通知不弹出、错误级仍然弹出
- [ ] `zenMode.restore=true` 时在 Zen 模式下关闭窗口，重开后仍在 Zen 模式；设为 false 时不恢复
- [ ] `workbench.action.toggleCenteredLayout` 单独开关正常；居中布局下**鼠标点击命中正确的字符位置**（坐标偏移回归用例）
- [ ] 居中布局 + 两个编辑器组时，整个网格居中而不是每组各自居中
- [ ] Zen 模式下命令面板、快速打开、悬停卡片、速览窗口的浮层定位正确（截图）
- [ ] 在 `keybindings.json` 中给 `workbench.action.toggleZenMode` 换一个键位后生效
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增「进入 Zen → 断言各区域可见性 → 退出 → 断言还原」的集成测试
- [ ] `docs/ARCHITECTURE.md` 的对照表已更新；本 PR 没有修改 R01（【接口先行 A1】架构与接口契约） 的接口

## 风险与提示

- **「还原」比「隐藏」难**。进入 Zen 时要保存的是一份完整快照（哪些区域可见、宽高、活动视图、`editor.lineNumbers` 的原值），而且中途用户可能手工开了侧边栏再退出 Zen。照搬 VS Code 的做法：Zen 期间的手工改动在退出时被快照覆盖。
- 居中布局最容易踩的坑是**命中测试与浮层定位的坐标系**：编辑器被整体平移后，如果某处还在用窗口坐标算字符位置，点击就会错位。验收标准里专门列了这一条，先写用例再写实现。
- 全屏切换在 Windows 上会触发 DPI 变化事件（跨屏全屏时），`M0_UI_SPIKE_REPORT.md` 把高 DPI 列为未充分验证项。遇到 gpui 层面的缺陷，按 R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随） 的补丁流程改 `vendor/gpui` 并记进 `ROVE_PATCHES.md`，**不要在 Rove 侧绕过去**。
- 这是难度 1 的 issue，范围要守住：不要顺手改 R16（工作台） 的区域显隐实现，也不要加 VS Code 没有的「专注模式」。
