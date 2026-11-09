# R16 工作台：标题栏与命令中心、菜单、活动栏、侧边栏、面板、状态栏容器、布局持久化、启动预算，以及反馈分级（临时消息、提示条、通知、模态框、勿扰）

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 3 困难 | ui | 第 3 批 | R02（【接口先行 A2】行走骨架）、R06（WebView2 嵌入验证） | R27（状态栏条目）、R28（欢迎页、最近打开、启动参数打开，以及首选项入口）、R30（高 DPI 与多显示器验证与修复）、R31（【集成验收】M1）、R36（索引生命周期与状态）、R48（保存与外部修改）、R49（热退出与崩溃恢复）、R60（JSON/YAML/XML）、R67（英文界面翻译与界面语言切换）、R69（Zen 模式、全屏、居中布局）、R75（Windows 发布流水线、安装包、体积守护、第三方许可声明） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

R02（【接口先行 A2】行走骨架） 用一个最简工作台（左树、右编辑区、底部状态栏）跑通了链路。本 issue 把它替换为 VS Code 的完整工作台：自绘标题栏与命令中心、菜单栏、活动栏、主侧边栏、面板、状态栏容器、布局持久化，以及 `UX_SPEC.md` §6 的全局反馈规则。

工作台是其余界面 issue 的挂载点：R23（资源管理器）、R25（全局搜索）、R65（上下文篮子） 往活动栏与侧边栏登记视图，R27（状态栏条目）、R36（索引生命周期与状态） 往状态栏登记条目，R60（JSON/YAML/XML） 往面板登记“问题”视图，所有 issue 都通过本 issue 的反馈接口给用户提示。因此**视图、状态栏条目、菜单项、反馈的登记方式必须先定稳**。

必读：`FEATURES.md` §2、§8，`UX_SPEC.md` §1.1–§1.7、§6.1–§6.4，`PROJECT_CHARTER.md` §6，`docs/compat/KEYBINDINGS.md`（布局、通知、窗口与设置），`SETTINGS.md`（`workbench.*`、`window.*`），`THEME_COLORS.md`。

## 目标

### 1. 标题栏与菜单（UX_SPEC §1.2）

- 自绘标题栏（`window.titleBarStyle` 默认 `custom`、`window.customTitleBarVisibility`、`window.controlsStyle`）。
- 命令中心（`window.commandCenter`）：点击后打开快速打开；R24（快速打开与命令面板） 合并前先显示“未实现”的状态栏临时消息。
- 菜单栏：文件、编辑、选择、查看、转到、帮助；**没有运行与终端**。菜单项由各 issue 登记，**未实现的菜单项直接不显示，不置灰**；`window.menuBarVisibility`、`window.menuStyle`。
- 布局控制按钮只保留“切换主侧边栏”“切换面板”（`workbench.layoutControl.enabled`）；没有账号与 AI 图标。

### 2. 活动栏、主侧边栏与面板（UX_SPEC §1.3、§1.4）

- 活动栏：资源管理器、搜索两个视图容器（上下文篮子由 R65（上下文篮子） 追加），底部只有齿轮菜单（命令面板、设置、键盘快捷方式、颜色主题；具体入口由 R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 填充）。首版不支持拖动重排。
- 主侧边栏：`workbench.action.toggleSidebarVisibility`（`ctrl+b`）、`workbench.action.focusSideBar`（`ctrl+0`）、`workbench.sideBar.location` 支持左右切换；视图容器内支持可折叠小节（§1.3 的顺序：打开的编辑器（P2）、文件夹、大纲）。
- 面板：**只有“问题”，默认隐藏**；`workbench.action.togglePanel`（`ctrl+j`）、`workbench.actions.view.problems`（`ctrl+shift+m`）、`workbench.panel.defaultLocation`、`workbench.panel.showLabels`。问题视图的内容由 R60（JSON/YAML/XML） 填充，本 issue 只提供容器与空状态“未在工作区检测到问题。”
- 焦点环：`workbench.action.focusNextPart`（`f6`）、`focusPreviousPart`（`shift+f6`）、`workbench.statusBar.focus*` 全套。

### 3. 状态栏容器（UX_SPEC §1.5）

提供左右两侧的条目登记接口（标识、对齐、优先级、文本与 `$(icon)` 语法、悬停提示、点击命令、颜色级别：普通 / prominent / warning / error），以及 `workbench.statusBar.visible`。**条目本身不在本 issue**（行列、缩进、编码、换行符、语言、只读标记属于 R27（状态栏条目），C# 索引状态属于 R36（索引生命周期与状态）），本 issue 只登记“问题计数”一项作为示例。

### 3.1 产品图标（codicons）

打包 VS Code 的 codicons 图标字体（CC-BY-4.0，在 `THIRD_PARTY_NOTICES.md` 中注明来源，FEATURES §7），并实现 `$(图标名)` 语法，供活动栏、状态栏、菜单、通知、列表等所有界面元素使用。其他 issue 只引用图标名，不各自打包图标。

### 4. 窗口缩放

与 VS Code 相同的窗口级缩放：`workbench.action.zoomIn` / `zoomOut` / `zoomReset`（默认键位 `ctrl+oem_plus`、`ctrl+oem_minus`、`ctrl+numpad0`，见 `KEYBINDINGS.md`“窗口与设置”），遵循 `window.zoomLevel`。编辑器字体缩放（`editor.action.fontZoom*`）是另一套机制，属于 R19（代码视图 Element）（FEATURES §3）。

### 4.1 布局持久化

侧边栏宽度与可见性、面板高度与可见性、活动视图、窗口位置与大小、最大化状态按工作区保存并恢复（`window.restoreWindows`）。标签与编辑器组的恢复属于 R17（编辑器组与标签），本 issue 只负责区域级布局。

### 5. 反馈分级与加载规则（UX_SPEC §6.1、§6.2）

实现 R01（【接口先行 A1】架构与接口契约） 中“反馈”接口的五级形式，并提供给所有界面 issue：

1. 状态栏左侧临时消息（3 秒后消失）；2. 光标处提示；3. 编辑器顶部提示条（可关闭、可“不再提示”，按文件与原因去重）；4. 右下角通知（信息 / 警告 / 错误，**同一原因不重复弹出，只更新次数**，`workbench.notifications.position`、`workbench.notifications.showInTitleBar`、`notification.*` 与 `notifications.*` 全套键位）；5. 模态对话框，**只用于不立即处理就会丢失数据的情况**。照搬勿扰模式（只显示错误级通知）。正常的后台工作永不弹通知，只在状态栏显示进度。

加载规则：窗口 400 ms 内出现并恢复上次布局；打开文件超过 100 ms 时在标签栏下方显示细进度条（进度条组件由本 issue 提供，R17（编辑器组与标签） 调用）；可能超过 1 秒的操作必须可取消。

### 6. 登记与替换

- 新增的命令（VS Code 命令 ID 与默认键位见 KEYBINDINGS.md 布局 / 通知 / 窗口与设置三节）、设置项、颜色键（`titleBar.*`、`commandCenter.*`、`activityBar*`、`sideBar*`、`panel*`、`statusBar*`、`notification*`、`menu*`、`menubar.*`、`progressBar.background`、`window.activeBorder`/`inactiveBorder`）、界面字符串、视图与状态栏条目**全部通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死**。
- 替换 R02（【接口先行 A2】行走骨架） 的 `SKELETON(shell)` 标记并删除之，更新 `docs/ARCHITECTURE.md` 的对照表；状态栏那一处的标记在 R27（状态栏条目） 完成后才删除，本 issue 在表中把它的说明改为“容器已完成，等待条目”。
- 不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口；确需修改时按契约变更流程单独提小 PR。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 状态栏的具体条目（R27（状态栏条目）、R36（索引生命周期与状态））、资源管理器与搜索视图的内容（R23（资源管理器）、R25（全局搜索））、大纲小节（R26（大纲））。
- 编辑器组与标签（R17（编辑器组与标签））、快速打开与命令面板（R24（快速打开与命令面板））、欢迎页与设置入口（R28（欢迎页、最近打开、启动参数打开，以及首选项入口））。
- 全屏、Zen 模式、居中布局（R69（Zen 模式、全屏、居中布局），含 `workbench.action.toggleFullScreen`、`toggleZenMode`、`exitZenMode`）。
- 英文界面与语言切换（R67（英文界面翻译与界面语言切换））；本 issue 只往中文字符串表登记。

## 验收标准

- [ ] codicons 字体已打包，`$(图标名)` 语法在状态栏、活动栏、通知中都能正确显示
- [ ] 打开中型 Unity 测试项目（`ROVE_TEST_UNITY_MEDIUM`）后，标题栏、命令中心、菜单、活动栏、侧边栏、面板、状态栏都按 UX_SPEC §1 显示；截图附在 PR 中（深色 + 浅色）
- [ ] 菜单中不存在未实现的项（用登记表断言：菜单项数 == 已登记且已实现的命令数）
- [ ] 窗口缩放三个命令与 `window.zoomLevel` 行为与 VS Code 一致（缩放后界面与编辑器整体缩放，不触发重新扫描或重新解析）
- [ ] `ctrl+b`、`ctrl+j`、`ctrl+shift+m`、`ctrl+0`、`f6` / `shift+f6` 行为与 VS Code 一致；`workbench.sideBar.location` 切到 `right` 后活动栏与侧边栏一并换边
- [ ] 关闭窗口再打开，侧边栏宽度与可见性、面板高度与可见性、当前活动视图、窗口大小位置全部恢复
- [ ] 冷启动到首帧**中位数 < 400 ms**（5 次测量，附原始数据）；空闲内存 < 60 MB（`PROJECT_CHARTER.md` §6）
- [ ] 五级反馈各有一个集成测试：临时消息 3 秒后消失；提示条可关闭且“不再提示”后不再出现；同一原因的通知连续触发 3 次只产生 1 条并显示次数；勿扰模式下信息级通知不显示；模态框只在数据丢失场景出现
- [ ] 视图、状态栏条目、菜单项三类登记各有一个来自其他 crate 的示例测试，证明后续 issue 可以直接登记
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增工作台布局与反馈的集成测试
- [ ] `SKELETON(shell)` 标记已删除，`docs/ARCHITECTURE.md` 对照表与代码一致

## 风险与提示

- 启动预算是本 issue 最容易失守的地方：M0 实测 GPUI 自身初始化就占了约 396 ms 中的绝大部分（`M0_UI_SPIKE_REPORT.md` §4.1）。工作台的视图要**按需创建**（面板默认隐藏就不要创建），布局恢复不要在首帧前读取大文件。
- 反馈接口一旦被几十处调用就很难改。先把“谁来决定级别”定清楚：调用方给出事件与严重度，级别由反馈层按 §6.1 的表决定，不要让每个调用方自己选级别。
- `window.titleBarStyle` 改为 `native` 时命令中心与菜单栏不可用，这与 VS Code 一致；不要自己发明回退行为。
