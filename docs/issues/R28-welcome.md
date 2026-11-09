# R28 欢迎页、最近打开、启动参数打开，以及首选项入口（选择主题、打开共用 settings.json 并显示提示条、齿轮菜单）

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 1 简单 | ui | 第 4 批 | R17（编辑器组与标签）、R13（设置层）、R14（键位与命令层）、R16（工作台）、R10（主题引擎）、R18（通用列表、树与快速输入组件） | R31（【集成验收】M1） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

这是用户见到 Rove 的第一屏，也是他们第一次发现“设置和 VS Code 是同一份文件”的地方（D4）。两件事都很小，但都直接决定第一印象，所以放在同一个 issue 里。

必读：`UX_SPEC.md` §1.6（欢迎页与空窗口）、§1.7（设置与键位入口）、§6.1（反馈分级：提示条是第 3 级）、§6.3（空状态）、§6.4（错误占位页），`FEATURES.md` §6、§7、§13，`docs/compat/SETTINGS.md`（`workbench.startupEditor`、`workbench.colorTheme`、`window.open*InNewWindow`）、`KEYBINDINGS.md`、`THEME_COLORS.md`（`welcomePage.*`）。

## 目标

### 1. 欢迎页（UX_SPEC §1.6）

作为一个编辑器类型注册，通过 R17（编辑器组与标签） 的打开文档服务在编辑器组中打开：

- **开始**：「打开文件」（`workbench.action.files.openFile`，`ctrl+o`）、「打开文件夹」（`workbench.action.files.openFolder`，`ctrl+k ctrl+o`）。没有「新建文件」「克隆仓库」「连接远程」。
- **最近**：最近打开的**文件夹和文件**，显示完整路径，每项可以从列表中移除；底部「更多…」打开 `workbench.action.openRecent`（`ctrl+r`）的快速选择。列表持久化在 Rove 自己的状态目录中，不写入项目目录。
- 「启动时显示欢迎页」复选框，绑定 `workbench.startupEditor`（默认 `welcomePage`），通过 R13（设置层） 的保留格式写入。
- 颜色只用登记过的 `welcomePage.background`、`welcomePage.tileBackground`、`welcomePage.tileHoverBackground`。
- 没有演练（Walkthroughs），不注册 `workbench.action.openWalkthrough`、`welcome.showNewFileEntries`。

### 2. 空窗口与水印

- 没有打开文件夹的窗口显示欢迎页（UX_SPEC §6.3）；资源管理器显示「尚未打开文件夹。」+「打开文件夹」按钮。
- 打开了文件夹但没有打开任何文件时，编辑区显示快捷键水印；键位文本由 R14（键位与命令层） 解析（含用户在共用 `keybindings.json` 中的覆盖），不写死。

### 3. 启动参数 `rove.exe <路径>`

- 参数是文件则打开文件，是文件夹则作为工作区打开；支持多个参数、相对路径、长路径（> 260 字符）与大小写不敏感路径（立项文档 §7）。
- 路径不存在或无权限时：窗口照常打开，内容区显示错误占位页（UX_SPEC §6.4），不弹模态框。
- 是否复用当前窗口遵循 `window.openFilesInNewWindow`、`window.openFoldersInNewWindow`；一个窗口一个文件夹（FEATURES §6，不做多根工作区）。
- 不提供 `code` 那样的 shell 命令行工具。

### 4. 首选项入口（UX_SPEC §1.7）

- **齿轮菜单**（活动栏底部，菜单内容由本 issue 登记，容器由 R16（工作台） 提供）：命令面板、设置、键盘快捷方式、颜色主题。没有账号项。
- **设置**：`workbench.action.openSettings`（`ctrl+oem_comma`）直接打开**与 VS Code 共用的那一份** `settings.json`（路径由 R13（设置层） 解析，受启动参数 `--vscode-user-dir` / 环境变量 `ROVE_VSCODE_USER_DIR` 影响）；不注册 `workbench.action.openSettings2`（图形化设置界面是 R70（图形化设置界面））。
- **共用提示条**：首次打开共用设置文件时，在编辑器顶部显示第 3 级提示条（UX_SPEC §6.1）：「此文件与 VS Code 共用。Rove 专有的设置放在 `rove.*` 下。」可关闭、可选「不再提示」（状态记在 Rove 自己的状态目录）。打开共用 `keybindings.json` 时同样显示。
- **键盘快捷方式**：`workbench.action.openGlobalKeybindings`（`ctrl+k ctrl+s`）打开共用的 `keybindings.json`（图形化键位编辑器是 P2，不做）。
- **颜色主题**：`workbench.action.selectTheme`（`ctrl+k ctrl+t`）快速选择器，复用 R18（通用列表、树与快速输入组件）；**移动选中项时实时预览**，`Esc` 还原，`Enter` 通过 R13（设置层） 写入 `workbench.colorTheme`（保留注释、格式与键顺序，写入前备份）。主题数据来自 R10（主题引擎）。

### 5. 登记与替换

欢迎页编辑器类型、命令、设置项、颜色键、界面字符串都通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入。替换 R02（【接口先行 A2】行走骨架） 中的 `SKELETON(welcome)` 标记（如有），删除标记并更新 `docs/ARCHITECTURE.md` 的对照表。

## 不包含

- 图形化设置界面：R70（图形化设置界面）；图形化键位编辑器：P2，不做。
- 演练、引导流程、新建文件入口、Git 与远程相关入口。
- 安装包里的「用 Rove 打开」右键菜单项（UX_SPEC §1.6 标为 P1 的安装可选项）：R75（Windows 发布流水线、安装包、体积守护、第三方许可声明）。
- 从扩展目录只读加载主题（`%USERPROFILE%\.vscode\extensions`）：R68（只读加载 VS Code 扩展主题、Seti 文件图标主题）；本 issue 只列出 R10（主题引擎） 已提供的内置主题。
- 热退出恢复与「已恢复上次未保存的修改」提示：R49（热退出与崩溃恢复）。
- 修改 R01（【接口先行 A1】架构与接口契约） 的接口：需要改时走契约变更流程，单独提小 PR。

## 验收标准

- [ ] 不带参数启动显示欢迎页；「打开文件夹」后水印显示正确的快捷键（在共用 `keybindings.json` 中覆盖一个键位后，水印随之变化）
- [ ] 最近列表显示完整路径、可移除、重启后仍在；「更多…」打开 `Ctrl+R` 的快速选择
- [ ] 「启动时显示欢迎页」取消勾选后，重启不再显示，且 `settings.json` 中只有 `workbench.startupEditor` 这一个键被改动（注释与格式不变）
- [ ] `rove.exe <文件>`、`rove.exe <文件夹>`、长路径、不存在的路径四种情况都符合预期（最后一种显示错误占位页）
- [ ] 齿轮菜单四项都可用；`Ctrl+,` 打开的是共用的 `settings.json`，且首次打开显示共用提示条，「不再提示」后不再出现
- [ ] `Ctrl+K Ctrl+T` 主题选择器在移动选中项时实时预览，`Esc` 还原，`Enter` 写入 `workbench.colorTheme`
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；另加覆盖本 issue 的集成测试（空窗口 → 欢迎页 → 最近项打开文件夹；打开设置 → 断言提示条出现一次）
- [ ] 无运行时网络访问；不读取 `.gitignore`

## 风险与提示

- 齿轮菜单挂在 R16（工作台） 的活动栏上，主题预览使用 R10（主题引擎） 与 R18（通用列表、树与快速输入组件）（都已列为前置依赖）。不要在本 PR 里改 R16（工作台） 的布局。
- 实时预览主题时不要写文件：预览只改内存中的主题，确认后才写一次 `settings.json`。
- 「不再提示」的状态属于 Rove 自己的状态，绝不能写进共用的 `settings.json`。
- 欢迎页命令本身没有默认键位，`KEYBINDINGS.md` 的导出清单里查不到它的 ID；按该文件 §2 的规定沿用 VS Code 的命令 ID，并在 PR 中列出本 issue 新登记的全部命令 ID 供核对。
