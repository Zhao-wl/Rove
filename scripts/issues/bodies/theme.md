## 背景

`FEATURES` §7 要求**直接加载 VS Code 的主题 JSON**，这是"零成本迁移"的关键一环；又因为共用设置（D4），`workbench.colorTheme` 可能指向用户在 VS Code 里装的任何主题。所以主题引擎必须按 VS Code 的真实语义工作：`colors`、`tokenColors`、`semanticTokenColors`、`include` 链、用户覆盖、以及主题没有提供某个键时的默认值回退。

实现放在 `rove-theme`，在 [[contracts]] 定义的主题与颜色键登记接口背后；本 PR 不修改这些接口。GPUI 的类型不得出现在这里，颜色用核心层自己的类型表示，到 `rove-ui` 再转换。替换 [[skeleton]] 中的 `SKELETON(theme)` 标记（"硬编码 Dark Modern 中最常用的几条颜色"），并更新 `docs/ARCHITECTURE.md` 的对照表。

## 目标

### 1. 加载 VS Code 主题 JSON

- `colors`（界面颜色）、`tokenColors`（TextMate 规则）、`semanticTokenColors`、`type`（`dark` / `light` / `hc`）、`semanticHighlighting`；
- `include` 链：相对路径解析、多层嵌套、循环检测、被包含文件的规则与本文件的合并顺序（内置的 Dark+ / Light+ 就是通过 include 组装的）；
- 容错：未知字段忽略、颜色值非法时跳过该键并记录，不要整份主题加载失败。

### 2. 移植 vscode-textmate 的主题匹配

从 `vscode-textmate`（MIT，在 `THIRD_PARTY_NOTICES.md` 注明）移植：scope 选择器、**父 scope 选择器**（`meta.function entity.name`）、逗号分隔的多选择器、规则优先级（specificity：按匹配段数与 scope 深度打分，后定义者在同分时胜出）、`fontStyle`（粗体、斜体、下划线、删除线，含显式清空）。输入是 [[syntax]] 产生的 scope 列表，输出是前景色 + 字体样式。

### 3. 颜色键登记与默认值表

- `docs/compat/THEME_COLORS.md` 中"支持 P0"的 403 个键**全部**通过 [[contracts]] 的颜色键登记机制注册（键名 + 深色 / 浅色 / 高对比度默认值），**不允许在使用处写死颜色**。P1 / P2 的键可以随对应功能的 issue 再登记。
- 默认值从 VS Code 源码里 `registerColor` 的注册整理成一张数据表（`THEME_COLORS.md` §1 明确说这张表要在实现时整理）。支持 VS Code 的默认值派生写法：引用另一个键、`transparent(key, 0.5)`、`lighten/darken`，否则大量键的默认值无法表达。
- 主题没有提供某个键时按该键的默认值回退，与 VS Code 一致；主题 `type` 决定用哪一套默认值。

### 4. 用户覆盖

- `workbench.colorCustomizations`：支持全局形式与按主题形式（`"[Dark Modern]": { ... }`）；
- `editor.tokenColorCustomizations`：支持 `comments` / `strings` / `keywords` 等简写键、`textMateRules` 数组、以及按主题形式；
- 合并语义与 VS Code 一致（用户覆盖优先于主题，按主题的覆盖优先于全局）。设置一律通过 [[contracts]] 的设置读取接口获取。

### 5. 内置主题

打包 VS Code 当前的默认主题 **Dark 2026、Light 2026**（VS Code 1.145 的 `workbench.colorTheme` 默认值是 `Dark 2026`，绝大多数用户没改过主题，首屏配色必须与 VS Code 一致），以及 Dark Modern、Light Modern、Dark+、Light+（`extensions/theme-defaults/themes/*.json`，MIT）。`workbench.colorTheme` 指向未安装的扩展主题时回退到内置主题（只读加载扩展主题是 [[ext-themes]]，本 issue 只做回退与一条可解释的日志）。

### 6. 主题变更通知

设置变化、主题文件被改动时重新加载并通知订阅方（接口由 [[contracts]] 定义）。

## 不包含

- 着色黄金测试（[[golden]]）。
- 只读加载用户安装的扩展主题、文件图标主题（[[ext-themes]]）。
- 语义 token 的**产出**（[[semantic]]）；本 issue 只解析 `semanticTokenColors` 与 `semanticHighlighting` 字段，"语义 token 类型 → TextMate scope"的回退表（移植自 `tokenClassificationRegistry`）由 [[semantic]] 补齐。
- 界面上的颜色应用（[[shell]]、[[codeview-hl]]）。

## 验收标准

- [ ] 六个内置主题都能加载，`include` 链正确展开（Dark+ / Light+ 的继承关系有测试）；未设置 `workbench.colorTheme` 时使用 Dark 2026
- [ ] 移植 `vscode-textmate` 的主题匹配测试用例，逐条通过；至少覆盖父 scope 选择器、同分时的后定义优先、`fontStyle` 清空
- [ ] `THEME_COLORS.md` P0 的 403 个键全部完成登记，并有一个测试对照该文档的清单防止漏项
- [ ] 每个 P0 键都有深色 / 浅色 / 高对比度三套默认值；"主题未提供该键 → 回退到默认值"有测试
- [ ] 默认值的派生写法（引用、transparent、lighten/darken）有测试
- [ ] `workbench.colorCustomizations` 与 `editor.tokenColorCustomizations` 的合并语义有测试，含按主题形式
- [ ] 主题文件含未知字段 / 非法颜色值时不致整份失败，并有可定位的日志
- [ ] `workbench.colorTheme` 指向不存在的主题时回退到 Dark 2026，并记录日志
- [ ] `SKELETON(theme)` 标记已删除，`docs/ARCHITECTURE.md` 的对照表已更新
- [ ] 新增集成测试覆盖"加载主题 → 按 scope 解析 token 样式 → 查某个颜色键"；[[skeleton]] 的端到端测试继续通过
- [ ] `cargo tree -p rove-theme -i gpui` 为空

## 风险与提示

- **默认值表是这个 issue 里最费时的部分**，有 403 个 P0 键，而且不少是派生值。建议写一个一次性的提取脚本从 VS Code 源码生成，而不是手抄；脚本和它的输出都提交，并在表头注明 VS Code 版本（当前兼容清单基于 1.145.2）。
- 主题匹配算法的细节（specificity 的计算方式）很容易"看起来对"，但在真实主题上差之千里。一定要先把 `vscode-textmate` 的测试用例移过来跑通，再去跑 [[golden]]。
- 颜色键的登记要在本 crate 完成，但**使用方**遍布界面层。登记时顺手把"哪个功能用这个键"写进注释，可以省掉后续 issue 的反复查找。
