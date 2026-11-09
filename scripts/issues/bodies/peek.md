## 背景

决策 D3（`FEATURES.md` §12）把速览窗口从“顺手照搬的小功能”变成了 Rove 查找引用的**默认展示方式**，所以它在 `FEATURES.md` §5 中被提升为 P0：`Shift+F12` 不打开侧边视图，而是在当前行下方内嵌展开——左侧只读代码预览，右侧按文件分组的结果列表。

这个窗口要正面扛住 C# 项目的规模。`M0_LANG_SPIKE_REPORT.md` §2 实测最重的符号有 28,671 处引用（`UX_SPEC.md` §2.3 按“最多 4.7 万”设计上限），查询本身只要 1.95 ms，**慢的只可能是界面**。因此 `UX_SPEC.md` §2.3 规定了一条与 VS Code 不同的展开规则：结果总数 ≤ 100 时展开全部文件，超过时只展开第一个文件、其余折叠并在文件名旁显示数量。

窗口作为“行下方的嵌入区域”挂进 [[codeview]]（见 [[codeview]] 目标第 7 节预留的接口），结果列表复用 [[lists]] 的虚拟化树，数据来自 [[router]]，在编辑器中打开结果走 [[groups]] 的打开文档服务。

必读：`UX_SPEC.md` §2.2（近似标记）、§2.3（速览窗口表格）、§6.2（加载）、§6.3（空状态），`FEATURES.md` §5、§12（D3），`M0_LANG_SPIKE_REPORT.md` §2、§3，`docs/compat/KEYBINDINGS.md`（导航 60 条），`SETTINGS.md`（`editor.stablePeek`、`editor.peekWidgetDefaultFocus`、`editor.gotoLocation.*`），`THEME_COLORS.md`（Peek view colors，16 个键）。

## 目标

### 1. 窗口骨架

- 以 [[codeview]] 的“行下方嵌入区域”实现，而不是浮层：窗口占据文档流中的若干行高，上下文代码不被遮挡，可拖动边框调整高度，滚动时随行移动。
- 三块结构与颜色键全部照搬：标题栏（`peekViewTitle.background`、`peekViewTitleLabel.foreground`、`peekViewTitleDescription.foreground`）、左侧预览（`peekViewEditor.background`、`peekViewEditorGutter.background`、`peekViewEditor.matchHighlightBackground` / `matchHighlightBorder`）、右侧结果树（`peekViewResult.background`、`fileForeground`、`lineForeground`、`matchHighlightBackground`、`selectionBackground` / `selectionForeground`），外框 `peekView.border`。粘性滚动的两个键（`peekViewEditorStickyScroll*`）随 [[sticky]]（M4）接入，本 issue 只登记。
- **左侧预览只读**（`UX_SPEC.md` §2.3 与 VS Code 的差异项）：复用 [[codeview]] 的只读选区与高亮，可滚动、可选中、可复制，不能编辑。上下文键：`referenceSearchVisible`（窗口可见）、`inReferenceSearchEditor`（焦点在左侧预览）、`isInEmbeddedEditor`；嵌套速览（在速览里再开速览）时维护一个窗口栈，关闭时逐层返回。

### 2. 三个入口

| 命令 | 默认键 | 数据 |
|---|---|---|
| `editor.action.goToReferences` | `shift+f12` | [[router]] 的引用表（按文件分组，带读 / 写 / 调用标注） |
| `editor.action.peekDefinition` | `alt+f12` | 定义；也被 [[nav-ui]] 在多候选时调用（`editor.gotoLocation.multipleDefinitions` 默认 `peek`） |
| `editor.action.peekImplementation` | `ctrl+shift+f12`（P1） | [[router]] 暴露关系表后才有数据 |

`editor.action.referenceSearch.trigger`（P0，无默认键位）同样登记。实现关系尚不可用时，`peekImplementation` 不打开空窗口，而是在光标处提示“未找到实现”（`UX_SPEC.md` §6.1 第 2 级）——这是一条**随 [[router]] 能力变化而变化的降级路径**，要写成一处开关，不要散落在界面代码里。结果为空时同样不打开窗口：按 §6.3 在光标处提示“未找到 `X` 的引用”，近似结果时追加“（语义索引构建中）”。

### 3. 结果列表与展开规则

- 两级树：文件节点（路径 + 该文件内的结果数）→ 结果行（行号 + 单行代码片段 + 匹配高亮）。
- **展开规则（`UX_SPEC.md` §2.3）**：结果总数 ≤ 100 时展开全部文件；> 100 时只展开第一个文件，其余折叠，文件名旁显示数量。
- 虚拟化用 [[lists]] 的树组件，必须支持 4.7 万条结果：构建树时不得为每条结果建立界面节点，分组计数要由数据层一次算出，不能遍历界面模型。
- 选中结果时左侧预览切换到该位置并高亮匹配区间；选中文件节点时预览定位到该文件的第一条结果。

### 4. 键盘与打开（`UX_SPEC.md` §2.3 + KEYBINDINGS 导航一节）

- `↑` / `↓` 在结果间移动，`enter` → `revealReference`、`ctrl+enter` → `openReferenceToSide`（when 条件照搬，均要求 `listFocus && referenceSearchVisible && !inputFocus`），双击等同 `Enter`。打开一律经 [[groups]] 的打开文档服务，并传“来自跳转 / 引用”的来源语义（`.md` 等类型要打开源码并定位）。
- `escape` / `shift+escape` → `closeReferenceSearch`，when 中带 `!config.editor.stablePeek`：`editor.stablePeek` 为 true 时双击内容或按 `Esc` 都不关闭窗口。
- `f4` / `shift+f4` 与 `f12` / `shift+f12` → `goToNextReference` / `goToPreviousReference`（when `inReferenceSearchEditor || referenceSearchVisible`）；焦点在左侧预览中时的两个变体 `goToNextReferenceFromEmbeddedEditor` / `goToPreviousReferenceFromEmbeddedEditor`（P0，无默认键位）同样实现。
- `ctrl+k f2` → `togglePeekWidgetFocus` 在左侧预览与右侧树之间切换焦点；初始焦点遵循 `editor.peekWidgetDefaultFocus`（默认 `tree`）。
- 在左侧预览中继续导航：`Alt+F12` 开嵌套速览、`Ctrl+点击`跳转（`F12` 在窗口打开期间已被 `goToNextReference` 占用，见“风险与提示”）。

### 5. 近似标记

标题为“符号名 + 结果数”，近似结果时加“近似”标记，多候选定义时为“近似 · N 个候选”（`UX_SPEC.md` §2.2、§2.3）。标记文案由 [[nav-ui]] 随调用传入，本 issue 只负责呈现，并且**精确结果不加任何标记**。按名称匹配可能包含无关项这一点只体现在标题的“近似”二字上，不在每一条结果上重复标注。

### 6. 外部符号片段（与 [[meta-ui]] 的接缝）

结果或定义落在外部符号（DLL）上时，左侧预览显示元数据视图的片段（`UX_SPEC.md` §2.3 最后一行）。[[meta-ui]] 依赖 [[nav-ui]] 而不是本 issue，**合并顺序上很可能晚于本 issue**，所以这里定义接缝：

- 本 issue 定义一个“外部符号片段提供方”登记点，并自带一个保底实现：只显示一行 `外部符号：<签名>`（签名来自 [[router]] 的悬停数据）。
- [[meta-ui]] 合并后向这个登记点注册真正的片段提供方，本 issue 的代码不需要改动。
- 两种状态都要有测试，PR 中说明当前生效的是哪一种。

### 7. 加载与性能

300 ms 内没有结果时在窗口内显示“正在加载…”（`UX_SPEC.md` §6.2）；窗口本身立即出现，不等数据。**预算：最重的符号（M0 实测 28,671 处引用，设计上限 4.7 万）打开速览到首次绘制 < 100 ms**，之后滚动结果树仍满足 [[codeview]] / [[lists]] 的帧预算；查询侧预算（引用 < 50 ms）属于 [[router]]，本 issue 单独报界面侧耗时。关闭窗口要彻底释放结果数据与预览文档，重复开关 100 次不增长常驻内存。

### 8. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。[[skeleton]] 中没有速览的最简实现，因此本 issue 不删除 `SKELETON(...)` 标记，但必须保持它的端到端测试通过，并在 `docs/ARCHITECTURE.md` 中登记速览窗口的归属。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 查询路由、引用 / 关系数据本身、精确度判定（[[router]]）。
- `F12`、`Ctrl+点击`、悬停、导航历史、语义同名高亮、近似提示的文案决策（[[nav-ui]]）。
- 元数据视图文档与其界面（[[meta-ui]]）、元数据内容生成（[[idx-meta]]）。
- 侧边“引用”视图（`references-view.*`、`shift+alt+f12`）与转到实现命令本身：[[nav-extra]]（M4，P1 备选形态）。
- 调用层级、类型层级、测试输出等其他使用 peek 形态的窗口（P2）；`editorMarkerNavigation*` 的问题速览属于 [[structure]]。
- 从速览结果加入上下文篮子（[[basket]]，`rove.contextBasket.contextLines` 在那里生效）。

## 验收标准

- [ ] `Shift+F12`、`Alt+F12`、`Ctrl+Shift+F12` 三个入口可用，when 条件与 `KEYBINDINGS.md` 导航一节逐条一致
- [ ] 左侧预览**只读**：可滚动、可选中复制，输入不改变内容
- [ ] 展开规则：构造 100 条与 101 条两个用例，分别断言“全部展开”与“只展开第一个文件、其余折叠且显示数量”
- [ ] **最重符号（≥ 2.8 万处引用）打开速览到首次绘制 < 100 ms**（p99，附测量数据，基准接入 [[perf]]）；4.7 万条结果下滚动结果树满足帧预算
- [ ] 键盘全套：`↑↓`、`Enter`、`Ctrl+Enter`（旁边打开）、`F4` / `Shift+F4`、`Ctrl+K F2`、`Esc`；`editor.stablePeek` 为 true 时 `Esc` 与双击都不关闭
- [ ] `editor.peekWidgetDefaultFocus` 的两个取值（`tree` / `editor`）都生效
- [ ] 双击与 `Enter` 经打开文档服务在编辑器中打开并定位，来源语义正确（`.md` 打开源码）
- [ ] 嵌套速览：在预览中 `Alt+F12` 再开一层，`Esc` 逐层返回，上下文键随层级正确
- [ ] 近似结果时标题带“近似”/“近似 · N 个候选”；精确结果标题无额外标记
- [ ] 外部符号：[[meta-ui]] 未合并时显示 `外部符号：<签名>` 保底片段，注册真实提供方后显示元数据片段——两种状态各有测试
- [ ] 实现关系不可用时 `Ctrl+Shift+F12` 提示“未找到实现”，不打开空窗口；结果为空时按 §6.3 在光标处提示
- [ ] 重复开关速览 100 次后常驻内存不增长（附数据）
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（`Shift+F12` → 断言窗口可见、分组与计数正确、`Enter` 后文档已打开并定位）

## 风险与提示

- `UX_SPEC.md` §2.3 写“在预览中可以再按 `F12`”，而 `KEYBINDINGS.md` 导航一节中 `f12` 在 `inReferenceSearchEditor || referenceSearchVisible` 时绑定的是 `goToNextReference`（与 VS Code 一致）。本 issue 按**键位表**实现（§0.1 基线：没写到的地方一律与 VS Code 一致），“继续跳转”通过 `Alt+F12` 嵌套速览与 `Ctrl+点击`达成。这条差异要在 PR 中写明，必要时回头修 `UX_SPEC.md` §2.3 的措辞。
- 4.7 万条结果最容易在**树的构建阶段**而不是滚动阶段超预算。先让数据层给出“文件 → 计数 + 偏移”的分组索引，界面只按需取某个文件的某一段结果；千万不要先把 `Vec<Location>` 展平成界面节点。
- 嵌入区域会改变“屏幕行 ↔ 文档行”的映射，这与 [[codeview-hl]] 的折叠共用同一套映射。接入时要用 [[codeview]] 已有的映射结构，不要在速览里另算一份坐标。
- `editor.stablePeek` 的 when 条件出现在四条 `closeReferenceSearch` 键位里，很容易只实现其中一条。建议把“能否关闭”做成一个统一判定，四条键位共用。
- 预览里的代码也要走正常的高亮与整形缓存路径（[[codeview-hl]]），不要为速览复制一套简化渲染——否则同一段代码在主编辑器和速览里颜色不一致。
