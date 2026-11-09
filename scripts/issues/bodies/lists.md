## 背景

资源管理器、搜索结果、大纲、快速打开、命令面板、速览结果列表、编码选择器、问题面板、上下文篮子——Rove 界面里有十来处列表与树。如果各自实现，键盘行为和 `workbench.list.*` 设置就会各不相同，和 VS Code 的使用习惯也对不上。

本 issue 实现**一套通用的虚拟化列表、树与快速输入框组件**，兑现 [[contracts]] 中“列表、树与快速输入”接口，让后续 issue 只做数据源与行渲染。`docs/compat/KEYBINDINGS.md` 的“通用列表（35）”一节是本 issue 的行为规格：所有 `list.*` 命令都在这里实现，各视图只负责提供 `listFocus` 等上下文键的取值。

与 [[codeview]] 不同，这里的条目数有上界（最大是文件列表与搜索结果，量级在十万），`Pixels`（f32）精度足够，可以使用 GPUI 的 `uniform_list`（`M0_UI_SPIKE_REPORT.md` §6.1 的精度问题只影响千万行级的代码视图）。

必读：`UX_SPEC.md` §6.2、§6.3，`SETTINGS.md`（`workbench.list.*`、`workbench.tree.*`），`THEME_COLORS.md`（Lists and trees、Quick picker colors）。

## 目标

### 1. 虚拟化列表

- 只布局与绘制可见行；行高可以按视图固定或按行计算；支持分组分隔符（`pickerGroup.*`）。
- 滚动：鼠标滚轮（`workbench.list.mouseWheelScrollSensitivity`、`fastScrollSensitivity`）、平滑滚动（`workbench.list.smoothScrolling`）、滚动条点击按页或跳转（`workbench.list.scrollByPage`）、水平滚动（`workbench.list.horizontalScrolling`，默认关闭）。
- 打开方式：`workbench.list.openMode`（单击 / 双击）、`workbench.list.multiSelectModifier`。

### 2. 树

- 可展开节点、按需加载子节点（展开时才向数据源要）、缩进（`workbench.tree.indent`）、缩进参考线（`workbench.tree.renderIndentGuides`、`tree.indentGuidesStroke`、`tree.inactiveIndentGuidesStroke`）。
- 粘性滚动：`workbench.tree.enableStickyScroll`、`stickyScrollMaxItemCount`，以及 `list.stickyScroll.collapse`、`list.stickyScrollselect`、`list.stickyScrolltoggleExpand` 与 `treestickyScrollFocused` 上下文键。
- 展开 / 折叠：`workbench.tree.expandMode`、`list.expand`、`list.collapse`、`list.collapseAll`、`list.toggleExpand`。

### 3. 键盘导航与多选（照搬 `list.*` 全套）

`list.focusUp` / `focusDown` / `focusFirst` / `focusLast` / `focusPageUp` / `focusPageDown` / `focusAnyUp` / `focusAnyDown` / `focusAnyFirst` / `focusAnyLast`、`list.scrollUp` / `scrollDown`、`list.select`、`list.clear`、`list.selectAll`、`list.toggleSelection`、`list.expandSelectionUp` / `Down`、`list.showHover`。多选由视图通过 `listSupportsMultiselect` 开启。`listScrollAtBoundary` 上下文键要按滚动位置正确给出 `top` / `bottom` / `both`，否则 `ctrl+up` / `ctrl+down` 的穿透导航会失效。

### 4. 类型过滤与查找

- 输入即过滤 / 高亮：`workbench.list.defaultFindMode`（`highlight` / `filter`）、`defaultFindMatchType`（`fuzzy` / `contiguous`）、`typeNavigationMode`，以及 `list.find`（`ctrl+alt+f` / `f3`）、`list.closeFind`、`treeFindOpen` 与 `listSupportsFind` 上下文键。
- 过滤框外观用 `listFilterWidget.background` / `outline` / `noMatchesOutline` / `shadow`，匹配高亮用 `list.filterMatchBackground` / `Border`、`list.highlightForeground` / `focusHighlightForeground`。
- 模糊匹配**打分算法**本身由 [[quickopen]] 移植（VS Code `fuzzyScorer`）；本 issue 只定义“打分函数 + 高亮区间”的接口，并先提供一个简单的包含匹配实现。

### 5. 快速输入框（quick input）

供 [[quickopen]]（快速打开、命令面板、转到行、转到符号）、[[statusbar]]（编码、换行符、语言选择器）、[[welcome]]（主题选择）共用：

- 形态：输入框 + 虚拟化结果列表 + 可选标题 + 可选复选框 + 右侧按钮；内部复用上面的列表组件。
- 命令全套（KEYBINDINGS.md“快速访问”一节）：`quickInput.accept`（含各种修饰键组合）、`acceptInBackground`、`hide`、`first` / `last` / `next` / `previous`、`pageNext` / `pagePrevious`、`nextSeparator` / `previousSeparator`（及 `*WithQuickAccessFallback`）、`toggleCheckbox`、`toggleHover`。
- 上下文键：`inQuickInput`、`quickInputType`（`quickPick` / `quickTree` / `quickWidget`）、`inputFocus`、`cursorAtEndOfQuickInputBox`。
- 输入历史导航：`history.showPrevious` / `showNext` 与 `historyNavigationWidgetFocus`、`historyNavigationBackwardsEnabled` / `ForwardsEnabled`（[[findwidget]] 和 [[search]] 的输入框也用这一套）。
- 颜色：`quickInput.background` / `foreground`、`quickInputList.focus*`、`quickInputTitle.background`、`input.*`、`inputValidation.*`。

### 6. 空状态与加载（UX_SPEC §6.2、§6.3）

列表组件提供统一的空状态与“正在加载…”插槽（文案由各视图给出，例如资源管理器的“尚未打开文件夹。”加按钮、搜索的“未找到结果。”加链接），以及“列表底部追加加载中行”的能力（工作区扫描未完成时快速打开用）。

### 7. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。[[skeleton]] 里的简易可展开列表标记为 `SKELETON(explorer)`，由 [[explorer]] 接入本 issue 的组件后删除；本 issue 负责把 `docs/ARCHITECTURE.md` 对照表中该行的说明更新为“列表与树组件已完成，等待 [[explorer]] 接入”（若骨架另外打了 `SKELETON(lists)` 标记，则由本 issue 删除）。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 任何具体视图：资源管理器（[[explorer]]）、搜索视图（[[search]]）、大纲（[[outline]]）、问题面板（[[structure]]）、速览结果列表（[[peek]]）、篮子视图（[[basket]]）、图形化设置界面（[[settings-ui]]）。
- 模糊匹配打分算法与 `:行号` / `@符号` / `>命令` 前缀（[[quickopen]]）。
- 列表的右键菜单项内容（各视图自己登记；本 issue 只提供菜单载体与 `shift+f10` 的入口）。
- 拖放的业务语义（文件移动属于 [[fileops]]，标签拖动属于 [[groups]]）；本 issue 只提供拖放事件与 `list.dropBackground` / `list.dropBetweenBackground` 的绘制。

## 验收标准

- [ ] 一个 10 万条目的列表：连续滚动时每帧 CPU 平均 < 1 ms、帧间隔 p99 ≤ 1.3 倍刷新间隔（附测量数据，口径与 `M0_UI_SPIKE_REPORT.md` §2.5 一致）
- [ ] 一棵 1 万个子节点的目录树：展开耗时 < 50 ms（附测量）
- [ ] KEYBINDINGS.md“通用列表”一节的每个 `list.*` 命令都有一个单元测试或集成测试，断言焦点、选中、展开状态的变化与 VS Code 一致
- [ ] `listScrollAtBoundary` 在顶部 / 底部 / 内容不足三种情况下取值正确，`ctrl+up` / `ctrl+down` 的穿透导航按 when 条件生效
- [ ] 输入过滤：`highlight` 与 `filter` 两种模式都正确；`fuzzy` 与 `contiguous` 两种匹配类型都正确；高亮区间与匹配位置一致
- [ ] 多选：`shift+↑↓` 扩选、`ctrl+a` 全选、`ctrl+shift+enter` 切换选中；`workbench.list.multiSelectModifier` 生效
- [ ] 快速输入框可用键盘完成全部操作（打开、输入、翻页、分组跳转、接受、后台接受、关闭），并有一个消费方示例测试演示如何用它做一个选择器
- [ ] `workbench.list.*` 与 `workbench.tree.*` 中 P0 的每一项都有生效验证（可用一个参数化测试覆盖）
- [ ] [[skeleton]] 的端到端测试仍然通过；新增列表、树、快速输入三者各一个集成测试
- [ ] `docs/ARCHITECTURE.md` 对照表已更新

## 风险与提示

- 行渲染接口的粒度决定了后续所有视图的写法。建议让视图提供“按索引生成一行元素”的闭包 + 行高策略，而不是让列表组件了解条目类型。
- 树的数据源要区分“已知子节点数”和“未知”两种状态：工作区扫描中、索引构建中都会出现未知，接口里不要用 `len()` 把这一点抹掉。
- 过滤状态下的键盘导航与展开状态很容易出错（过滤时父节点要自动展开、退出过滤要还原）。照搬 VS Code 的行为，并为此单独写测试。
- 不要在列表里做防抖以外的异步：输入到列表刷新必须在一帧内完成，耗时的数据获取由数据源自己流式推送。
