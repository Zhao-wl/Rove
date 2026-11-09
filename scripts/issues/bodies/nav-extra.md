## 背景

M2 的 [[nav-ui]] 与 [[peek]] 已经把 C# 导航的主干做完了（`F12`、`Ctrl+点击`、悬停、历史、`Shift+F12` 速览引用、`Ctrl+F12` 转到实现）。本 issue 收尾三件 P1 的事，它们的共同点是**数据都已经在手，缺的只是入口**：

1. **转到类型定义**（`FEATURES.md` §5「转到定义 / 声明 / 类型定义」，P0 的那一行里唯一还没做的一项；命令在 `KEYBINDINGS.md` §7 附录「P0 · 导航」中，VS Code 没有默认键位）。
2. **转到工作区中的符号**（`Ctrl+T`，`FEATURES.md` §5 判定改造、P1，数据来自 L1 快速索引）。它同时补上 [[quickopen]] 里那个还在显示「工作区符号搜索尚不可用」的 `#` 前缀。
3. **侧边「引用」视图**（`FEATURES.md` §5：D3 让速览成为默认展示方式，侧边树形分组视图作为**备选**，P1）。

必读：`FEATURES.md` §5（转到类型定义、查找所有引用、转到工作区符号三行）、§12（D3），`UX_SPEC.md` §1.4（面板中只有「问题」，「引用」视图启用后也放在这里）、§2.2（近似标记）、§2.4、§6.3（空状态），`LANGUAGE_INTELLIGENCE.md` §1（工作区符号搜索 < 30 ms）、§3.4，`docs/compat/KEYBINDINGS.md`（导航一节的 `references-view.*` 与 `ctrl+t`，§7 附录「P0 · 导航」的 `editor.action.goToTypeDefinition` 与 13 条 `references-view.*`，「P0 · 布局」的 `workbench.view.extension.references-view`），`SETTINGS.md`（`editor.gotoLocation.multipleTypeDefinitions`、`references.preferredLocation`）。

## 目标

### 1. 转到类型定义

- `editor.action.goToTypeDefinition`（**无默认键位**，不要自行分配；出现在命令面板中，用户可自行绑定）。
- 语义：光标在一个表达式 / 变量 / 参数上时，跳到它**声明类型**的定义处（而不是它自己的声明处）。候选数的呈现方式照搬 `editor.gotoLocation.multipleTypeDefinitions`（默认 `peek`）：单个候选直接跳转，多个候选打开 [[peek]] 的速览窗口。
- 上下文键 `editorHasTypeDefinitionProvider` 由本 issue 按当前文档语言设置（C# 为真，其余为假，D2）。它不在 `KEYBINDINGS.md` §6 的上下文键清单里，因为该命令没有默认键位；仍需登记，用于命令面板的可用性与用户自定义键位的 `when`。
- 跳转前压入 [[nav-ui]] 的导航历史，跳转后用 [[codeview]] 的滚动定位接口按 `revealRangeInCenterIfOutsideViewport` 定位；外部符号交给 [[meta-ui]]；无结果时按 `UX_SPEC.md` §6.3 在**光标处**提示「未找到 `X` 的类型定义」，近似时追加「（语义索引构建中）」。
- **数据来源**：[[contracts]] 的 C# 导航查询接口已包含「类型定义」查询，由 [[router]] 实现。

### 2. 转到工作区中的符号（`Ctrl+T`）

- `workbench.action.showAllSymbols`（`ctrl+t`，P1），以及 [[quickopen]] 中的 `#` 前缀——**两者是同一份数据、同一套排序、同一个实现**，`#` 只是把 `Ctrl+T` 的内容放进快速打开。
- 数据来自 [[l1]] 的工作区符号模糊搜索（L1 已经复用 [[quickopen]] 移植的 VS Code 模糊匹配算法，这里**不要再写一套打分规则**）；[[router]] 判定 L2 可用时优先用 L2 的符号表，结果精确度按 `UX_SPEC.md` §2.2 处理。
- 列表项：符号名 + 种类图标（`symbolIcon.*`）+ 容器名 + 文件相对路径；复用 [[lists]] 的快速输入组件。选中后经 [[groups]] 的打开文档服务打开并定位（`.md` 打开源码，`UX_SPEC.md` §4.3）。
- **近似标记**：结果来自 L1 时在列表顶部加一行克制的说明（`UX_SPEC.md` §2.2「近似结果永远要标明，但标注要克制」），不在每一条结果上重复标注；精确结果不加任何标记。
- 空状态按 `UX_SPEC.md` §6.3 的措辞风格；工作区中没有 C# 代码时不报错。
- 预算：**每次按键到列表刷新 < 30 ms**（`LANGUAGE_INTELLIGENCE.md` §1，在 `ROVE_TEST_UNITY_LARGE` 上测），与 [[quickopen]] 的 16 ms 预算分开报。

### 3. 侧边「引用」视图（D3 的备选形态）

- 位置：按 `UX_SPEC.md` §1.4 放在**面板**中（与「问题」并列），通过 [[shell]] 的视图贡献点登记；`workbench.view.extension.references-view`（§7 附录，无默认键位）用于显示该视图。
- 入口与导航（`KEYBINDINGS.md` 导航一节，when 条件照搬）：

| 命令 | 默认键 | when |
|---|---|---|
| `references-view.findReferences` | `shift+alt+f12` | `editorHasReferenceProvider` |
| `references-view.next` | `f4` | `reference-list.hasResult && references-view.canNavigate` |
| `references-view.prev` | `shift+f4` | 同上 |

- `references.preferredLocation`（默认 `"peek"`）决定 `Shift+F12` 用速览还是侧边视图；为 `"view"` 时 [[peek]] 的 `editor.action.goToReferences` 改为在本视图中展示。**这个判定要做成一处开关**，由本 issue 提供、[[peek]] 调用，不要在两边各写一份。
- `KEYBINDINGS.md` §7 附录「P0 · 导航」中的其余 `references-view.*` 命令全部以相同 ID 实现：`clear`、`clearHistory`、`copy`、`copyAll`、`copyPath`、`findImplementations`、`pickFromHistory`、`refind`、`refresh`、`removeReferenceItem`、`tree.focus`、`tree.resetViewLocation`。`removeCallItem` / `removeTypeItem` / `showIncomingCalls` / `showOutgoingCalls` / `showSubtypes` / `showSupertypes`（调用层级与类型层级，P2）只登记命令、行为为空并在命令面板中隐藏。
- 结果树复用 [[lists]] 的虚拟化树，分组与计数规则、4.7 万条结果下的构建方式与 [[peek]] 一致（数据层给出「文件 → 计数 + 偏移」的分组索引，界面按需取段）。上下文键 `reference-list.hasResult`、`references-view.canNavigate` 由本 issue 维护。

### 4. 登记与替换

新增命令、设置项、视图、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。查询一律经 [[contracts]] 的 C# 导航查询接口，打开文件一律经打开文档服务。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- `F12`、`Ctrl+悬停`、悬停卡片、导航历史、语义同名高亮、近似提示的文案决策（[[nav-ui]]）；**转到实现（`editor.action.goToImplementation`）已在 M2 的 [[nav-ui]] 中完成**，本 issue 不重做。
- 速览窗口本身（[[peek]]）；元数据视图（[[meta-ui]]）。
- 查询路由、引用 / 关系数据、精确度判定（[[router]]）；L1 索引的构建与持久化（[[l1]]）。
- 快速打开的框架与模糊匹配算法移植（[[quickopen]]）；`@` 前缀的文件内符号（[[outline]]）。
- 调用层级、类型层级、引用数量提示（P2）；`editor.showTypeHierarchy`、`references-view.showTypeHierarchy`（P2，只登记）。
- 「问题」视图与 `editor.action.marker.*`（[[structure]]）。

## 验收标准

- [ ] `editor.action.goToTypeDefinition` 可用且**无默认键位**；在 `var` 变量、字段、参数、属性、泛型类型参数上各验证一例
- [ ] `editor.gotoLocation.multipleTypeDefinitions` 的取值生效：单候选直接跳转、多候选打开速览；`editorHasTypeDefinitionProvider` 在非 C# 文件为假且命令不可用
- [ ] 转到类型定义无结果时光标处提示「未找到 `X` 的类型定义」，近似时追加「（语义索引构建中）」；外部类型交给 [[meta-ui]]
- [ ] `Ctrl+T` 与 [[quickopen]] 的 `#` 前缀是同一份数据、同一套排序（断言两者对同一输入返回相同序列）；[[quickopen]] 中「尚不可用」的降级提示已删除
- [ ] 工作区符号搜索在 `ROVE_TEST_UNITY_LARGE` 上**每次按键 < 30 ms**（p99，附测量数据，基准接入 [[perf]]）
- [ ] 工作区符号结果来自 L1 时有一行克制的近似说明、精确时无任何标记；选中后经打开文档服务打开并定位（`.md` 打开源码）
- [ ] 侧边「引用」视图出现在面板中，`shift+alt+f12` 打开、`f4` / `shift+f4` 导航，when 条件与 `KEYBINDINGS.md` 导航一节逐条一致
- [ ] `references.preferredLocation` 的两个取值都生效：`peek` 时 `Shift+F12` 开速览、`view` 时在侧边视图展示；判定只有一处实现
- [ ] §7 附录中的 12 条 `references-view.*` 命令逐条可用并有测试；层级相关的 6 条只登记、不出现在命令面板
- [ ] 4.7 万条结果下视图可用且满足 [[lists]] 的帧预算（附测量数据）
- [ ] `reference-list.hasResult` 与 `references-view.canNavigate` 取值正确
- [ ] 新增的命令、设置项、视图、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（`Ctrl+T` 搜索 → 打开并定位；`Shift+Alt+F12` → 断言视图中的分组与计数）

## 风险与提示

- 如果 [[router]] 的类型定义查询在某些写法（`var`、泛型、元组）上返回空结果，按 UX_SPEC §6.3 的空状态提示处理，并把用例补进 [[navtests]]。
- `Ctrl+T` 和 `#` 必须共用实现。它们在 VS Code 里本来就是同一个 quick access provider，分成两份一定会出现「两个入口排序不一样」的 bug。
- 侧边引用视图是 D3 的**备选**，不是默认。不要把它做成和速览并列的主路径，也不要在没有用户设置的情况下改变 `references.preferredLocation` 的默认值（`FEATURES.md` §13 规则 5）。
- `f4` / `shift+f4` 在 `KEYBINDINGS.md` 中同时绑定给 `goToNextReference`（速览，when `inReferenceSearchEditor || referenceSearchVisible`）和 `references-view.next`（视图，when `reference-list.hasResult && references-view.canNavigate`）。两条 when 条件要能同时成立而不打架——实现时逐条照抄，不要「合并简化」。
- 4.7 万条结果的坑与 [[peek]] 完全一样：慢的是树的**构建**而不是滚动。直接复用 [[peek]] 已经做好的分组索引，不要把 `Vec<Location>` 展平成界面节点。
