# R40 C# 导航交互：转到定义、转到实现、Ctrl+点击、悬停、导航历史、语义级同名高亮、近似结果提示

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M2 C# 导航 | 3 困难 | ui | 第 6 批 | R38（查询路由与未保存修改的位置映射）、R20（代码视图）、R17（编辑器组与标签） | R42（元数据视图界面）、R44（【集成验收】M2）、R58（C# 语义高亮与不生效预处理代码置灰） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

R38（查询路由与未保存修改的位置映射） 已经能回答“这个位置的符号定义在哪、被谁引用”，并给每个结果打上精确度标记（精确 / 近似 / 语法，见 `LANGUAGE_INTELLIGENCE.md` §3.2）。本 issue 是这套数据的**手指**：按下 `F12` 要落到正确的行，`Ctrl+悬停`要出现下划线和定义预览，悬停卡片要给出签名，`Alt+←` 要退回来，光标停在标识符上时要只高亮**同一个符号**。

两件事决定了本 issue 的形态：

1. **近似结果必须标明，但标注要克制**（`UX_SPEC.md` §2.2 的原则）。精确结果没有任何额外标记——这是 Rove 与“常驻语言服务型”编辑器最直观的差别：大多数时候界面上什么都没多。
2. **C# 的同名高亮必须是语义级的**。`M0_LANG_SPIKE_REPORT.md` §3 实测：大型项目里 `Init` 有 789 个同名符号、按名称命中 1761 处，而真正属于某一个 `Init` 的平均只有 2.2 处。R20（代码视图） 的文本同名高亮在 C# 文件里基本等于噪音，本 issue 负责把它替换掉。

必读：`UX_SPEC.md` §2.2、§2.4、§6.1（反馈分级）、§6.3（空状态），`FEATURES.md` §3（悬停、同名高亮两行）、§5，`LANGUAGE_INTELLIGENCE.md` §3.2，`M0_LANG_SPIKE_REPORT.md` §2、§3，`PROJECT_CHARTER.md` §6，`docs/compat/KEYBINDINGS.md`（导航 60 条），`SETTINGS.md`（`editor.gotoLocation.*`、`editor.hover.*`、`editor.occurrencesHighlight*`、`editor.definitionLinkOpensInPeek`），`THEME_COLORS.md`（Editor widget colors、Editor colors 中的 `editor.wordHighlight*`、`editorLink.activeForeground`）。

## 目标

### 1. 转到定义

- `editor.action.revealDefinition`（`f12`，when `editorHasDefinitionProvider && editorTextFocus`）、`editor.action.revealDefinitionAside`（`ctrl+k f12`，经 R17（编辑器组与标签） 的打开文档服务在旁边的组打开）。`editor.action.goToDeclaration`（无默认键位）在 C# 中与转到定义等价，登记为同一行为。
- **上下文键** `editorHasDefinitionProvider` / `editorHasReferenceProvider` 由本 issue 按当前文档语言设置：C# 文件为真，其他语言为假（D2：其他语言只有高亮和折叠）。
- 候选数决定呈现方式，取值照搬 `editor.gotoLocation.multipleDefinitions`（默认 `peek`）：单个候选直接跳转；多个候选（典型是 `partial` 类、重载）打开速览窗口，窗口本身是 R41（速览窗口）。`editor.gotoLocation.multipleReferences` / `multipleDeclarations` 同样遵循。
- 多候选时进入 VS Code 的**符号导航模式**：设置 `hasSymbols` 上下文键，`f12` 触发 `editor.gotoNextSymbolFromResult` 在候选间循环，`escape` 触发 `editor.gotoNextSymbolFromResult.cancel`，并在状态栏显示“第 N / 共 M 个符号”。
- 目标是外部符号（`UnityEngine.Transform` 等）时，交给 R42（元数据视图界面） 打开元数据视图。R42（元数据视图界面） 尚未合并时，按 `UX_SPEC.md` §4.1 末行显示状态栏临时提示“未找到 `Xxx` 的元数据”，**不打开空标签**。
- 跳转前把当前位置压入导航历史（见第 4 节）；跳转后按 R19（代码视图 Element） 的滚动定位接口把目标行置入视口（照搬 VS Code 的 `revealRangeInCenterIfOutsideViewport`）。
- 没有结果时按 `UX_SPEC.md` §6.3：**光标处**提示“未找到 `X` 的定义”（§6.1 的第 2 级反馈，用 R16（工作台） 的接口）；当前是近似结果时追加“（语义索引构建中）”。

### 2. `Ctrl+悬停`与 `Ctrl+点击`

按住 `Ctrl` 悬停在标识符上时显示下划线（`editorLink.activeForeground`）并弹出定义预览；`Ctrl+点击` 跳转（`UX_SPEC.md` §2.4）。命中测试与下划线装饰走 R19（代码视图 Element） 的装饰与命中测试接口。`editor.definitionLinkOpensInPeek`（P1，默认 false）为真时改为打开速览。

### 3. 悬停卡片

- `editor.action.showHover`（`ctrl+k ctrl+i`）与鼠标悬停，遵循 `editor.hover.enabled` / `delay`（300 ms）/ `sticky` / `hidingDelay` / `above` / `showLongLineWarning`；颜色 `editorHoverWidget.background` / `foreground` / `border` / `statusBarBackground` / `highlightForeground`。
- 内容：签名 + XML 文档注释（来自 R38（查询路由与未保存修改的位置映射））。**只有近似结果**才在底部加一行灰色小字“近似结果”（`UX_SPEC.md` §2.2）；精确结果不加任何标记，也不显示“精确”字样。不显示诊断与快速修复（`FEATURES.md` §3）。
- 卡片内键盘导航照搬：`editor.action.scrollUpHover` / `scrollDownHover` / `scrollLeftHover` / `scrollRightHover`、`pageUpHover` / `pageDownHover`、`goToTopHover` / `goToBottomHover`，以及 `editorHoverFocused` 上下文键。悬停详细程度的两个命令（`increase/decreaseHoverVerbosityLevel`，P0 无默认键位）本 issue 只登记命令，行为是“只有一级”。
- 300 ms 内没有结果时显示“正在加载…”（`UX_SPEC.md` §6.2；查询本身是微秒级的，只有索引冷读取时才会出现）。

### 4. 导航历史

导航历史由 R17（编辑器组与标签） 在 M1 实现。本 issue 的每一次跳转（转到定义、转到实现、`Ctrl+点击`、从速览打开）都通过打开文档服务进行，因此自动进入历史，不另外实现。

### 5. C# 语义级同名高亮

- 替换 R20（代码视图） 在 C# 文件中的文本同名高亮：只高亮**同一个符号**的出现，按 R38（查询路由与未保存修改的位置映射） 返回的读 / 写 / 其他区分 `editor.wordHighlightBackground` / `Border`（读）、`wordHighlightStrongBackground` / `Border`（写）、`wordHighlightTextBackground` / `Border`（文本匹配回退）。
- 结果是近似时**退化为按文本高亮，并且不加任何提示**（`UX_SPEC.md` §2.2 最后一行）；非 C# 文件仍走 R20（代码视图） 的文本实现。
- 遵循 `editor.occurrencesHighlight`（默认 `singleFile`）、`editor.occurrencesHighlightDelay`、`editor.occurrencesHighlightFromAllProviders`；维护 `hasWordHighlights` 上下文键，并实现 `editor.action.wordHighlight.next` / `prev`（`f7` / `shift+f7`，P1）与 `editor.action.wordHighlight.trigger`（P1，无默认键位）。
- 概览标尺上的 `editorOverviewRuler.wordHighlight*` 标记由 R55（缩略图与概览标尺） 绘制，本 issue 只提供位置集合。

### 6. 近似结果的呈现规则（`UX_SPEC.md` §2.2 逐行落地）

| 场景 | 本 issue 的行为 |
|---|---|
| 转到定义，单个候选 | 直接跳转，状态栏临时显示“近似结果：语义索引未就绪”，3 秒后消失（§6.1 第 1 级） |
| 转到定义，多个候选 | 打开速览，标题中注明“近似 · N 个候选”——标题文案由本 issue 传给 R41（速览窗口） |
| 查找引用 | 速览标题注明“近似”（同上） |
| 悬停 | 底部一行灰色“近似结果” |
| 同名高亮 | 退化为文本高亮，无提示 |

同一次会话里对**同一个符号**的重复近似跳转只提示一次，避免 3 秒提示连续刷屏。索引状态栏条目本身属于 R36（索引生命周期与状态），本 issue 不画也不改它。

### 6.1 转到实现

`editor.action.goToImplementation`（`ctrl+f12`）：使用 R38（查询路由与未保存修改的位置映射） 提供的实现与派生关系；只有一个结果时直接跳转，多个结果时按 `editor.gotoLocation.multipleImplementations` 打开速览（与 R41（速览窗口） 的“速览实现”使用同一份数据）。没有结果时在光标处提示“未找到 `X` 的实现”。放在 M2 是为了与 R41（速览窗口） 的 `ctrl+shift+f12` 速览实现对称。

### 7. 性能预算

`F12` → 目标行画面完成 **< 50 ms**（目标文件已打开时）。R38（查询路由与未保存修改的位置映射） 的查询在索引就绪时是微秒级（`M0_LANG_SPIKE_REPORT.md` §2：定义跳转 avg 3.5 µs / p99 12.9 µs），所以这 50 ms 几乎全部是界面成本，必须有预算分解。目标文件未打开时，打开 + 定位的耗时计入 R17（编辑器组与标签） 打开文档服务的预算（100 ms 内不显示加载状态）。同名高亮的计算不得进入绘制关键路径：装饰按 R19（代码视图 Element） 的协议可缓存、可按行失效（见 R19（代码视图 Element） 的 1 ms 每帧预算）。

### 8. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死。查询一律经 R01（【接口先行 A1】架构与接口契约） 的“C# 导航查询”接口，打开文件一律经打开文档服务，装饰 / 选区 / 滚动一律经 R19（代码视图 Element） 的接口。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 速览窗口本身（R41（速览窗口））：本 issue 只决定“什么时候开、标题写什么”，窗口与结果列表由 R41（速览窗口） 实现。
- 元数据视图文档的打开与界面（R42（元数据视图界面））、元数据内容的生成（R34（外部符号元数据视图的生成））。
- 查询路由、位置映射、精确度判定（R38（查询路由与未保存修改的位置映射））；索引状态栏条目与失败通知（R36（索引生命周期与状态））。
- 转到类型定义（`editor.action.goToTypeDefinition`）、转到工作区符号（`Ctrl+T`）、侧边引用视图（`references-view.*`、`shift+alt+f12`）：R59（转到类型定义、工作区符号、侧边引用视图）（M4）。
- C# 语义高亮（着色）：R58（C# 语义高亮与不生效预处理代码置灰）（M4）。调用层级、类型层级、引用数量提示：P2。
- 非 C# 文件的文本同名高亮与括号匹配（R20（代码视图））。

## 验收标准

- [ ] `ctrl+f12` 转到实现：单个结果直接跳转，多个结果打开速览，无结果时光标处提示；在接口、虚方法、抽象方法上各验证一例
- [ ] 用 R39（C# 导航准确率测试集） 的用例在界面中手工抽查 **≥ 20 个**，逐条记录（分类覆盖 `var`、重载、扩展方法、泛型、partial、接口实现、Unity API）
- [ ] `F12` → 目标行画面完成 **< 50 ms**（目标文件已打开，p99，附测量数据与预算分解，基准接入 R04（性能基准与预算守护））
- [ ] 多定义（`partial` 类）按 `editor.gotoLocation.multipleDefinitions` 打开速览；`hasSymbols` 模式下 `F12` 循环、`Esc` 取消、状态栏显示“第 N / 共 M 个”
- [ ] `Ctrl+悬停`出现下划线与定义预览，`Ctrl+点击`跳转；`editor.definitionLinkOpensInPeek` 为 true 时改为速览
- [ ] 悬停卡片：精确结果**没有任何**精确度标记；近似结果底部有灰色“近似结果”；`editor.hover.delay` / `enabled` / `sticky` 生效
- [ ] 导航历史跨文件、跨标签、跨分屏正确，目标标签已关闭时能重新打开；`canNavigateBack` / `canNavigateForward` 取值正确
- [ ] 语义同名高亮：在 `M0_LANG_SPIKE_REPORT.md` §3 列出的 `Init`、`Update`、`Dispose`、`Get`、`Value` 五个名称上各取一处，断言高亮数量等于语义引用数而不是按名称命中数；近似时退化为文本高亮且无提示
- [ ] `F7` / `Shift+F7` 在同名高亮间跳转，`hasWordHighlights` 上下文键正确
- [ ] 近似呈现的五种场景逐条有测试；同一符号的重复近似跳转只提示一次
- [ ] 空状态：光标处“未找到 `X` 的定义”，近似时追加“（语义索引构建中）”
- [ ] 非 C# 文件不设置 `editorHasDefinitionProvider`，`F12` 不弹错误、不显示空速览
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增集成测试（`F12` → 断言文档已打开并定位、历史可回退、同名高亮区间集合）

## 风险与提示

- 最大的坑是把同名高亮做成“光标一动就查一次 + 全文重算装饰”。查询虽然是微秒级，但装饰提交如果每帧重来，R19（代码视图 Element） 的 1 ms 预算立刻失守。正确做法是：按 `occurrencesHighlightDelay` 去抖、结果按行缓存、光标仍在同一符号内时不重查。
- 近似与精确之间会**在用户眼皮底下切换**（索引构建完成、文件保存后重建完成）。切换时不要重弹提示、不要闪烁高亮；建议订阅 R38（查询路由与未保存修改的位置映射） 的精确度变更事件，静默刷新装饰。
- `PROJECT_CHARTER.md` §6 的“定义跳转 < 16 ms”是索引侧的查询口径（`M0_LANG_SPIKE_REPORT.md` §2 即按此测量）；本 issue 的 50 ms 是“按键到画面完成”的端到端口径，PR 中要分开报。冷缓存下定义跳转 p99 曾到 166 ms、max 2.8 s（§2 的 HDD 复测），这由 R36（索引生命周期与状态） 的后台预读兜底，但界面必须按 §6.2 在 300 ms 后显示“正在加载…”而不是卡住。
- 措辞按 `UX_SPEC.md` §6 的风格：先说发生了什么，再说可以怎么做，不要“错误：”前缀，也不要在精确结果上加“精确”字样。
