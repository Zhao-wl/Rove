# R23 资源管理器（文件树、紧凑文件夹、定位当前文件、过滤）

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 2 中等 | ui | 第 4 批 | R15（工作区）、R18（通用列表、树与快速输入组件）、R17（编辑器组与标签） | R31（【集成验收】M1）、R62（资源管理器文件操作）、R68（只读加载 VS Code 扩展主题、Seti 文件图标主题） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

资源管理器是打开 Rove 后最先看到的东西，也是 R02（【接口先行 A2】行走骨架） 里最简实现最明显的一处（一个简单的可展开列表）。本 issue 用 R18（通用列表、树与快速输入组件） 的树组件 + R15（工作区） 的文件模型 + R17（编辑器组与标签） 的打开文档服务，把它替换为 VS Code 的文件树：紧凑文件夹、键盘导航、定位当前文件、按名称过滤、排序、文件装饰、空状态。

`FEATURES.md` §6 把资源管理器判定为照搬、P0，文件操作（新建、重命名、删除、复制路径）判定为 P1——后者属于 R62（资源管理器文件操作）（M4），**本 issue 只读不写**。

必读：`FEATURES.md` §6，`UX_SPEC.md` §1.3、§6.2、§6.3，`docs/compat/KEYBINDINGS.md`（文件一节），`SETTINGS.md`（`explorer.*`、`files.exclude`、`rove.files.extraExclude`），`THEME_COLORS.md`（Lists and trees、Side Bar）。

## 目标

### 1. 文件树

- 数据来自 R15（工作区）：目录树模型、文件事件流、排除规则（`files.exclude` 与 Rove 的附加规则 `rove.files.extraExclude`，默认排除 `**/Library`、`**/Temp`、`**/obj`、`**/Logs`；**不读取 `.gitignore`**）。
- 渲染与虚拟化、缩进、缩进参考线、粘性滚动、键盘导航全部复用 R18（通用列表、树与快速输入组件） 的树组件，不另起炉灶。
- 展开时才向 R15（工作区） 要子节点；文件系统事件到来时增量更新，**保持展开状态与选中项**。
- 排序：`explorer.sortOrder`、`explorer.sortOrderLexicographicOptions`、`explorer.sortOrderReverse`；`explorer.expandSingleFolderWorkspaces`。
- 图标：先用内置 codicons 的文件 / 文件夹图标，按扩展名区分常见类型；Seti 文件图标主题属于 R68（只读加载 VS Code 扩展主题、Seti 文件图标主题）（P1）。颜色用 `symbolIcon.fileForeground` / `folderForeground`、`list.*`、`sideBar*`。

### 2. 紧凑文件夹

`explorer.compactFolders`（默认 true）：单子文件夹压缩为一个组合树节点。随之实现四个命令与三个上下文键：`firstCompressedFolder`（`home`）、`lastCompressedFolder`（`end`）、`nextCompressedFolder`（`right`）、`previousCompressedFolder`（`left`），以及 `explorerViewletCompressedFocus`、`explorerViewletCompressedFirstFocus`、`explorerViewletCompressedLastFocus`。

### 3. 打开文件

- `explorer.openAndPassFocus`（`enter`）、`explorer.openToSide`（`ctrl+enter`）、`filesExplorer.openFilePreserveFocus`（`space`，P1）；鼠标行为遵循 `workbench.list.openMode`：单击打开为预览标签、双击转为正式标签（预览语义由 R17（编辑器组与标签） 实现）。
- 一律通过 R17（编辑器组与标签） 的打开文档服务打开，并按 `UX_SPEC.md` §4.3：**从文件树打开 `.md` 进入预览**（D6），不是源码。
- 上下文键：`filesExplorerFocus`、`explorerViewletFocus`、`foldersViewVisible`、`explorerResourceIsFolder`、`explorerResourceIsRoot`、`explorerResourceReadonly`。

### 4. 定位当前文件

- `explorer.autoReveal`（默认 true）与 `explorer.autoRevealExclude`：切换活动编辑器时自动在树中展开并选中对应文件；取值为 `focusNoScroll` 时不滚动。
- 命令“在资源管理器中显示活动文件”（沿用 VS Code 的 `workbench.files.action.showActiveFileInExplorer`，无默认键位）与 `workbench.view.explorer`（`ctrl+shift+e`）。
- 折叠全部走 R18（通用列表、树与快速输入组件） 的 `list.collapseAll`（`ctrl+left`）。

### 5. 过滤

按名称过滤 / 高亮复用 R18（通用列表、树与快速输入组件） 的查找过滤（`list.find`、`workbench.list.defaultFindMode`、`defaultFindMatchType`），只做数据源适配与“过滤时自动展开命中节点的父级、退出过滤时还原”的行为。

### 6. 文件装饰（受限）

- 提供装饰接口并接入两类来源：**只读标记**（来自 R15（工作区） 的只读规则，含 `rove.files.extraReadonlyInclude`）与**错误 / 警告计数**（来自问题面板，R60（JSON/YAML/XML） 合并前为空）。
- `workbench.editor.decorations.badges` / `colors` 控制徽标与颜色；颜色用 `list.errorForeground`、`list.warningForeground`、`list.deemphasizedForeground`。
- **不接 Git**（`FEATURES.md` §10：不做版本控制），`gitDecoration.*` 颜色键按 `THEME_COLORS.md` 静默忽略。

### 7. 空状态与加载（UX_SPEC §6.2、§6.3）

- 没有打开文件夹时：“尚未打开文件夹。”加“打开文件夹”按钮（按钮执行 `workbench.action.files.openFolder`，`ctrl+k ctrl+o`；对话框由 R28（欢迎页、最近打开、启动参数打开，以及首选项入口） / R16（工作台） 提供）。
- 打开文件夹后**立即显示根目录**，不等待扫描完成；扫描进行中不显示阻塞态。
- 目录读取失败（无权限、路径过长）时该节点显示一句话原因，不影响其余节点。

### 8. 登记与替换

新增命令、设置项、颜色键、界面字符串、视图（资源管理器视图容器中的“文件夹”小节）全部通过 R01（【接口先行 A1】架构与接口契约） 与 R16（工作台） 的登记机制加入，不得写死。替换并删除 R02（【接口先行 A2】行走骨架） 的 `SKELETON(explorer)` 标记，更新 `docs/ARCHITECTURE.md` 对照表。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口（契约变更走单独小 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- **文件操作**（R62（资源管理器文件操作），M4）：`renameFile`（`f2`）、`deleteFile` / `moveFileToTrash`（`delete`）、`filesExplorer.copy` / `cut` / `paste`、`copyFilePath` / `copyRelativeFilePath`、`revealFileInOS`、`workbench.action.files.newUntitledFile`、拖放移动文件（`explorer.enableDragAndDrop`、`explorer.confirmDragAndDrop`、`explorer.confirmDelete`、`explorer.enableUndo`、`explorer.incrementalNaming`），本 issue 一律不注册；右键菜单只保留本 issue 已实现的项。
- 工作区扫描、文件监听、排除与只读规则的实现（R15（工作区））；列表与树组件本身（R18（通用列表、树与快速输入组件））；打开文档服务（R17（编辑器组与标签））。
- 大纲小节（R26（大纲））、搜索视图（R25（全局搜索））、“打开的编辑器”小节（P2）；`filesExplorer.findInFolder`（`shift+alt+f`，P1，随 R25（全局搜索））。
- 文件图标主题（R68（只读加载 VS Code 扩展主题、Seti 文件图标主题））、文件嵌套（P2）。

## 验收标准

- [ ] 在中型 Unity 测试项目（`ROVE_TEST_UNITY_MEDIUM`）上打开文件夹后立即看到根目录，`Library`、`Temp`、`obj`、`Logs` 默认不出现；把 `rove.files.extraExclude` 中某一项改为 false 后该目录出现
- [ ] **展开一个有 1 万个子项的目录 < 50 ms**（附测量数据）；大型项目中滚动流畅，复用 R18（通用列表、树与快速输入组件） 的帧预算
- [ ] 外部批量增删文件（脚本创建 / 删除 500 个文件）后，树最终与磁盘一致，且**展开状态与选中项不丢失**
- [ ] 紧凑文件夹：Unity 项目中 `Assets/Scripts/Runtime/...` 这类单链被压缩显示；`home` / `end` / `left` / `right` 在压缩节点内部的焦点移动与 VS Code 一致；`explorer.compactFolders` 设为 false 后恢复逐级显示
- [ ] 键盘可完成全部导航与打开（`↑↓` / `←→` / `enter` / `ctrl+enter` / `space`）；`enter` 打开并移交焦点、`space` 打开但保持焦点
- [ ] 单击打开为预览标签（斜体）、双击转为正式标签；打开 `.md` 时进入预览（R29（Markdown 预览） 合并前断言解析出的编辑器类型）
- [ ] `explorer.autoReveal` 为 true 时切换活动编辑器会在树中定位并选中；设为 false 时不动；`autoRevealExclude` 生效
- [ ] 过滤输入时命中节点的父级自动展开，清空过滤后展开状态还原
- [ ] `Library/PackageCache/**` 下的文件显示只读装饰；没有任何 Git 装饰
- [ ] 没有打开文件夹时显示 §6.3 的空状态与按钮
- [ ] 右键菜单中不存在未实现的文件操作项
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过（点击 `.cs` 文件打开并高亮）；新增资源管理器入口的集成测试
- [ ] `SKELETON(explorer)` 标记已删除，`docs/ARCHITECTURE.md` 对照表与代码一致

## 风险与提示

- 文件事件与展开状态的合并是最容易出 bug 的地方：外部删除一个已展开的目录、重命名一个父目录、批量写入时的事件风暴都要有用例。R15（工作区） 提供的是事件流，树要做幂等的增量应用，不要收到事件就整棵重建。
- 紧凑文件夹会让“树节点”与“路径”不再一一对应（一个节点代表多级目录）。选中、定位、过滤、装饰都要基于路径而不是节点下标，否则 `autoReveal` 会定位到错误的层级。
- `explorer.autoReveal` 与预览标签配合时容易打架：从树里单击打开 → 自动定位又回到树 → 焦点抖动。照搬 VS Code：由树主动打开时不触发自动定位。
- 不要为了“顺便做好一点”而加上重命名或新建。R62（资源管理器文件操作） 会重做这块，提前做会直接冲突。
