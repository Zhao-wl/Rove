# R70 图形化设置界面

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 3 困难 | ui | 第 4 批 | R13（设置层）、R18（通用列表、树与快速输入组件） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

`FEATURES.md` §7 把设置界面判定为改造、P1：「首版只提供 JSON 编辑（打开 `settings.json`），图形化的设置界面以后再做」；`UX_SPEC.md` §1.7 同样写着「**直接打开 `settings.json`**；图形化设置界面是 P1」。R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 已经按这个口径实现了 `workbench.action.openSettings`（`ctrl+oem_comma`），并明确不注册 `workbench.action.openSettings2`——留给本 issue。

本 issue 就是那个 P1：在**共用的** `settings.json`（D4）之上做一个图形化编辑器。难点不在控件，而在两件事：

1. **只显示 Rove 真正支持的设置项**。共用文件里混着上千个 VS Code 专有项（`SETTINGS.md` 统计：不支持 1112 项、扩展贡献 645 项），图形界面如果把它们也列出来，用户改了却没有任何效果，比不提供界面更糟。
2. **写入不能破坏文件**。写入一律走 R13（设置层） 的保留格式写入（JSONC 区间编辑 + 备份 + 原子写），本 issue 不自己碰文件。

必读：`FEATURES.md` §7（设置界面一行）、§13（共用设置的 7 条规则）；`UX_SPEC.md` §1.7（设置与键位入口）、§6.1（反馈分级）、§6.4（设置文件有语法错误时的处理）；`docs/compat/SETTINGS.md` §2–§4 与「设置」一节（6 个 `workbench.settings.*`）；`docs/compat/KEYBINDINGS.md`「设置」一节（12 条 P1 键位）与 §7 附录（`workbench.action.openSettings2`、`openSettingsJson`、`settings.switchToJSON`）；`THEME_COLORS.md` §4（Settings Editor colors，21 个键）。

## 目标

### 1. 设置编辑器

- 通过 R17（编辑器组与标签） 的编辑器类型登记机制注册设置编辑器（在编辑器组里以标签形式打开，与 VS Code 一致），命令 `workbench.action.openSettings2`、`workbench.action.openSettingsJson`、`settings.switchToJSON`。
- **接管 `workbench.action.openSettings` 的分发**：按 `workbench.settings.editor`（默认 `"ui"`）决定打开图形界面还是 JSON 文件，与 VS Code 一致。R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 的齿轮菜单入口不变，只是目标改由本 issue 决定。⚠️ 这与 `UX_SPEC.md` §1.7 当前那行「直接打开 `settings.json`」不一致——那行描述的是本 issue 完成之前的状态，**需要在本 PR 中同步修订**。
- 顶部显示一次 §1.7 的共用说明（「此文件与 VS Code 共用。Rove 专有的设置放在 `rove.*` 下。」），复用 R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 的提示条组件与「不再提示」状态，文案一致。

### 2. 可见的设置项 = 已登记且已生效的设置项

- 数据源只有一个：R13（设置层） 的设置项登记表（键名、类型、VS Code 默认值、Rove 默认值、是否生效）。**不另外维护一份清单**，否则必然和 `SETTINGS.md` 漂移。
- 判定为「不支持」的 VS Code 设置项**不显示**；已登记但对应功能尚未实现的（P1 / P2 功能未合并）同样不显示——与 `UX_SPEC.md` §1.2「命令面板只列出已实现的命令」同一个原则。
- `rove.*` 专有项单独成一个分类（`SETTINGS.md` §4），并在分类说明里写明「VS Code 会忽略这些项」。
- Rove 与 VS Code 默认值不同的四项（`SETTINGS.md` §3）在界面上显示的是 **Rove 的默认值**，并标注「Rove 默认值与 VS Code 不同」；用户没有改过时**不写入文件**（§13 规则 5）。

### 3. 搜索、分类与状态

- 搜索框（`settings.action.search`，`ctrl+f`）按键名、标题、说明搜索，匹配算法复用 R24（快速打开与命令面板） 已移植的 VS Code 模糊匹配打分，不另写一套；匹配片段高亮。
- 支持 `@modified` 筛选（只看改过的项）。`workbench.settings.settingsSearchTocBehavior`（默认 `filter`）控制搜索时目录的行为。
- 左侧分类目录（TOC）用 R18（通用列表、树与快速输入组件） 的树组件，分类划分沿用 `SETTINGS.md` 的功能区分组。
- **修改指示器**：被用户显式设置过的项在行首显示指示条（`settings.modifiedItemIndicator`），与 VS Code 一致；每行的齿轮菜单提供「重置为默认值」「复制设置 ID」「复制设置为 JSON」。
- 文件有语法错误时（`UX_SPEC.md` §6.4）设置界面**只读展示上一份有效设置**，顶部提示条给出错误位置并提供「在 JSON 中打开」，不允许在错误状态下写入。

### 4. 控件与写入

- 控件按类型：布尔 → 复选框；枚举 → 下拉框（显示 VS Code 的枚举说明文字）；数字 / 字符串 → 输入框（带类型与范围校验）；数组 / 对象（`files.exclude`、`rove.files.extraExclude` 等）→ 与 VS Code 一致，显示「在 settings.json 中编辑」链接并定位到该键。
- 语言级覆盖（`"[csharp]": {...}`）在界面上**只读显示**是否存在覆盖，编辑一律跳转到 JSON（VS Code 的 per-language 编辑器不在范围内）。
- 每次修改都通过 R13（设置层） 的保留格式写入接口落盘（它负责备份、原子写、避免自激重载）；**界面不做批量写**，一次改一个键，与 VS Code 一致。
- 外部（VS Code 或用户手改）改动文件时，界面通过 R13（设置层） 的热加载通知实时刷新，焦点与滚动位置不丢。

### 5. 键盘操作与上下文键

实现「设置」一节的 12 条 P1 键位与对应上下文键：`settings.action.clearSearchResults`、`focusLevelUp`、`focusSearchFromSettings`、`focusSettingControl`、`focusSettingsFromSearch`、`focusSettingsFromSearchOnEnter`、`focusSettingsList`、`focusTOC`、`search`、`showContextMenu`、`showPreviousSearch`，以及 `settings.action.focusSettingsFile`（§7 附录 P1）；上下文键 `inSettingsEditor`、`inSettingsSearch`、`settingRowFocus`、`settingFirstRowFocus`、`settingsTocRowFocus`、`inSettingsJSONEditor` 全部通过 R01（【接口先行 A1】架构与接口契约） 登记。**全部操作可以纯键盘完成**。

### 6. 登记与替换

新增编辑器类型、命令、上下文键、界面字符串与 `THEME_COLORS.md` §4 的 21 个 Settings Editor 颜色键全部通过 R01（【接口先行 A1】架构与接口契约） / R16（工作台） / R10（主题引擎） 的登记机制加入，不得写死颜色。R02（【接口先行 A2】行走骨架） 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增自己的一行。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 与 R13（设置层） 的接口（需要「是否由用户显式设置」「设置项元数据」之类的新查询时，走契约变更流程单独提小 PR）。

## 不包含

- 设置层本身：JSONC 解析、语言级覆盖、热加载、保留格式写入、默认值差异（全部由 R13（设置层） 完成）。
- **图形化键位编辑器**（P2，`FEATURES.md` §7）：`keybindings.editor.*` 的 14 条 P2 键位、`keybindingTable.*` 颜色键一律不实现、不注册；`workbench.action.openGlobalKeybindings`（`ctrl+k ctrl+s`）继续由 R28（欢迎页、最近打开、启动参数打开，以及首选项入口） 打开共用的 `keybindings.json`。
- **工作区级设置**（`.vscode/settings.json`）：R13（设置层） 已记录「首版不做」，因此 `workbench.action.openWorkspaceSettings` / `openWorkspaceSettingsFile`（P0 附录命令）**本 issue 不注册**——作为无主项报出。
- **AI 搜索**：`settings.action.toggleAiSearch` 与上下文键 `aiSettingResultsAvailable` 需要网络与模型，违反 `PROJECT_CHARTER.md` §2，不实现（该键位在 `KEYBINDINGS.md` 中应改判为「不做」，在 `rules.py` 中修正）。
- `workbench.settings.openDefaultSettings`、`workbench.settings.useSplitJSON`、`workbench.settings.openDefaultKeybindings`、`workbench.settings.alwaysShowAdvancedSettings`：都需要「默认设置只读文档」或「advanced 标签」这类额外数据，本 issue **只登记不生效**，并在 PR 中说明；需要时另提 issue。
- 设置同步与配置文件 profiles（`FEATURES.md` §7 明确不做）。

## 验收标准

- [ ] 设置编辑器以标签形式打开；`workbench.settings.editor` 为 `ui` 时 `ctrl+,` 打开图形界面、为 `json` 时打开共用的 `settings.json`
- [ ] 界面中**不存在任何** `SETTINGS.md` 判定为「不支持」的设置项（用一个对照 `SETTINGS.md` 的测试防止漏判）
- [ ] 已登记但功能未实现的设置项不显示；`rove.*` 单独成组并有「VS Code 会忽略这些项」的说明
- [ ] `SETTINGS.md` §3 的四项显示 Rove 默认值并带标注；未改动时**文件中不出现这些键**（字节级断言）
- [ ] 修改一个布尔项、一个枚举项、一个数字项后：文件中只有该键的值变化，**其余字节完全不变**（注释、空行、键顺序、缩进）；备份文件已生成
- [ ] 数组 / 对象类型的项显示「在 settings.json 中编辑」并能定位到该键所在行
- [ ] 搜索：按键名、标题、说明都能搜到；高亮片段正确；`@modified` 只列出改过的项；`settingsSearchTocBehavior` 的两种取值行为不同
- [ ] 修改指示器正确标出被用户显式设置过的项；「重置为默认值」后该键从文件中删除（而不是写入默认值）
- [ ] 「复制设置 ID」「复制设置为 JSON」的内容正确
- [ ] 在 VS Code 中实时修改同一份文件后，设置界面自动刷新，当前焦点与滚动位置不丢失
- [ ] 文件有语法错误时界面只读、顶部提示条给出行列并可跳转到 JSON，期间任何修改操作都被阻止
- [ ] 12 条 P1 键位逐条生效，`when` 条件与 `KEYBINDINGS.md` 一致；**纯键盘**可以完成搜索 → 定位 → 修改 → 返回
- [ ] 21 个 Settings Editor 颜色键全部登记，深色 / 浅色 / 高对比度三套主题下界面可读（截图）
- [ ] 打开设置界面到可交互 **< 100 ms**（`UX_SPEC.md` §6.2：超过 100 ms 才显示加载状态；附测量数据）
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增「打开设置界面 → 搜索 → 改一个键 → 断言文件内容与热加载」的集成测试
- [ ] `UX_SPEC.md` §1.7 已同步修订；`docs/ARCHITECTURE.md` 的对照表已更新；本 PR 没有修改 R01（【接口先行 A1】架构与接口契约） / R13（设置层） 的接口

## 风险与提示

- **「只显示支持的设置项」是本 issue 的核心价值，也是最容易偷懒的地方**。一旦从 `SETTINGS.md` 复制一份清单到界面代码里，两边就会立刻开始漂移。必须从 R13（设置层） 的登记表读，并加一个对照文档的测试。
- 「重置为默认值」在 VS Code 里是**删除该键**，不是写入默认值。写错会把 Rove 的默认值差异固化进共用文件，违反 §13 规则 5。
- 设置项数量很大（`SETTINGS.md` 支持 P0 372 项 + P1 120 项），列表必须虚拟化，否则打开就卡。复用 R18（通用列表、树与快速输入组件） 的虚拟化，不要自己撸一个。
- 枚举项的说明文字来自 VS Code 的 `configurationRegistry`，属于可复用资产（MIT，`FEATURES.md` §11）。建议和 R13（设置层） 的登记表一起由生成脚本产出，并在 `THIRD_PARTY_NOTICES.md` 注明。
- 这是 M4 里难度 3 的 issue，范围边界要守死：**键位编辑器是 P2**，做着做着很容易「顺手也做了」，那会直接撞上 14 条 P2 键位和一整套冲突检测。
