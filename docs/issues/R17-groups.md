# R17 编辑器组与标签：网格分屏、预览标签、拖动、固定、“打开文档”服务

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 3 困难 | ui | 第 3 批 | R02（【接口先行 A2】行走骨架） | R23（资源管理器）、R24（快速打开与命令面板）、R25（全局搜索）、R26（大纲）、R28（欢迎页、最近打开、启动参数打开，以及首选项入口）、R29（Markdown 预览）、R40（C# 导航交互）、R63（图片预览） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

R02（【接口先行 A2】行走骨架） 的“打开文档服务”是一个最简实现：只有一个编辑区，打开新文件直接替换。本 issue 把它替换为 VS Code 的编辑器组网格与标签系统，并实现正式的**打开文档服务**。

这个服务是 Rove 界面的枢纽：R23（资源管理器）、R24（快速打开与命令面板）、R25（全局搜索）、R26（大纲）、R40（C# 导航交互）、R41（速览窗口）、R65（上下文篮子）、R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 都通过它“打开某个文件并定位到某个位置”。所以服务的语义（打开方式、预览与固定、定位、编辑器类型解析）必须在本 issue 里一次定稳。

必读：`FEATURES.md` §2、§12（D6）、`UX_SPEC.md` §4.3（打开方式）、§6.2（加载）、§6.4（错误），`docs/compat/KEYBINDINGS.md`（编辑器组 65 条），`SETTINGS.md`（`workbench.editor.*`、`workbench.editorAssociations`、`window.density.editorTabHeight`），`THEME_COLORS.md`（Editor Groups & Tabs）。

## 目标

### 1. 编辑器组网格

- 任意方向分屏与多级嵌套：`workbench.action.splitEditor`（`ctrl+oem_5`）、`splitEditorDown` / `Up` / `Left` / `Right` / `Orthogonal`（`ctrl+k ctrl+oem_5`）、`splitEditorInGroup`、`joinEditorInGroup`、`toggleEditorGroupLayout`（`shift+alt+0`）、`workbench.editor.splitInGroupLayout`。
- 组焦点与移动：`focusFirstEditorGroup` … `focusEighthEditorGroup`（`ctrl+1`–`ctrl+8`）、`focusAboveGroup` / `BelowGroup` / `LeftGroup` / `RightGroup`、`moveActiveEditorGroupUp/Down/Left/Right`、`moveEditorToNextGroup` / `PreviousGroup` / `FirstGroup` / `LastGroup`。
- 组的锁定与最大化：`workbench.action.toggleMaximizeEditorGroup`（`ctrl+k ctrl+m`）与“锁定编辑器组”命令；`workbench.editor.closeEmptyGroups`、`closeGroup`、`closeAllGroups`（`ctrl+k ctrl+shift+w`）。
- 同一个文件在多个组中打开时共享同一份文档与 `TextSource`，不重复读文件。

### 2. 标签

- 打开、关闭（`closeActiveEditor` `ctrl+w`/`ctrl+f4`、`closeEditorsInGroup` `ctrl+k w`、`closeAllEditors` `ctrl+k ctrl+w`、`closeUnmodifiedEditors` `ctrl+k u`、`reopenClosedEditor` `ctrl+shift+t`）、拖动排序、拖到另一组或空白处分屏、按索引切换（`openEditorAtIndex1`–`9`、`lastEditorInGroup`）、前后切换（`nextEditor` `ctrl+pagedown`、`previousEditor` `ctrl+pageup`、`moveEditorLeftInGroup` / `RightInGroup`）。
- **预览标签**：单击打开的标签为斜体预览，下一次预览会替换它；`workbench.action.keepEditor`（`ctrl+k enter`）或双击转为正式标签。遵循 `workbench.editor.enablePreview`、`enablePreviewFromQuickOpen`、`enablePreviewFromCodeNavigation`。
- **固定标签**：`workbench.action.pinEditor` / `unpinEditor`（`ctrl+k shift+enter`）、`workbench.editor.pinnedTabsOnSeparateRow`、`tab.lastPinnedBorder`。
- 外观设置照搬：`showTabs`、`tabSizing`、`tabSizingFixedMinWidth` / `MaxWidth`、`wrapTabs`、`tabActionLocation`、`showTabIndex`、`customLabels.enabled` / `patterns`、`decorations.badges` / `colors`、`doubleClickTabToToggleEditorGroupSizes`、`window.density.editorTabHeight`。
- 打开位置与数量：`workbench.editor.openPositioning`、`revealIfOpen`、`closeOnFileDelete`、`limit.enabled` / `value` / `perEditorGroup` / `excludeDirty`。
- **脏状态挂钩**：标签上的未保存圆点、`highlightModifiedTabs`、`tab.activeModifiedBorder` 等颜色键与“关闭时提示保存”的接入点在本 issue 预留并接通空实现，真正的脏状态来自 M3 的 R46（代码视图的编辑交互与输入法） 与 R48（保存与外部修改）。

### 3. 标签状态恢复

每个标签保存自己的视图状态（滚动位置、选区、折叠状态），切换标签时恢复；`workbench.editor.restoreViewState` 为 true 时关闭后重新打开也恢复。状态由 R19（代码视图 Element） 的视口接口读写，本 issue 只负责按标签存取。

### 4. 打开文档服务（本 issue 的主要产出）

实现 R01（【接口先行 A1】架构与接口契约） 中的“打开文档服务”：

- `open(路径, 选项)`：选项包含目标组（当前 / 旁边 / 指定）、是否预览、是否抢焦点（`explorer.openFilePreserveFocus` 场景）、定位到的位置（行、UTF-8 列）、是否选中一个区间。打开后滚动到位置并把该行置于视口内（照搬 VS Code 的 `revealRangeInCenterIfOutsideViewport`）。
- 查找已打开的文档、关闭、按组枚举。
- **文档级别解析**：按 R08（文件大小分级与超大文件） 的规则选择普通 / 大文件 / 超大文件三级（`rove.largeFile.threshold`、`editor.largeFileOptimizations`），并把级别写进文档模型，供 R20（代码视图） 与 R22（超大文件与超长行的界面） 关闭对应功能。
- **编辑器类型解析**（D6）：按 `workbench.editorAssociations` 把文件解析到一个已登记的编辑器类型；Rove 的默认值 `{"*.md": "vscode.markdown.preview.editor"}` 只在用户未设置时生效，不写入共用设置文件。本 issue 提供“编辑器类型登记 + 解析”这一步并注册默认的文本编辑器，Markdown 预览编辑器由 R29（Markdown 预览） 登记，图片预览由 R63（图片预览） 登记。
- **带位置的打开必须打开源码**（UX_SPEC §4.3）：从文件树与快速打开进入 `.md` 时打开预览；从搜索结果、跳转、引用进入时打开**源码并定位到匹配处**。这条规则在服务层实现，调用方只需说明来源。

### 4.1 导航历史（从 M1 起）

所有经过打开文档服务的跳转（快速打开、转到行、转到符号、搜索结果、大纲、面包屑，以及 M2 起的 C# 导航）都记录到一个**跨文件、跨标签、跨编辑器组**的全局历史栈：文件 + 行列 + 所属组。`workbench.action.navigateBack`（`alt+left`）、`navigateForward`（`alt+right`）、`navigateToLastEditLocation`（`ctrl+k ctrl+q`；M3 之前记录“最后一次打开文档的位置”，R46（代码视图的编辑交互与输入法） 合并后改为编辑位置），以及 `canNavigateBack` / `canNavigateForward` 上下文键。目标标签已关闭时重新打开。入栈规则照搬 VS Code。

### 5. 加载与错误

- 打开文件 100 ms 内完成时不显示任何加载状态，超过时在标签栏下方显示细进度条（复用 R16（工作台） 的进度条）。
- 无法打开（不存在、无权限、被占用、路径过长）时标签照常打开，内容区显示错误占位页：一句话原因 + “重试”“在资源管理器中显示”（UX_SPEC §6.4）。
- 关闭最后一个标签后显示快捷键水印空状态（UX_SPEC §1.6）。

### 6. 登记与替换

新增命令、设置项、颜色键（`tab.*`、`editorGroup*`、`editorGroupHeader.*`、`editorPane.background`、`sideBySideEditor.*`）、界面字符串全部通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。替换并删除 R02（【接口先行 A2】行走骨架） 的 `SKELETON(groups)` 标记，更新 `docs/ARCHITECTURE.md` 对照表。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口（契约变更走单独小 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 代码视图本身（R19（代码视图 Element））、Markdown 预览编辑器（R29（Markdown 预览））、图片预览（R63（图片预览））、元数据视图（R42（元数据视图界面））。
- 真正的编辑与保存、关闭时的保存对话框（R46（代码视图的编辑交互与输入法）、R48（保存与外部修改））、热退出（R49（热退出与崩溃恢复））。
- 对比视图、已打开的编辑器列表（P2）；居中布局（R69（Zen 模式、全屏、居中布局））。
- `workbench.action.showAllEditors`、`quickOpenPreviousRecentlyUsedEditorInGroup`（`ctrl+tab`）等基于快速输入的编辑器选择器（R24（快速打开与命令面板））。

## 验收标准

- [ ] 导航历史：转到行、快速打开、搜索结果、大纲跳转之后，`alt+left` / `alt+right` 能跨文件、跨标签正确往返；目标标签已关闭时重新打开
- [ ] 打开文档服务的接口有文档注释，并附一个“调用方示例测试”：传入路径 + 行列 + 来源，断言文档已打开、视口包含目标行、选区正确
- [ ] 任意方向分屏、多级嵌套、拖动标签到另一组、组最大化与还原、`ctrl+1`–`ctrl+8` 全部可用；截图附在 PR 中
- [ ] 预览标签语义正确：单击 A 再单击 B 时只剩一个预览标签；`ctrl+k enter` 后不再被替换；`enablePreviewFromQuickOpen` / `FromCodeNavigation` 为 false 时按 VS Code 行为打开正式标签
- [ ] 固定标签不会被预览替换，关闭全部时保留；`pinnedTabsOnSeparateRow` 为 true 时换行显示
- [ ] 切换标签后滚动位置与选区恢复；文档已打开时**切换标签 < 16 ms**（附测量）
- [ ] `.md` 从文件树打开进入预览、从搜索结果打开进入源码并定位（用 R29（Markdown 预览） 合并前的占位编辑器类型也要能断言解析结果）
- [ ] 同一文件在两个组中打开时只有一份 `TextSource`（用版本号或实例计数断言）
- [ ] 打开不存在的文件时显示错误占位页而不是崩溃或空白
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增“打开文档服务 + 分屏 + 预览标签”的集成测试
- [ ] `SKELETON(groups)` 标记已删除，`docs/ARCHITECTURE.md` 对照表与代码一致

## 风险与提示

- 打开文档服务的参数一旦不足，后续 7 个 issue 都要改契约。先把 R25（全局搜索）（定位 + 选中匹配）、R41（速览窗口）（在嵌入编辑器里打开）、R23（资源管理器）（预览 + 保持焦点）、R40（C# 导航交互）（旁边打开 + 导航历史）的需求列出来再定签名。
- 编辑器类型解析是 D6 的落点，容易做成“对 `.md` 写死一个分支”。必须做成登记表 + glob 匹配，否则 R29（Markdown 预览）、R63（图片预览）、R42（元数据视图界面） 都要回来改这里。
- 标签视图状态按组存储、组关闭即丢弃，这是 VS Code 的语义，不要自作主张做成全局持久化。
