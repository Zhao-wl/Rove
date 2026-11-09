## 背景

语法高亮只认识「这是一个标识符」，语义高亮认识「这是一个**只读静态字段**」。`FEATURES.md` §3 对语义高亮的判定是**改造**、P1：VS Code 靠语言服务实时产出语义 token，Rove 直接从 C# 预计算索引里取——索引里本来就有每个符号的种类与修饰符（`LANGUAGE_INTELLIGENCE.md` §3.3 的符号表），不需要任何常驻语言服务。

`SYNTAX_AND_LANGUAGES.md` §4 已经把方案写死了四条，本 issue 就是把这四条落地：标准的语义 token 类型与修饰符、主题 `semanticTokenColors` 优先、主题没定义时按**移植自 `tokenClassificationRegistry` 的回退表**映射到 TextMate scope 再着色、遵循 `editor.semanticHighlighting.enabled` 与主题的 `semanticHighlighting` 字段。

数据路径已经就绪：[[router]] 在 [[contracts]] 的「C# 导航查询」接口背后提供带精确度的符号信息，[[nav-ui]] 是它的第一个消费方。[[theme]] 已经解析了 `semanticTokenColors` 与 `semanticHighlighting`，并明确把回退表留给本 issue。

必读：`docs/compat/SYNTAX_AND_LANGUAGES.md` §4（四条全部），`FEATURES.md` §3、§5，`UX_SPEC.md` §2.2（近似结果原则）、§3.1，`LANGUAGE_INTELLIGENCE.md` §3.1、§3.2，`PROJECT_CHARTER.md` §6，`docs/compat/SETTINGS.md`（`editor.semanticHighlighting.enabled`、`editor.semanticTokenColorCustomizations`、`editor.maxTokenizationLineLength`），`THEME_COLORS.md`（本 issue 不新增颜色键，着色走 `semanticTokenColors` 与 `tokenColors`）。

## 目标

### 1. 语义 token 的产出

- 按文件取语义 token：`(行, 起始 UTF-8 列, 长度, token 类型, 修饰符位集)`，一律经 [[contracts]] 的 C# 导航查询接口（已包含“按文件取语义 token”）取得，由 [[router]] 实现。
- token 类型用 VS Code 的标准集合（`namespace`、`class`、`struct`、`interface`、`enum`、`enumMember`、`typeParameter`、`type`、`parameter`、`variable`、`property`、`field`、`event`、`method`、`function`、`macro`、`keyword`、`comment`、`string`、`number`、`operator`、`decorator` 中 C# 用得到的部分），修饰符用标准集合（`declaration`、`definition`、`readonly`、`static`、`abstract`、`async`、`deprecated`、`modification`、`documentation`、`defaultLibrary`）。**不发明自己的类型名**，否则用户主题里的 `semanticTokenColors` 规则对不上。
- C# 符号种类 → 标准 token 类型 + 修饰符的映射表写成一张数据表并逐条测试（例如 `const` 字段 → `property`/`variable` 的选择、`readonly` 字段、`static` 方法、`partial` 类、局部函数、Unity 的 `[SerializeField]` 字段）。
- 只对 C# 生效（D2：其他语言只有高亮和折叠）。

### 2. 着色与回退表

1. 主题的 `semanticTokenColors` 中有匹配规则 → 用它（选择器语法、修饰符匹配、优先级照搬 VS Code）。
2. 没有匹配 → 按**语义 token → TextMate scope 的回退表**（移植自 VS Code 源码 `tokenClassificationRegistry`，例如 `class` → `entity.name.type.class`）映射到 scope，再走 [[theme]] 的 `tokenColors` 匹配。
3. 用户的 `editor.semanticTokenColorCustomizations` 按 VS Code 的语义合并（含按主题的形式），优先级高于主题。

回退表是数据资产，放在 `rove-lang-cs` 之外的可共享位置（与 [[theme]] 协商具体 crate，但**不改 [[theme]] 的匹配算法语义**）。

### 3. 与语法高亮的叠加

- 语义样式以**行内样式区间**的形式提交给 [[codeview]]，优先级高于 [[codeview-hl]] 的 token 样式；没有语义 token 覆盖的位置保持语法着色。
- 叠加只改前景色与 `fontStyle`，不改整形结果——但 `fontStyle` 中的粗体 / 斜体会改变字形宽度，必须与 [[codeview-hl]] 一样进入整形缓存键。
- 语义结果到达时只让**被覆盖的行**失效，不整文件重画；结果过期时沿用旧样式，**不闪烁**（与 [[codeview-hl]] 同一原则）。

### 4. 开关

- `editor.semanticHighlighting.enabled` 三个取值照搬：`true` / `false` / `configuredByTheme`（默认），后者取主题 JSON 中的 `semanticHighlighting` 字段（由 [[theme]] 解析）。
- 支持语言级覆盖（`"[csharp]": { "editor.semanticHighlighting.enabled": false }`），经 [[settings]] 读取。
- 关闭时**完全不发起查询**，不能只是「查了但不画」。

### 5. 未保存修改与索引未就绪时的降级

- `SYNTAX_AND_LANGUAGES.md` §4 最后一条：**文件已修改但未保存时，修改过的区域退回到纯语法高亮**，直到索引更新完成。区域粒度沿用 [[router]] 的「该区域是否被修改」判定，不要另定一套。
- [[router]] 返回近似（L1）或语法（L0）精确度时，本 issue **不产出语义 token**，也**不加任何提示**（`UX_SPEC.md` §2.2 最后一行的原则：同名高亮近似时退化且无提示，语义着色同理）。
- 精确度变化时（索引构建完成、保存后重建完成）静默刷新装饰：订阅 [[router]] 的精确度变更事件，不重弹提示、不闪烁。

### 5.1 不生效的预处理代码置灰（D9）

与 VS Code 的 C# 扩展一致：`#if UNITY_EDITOR` 等条件在当前项目宏下不成立时，对应的代码以降低不透明度的方式绘制（数值取 VS Code `editorUnnecessaryCode.opacity` 的默认值，主题可覆盖）。区间数据来自索引（[[idx-format]] 的不生效预处理区域），经 [[contracts]] 的 C# 导航查询接口取得。文件已修改但未保存时，修改过的区域按语法退回到不置灰。大文件级别以上关闭（UX_SPEC §3.1）。

### 6. 按级别关闭与性能预算

- `UX_SPEC.md` §3.1：**大文件**（≥ 20 MB 或 ≥ 30 万行）与**超大文件**都关闭语义高亮。关闭原因登记到 [[codeview-hl]] 的「当前关闭了哪些功能」接口。
- 遵循 `editor.maxTokenizationLineLength`（`[csharp]` 覆盖为 2500）：超长行不产出语义 token。
- 预算：打开 5000 行 C# 文件后，**语义着色在首屏之后 < 100 ms 内补齐**（索引就绪时；查询本身是微秒级的，成本在装饰构建）；连续滚动时语义装饰带来的每帧额外 CPU **< 100 µs**。

### 7. 登记与替换

新增设置项与界面字符串通过 [[contracts]] 的登记机制加入，不得写死。装饰一律经 [[codeview]] 的接口，查询一律经 [[contracts]] 的 C# 导航查询接口。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 索引中符号种类与修饰符的**生成**（[[idx-format]]、[[idx-perf]]）、查询路由与精确度判定（[[router]]）。
- 主题 JSON 的解析、`semanticTokenColors` 字段与 `semanticHighlighting` 字段的读取、scope 匹配算法（[[theme]]）。
- 语法高亮本体（[[syntax]]、[[codeview-hl]]）、着色黄金测试框架（[[golden]]）。
- 语义级**同名高亮**（只高亮同一个符号）与悬停（[[nav-ui]]，M2 已完成）。
- C# 以外语言的语义高亮（D2）。

## 验收标准

- [ ] 不生效的 `#if` 区域置灰：在两个 Unity 项目中各抽查 5 处（含嵌套 `#if` 与 `#elif`），结果与 Unity 当前编辑器宏一致；切换主题后不透明度正确
- [ ] C# 符号种类 → 标准 token 类型 + 修饰符的映射表逐条有测试，至少覆盖：类 / 结构 / 接口 / 枚举 / 委托、字段 / `readonly` 字段 / `const`、属性、局部变量、参数、方法 / `static` 方法 / `async` 方法 / 抽象方法、事件、命名空间、类型参数、局部函数
- [ ] 主题定义了 `semanticTokenColors` 时按主题着色；未定义时走回退表 → `tokenColors`，两条路径各有测试
- [ ] 回退表与 VS Code 的 `tokenClassificationRegistry` 逐条对照（PR 中附对照表）
- [ ] `editor.semanticTokenColorCustomizations` 的合并语义有测试，含按主题的形式
- [ ] `editor.semanticHighlighting.enabled` 三个取值 + `[csharp]` 语言级覆盖都生效；为 false 时不发起任何查询（断言查询次数为 0）
- [ ] 在中型 Unity 项目（`ROVE_TEST_UNITY_MEDIUM`）的真实脚本上与 VS Code（启用 C# Dev Kit）的截图逐处对照，**至少 10 个文件**，差异逐条记录并说明原因
- [ ] 未保存修改：修改一个方法体后，该区域退回纯语法高亮、其余区域保持语义着色；保存并增量重建后自动恢复，全程无闪烁、无提示
- [ ] 索引未就绪 / 近似结果时不产出语义 token 且无任何提示；精确度变更后静默刷新
- [ ] 语义结果到达时只让被覆盖的行失效（断言失效行集合）；主题切换后语义颜色立即更新且缓存不泄漏
- [ ] 5000 行 C# 文件首屏后 < 100 ms 补齐语义着色；连续滚动每帧额外 CPU < 100 µs（附测量数据，基准接入 [[perf]]）
- [ ] 20 MB 文件与超大文件打开后语义高亮已关闭，关闭原因可被查询到；超长行（> 2500 字符）不产出语义 token
- [ ] 新增的设置项与界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（打开 `.cs` → 断言某个字段的 token 类型、修饰符与最终颜色）

## 风险与提示

- 回退表是本 issue 最容易被低估的部分。它决定了「用户装的那个主题没写 `semanticTokenColors` 时，语义高亮还能不能看」——而大多数第三方主题都没写。照抄 `tokenClassificationRegistry`，不要凭直觉编映射。
- 「先显示已有内容、逐步补全」在这里的具体含义是：**首屏必须是语法高亮**，语义着色随后覆盖上去。千万不要为了等语义结果而延后首屏，那会直接违反 `UX_SPEC.md` §6.2。
- 语义与语法在用户眼皮底下来回切换（保存、索引重建、改一行又撤销）。每一次切换都不要闪烁，也不要弹任何提示——这和 [[nav-ui]] 处理近似结果的要求是同一条原则。
- `fontStyle` 进整形缓存键这件事 [[codeview-hl]] 已经踩过一次（「颜色对了但列对不上」）。语义着色会在更多位置改 `fontStyle`，接入前先确认缓存键里已经包含它。
