# R26 大纲：符号数据、大纲视图、面包屑、转到文件中的符号

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 2 中等 | ui | 第 4 批 | R09（tree-sitter 集成）、R12（语言配置）、R18（通用列表、树与快速输入组件）、R17（编辑器组与标签） | R24（快速打开与命令面板）、R31（【集成验收】M1）、R54（粘性滚动）、R60（JSON/YAML/XML）、R65（上下文篮子） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

大纲是“看代码”的第二张地图。本 issue 是一个**纵向条目**：符号数据 + 大纲小节 + 面包屑 + 转到文件中的符号一起做，因为这几件事共用同一棵符号树，分开做会让接口在多个 PR 之间来回改。

符号树的**类型**由 R01（【接口先行 A1】架构与接口契约） 定义（折叠区间与符号树是给折叠、大纲、面包屑、粘性滚动、上下文篮子共用的统一类型），本 issue 只负责产出数据并消费它，不改契约。

必读：`FEATURES.md` §5、§9，`UX_SPEC.md` §1.3（大纲是资源管理器中的一个小节）、§6.3（空状态），`docs/compat/SETTINGS.md`（`outline.*`、`breadcrumbs.*`）、`KEYBINDINGS.md`（`breadcrumbs.*`、`workbench.action.gotoSymbol`）、`THEME_COLORS.md`（`symbolIcon.*`、`breadcrumb*`）。

## 目标

### 1. 符号数据（核心层，不依赖 gpui）

在 R09（tree-sitter 集成） 的 tree-sitter 基础上，为每种语言写一套符号查询，产出 R01（【接口先行 A1】架构与接口契约） 的符号树（名称、种类、完整区间、选择区间、父子关系、容器名）：

| 语言 | 符号来源 |
|---|---|
| C# | 命名空间、类型、成员（字段、属性、方法、构造函数、事件、枚举成员） |
| Markdown | 标题层级（`#`–`######`） |
| JSON | 键；对象、数组按种类标注 |
| YAML | 键；多文档（`---`）各成一棵子树 |
| XML | 元素名 |

- 语言与扩展名的对应关系来自 R12（语言配置）；符号种类用契约里的枚举，不新造。
- 增量：文件重新解析后只重建受影响的子树，并给出“哪些节点变了”；结果过期时沿用旧结果，不闪烁（与 R20（代码视图） 同一原则）。
- 分级降级（UX_SPEC §3.1）：大文件在后台构建；超大文件不提供符号，大纲中显示原因。

### 2. 资源管理器中的大纲小节（UX_SPEC §1.3）

- 作为资源管理器里的一个可折叠小节注册（顺序：文件夹、大纲），通过 R01（【接口先行 A1】架构与接口契约） 的视图贡献点登记，不写死在侧边栏里；树与键盘导航复用 R18（通用列表、树与快速输入组件）。
- 遵循 `outline.collapseItems`、`outline.icons`、`outline.show*`（各符号种类的开关）；图标颜色取自登记过的 `symbolIcon.*` 颜色键。
- 与光标联动（UX_SPEC §4.2 的“跟随光标”）：光标移动时自动选中并滚动到对应节点，可以在小节菜单中关闭。
- 直接输入即可过滤；点击节点通过 R17（编辑器组与标签） 的打开文档服务跳转并定位。
- 空状态（UX_SPEC §6.3）：「活动编辑器无法提供大纲信息。」

### 3. 面包屑

- 编辑器内容上方的面包屑：文件路径段 + 符号路径段，遵循 `breadcrumbs.enabled`、`breadcrumbs.filePath`、`breadcrumbs.symbolPath`、`breadcrumbs.icons`、`breadcrumbs.show*`、`breadcrumbs.symbolSortOrder`、`breadcrumbs.symbolPathSeparator`。
- 选择器：点击某一段弹出同级列表（`breadcrumbPicker.background`），命令与默认键全部沿用 VS Code：`breadcrumbs.focus`、`focusAndSelect`、`focusNext` / `focusPrevious`（含 `...WithPicker`）、`revealFocused`、`revealFocusedFromTreeAside`、`selectFocused`、`selectEditor`、`toggleToOn`，以及对应的 `when` 上下文键（`breadcrumbsVisible`、`breadcrumbsActive`、`breadcrumbsPossible`）。
- 性能：5000 行 C# 文件中移动光标，面包屑更新 < 1 ms（不得在光标移动时重新遍历整棵树）。

### 4. 转到文件中的符号（`Ctrl+Shift+O`）

`workbench.action.gotoSymbol`，复用 R18（通用列表、树与快速输入组件） 的快速输入组件，与 R24（快速打开与命令面板） 的 `@` 前缀是同一份符号数据与同一套排序；两者谁先合并，另一方接入即可。

### 5. 登记与替换

新增的设置项、颜色键、界面字符串、命令、视图小节都通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入。替换 R02（【接口先行 A2】行走骨架） 中的 `SKELETON(outline)` 标记（如有），删除标记并更新 `docs/ARCHITECTURE.md` 的对照表。

## 不包含

- JSON/YAML/XML 的**值预览、复制路径、复制值、折叠同级**与语法校验：R60（JSON/YAML/XML）（M4）。本 issue 只显示键名 / 元素名 + 类型图标。
- 折叠区间（语法、`#region`、缩进）：R20（代码视图）。
- 粘性滚动：R54（粘性滚动）；工作区符号：R59（转到类型定义、工作区符号、侧边引用视图）；C# 语义符号与元数据视图的面包屑：M2。
- 大纲上的问题徽章（`outline.problems.*`）：问题面板在 R60（JSON/YAML/XML） 之后才有数据，这里先不显示。
- 修改 R01（【接口先行 A1】架构与接口契约） 的符号类型：需要改时走契约变更流程，单独提小 PR。

## 验收标准

- [ ] C#、Markdown、JSON、YAML、XML 五种类型都能产出符号树，各有单元测试（含嵌套、空文件、语法错误的文件）
- [ ] 大纲小节出现在资源管理器中，支持过滤、跟随光标、点击跳转，并遵循 `outline.show*` 与 `outline.collapseItems`
- [ ] 面包屑显示路径段与符号段，选择器可用，`breadcrumbs.*` 设置项生效
- [ ] 5000 行 C# 文件中移动光标，面包屑更新 < 1 ms（附测量数据，基准接入 R04（性能基准与预算守护））
- [ ] `Ctrl+Shift+O` 可用，排序与 R24（快速打开与命令面板） 的 `@` 一致
- [ ] 当前文件类型不支持时显示「活动编辑器无法提供大纲信息。」；超大文件显示原因
- [ ] 新增的设置项、颜色键、界面字符串、命令都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；另加覆盖本 issue 入口的集成测试（打开 `.cs` → 断言大纲节点与面包屑文本）
- [ ] 无运行时网络访问

## 风险与提示

- tree-sitter 查询容易写成“能跑就行”，但大纲的节点顺序、容器名和 VS Code 的 C# 大纲差异会立刻被看出来。用真实的 Unity 脚本（`ROVE_TEST_UNITY_MEDIUM`）逐个对照 VS Code 的大纲截图。
- 面包屑的 1 ms 预算要靠“按位置二分查找符号路径”实现，缓存上一次的路径并在同一节点内直接复用。
- “跟随光标”是大纲视图菜单里的开关，不是设置项（UX_SPEC §4.2 已修正），不要登记新的设置项。
