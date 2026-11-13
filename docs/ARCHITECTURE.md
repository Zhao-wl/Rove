# Rove 架构与共享接口契约

> #1 / A1，2026-11-09。合并后本文件及下列契约进入冻结状态。
> 本阶段提供可编译的跨 issue 类型、登记机制及最简替身，不提供正式功能或界面绘制。

## 1. 依据与范围

依据 [PROJECT_CHARTER](PROJECT_CHARTER.md)、[FEATURES](FEATURES.md)（D1–D10、§13）、[UX_SPEC](UX_SPEC.md)、[拆分原则](issues/README.md#0-拆分原则)、[语言智能](LANGUAGE_INTELLIGENCE.md)、[M0 UI 报告](M0_UI_SPIKE_REPORT.md#7-建议)、[M0 语言报告](M0_LANG_SPIKE_REPORT.md) 与 [compat](compat/)。

本次采用 #1 的十二个 crate 划分，不调整业务范围。新增 rove-base、rove-theme、rove-config、rove-commands、rove-i18n。根 workspace 继续使用 `crates/*`，只追加 workspace.dependencies。核心 crate 均无 GPUI 依赖；UI/app 在 #2 开始接入 GPUI 0.2.2，vendor/fork 由 #5 维护。本 PR 不改 vendor、spikes、索引存储格式或 CI。

## 2. 依赖方向

箭头表示“依赖”，不可反向或成环；下面列出直接依赖（标准库与现有 mmap 库省略）。

```mermaid
flowchart TD
 app[rove-app] --> ui[rove-ui]
 ui --> base[rove-base]
 ui --> buffer[rove-buffer]
 ui --> syntax[rove-syntax]
 ui --> theme[rove-theme]
 ui --> config[rove-config]
 ui --> commands[rove-commands]
 ui --> i18n[rove-i18n]
 ui --> workspace[rove-workspace]
 ui --> cs[rove-lang-cs]
 ui --> docview[rove-docview]
 ui --> fuzzy[rove-fuzzy]
 buffer --> base
 syntax --> buffer
 syntax --> base
 theme --> base
 theme --> config
 config --> base
 commands --> base
 i18n --> base
 workspace --> buffer
 workspace --> base
 cs --> syntax
 cs --> base
 docview --> syntax
 docview --> base
 cs --> config
 cs --> buffer
 ui --> search[rove-search]
 search --> workspace
 search --> buffer
 search --> config
 search --> base
```

| crate | 职责 | GPUI |
|---|---|---|
| rove-base | 坐标、资源身份、版本、编辑、取消、订阅、错误、通用登记容器 | 禁止 |
| rove-buffer | 不可变文本快照、文档元信息、后台打开服务；编辑核心（事务、撤销重做、选区、单词边界，#46，见 §24） | 禁止 |
| rove-syntax | token/scope、折叠、符号树、语言配置数据 | 禁止 |
| rove-theme | 颜色登记与主题读取、token 样式 | 禁止 |
| rove-config | 设置登记、强类型读取、显式写入、变更 | 禁止 |
| rove-commands | 命令登记、参数、上下文键、when/键位接口 | 禁止 |
| rove-i18n | 中文字符串表、Message、tr! | 禁止 |
| rove-workspace | 文件列表/树、规则、文件事件 | 禁止 |
| rove-lang-cs | 导航查询契约；既有 index.rs 保持独立 | 禁止 |
| rove-docview | 结构树节点，复用语法符号模型 | 禁止 |
| rove-fuzzy | VS Code `fuzzyScorer` 的逐条移植（快速打开排序）与命令面板的单词过滤 | 禁止 |
| rove-search | 全局搜索引擎（#25，见 §32）：`search.*` 语义、并行流式结果、取消、编码识别 | 禁止 |
| rove-ui | 打开/定位/关闭、反馈、列表/树、快速输入、贡献点 | 允许 |
| rove-app | 入口、服务装配与窗口生命周期 | 允许 |

核心类型不引用 GPUI Color、Entity、Context 或窗口；UI 在边界转换。UI 服务不要求 Send/Sync，必须只由所属窗口线程调用。后台核心服务要求 Send + Sync，使用 Arc 不可变快照。

## 3. 线程、生命周期和过期结果

1. UI 事件处理只读取缓存、登记请求和更新轻量状态；布局与绘制不执行文件 I/O、全文遍历、解析、正则编译、搜索、mmap 冷页读取或同步整形。
2. `Task<T>` 是 executor-neutral 的 Send Future。app 在后台执行器运行；C# 查询虽为同步 API，也必须放在后台线程，不能把“热查询很快”解释为 UI 安全。
3. UI 发请求时保存资源身份、DocumentId、Version、请求 ID/代次及 Cancellation。后台持有不可变输入，定期检查取消，输出结果而非 UI 对象。
4. 结果通过应用的 UI 调度桥回到所属窗口；窗口销毁、请求被替代或版本不再匹配时丢弃。异步完成不得抢回已经转移的焦点。#2 提供调度桥的最简装配。
5. Events 回调在生产线程、释放服务锁以后执行；回调只能入队并迅速返回，不能直接修改 UI。UI 队列必须按资源/版本合并失效通知，不能每个文件事件触发一次绘制。
6. Subscription 拥有注销生命周期；drop 非阻塞，已开始的回调可能完成，故消费方仍需弱引用/代次检查。先订阅再发起请求。NoEvents 不保留回调。
7. 同一个文本快照的内容与 Version 不可变。语法允许绘制旧结果，但必须知道其版本；编辑/跳转不能把旧坐标当作当前坐标。SyntaxUpdate 使用新版本的受影响行范围，缺少基线时全量失效。
8. 取消是协作式的。返回 Cancelled 不等于撤销已经生效的写操作；写入成功的线性化点之后必须报告成功。空提供器绝不假装写入、执行命令或接受模态选择成功。

任务可从调用方令牌派生取消作用域。取消子作用域不会影响父级或兄弟，取消父级会被所有后代观察到。检查复杂度与深度成正比，独占的深链迭代释放。用 `cargo xtask bench --filter cancellation/` 测量公开接口的检查开销。

## 4. 统一数据约定

- `ByteOffset`：解码后的 UTF-8 文本字节偏移；不是磁盘字节偏移。
- `Utf8Column`：从行首算的 UTF-8 字节列，必须位于 Unicode scalar 边界。
- `Utf16Column`：UTF-16 code unit 列。与前两者是不同 Rust 类型，无隐式转换。
- `Line` 从 0 开始；界面显示加 1。区间统一半开 `[start,end)`；Span 构造拒绝倒序，空区间允许。
- TextSource 承担坐标转换，拒绝 UTF-8 字符中部、UTF-16 surrogate 中部或越界；未知行索引返回 NotReady。空文本有一条空行。`line_range` 不含行终止符；`visit_chunks` 按顺序借用完整 UTF-8 片段，拼接后恰好为请求范围。不得为返回片段复制全文。
- WorkspacePath 是使用 `/` 的合法相对路径，禁止空段、`.`、`..`、盘符和反斜杠；不是文件系统安全沙箱。#15 负责工作区身份、大小写/路径别名及外部文件的规范化。Resource 同时支持工作区文件、外部文件和元数据等虚拟资源。
- DocumentId 只在打开模型生命周期内稳定；Version 只在同一文档中比较。打开别名不能制造重复模型；不同组可显示同一文档。
- 文档元信息显式保存编码、BOM、语言、混合换行及新输入的首选换行、多个只读原因和大小级别。不在契约替身里做编码检测或自动大小分级。
- `Error` 为程序可判断的类别，不是直接展示的中文文案。UI 用已登记的 Message key 呈现；诊断中的路径/系统错误属于数据。未就绪、无结果、取消和失败不得混为一谈。

I/O 错误保留 ErrorKind 和诊断文字，支持从 std::io::Error 转换。诊断文字可能采用系统语言，仅用于日志；UI 根据类别选择字符串 key。ErrorKind 为 non_exhaustive，下游匹配须包含兜底分支；当前可克隆错误摘要不保留原始错误链，source() 返回 None。Value::Object 与字符串/数组一样使用 Arc，共享不可变内容，克隆不复制整张表。

## 5. 接口清单与实现归属

下表路径均相对 `crates/`；表中公共类型及 trait 签名属于冻结契约，空实现可按负责 issue 替换。

| 接口 | 契约文件/主要类型 | 正式实现方 | 消费方与空实现 |
|---|---|---|---|
| 基础类型、事件、登记 | rove-base/src/lib.rs | #1；后续修改单独 PR | 全链；NoEvents/Registry |
| 文本源 | rove-buffer/src/contracts.rs / TextSource | #7、#8 | #19、#9；EmptyText |
| 文档元信息/后台打开 | 同上 / Document、Documents | #7、#8、#17 | #2、#17；EmptyDocuments |
| 界面打开/定位/关闭 | rove-ui/src/documents.rs / OpenDocuments | #17 | #23–#26、#40；EmptyOpenDocuments |
| 语法高亮/增量结果 | rove-syntax/src/contracts.rs / SyntaxSnapshot、SyntaxService | #9 | #19、#20；EmptySyntax/EmptySyntaxService |
| 折叠/文档符号 | 同上 / Fold、Symbol | #9、#20、#26 | 大纲、面包屑、粘性滚动、篮子 |
| 语言配置 | 同上 / LanguageConfiguration、LanguageConfigurations | #12 | #20、#26、#45、#52；EmptyLanguages |
| 主题/颜色键 | rove-theme/src/lib.rs / Theme、ColorRegistry | #10 | 全 UI；DefaultTheme |
| 设置 | rove-config/src/lib.rs / Settings、Key<T>、SettingRegistry | #13 | 全链；DefaultSettings |
| 命令/上下文/when/键位 | rove-commands/src/lib.rs | #14 | #24 等；EmptyCommands/Context/When/Keybindings |
| 界面字符串 | rove-i18n/src/lib.rs / StringTable、Message、tr! | 各功能登记；#67 英文 | 所有 UI；chinese() 最小中文表 |
| 工作区 | rove-workspace/src/contracts.rs / Workspace | #15 | 文件树、快速打开；EmptyWorkspace |
| 五级反馈 | rove-ui/src/feedback.rs / FeedbackService | #16 | 所有功能；EmptyFeedback |
| 虚拟列表/树 | rove-ui/src/lists.rs / ListModel、ListController | #18 | #23、#25、#26；EmptyList |
| 快速输入 | 同上 / QuickInput | #18 | #24；EmptyQuickInput |
| 视图/状态栏贡献 | rove-ui/src/contributions.rs / ContributionRegistry | #16（#2 装配） | 各视图负责方；空 Registry |
| C# 导航 | rove-lang-cs/src/query.rs / CsQueries | #38；#32–#37 提供数据 | #40、#41、#58、#59；EmptyCsQueries |
| 文档结构树 | rove-docview/src/contracts.rs / Structure | #60 | 大纲、面包屑；EmptyStructure |

每个接口的语义、线程、复杂度/预算和错误说明见对应 trait 的 rustdoc；就近消费示例测试展示注册、读取、失效、取消或不可用分支。替身的“空/Unsupported/NotReady”不表示正式功能已实现，#2 必须替换主链所需的提供器。正式文件扫描、JSONC 解析、主题算法、命令解析和界面绘制均留给表中负责 issue。

### C# 与 G 链的接缝

`query.rs` 不引用 `index::Location`；v1 存储使用 UTF-16，v2 使用 UTF-8，不得把旧整数直接塞进 Utf8Column。#38 适配器按索引版本转换。导航包括定义、类型定义、引用、实现、派生/基类/重写，另有悬停、文件符号、工作区符号、语义 token 与非活动区域。

语义 token 的列和长度是 UTF-8 字节单位，单个 token 不跨行；type/modifiers 使用 VS Code 标准名称。每个结果带精确度、来源版本、索引代次及近似原因。混合来源使用最弱精确度，不能将近似候选标成 Exact。元数据目标通过 Resource::Virtual 表达。索引替换后已返回结果仍须持有需要的底层数据生命周期。

### 结构树懒展开

StructureNodeId 在同一 Structure 提供器和 Version 内唯一，不能用可重复的显示路径作为身份。roots() 只读取缓存根节点；children 为 None 时，消费方调用 `Structure::children(node_id, expected_version, cancel)` 并在后台执行返回的 Task，仅构造该节点的直接子节点，孙节点仍可未加载。symbol.children 必须为空，避免通过 Symbol 隐藏地构建整棵树。

未知节点返回 NotFound，已知叶子返回空切片，版本不匹配返回 Stale，取消返回 Cancelled，前置解析未就绪返回 NotReady；这些状态不得互相代替。完成结果属于请求版本，消费方回到 UI 后再次检查提供器身份和版本，再更新列表模型。返回的节点不可变，消费方可缓存子节点结果，无需等待原来的 roots 切片被修改；折叠节点可取消进行中的加载。空实现不虚构节点。#60 负责真正的解析、缓存和大文件预算，本阶段的测试提供器只演示通过 trait 完成按需展开。

### 打开、列表与反馈的接缝

打开明确位置时必须显示源码（包括 Markdown），普通打开遵循编辑器关联。OpenRequestId 对应异步完成/失败事件；关闭脏文档返回 NeedsUserDecision，不丢弃内容。磁盘打开失败可保留错误占位页，不能出现空白成功页。

ListModel 是展平的可见树快照；控件按可视范围取行，以稳定 ItemId 保留选择。过滤和树展开不在绘制时扫描全量数据。QuickInput 不实现模糊评分和前缀模式（#24 负责），接受与取消有不同事件。

反馈采用状态栏临时消息、光标提示、编辑器提示条、通知、模态五级。通知 key 去重；勿扰仅保留错误；正常后台工作只显示状态。模态只用于数据丢失风险和与 VS Code 一致的安全确认，不能同步阻塞 UI 线程。

打开请求由安装好的服务调度后台工作并在所属窗口应用结果。每个已接受请求发出一次终态事件，带标签当时所在组。按标签 ID 可关闭加载中或失败的标签；脏标签返回 NeedsUserDecision。关闭或替换未完成请求时报告 Cancelled。调用方取消会移除尚未就绪的标签，保留已有的就绪视图。服务不取消调用方令牌。

按组枚举保留标签顺序、预览、固定、活动和脏状态，也包括加载及失败项。Ready 快照表示已有视图，即使后续请求仍在运行；Ready 事件则确认该请求已经应用。定位与选区并存时优先显示选区起点。

## 6. 五类登记机制

全部使用 `Registry<T>`：显式 register，空 ID 拒绝，重复 ID 返回 Duplicate 且不覆盖；查询 O(log n)，遍历按 ID 稳定排序。应用启动装配后共享只读表，不在绘制中反复登记。这里是产品内贡献点，不是插件系统。

| 类别 | 登记内容 | 消费示例测试 |
|---|---|---|
| 命令 | VS Code ID、标题字符串 key、分类 key；Invocation 带 Value 参数 | palette_registers_vscode_command |
| 设置 | key、VS Code 默认值、Rove 缺省差异、语言覆盖能力；Key<T> 强类型读取 | encoding_consumer_reads_approved_default |
| 颜色 | key 与深色/浅色/深浅高对比默认值；None 表示明确未设色 | view_registers_and_resolves_color |
| 字符串 | 稳定 key、中文模板，Message 带命名参数，tr! 统一读取 | component_uses_chinese_key |
| 视图/状态栏 | ID、标题、位置、排序、图标、命令、when、惰性工厂 key | workbench_registers_view_and_status |

命令执行与登记元数据分开：#14 的执行桥将 ID 交给窗口拥有的 handler，耗时任务返回 Task。贡献工厂由 UI/app 装配，禁止核心层引用 GPUI Entity；#2/#16 负责验证 factory_key、命令与字符串引用存在。未实现功能不得注册成可执行菜单项。

设置只在用户显式修改时写回：保留 JSONC 注释、顺序、格式，先备份再修改目标键。优先级为语言用户覆盖 > 通用用户值 > Rove 未设置时的默认值 > VS Code 默认值；不支持的外部设置静默忽略，错误重载保留旧有效快照。

**2026-11-09 用户确认的规格澄清**：compat/SETTINGS.md §3 四项是 FEATURES §13.5 的明确例外：files.autoGuessEncoding、files.candidateGuessEncodings、workbench.editorAssociations、markdown.preview.doubleClickToSwitchToEditor。只有用户未设置时才使用 Rove 默认值，绝不自动写回。其余差异继续通过 rove.* 附加规则表达。此处记录确认，不直接修改自动生成的兼容清单。

字符串模板支持 `{name}` 与字面量括号 `{{` / `}}`；占位符后的第一个右括号结束占位符，因此 `{name}}}` 表示值后跟字面量 `}`，`{name}}` 则无效。不匹配括号、空/嵌套占位符、缺少参数返回 InvalidInput，参数值不递归解析。默认值例外同时记录于 FEATURES §13.5。

## 7. 性能约束与证据边界

以最新 PROJECT_CHARTER 的 <400ms 可交互启动预算为准；M0 报告 §7 中旧的 150/250ms 探索建议不是新验收门槛。空窗口 <60MB；1GB 首屏 <1s；120fps；缓存命中每帧 CPU <1ms；按键到上屏 <8ms；已打开目标的导航端到端 <50ms。索引查询预算与 UI 端到端分别记录。

代码视图自写 Element，使用 f64 行偏移；不能用 f32 巨大像素偏移，也不能直接用通用列表代替。整形按内容缓存并沿滚动方向预取，绘制关键路径禁止同步整形。通用列表只负责树/快速输入等 UI 控件。WebView2 按需创建，不影响冷启动；#6 验证合并后才实现相关容器。

M0 的热缓存查询结果不能证明冷 mmap 安全：冷 HDD 查询曾达 2.8s，故 C# 同步接口一律后台执行；索引预读和缓存目录由 #36 实现。取消、分页、范围访问、Arc 快照减少长工作和复制。这里定义的是接口约束与目标，本 PR 的空实现测试不能充当性能或真实项目验收；#4/#31/#44 负责对应测量。

## 8. 契约变更与并行协作

#1 合并后，后续 issue 不得在功能 PR 中顺手修改上述公共契约。需要调整时单独提出小 PR，写明具体原因、受影响实现/消费 issue、迁移步骤和验证结果；合并后其他链 rebase，再使用新契约。内部实现可以更换，不能改变承诺的错误、坐标、线程和生命周期语义。

待 #3 负责人写入 AGENTS.md 的规则（本 PR 不修改该文件）：

- 命令、设置、颜色、字符串、视图必须通过登记机制加入。
- 架构和共享契约以本文件为准；契约修改走独立小 PR。
- 各功能删除自己负责的 SKELETON 标记，保持 #2 端到端测试并为入口补集成测试。
- 核心层任何传递依赖都不得包含 gpui；只允许 rove-ui/rove-app 引入。

文件归属保持用户约束：index.rs/tools/rove-indexer-cs 属 G 链，vendor/spikes 属 C 链，.github/AGENTS.md 属 F 链。根 Cargo.toml 仅追加 workspace.dependencies。Cargo.lock 冲突取 main 后 cargo build 重新生成，不手改。

## 9. 本地验证

提交 PR 前 fetch/rebase origin/main，然后执行：

```text
cargo fmt --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

另检查各核心 crate 的依赖闭包没有 gpui：存在 gpui 时可用 `cargo tree -p <crate> -i gpui`；本阶段整个 workspace 尚无 gpui，Cargo 对该反向查询报告 package ID 不匹配，故以 cargo metadata 的依赖闭包为确切证据，不将命令失败本身当作通过。最终运行结果见 PR，任何既有基线阻断必须如实记录，不能用 lint allow 或删掉测试规避。

当前核心层无 GPUI 是已有依赖图的静态结果；真实 GPUI 接入后必须重新检查依赖隔离，不能以本次结果代替。


## 10. 行走骨架与替换责任（#2）

骨架的临时实现集中在 `crates/rove-ui/src/skeleton/`；核心 crate 继续只提供冻结的接口。
`rove-app` 负责参数与 GPUI 启动。后续负责人将实现迁入所属 crate 后，在装配处替换服务，
删除自己的标记并更新本表，不要删除其他 issue 的并列标记。

| 标记 | 最简实现与位置（相对 skeleton/） | 替换 issue |
|---|---|---|
| `buffer` | 已由 `rove-buffer` 实现：编码识别（BOM/自动/候选）、换行符、rope 快照与坐标转换；`text.rs` 只保留文档缓存与打开策略 | #7（已完成） |
| `syntax` | 已迁入 `rove-syntax::highlight`：五种语言、增量解析和逐行 scope | #9 |
| `theme` | 已由 `rove-theme::ThemeService` 实现：VS Code 主题 JSON（include 链）、vscode-textmate scope 匹配、颜色默认值表与用户覆盖；`workbench.rs` 直接装配该服务 | #10（已完成） |
| `commands` | 已由 `rove-commands` 实现：命令登记与分发、keybindings.json、键表达式、when、和弦；`commands.rs` 只保留窗口的命令装配，`workbench.rs` 的 `on_key_down` 接 `rove_ui::keys` | #14（已完成） |
| `workspace` | 已由 #15 替换为 `rove-workspace`：流式并行扫描（不读 `.gitignore`）、`files.exclude`/`rove.files.extraExclude` 合并、只读规则真值表、文件监听（溢出重扫、重命名还原） | #15 |
| `workbench` | 已由 `rove-ui::shell` 实现（见 §14）：标题栏与命令中心、菜单、活动栏、侧边栏、面板、状态栏容器、布局持久化、五级反馈；骨架只保留装配（`workbench.rs` 实现 `ShellHost`） | #16（已完成） |
| `editors` | 已由 `rove-ui::editors` 实现（见 §17）：编辑器组网格、标签（预览/固定/拖动）、打开文档服务、导航历史、编辑器类型解析；`workbench/editor_panes.rs` 绘制组与标签栏，每个组挂一个 `CodeView`；`text.rs` 只保留文档弱引用缓存 | #17（已完成） |
| `lists` | 已由 `rove-ui::list_widget` 实现：虚拟化列表/树/快速输入与全部 `list.*`、`quickInput.*` 命令（见下文“列表、树与快速输入组件”）；骨架文件树的数据与控制已改用 `ListWidget` + `TreeModel` | #18（已完成） |
| `code-view` | 已由 `rove-ui::code_view` 实现：自定义 Element、f64 滚动、整形缓存与预取、行号、只读选区、命中测试、滚动条；`workbench.rs` 只挂载 `CodeView` | #19（已完成） |
| `code-features` | 已由 `rove-ui::code_view` 实现（见 §16“高亮、折叠与只读”）：语法着色层、括号匹配、同名高亮、折叠、缩进参考线、空白字符、只读表现与按级别关闭功能；`workbench.rs` 只把快照、级别与只读原因交给 `CodeView` | #20（已完成） |
| `find` | 无骨架标记（#2 没有对应的临时实现）：已由 `rove-ui::find` 实现（见 §27）：当前文件查找条、查找引擎与模型、`editor.find.*`、匹配装饰、历史；`workbench.rs` 只给每个代码视图包一层 `FindHost`，替换由 #48 接入 | #21（已完成） |
| `nav` | 无骨架标记（#2 没有对应的临时实现）：已由 `rove-ui::nav` 与 `skeleton/workbench/nav_host.rs` 实现（见 §31）：转到定义 / 声明 / 实现、`Ctrl+悬停` 与 `Ctrl+点击`、悬停卡片、符号导航模式、语义同名高亮、近似结果提示，以及 #38 `CsRouter` 的装配（语义来源、L1、`BufferStates` 的打开 / 重新载入 / 关闭） | #40（已完成） |
| `quickopen` | 已由 `rove-ui::quick_access` 与 `rove-fuzzy` 实现（见 §28）：`Ctrl+P` / `Ctrl+Shift+P` / `F1` / `Ctrl+G`、`:` `@` `#` `>` 四个前缀、命令中心入口；`keys.rs` 不再预登记两个“未实现”命令，`workbench/quick_access_host.rs` 只做窗口装配 | #24（已完成） |
| `save` | 无骨架标记（骨架只读，#2 没有保存的临时实现）：已由 `rove-buffer::save` 与 `rove-ui::save` 实现（见 §30）：原样写回（编码 / BOM / 混合换行）、原子写入、占用重试、只读属性、保存冲突、自写去重、外部修改与自动重载、关闭确认；窗口装配（`ctrl+s` / 另存为 / 全部保存 → `SaveController`、`ctrl+n`、重载后刷新视图、脏点与“已删除”、窗口关闭确认）由 #47 在 `workbench/edit_host.rs` 完成（见 §37） | #49（已完成） |
| `search` | 无骨架标记（#2 没有对应的临时实现）：已由 `rove-search`（引擎）与 `rove-ui::search`（视图）实现（见 §32）；`workbench/search_host.rs` 只做窗口装配，工作台原先“`findInFiles` 只打开视图容器”的行为保留，焦点随后交给查询框 | #25（已完成） |
| `peek` | 无骨架标记（#2 没有速览的临时实现）：已由 `rove-ui::peek` 实现（见 §33）：速览定义 / 引用 / 实现窗口挂在代码视图的嵌入区域里，结果树复用 #18、预览复用 #19；`workbench/peek_host.rs` 只做窗口装配（命令、上下文键、打开结果、光标处提示；查询用 #40 的 `CsRouter`） | #41（已完成） |
| `explorer` | 已由 `rove-ui::explorer` 实现（见 §36）：`SKELETON(explorer)` 标记与骨架的 `WorkspaceTree` 适配器、固定绘制已删除；“文件夹”小节是 #18 `ListView` 的包装，数据来自流式扫描的 #15 工作区（根目录立即显示）；`skeleton/workbench/explorer_host.rs` 只做窗口装配 | #23（已完成） |
| `status` | 容器已完成，等待条目：`appearance.rs` 的 `rove.status.file` 与 `workbench.rs` 的文件名/行数/编码内容 | #27 |
| `edit` | 无骨架标记（骨架只读，#2 没有编辑的临时实现）：已由 `rove-ui::code_view::edit` 实现（见 §37）：可编辑代码视图、`EntityInputHandler` 与内嵌组合串、编辑基础命令、剪切粘贴、改写模式、只读拒绝；`workbench/edit_host.rs` 只做窗口装配 | #47（已完成） |
| `markdown` | 无骨架标记（#2 只把 `*.md` 解析成占位编辑器 `vscode.markdown.preview.editor`，显示“尚未实现”）：已由 `rove-ui::markdown` 与 `skeleton/workbench/markdown_host.rs` 实现（见 §38）：WebView2 预览（按需创建、一个窗口一个实例、空闲回收）、默认以预览打开、切换与并排、滚动同步、远程资源阻止 | #29（已完成） |

数据链路：后台流式扫描返回 `Arc<dyn Workspace>`（根目录已列出），资源管理器经 #18 的树组件激活资源；
`OpenDocuments` 入队后台 `Documents::open` 与 `SyntaxService::analyze`，只接受最新请求，
应用位置后发出 Ready。代码视图只绘制缓存；帧间空闲切片读取 TextSource、调用装饰提供方（语法着色层
读 SyntaxSnapshot、Theme）并整形，paint/layout 不读取文件、不解析、不整形（见 §16）。
命令走 CommandExecutor，提示走 FeedbackService，所有固定文字走 Strings/Message；
视图和状态贡献登记时不做 IO，装配校验 factory_key、placement 与标题引用。

最小边界：只读、整个 UTF-8 文件、单编辑区、C# 少量 scope、纯文本回退、无监听。
不跟随符号链接或 Windows reparse point，避免递归环；这不实现正式的忽略规则。
不承诺大文件、长行优化、水平滚动或完整 TextMate 匹配。

保留入口测试 `walking_skeleton_opens_csharp_through_real_window`：GPUI TestAppContext
中建立窗口、点击文件，检查文档、已绘制首行、keyword scope 与 Dark 2026 颜色。
它是 `cargo test --workspace` 的普通测试；后续替换必须保留此链路。
实际窗口验收使用 `cargo run -p rove-app -- <ROVE_TEST_UNITY_MEDIUM>`，路径只来自本地环境。
日志 `MAIN_EPOCH_MS` 为 main 入口时间，`FIRSTFRAME_SINCE_PROCESS_MS` 为 Windows 进程创建至首次帧回调；
通过 GetProcessTimes 纳入加载时间。系统时钟读取失败时输出 PROCESS_CLOCK_UNAVAILABLE 并回退至 main。
帧回调是 GPUI 首次绘制完成的近似点，不代表显示器扫描输出；#4 可补充外部采样。

## 11. 语言配置服务（#12）

位置：`crates/rove-syntax/src/language_config/`，数据在 `crates/rove-syntax/data/vscode-extensions/`。
本模块只向 `lib.rs` 追加 `pub mod language_config;`，不修改 `contracts.rs` 的冻结契约，也不涉及 tree-sitter（#9）。

数据：VS Code 1.145.2（提交 `c4453f8ea430795f941c1b2ff6c9321135a74a02`，MIT）的五个
`language-configuration.json` 原样随二进制编译（`include_str!`），运行期不下载、不读取用户安装的扩展，
查询无 I/O。六个语言 ID（`csharp`、`json`、`jsonc`、`markdown`、`xml`、`yaml`）对应五份文件，
`jsonc` 与 `json` 共用一份，与 VS Code 相同；`hlsl`、`shaderlab`、`lua` 随其他语言高亮接入。

正则：配置里是 JavaScript 语法。`language_config::js_regex` 用 `fancy-regex` 编译（前瞻、后顾、
反向引用、命名组），并改写 JS 与 Rust 的语义差异：`\d`、`\w`、`\b` 保持 ASCII（带 `u` 标志也一样）、
`\s` 使用 ECMAScript 集合、`.` 排除全部行终止符、`\cX`/`\0` 等引擎不接受的转义，字符类按 ECMAScript
规则处理（`[]` 永不匹配、`[^]` 任意字符，`&`/`~`/`[` 转义以避开 Rust 的集合运算与嵌套类，
`-` 仅在两侧都是单字符原子时保留为区间），`\p{...}`/`\u{...}` 按 `u` 标志区分属性转义与字面量。
无法翻译的特性（例如字符类内的 `\D`、`v`/`y` 标志）返回 `LanguageConfigError`，消息包含配置路径、
语言 ID 与字段名（例如 `folding.markers.start`），不会静默退化成“永不匹配”。

服务与查询：

| 入口 | 用途 |
|---|---|
| `BuiltinLanguages::load()` / `load_partial()` | 启动时加载并编译全部内置配置；`load()` 全有或全无，`load_partial()` 保留成功语言并返回错误（按语言隔离，与 VS Code 一致） |
| `BuiltinLanguages::shared()` | 进程内单例，只解析并编译一次；返回 `(&Self, &[LanguageConfigError])`，成功加载的语言照常可用 |
| `LanguageConfigurations::get(id)` | 冻结契约：`Arc<LanguageConfiguration>` 无损数据，O(log n)、`Send + Sync`、UI 安全 |
| `BuiltinLanguages::compiled(id)` | 同一份数据 + 已编译正则（按字段缓存），供逐键/逐文档匹配 |
| `CompiledLanguageConfiguration::fold_ranges(text, tab_size)` | 由 `folding.markers` 与缩进产出 `Fold`（`Region`/`Indentation`），含 `folding.offSide` 的空行归属 |
| `CompiledLanguageConfiguration::word_at(text, byte_offset)` | 用 `wordPattern`（无则 VS Code 默认）取光标处单词的 UTF-8 字节区间 |
| `language_config::colorized_brackets(config)` | `colorizedBracketPairs` 缺省时回落到 `brackets` |
| `language_config::auto_close_before(_for)(config, quotes)` | 缺省时取 VS Code 的括号或引号默认值 |

下游用法：

- #20 代码视图：`compiled(id).fold_ranges(...)` 得到折叠区间（`FoldKind::Region` 为 `#region` 等标记，
  `FoldKind::Indentation` 为缩进折叠）；括号来自 `get(id).brackets`，括号对着色用 `colorized_brackets`。
- #26 大纲：折叠标记与语言数据都来自同一语言 ID，不需要自行解析配置。
- #45 编辑模型：`word_at` 提供双击选词、按单词移动与同名高亮的边界（字节区间）。
- #46：订阅本服务的语言数据即可，无需重复加载配置。
- #52 注释切换/自动缩进/自动闭合/回车规则：`comments`、`auto_closing`（含 `notIn`）、`surrounding`、
  `indentation`、`on_enter` 由 `get(id)` 提供，编译好的句柄由 `compiled(id)` 的 `on_enter()`/`indentation()` 提供。
- 语言级设置（`"[csharp]"` 覆盖）属于 #13：本服务不解析设置文件；消费方用
  `rove_config::Key<T>::read(&settings, &Scope { language: Some(id) })` 读取 `editor.maxTokenizationLineLength`
  等键，与语言配置数据分开。

验证：`cargo test -p rove-syntax` 遍历数据目录编译每份配置的全部正则；另有字段级断言、
Markdown/XML `wordPattern` 与 VS Code `getWordAtText` 的对照用例、C# `#region` 嵌套与未配对、
YAML/Markdown `offSide` 空行归属、`notIn` 解析，以及“不支持特性报出配置+字段”的用例。
`cargo tree -p rove-syntax -i gpui` 为空。

## 12. 设置层（#13）

位置：`crates/rove-config/src/`（`lib.rs` 的契约不变，只追加模块声明）、数据 `crates/rove-config/data/settings-catalog.json`。
核心层，不依赖 gpui；不读 `.gitignore`，不联网。

| 入口 | 用途 |
|---|---|
| `SettingsService::open(SettingsOptions, SettingRegistry)` | 实现 `Settings` 契约：读 `settings.json`、监听热加载；文件不存在时不创建，首次显式写入才创建 |
| `SettingsOptions::from_environment(arg)` / `paths::split_user_dir_arg` | 位置：`--vscode-user-dir` > `ROVE_VSCODE_USER_DIR` > `%APPDATA%\Code\User`；备份到 `%LOCALAPPDATA%\Rove\settings-backup`（保留 20 份） |
| `register_builtin(&mut registry)` / `builtin_registry()` | 登记 `compat/SETTINGS.md` 中全部“支持”的设置项（P0/P1/P2）、隐藏设置与 `rove.*`；已由消费方登记的键保持原样 |
| `catalog::catalog()` | 每项的优先级、来源（VS Code/隐藏/rove.*）与 VS Code 默认语言覆盖 |
| `Settings::read` / `Key<T>::read` | 内存快照，O(log n)，UI 线程安全；契约语义：类型不符返回 `InvalidInput`，不强制转换 |
| `SettingsService::read_typed(&Key<T>, &Scope)` | 逐层回落：类型不符的用户值被跳过并记录带行列的 `TypeMismatch` 警告，最终回落到默认值 |
| `SettingsService::is_user_set` / `layer` | 用户是否显式设置（#7、#25、#29 区分“用户值”与“Rove 默认值”）；生效值来自哪一层 |
| `Settings::write` / `SettingsService::remove` | 返回在后台执行的 `Task`：区间编辑目标键 → 备份（失败则不写）→ 文件未被他人改动（否则 `Stale`）→ 临时文件 + 重命名，占用时重试 |
| `problems()` / `problem_changes()` | 语法错误（带行列，沿用上一份有效设置）与类型警告，供 #16 的状态栏图标与问题面板 |
| `keybindings_changes()` | 同目录 `keybindings.json` 的内容变化，#14 自行解析 |
| `jsonc` / `edit` | JSONC 解析（注释、尾逗号、重复键后者胜出、BOM、CRLF）与保留格式的区间编辑 |

优先级：语言级用户覆盖 > VS Code 默认语言覆盖 > 全局用户值 > Rove 默认值（仅用户未设置时）> VS Code 默认值。
VS Code 的默认语言覆盖（如 `[markdown]` 的 `editor.wordWrap`）高于全局用户值，与 VS Code 的设置优先级一致。
Rove 默认值只有 `compat/SETTINGS.md` §3 的四项（数据测试保证没有第五项），永不写回共用文件。

热加载：后台线程每 250ms 比较两个文件的长度与修改时间，静止 100ms 后读内容并与最近一次已知内容
（包括自己写入的内容）逐字节比较，相同即不重载——这同时处理了“写临时文件再重命名”、去抖和自激。
只有已登记键的值发生变化时才发布 `SettingsChange`；不支持的键静默忽略，不产生问题或通知。

决定与边界：

- 工作区级设置（`.vscode/settings.json`）首版不做。
- issue 正文的第四条默认值差异写的是 `search.useIgnoreFiles`；以 `compat/SETTINGS.md` §3 与本文件 §6 的确认为准，
  第四条是 `markdown.preview.doubleClickToSwitchToEditor`。`search.useIgnoreFiles` 在清单中属于“不支持”，被静默忽略。
- 可否按语言覆盖：导出的默认设置不带 scope，`gen_settings_catalog.py` 把 `editor.*`、`diffEditor.*`、编码/换行相关的
  `files.*` 与默认语言覆盖中出现过的键视为可覆盖。
- 装配（#104 已完成）：`rove-app` 用 `split_user_dir_arg` 取出 `--vscode-user-dir`，`rove_ui::app_settings::open`
  先按模块登记各自的键、再 `register_builtin`，打开唯一的 `SettingsService`；见下文“设置层装配”。
  `files.associations`、C# 行长等语法设置接到文件打开路径属于 #94。

验证：`cargo test -p rove-config`。`tests/settings_md_coverage.rs` 对照 `SETTINGS.md` 检查登记与“不支持”清单；
`tests/settings_service.rs` 是入口集成测试（读 → 改一个键 → 写回 → 热加载，含字节级保留格式、备份上限、占用重试、
语法错误沿用旧设置、类型警告与 Rove 默认值四项）。

## 13. 主题引擎（#10）

位置：`crates/rove-theme/src/`，数据在 `crates/rove-theme/data/themes/`（VS Code 1.145.2 默认主题，原样复制）。
`lib.rs` 的冻结契约（`Theme`、`ColorRegistry`、`Color`、`TokenStyle`）未改动，只追加模块声明与再导出。
依赖：`rove-base`、`rove-config`（读取主题设置）、`jsonc-parser`/`serde_json`；不依赖 gpui。

| 模块 | 内容 |
|---|---|
| `service.rs` | `ThemeService`：实现 `Theme`；按 `workbench.colorTheme` 选主题（未设置 → `Dark 2026`，未安装 → 回退 `Dark 2026` 并记录诊断）；`ThemeCatalog` 内置十个默认主题，R68 可追加扩展主题 |
| `file.rs` | 主题 JSON 加载：`include` 链（相对路径、多层、循环检测）、`colors`/`tokenColors`/`semanticTokenColors`/`type`/`semanticHighlighting`、旧式 `settings`；非法值只跳过该键并产生 `Diagnostic`（文件 + 字段 + 原因） |
| `textmate.rs` | vscode-textmate `theme.ts` 移植：父 scope 选择器、`>`、逗号/数组多选择器、特异性排序、`fontStyle` 显式清空；`resolve` 按分词器的逐层合并得到最终样式 |
| `defaults/` | 976 个 VS Code 颜色键的四套默认值（`generated.rs` 由 `scripts/theme/gen_color_defaults.py` 生成），支持引用、`transparent`、`lighten/darken`、`opaque`、`oneOf`、`ifDefinedThenElse`、`lessProminent`、`mix`；`P0_KEYS` 为 THEME_COLORS.md 的 404 个 P0 键 |
| `resolved.rs` | 一份不可变快照：`workbench.colorCustomizations`（`[主题]` 与通配形式优先于全局，`"default"` 回到默认值）> 主题 `colors` > 默认值；`editor.tokenColorCustomizations` 的简写分组、`textMateRules`、按主题形式追加在主题规则之后 |

用法：

- 装配：`ThemeService::new(settings)` 返回 `Arc<ThemeService>`（实现 `Arc<dyn Theme>`），登记表含全部 P0 颜色键。
  #13 的 `SettingsService` 或任何 `Settings` 实现都可以；读不到的键按未设置处理。
- 颜色：`theme.color("tab.activeBackground")`，查构建时算好的表，O(log 键数)，UI 线程安全；未登记的键返回 `NotFound`，
  `Ok(None)` 表示 VS Code 默认值就是“不设色”。新功能需要 P1/P2 键时，在装配处用
  `defaults::register_vscode_colors(&mut registry, &[...])` 登记（默认值与派生关系自动取自表），再用
  `ThemeService::with_catalog` 创建服务。
- token：`theme.token_style(&scopes)`，`scopes` 是 #9 输出的由外到内的 scope 列表；结果按完整路径缓存，
  前景色总是已设置（无匹配为 `editor.foreground`），背景色只在规则显式设置时返回。
- 变更：设置的三个主题键变化时在后台线程重新加载，成功后 `changes()` 发布新修订号；主题文件被改动时由监听方调用
  `ThemeService::reload()`（内置主题打包在二进制中，不需要监听；扩展主题的监听随 R68）。加载失败保留旧快照。
- 诊断：`ThemeService::diagnostics()`，同时写到 stderr（前缀 `rove-theme:`）。
- 语义 token：本 issue 只解析 `semanticTokenColors` 与 `semanticHighlighting`（`ResolvedTheme::semantic_token_rules`），
  “语义 token 类型 → TextMate scope”的回退由 R58 补齐。

性能：`cargo xtask bench --filter theme/`（`xtask/benches/theme.rs`），见 PR 中的数据。

## 14. 工作台（#16）

位置：`crates/rove-ui/src/shell/`。纯数据模型不依赖 GPUI，GPUI 绘制与命令分发集中在 `view.rs`；
宿主（目前是骨架的 `Workbench`）实现 `ShellHost`，提供编辑区、视图工厂、文档身份与命令执行桥。

| 模块 | 内容 |
|---|---|
| `catalog.rs` | 全部登记项：工作台命令与 KEYBINDINGS.md 默认键位、`window.*`/`workbench.*` 设置、`titleBar.*` 等颜色键、中文字符串、菜单项、视图容器 / 小节 / 面板 / 状态栏条目 |
| `feedback.rs` | `FeedbackHub` 实现 `FeedbackService`：五级反馈、同因通知计次、勿扰、提示条按“文件 + 原因”去重与“不再提示”、模态只接受数据丢失 / 安全确认 |
| `layout.rs` | `LayoutState` 与按工作区持久化的 `LayoutStore`（`%APPDATA%\Rove\workspaceStorage\<哈希>\layout.json`、`globalState.json`） |
| `menus.rs` | `MenuRegistry`：文件、编辑、选择、查看、转到、帮助 + 齿轮菜单；命令未登记或未实现的项不显示 |
| `status.rs` | `StatusBar`：状态栏条目的运行期内容（`$(icon)` 文本、悬停提示、命令、普通 / prominent / warning / error） |
| `icons.rs` | 打包的 codicons 字体与 `$(图标名)` / `$(图标名~spin)` 语法 |
| `progress.rs` | 100 ms 后才显示的细进度条判定（#17 打开文件时用，绘制用 `view::progress_bar`） |
| `view.rs` | `Shell` 状态、`ShellHost`、`run_command`、`report`、各部件绘制、`action_for()`（命令 ID → GPUI action） |

**登记方式（其他 issue 只追加，不改工作台代码）**

- 视图：在 `ContributionRegistry` 登记 `Placement::ActivityBar`（视图容器，`factory_key = catalog::CONTAINER_FACTORY`）、
  `Placement::Sidebar { container }`（容器内的可折叠小节，`order` 决定顺序：打开的编辑器 0、文件夹 10、大纲 20）
  或 `Placement::Panel`；宿主的 `ShellHost::render_view` 按 `factory_key` 返回元素，未装配的工厂不显示。
  面板隐藏时不调用面板视图工厂。
- 状态栏：登记 `Placement::StatusLeft` / `StatusRight`（`order` 即 VS Code 的 priority，越大越靠左，
  `factory_key = catalog::STATUS_FACTORY`），运行期用 `shell.status_bar.set(&shell.contributions, id, StatusEntry)` 更新内容。
- 菜单：在 `shell.menus` 登记 `MenuItem`；命令实现后在装配期调用 `shell.mark_implemented(id)`，菜单才显示该项。
- 反馈：调用 `shell::view::report(host, key, Reason, Message, actions, cx)`，**由原因决定级别**
  （`Receipt`/`Background` → 状态栏 3 秒；`NoResult` → 光标提示；`FileContext` → 编辑器提示条；
  `Global(severity)` → 通知；`DataLoss`/`SecurityConfirmation` → 模态）。正常后台工作用 `Background`，永不弹通知。

**键位**：由 #14 的键位服务统一解析（见 §18），工作台不再自带键位表；`statusBarFocused`、`notificationFocus`、
`notificationToastsVisible`、`notificationCenterVisible` 作为 VS Code 上下文键发布在 GPUI key context 上，菜单与悬停提示的
键位文字取自生效的键位表（`catalog::key_label`，含用户覆盖）。GPUI 0.2.2 的 Windows 后端把 `ctrl+numpad0` 报成 `ctrl+0`，
所以 `ctrl+numpad0`（zoomReset）实际按不出来，`ctrl+0` 仍聚焦侧边栏。`ctrl+shift+f` 在 VS Code 默认表中是
`workbench.action.findInFiles`，搜索功能实现前由工作台打开搜索视图。模态框打开时独占键盘，Esc 按对话框原生行为取消。

**缩放**：`workbench.action.zoomIn/zoomOut/zoomReset` 与 `window.zoomLevel` 同 VS Code（每级 ×1.2，范围 ±8，
`window.zoomPerWindow` 为 true 时不写回设置）；工作台尺寸与代码视图行高一起缩放，只重新整形可见行，不重新扫描、不重新解析。

**启动**：首帧前只同步读取几百字节的布局文件以恢复窗口位置、大小与最大化状态；之后的布局写入都在后台执行器完成。
codicons 字体在 `launch` 中注册。

## 15. 列表、树与快速输入组件（#18）

位置：`crates/rove-ui/src/list_widget/`。兑现 §5 的 `ListModel` / `ListController` / `QuickInput` 契约，不修改 `lists.rs`。
`lib.rs` 只追加 `pub mod list_widget;`。

| 模块 | 职责 | GPUI |
|---|---|---|
| `widget.rs` / `ListWidget` | 焦点、选择、锚点、滚动（含平滑滚动、按页、水平）、查找/过滤、键入导航、粘性滚动、全部 `list.*` 命令与上下文键；实现 `ListModel` + `ListController` | 否 |
| `tree.rs` / `TreeModel`、`TreeSource` | 展开时才向数据源要子节点；`Children::Pending` 时显示加载行，数据源随后 `set_children` 推送；`ChildCount::{None, Unknown, Known(n)}` 区分“无子节点 / 未知 / 已知数量”；过滤时自动展开祖先，退出过滤还原用户的展开状态 | 否 |
| `quick_input.rs` / `QuickInputService` | 实现 `QuickInput` 契约与全部 `quickInput.*`、`history.*` 命令；分组、复选框、按钮、底部加载行；`pick_events()` 额外给出修饰键与后台接受 | 否 |
| `matcher.rs` / `Matcher` | “打分 + 高亮区间（UTF-8 字节）”接口；内置包含匹配与简单子序列匹配，#24 用 VS Code `fuzzyScorer` 替换（`set_matcher`） | 否 |
| `options.rs` / `ListOptions` | 登记并读取 `workbench.list.*`、`workbench.tree.*`（P0 全部 + 已弃用的 `keyboardNavigation` 回退） | 否 |
| `registry.rs` | 命令、界面字符串、颜色键（Lists and trees、Quick picker、`input.*`、`inputValidation.*`）及 KEYBINDINGS.md 默认键位表（`default_bindings()` 供 #14） | 否 |
| `view.rs` / `ListView` | 只构建与绘制可见行；行渲染闭包 `RowRenderer`、空状态/加载插槽、底部加载行、粘性行、缩进参考线、滚动条、查找框、拖放事件与 `list.drop*` 绘制、右键/`shift+f10` 菜单载体 | 是 |
| `quick_input_view.rs`、`text_input.rs` | 快速输入框界面；支持 IME 的单行输入框（#21/#25 可复用，带 `InputHistory`） | 是 |
| `keys.rs` | 每个命令一个 GPUI action（`action_for`），由 #14 的键位服务分发；`EXTRA_DEFAULTS` 是不在 VS Code 表里的列表默认键位（`shift+f10` 上下文菜单）；文本框的编辑键（退格、方向键、剪贴板）是输入框原生行为，只在没有键位命中时处理 | 是 |

接入方式：视图只提供数据（`ListModel` 或 `TreeSource`）与可选的行渲染闭包，列表不了解条目类型。
重复登记（中心负责方 #10/#13/#14 已先登记同名颜色、设置或命令）按“保留已有条目”处理，默认值同为 VS Code 值。
上下文键：`listFocus`、`listSupportsMultiselect`、`listHasSelectionOrFocus`、`listScrollAtBoundary`（`none`/`top`/`bottom`/`both`）、
`listSupportsFind`、`treeFindOpen`、`treeElement{CanCollapse,CanExpand,HasParent,HasChild}`、`treestickyScrollFocused`、
`inQuickInput`、`quickInputType`、`inputFocus`、`cursorAtEndOfQuickInputBox`、`historyNavigation*`。

性能：可见范围 O(log n)，取行 O(可见行)；过滤在 UI 线程同步完成（10 万条以内一帧内），不做防抖以外的异步。
固定行高 O(1) 布局；按行计算行高在每次模型修订时 O(n) 建前缀和。数据侧基准为 `cargo xtask bench --filter lists/`（每帧取行、1 万子节点展开、10 万条过滤）；含 GPUI 的帧测量见 `crates/rove-ui/examples/list_bench.rs`
（`cargo run -p rove-ui --release --example list_bench`，输出口径同 M0 报告 §2.5），数据记录在 #18 的 PR。

测试：`cargo test -p rove-ui`。单元测试在 `list_widget/widget/tests.rs`（每个 `list.*` 命令与每个 P0 设置项）与
`quick_input.rs`；集成测试 `tests/list_view.rs`、`tests/tree_view.rs`、`tests/quick_input_view.rs` 在真实 GPUI 窗口里
只通过键位/鼠标驱动，包含 `ctrl+up/down` 按 `listScrollAtBoundary` 穿透到外层的验证和一个编码选择器的消费方示例。

## 16. 代码视图（#19）

位置：`crates/rove-ui/src/code_view/`。只依赖 #1 的冻结契约（`TextSource`、`Settings`、`Theme`、各登记表），
不修改契约；GPUI 类型只出现在本模块与 `rove-app`。

| 文件 | 内容 |
|---|---|
| `view.rs` | `CodeView` 实体：对外接口、事件、命令执行、帧外准备（读文本 → 装饰 → 整形 → 预取） |
| `element.rs` | GPUI `Element`：布局可见行、绘制（行号、当前行、选区、字形、占位、插入符、滚动条）、鼠标 |
| `scroll.rs` | f64 滚动模型（虚拟行 + 嵌入区域）、滚轮换算、滑块与概览标尺坐标换算 |
| `columns.rs` | 字素簇边界、可见列（VS Code 语义：Tab 制表位、全角/emoji 记 2 列）、渲染上限、命中测试 |
| `selection.rs` | 只读选区与插入符移动、选词（`editor.wordSeparators`）、行选择 |
| `decorations.rs` | 装饰提供方、优先级合并、按行缓存与失效 |
| `cache.rs` | 按字节预算淘汰的 LRU（整形缓存、行缓存） |
| `options.rs` / `colors.rs` / `commands.rs` | `editor.*` 设置项、颜色键、命令与字符串的登记 |

帧内只读缓存：按 f64 滚动位置（`top` 为虚拟行，含上内边距与嵌入区域）算出可见行，从行缓存取显示形式
（Tab 展开、截断），从整形缓存取 `Arc<LineLayout>`，用 `paint_glyph` 按装饰上色。整形缓存键是
“显示文本 + 粗斜体分段 + 字体代次”（字体代次覆盖字族、字号、字重、连字、字间距、缩放、DPI），颜色
不进入键，所以主题 token 或查找高亮变化不触发重新整形。缓存未命中的行先按等宽近似画淡色占位，
帧后的空闲切片（每片约 3 ms，至少推进一行）读文本、调用装饰提供方、整形，可见行优先，再沿滚动方向
预取一屏、反方向半屏；行高与 y 坐标只由行号和 f64 滚动位置决定，替换时不跳动。

对外接口（详见 `code_view/mod.rs` 的 rustdoc 速览表）：文本源 `set_text`；装饰 `add_decorations` /
`invalidate_decorations`（优先级常量在 `decorations::priority`）；选区 `selection` / `set_selection` 与
`CodeViewEvent::SelectionChanged`；滚动定位 `reveal_line` / `reveal_position`（`Reveal::{Nearest, Center,
CenterIfOutside, Top}`）；视口事件 `CodeViewEvent::ViewportChanged`；行内覆盖物 `decorations::Overlay` 与
行下方嵌入区域 `set_view_zone` / `view_zone_bounds`；渲染上限挂钩 `set_line_render_limit`（#22 接入
`editor.stopRenderingLineAfter` 的界面表现）；命中测试 `position_for_point` / `point_for_position`；
命令 `execute(<VS Code 命令 ID>)`。

登记：装配时调用 `code_view::register(settings, colors, commands, strings)`；已被他人登记的键跳过而不报
`Duplicate`（同一 VS Code 设置/颜色/命令可能有多个消费方）。颜色先读当前主题，主题未登记时用
VS Code 内置默认值。默认键位来自 #14 生成的 VS Code 默认键位表，键位服务按 when 解析后经
`commands::action_for` 分发；代码视图聚焦时在 key context 上发布 `editorTextFocus`、`editorFocus`、
`textInputFocus`、`editorReadonly`，有选区时加 `editorHasSelection`。字体缩放三个命令没有默认键位。

验证：`cargo test -p rove-ui`（单元测试）与 `cargo test -p rove-ui --test code_view`（入口集成测试，
含装饰/选区/滚动定位/视口事件四类消费方示例、1429 万行合成文本源、7 种模式命中测试、设置逐项生效）。
性能：`cargo xtask gen-testdata --only small`、`--only uniq` 后 `cargo xtask bench --filter scroll`
（真实窗口，见 `xtask/benches/scroll_unique.rs`）。

### 高亮、折叠与只读（#20）

位置仍在 `crates/rove-ui/src/code_view/`，新增文件：

| 文件 | 内容 |
|---|---|
| `highlight.rs` | 语法着色层：`SyntaxSnapshot` 的按行 token 经 `Theme::token_style` 解析为样式（前景、背景、粗体、斜体、下划线、删除线），按行缓存；`editor.maxTokenizationLineLength` 以上的行不着色 |
| `brackets.rs` | 括号匹配（`editor.matchBrackets`）：按 #12 语言配置的 `brackets`，光标左右两侧；`always` 时高亮包围光标的一对；字符串与注释里的括号由语法 token 排除；每方向最多扫 5000 行 |
| `occurrences.rs` | 文本同名高亮（单词按语言 `wordPattern` 整词、大小写敏感）与选区相似匹配（`editor.selectionHighlight*`、`selectedTextMatchMode`） |
| `folding.rs` | 折叠：区间模型（语法、`#region` 标记、缩进、块注释、手动）、折叠状态、全部折叠命令，以及独立的“文档行 ↔ 屏幕行”映射 `FoldMap`（O(log 折叠数)；R54/R55/R57 共用） |
| `guides.rs` | 缩进参考线层级与 `editor.renderWhitespace` 五种取值的纯计算 |

- **装饰层**：语法（优先级 0）、同名高亮（25）、括号匹配（35）都是 `CodeView` 自己登记的装饰层，结果按行缓存，只在快照、
  主题、选区或设置变化时按行或整层失效；失效后在重算前继续画旧结果，所以重新解析、换主题都不闪烁。粗体/斜体经
  `font_runs` 进入整形缓存键，颜色不进入。
- **语法快照**：宿主调用 `set_syntax(snapshot, changed_lines)`；`changed_lines` 为 `None` 时整层失效，否则只失效这些行
  （`take_syntax_invalidation` 可断言）。没有快照时显示纯文本，快照到达后一次性替换。
- **折叠**：区间在后台线程按全文计算（`compute_ranges`：`foldingStrategy=auto` 且快照有语法区间时用语法区间 + 标记区间，
  否则用 #12 的 `fold_ranges` 缩进 + 标记区间；块注释另算），`editor.foldingMaximumRegions` 保留层级浅的。折叠状态按
  `(首行, 末行)` 记录，区间重算后按首行重新对应；变化时发出 `CodeViewEvent::FoldsChanged`，宿主写回标签的
  `ViewState::folds`，切换回来时先 `set_folds` 再恢复滚动（滚动坐标是屏幕行）。滚动模型、嵌入区域、命中测试、预取都按
  屏幕行计算，行缓存与装饰按文档行。语法区间目前由快照的 `folds()` 提供（`rove-syntax` 尚未产出，见 #118）。
- **只读**：宿主按文档计算只读原因（`presented_read_only`：超大文件、元数据视图、磁盘只读属性、`readonlyInclude` /
  `rove.files.extraReadonlyInclude`；加载器默认的 `Session` 不算），加上“在会话中切换只读模式”的按文档覆盖（超大文件除外），
  经 `set_read_only` 交给视图。视图发布 `editorReadonly`，在只读时输入字符发出 `CodeViewEvent::EditBlocked`，工作台按
  UX_SPEC §6.1 第 2 级在光标处提示原因；标签（单标签时即编辑器标题）显示锁形图标。只读规则在 `FileDocuments` 打开时求值。
- **文档级别**：`set_document_level(SizeClass)`；大文件关闭同名高亮与基于语法的折叠（保留缩进折叠），超大文件关闭语法高亮
  与全部折叠；`disabled_features()` 返回当前关闭的功能（字符串 key 见 `Feature::label_key`），供 #22 组装状态栏提示。
- **上下文键**：`foldingEnabled`、`hasWordHighlights`、`editorReadonly` 发布在代码视图的 key context 上。
- **颜色**：`colors::FEATURE_COLORS` 的默认值取 #10 的 VS Code 默认值表；其中 `editor.wordHighlight*` 是 P1 键，工作台用
  `colors::theme_registry()` 建主题服务，让用户主题也能覆盖它们。

验证：`cargo test -p rove-ui code_view`（各模块单元测试）、`cargo test -p rove-ui --test code_view_features`
（高亮、括号、同名高亮、折叠、参考线与空白、只读、级别的入口集成测试）与 `cargo test -p rove-ui readonly_`
（工作台：PackageCache 只读、锁形图标、光标处提示、会话切换）。

## 17. 编辑器组、标签与打开文档服务（#17）

位置：`crates/rove-ui/src/editors/`（服务与登记，无 GPUI 绘制）与 `crates/rove-ui/src/skeleton/workbench/editor_panes.rs`
（组网格、标签栏、错误占位页、空组水印）。实现 #1 的 `OpenDocuments` 契约（含 #86 的调度、目标组与选区语义），
不修改契约。

| 文件 | 内容 |
|---|---|
| `editors/mod.rs` | `EditorService`：组、标签、预览/固定、按组视图状态、请求终态事件、导航历史、文档按引用计数共享 |
| `editors/layout.rs` | 网格布局树：任意方向分屏、多级嵌套、方向切换、展开组时只调整祖先链比例 |
| `editors/actions.rs` | 编辑器组与标签命令（VS Code ID、标题字符串）登记与执行；`context()` 发布 `multipleEditorGroups`、`activeEditorGroupEmpty`、`activeEditorIsPinned` 等上下文键，默认键位与 when 来自 VS Code 默认表 |
| `editors/settings.rs` | `workbench.editor.*`、`workbench.editorAssociations`、`window.density.editorTabHeight` 登记与读取；编辑器类型登记表 + glob 解析 |

**组与代码视图**：每个编辑器组挂一个 `Entity<CodeView>`（#19），组内的“拆分编辑器”再加一个；不按标签建视图，
也不全窗口共用。活动标签的滚动与选区归 `CodeView` 所有，并经 `CodeViewEvent::{ViewportChanged, SelectionChanged}`
随时回写到该标签的 `ViewState`，所以经 `OpenDocuments` 关闭或移动标签时也拿得到最新状态。切换标签时
`set_text` 装入新标签的文本，再用 `set_scroll_position` / `set_selection` 恢复它的 `ViewState`；打开请求带位置时，
服务在标签上记下待定位，装入后用 `Reveal::CenterIfOutside` 定位，之后才发出 Ready。关闭后重新打开按
`workbench.editor.restoreViewState` 恢复（按组保存，组关闭即丢弃）。`ViewState::folds` 留给 #20。

**共享文档**：同一文件在多个组打开时共享一个 `Document` 和一份 `TextSource`，各组的 `CodeView` 指向同一实例；
别名（相对/绝对/非规范路径、符号链接）在加载器规范化后按文档去重，复用同组已有标签。

**入口与验证**：`Workbench::open_documents()` 返回窗口拥有的服务；`cargo test -p rove-ui editor_tests`
覆盖调用方示例（路径 + 行列 + 来源 → 视口包含目标行、选区正确）、预览/固定/分屏/最大化、导航历史跨组往返、
切换标签恢复滚动与选区、别名去重；`cargo test -p rove-ui expanding_a_group` 覆盖布局比例。
切换耗时：`ROVE_EDITOR_SWITCH_TRACE=1` 运行 release 版，stdout 输出 `EDITOR_SWITCH_TO_PAINT_MS`（从命令处理到下一帧）。

## 18. 键位与命令层（#14）

位置：`crates/rove-commands/src/` 下的 `keys`、`when`、`context`、`keybindings`、`chord`、`center`、`builtin` 模块（`lib.rs` 只追加了模块声明，冻结契约未改）；GPUI 适配在 `crates/rove-ui/src/keys.rs`。核心层不依赖 gpui。

| 入口 | 用途 |
|---|---|
| `center::CommandCenter` | 命令表。`register(id, Command)` 登记 VS Code 命令 ID，`implement(id, handler)` 挂上实现。只登记未实现的命令按键能解析到，执行时返回 `Unsupported`；命令面板只列 `implemented()`。各功能在自己的模块里登记，没有集中的大数组。 |
| `context::ContextKeyService` | `ContextKeys` 的实现：读 O(log n)，值真正变化时才在解锁后发出一次 `ContextChange`。`context::keys::*` 列出范围内的上下文键及负责设置它们的 issue；没人设置的键就是未知键，求值为 false。 |
| `when::WhenExpr` / `WhenEngine` | 移植 VS Code contextkey 的语法和语义：`\|\|` < `&&` < `!`，支持 `==`/`!=`/`===`/`!==`（JS 宽松相等）、`=~` 正则字面量（整体作为一个 token，标志 `i`/`m`/`s`）、`in`/`not in`、`<`/`<=`/`>`/`>=`、括号、`true`/`false`。未知键读作 undefined，因此 `!unknown` 为 true，与 VS Code 一致（`!isWeb` 等默认键位依赖这一点）。语法错误返回 `InvalidInput`，对应的键位永远不会生效。 |
| `keys::parse_keybinding` | 键表达式解析：修饰键（`ctrl`/`shift`/`alt`/`meta`/`win`/`cmd`，用 `+` 或 `-` 连接）、空格分隔的和弦，大小写不敏感。键名统一成 VS Code 的通用名称：`/` 与 `oem_2`、`=` 与 `oem_plus` 等价，支持 `numpad_*` 和 `[Slash]` 这类扫描码写法（按 US 布局映射，即 `keyboard.dispatch: code` 在该布局下的结果），不做布局推断。 |
| `keybindings::KeybindingService` | 默认键位加用户 `keybindings.json`（JSONC），裁决规则同 VS Code：后定义优先，用户规则排在默认规则之后，`-command` 按命令、键、when 移除默认规则，when 不满足时继续往前找，命令未登记的规则直接忽略（不报错）。热加载：`reload_if_changed` 比较修改时间和大小；文件解析失败时保留上一份有效的表。 |
| `chord::ChordState` | 和弦状态：匹配到前缀时进入待定状态；下一个键未绑定时中止（`Esc` 也是这样）；5 秒无操作（`CHORD_TIMEOUT`）或窗口失焦时退出；单独按修饰键不改变状态。 |
| `builtin` | 登记 KEYBINDINGS.md §5 的 Rove 专有命令（只登记，实现由各功能 issue 完成），以及命令层自己的中文字符串。 |

默认键位表：`python scripts/compat/gen_default_keybindings.py` 按 `scripts/compat/rules.py` 的判定，从 `docs/compat/vscode/default-keybindings.json` 生成 `crates/rove-commands/data/default-keybindings.json`。表中包括 P0/P1/P2 全部“实现”条目（保持导出顺序），再追加人工补充的键位和 Rove 专有键位，编译时嵌入二进制。CI 的行尾检查任务运行 `--check`，校验生成物与源数据一致。P1/P2 键位通过“命令未登记就忽略”自然分批生效，不需要额外的开关。

GPUI 接入（`crates/rove-ui/src/keys.rs`）：全应用只有一个按键入口。`keys::init` 安装 GPUI 全局 `Keys`（键位服务、上下文键服务、命令表 `CommandCenter`）和一个按键拦截器（`intercept_keystrokes`，先于 GPUI 自己的键位表运行）。拦截器把按键换成 VS Code 键名，用“焦点路径上各元素的 GPUI key context + 全局上下文键”求值 when，再把命中的命令派发为实现它的模块的 GPUI action（`keys::action_for`：工作台、代码视图、编辑器组、列表与快速输入）；没有模块 action 的命令派发为 `RunCommand`，交给窗口处理。焦点路径上没有处理者时，按键交给原生处理（例如输入框里的 `ctrl+c`）；和弦的最后一个键总是被消费。模态框打开时独占键盘。各模块不再调用 `bind_keys`，默认键位全部来自生成的 VS Code 表；少数不在表里的放在模块的 `EXTRA_DEFAULTS` 中，同样经键位服务登记，所以用户的 keybindings.json 可以覆盖或移除任何一条。各模块只在 key context 上发布 VS Code 上下文键（`editorTextFocus`、`listFocus`、`statusBarFocused`、`multipleEditorGroups` 等）。菜单与悬停提示的键位文字由 `keys::key_label` 从生效的表中取出。GPUI 会把 Shift 并入标点和数字的字符（`ctrl+shift+-` 上报为 `ctrl+_`），适配层按 US 布局还原成“键 + Shift”。GPUI 不区分小键盘与主键盘上的同一字符，`numpad_*` 键位目前无法从实际按键触发。用户目录由 #13 的 `rove_config::paths` 解析（依次为 `--vscode-user-dir`、`ROVE_VSCODE_USER_DIR`、`%APPDATA%\Code\User`）。

验证：`cargo test -p rove-commands`（键表达式、when（移植 VS Code 用例）、裁决与移除、和弦、热加载；`tests/keybindings_integration.rs` 对照 KEYBINDINGS.md 检查 P0 是否漏项，并覆盖“读 keybindings.json → 按键 → 分发”；`builtin` 检查 Rove 键位与 VS Code 默认键位无冲突），以及 `cargo test -p rove-ui keybindings_json_keys_dispatch_commands`（真实 GPUI 键盘事件：用户覆盖工作台键位、`-command`、未知命令、和弦的 Esc 与超时、菜单键位文字）和 `keys::tests::no_module_binds_keys_directly`（防止模块绕过键位服务直接 `bind_keys`）。

### 设置层装配（#104）

- 入口：`rove.exe [--vscode-user-dir <目录>] <工作区>`。`rove-app` 用 `rove_config::paths::split_user_dir_arg` 拆出参数，
  其余第一个参数是工作区；不传时按 `ROVE_VSCODE_USER_DIR` > `%APPDATA%\Code\User` 解析。
- `rove_ui::app_settings::registry()`：依次登记工作台（shell）、`rove_buffer::settings`、`code_view::options`、
  编辑器组（`editors::settings`）、列表、`rove_workspace`、`rove_syntax::highlight` 的键，先登记者胜出，最后
  `register_builtin`。单元测试校验各模块对同一键的默认值一致，且与 `compat/SETTINGS.md` 一致。
- `app_settings::open` 打开**唯一**的 `SettingsService`，`skeleton::launch` 把同一个 `Arc<dyn Settings>` 交给窗口设置
  读取、`Shell`、`ThemeService`、每个 `CodeView`、`FileDocuments`（打开服务）与工作区扫描。#94 直接从
  `Workbench` 持有的这个实例读取 `files.associations` 等语法设置。`app_settings::defaults_only()` 只给测试与
  `Workbench::new`（测试入口）用。
- 热加载：`Workbench` 订阅 `settings.changes()`，合并同一批变更后在 UI 线程：丢弃工作台对这些键的本窗口临时值
  （工作台按键现读设置）；`workbench.editor.*` 变化时重读编辑器组设置；所有代码视图 `reload_options` 并乘窗口缩放；
  `rove_workspace` 登记的键（`files.exclude`、只读规则等）变化时按新规则重新扫描。`ThemeService` 自己订阅主题相关键并在
  后台重载，`Workbench` 订阅主题的 `changes()` 刷新工作台明暗与代码视图配色。
- 主题重载竞态：`ThemeService::reload` 在读设置前领取代次，只有不低于已应用代次的结果才替换快照，被取代的旧加载返回
  `Stale`；连续切换两次主题，最后生效的是后切的那个。

## 19. 文件大小分级与超大文件（#8）

位置：`crates/rove-buffer/src/{size,line_index,mapped}.rs`，入口 `rove_buffer::open_file(path, options, SizePolicy)`；
不修改 #1 契约，超大文件同样以 `Arc<dyn TextSource>` 交给文档模型（`FileText::source()`）。骨架的
`FileDocuments::open` 改为调用 `open_file`，`editors::settings::SizePolicy` 只负责从设置读两个键。

- **分级**（`size.rs`，`UX_SPEC` §3.1）：`SizePolicy { huge_bytes, optimize }` 来自 `rove.largeFile.threshold`
  与 `editor.largeFileOptimizations`。超大文件 ≥ 阈值（只读，追加 `ReadOnlyReason::HugeFile`，
  `session_toggle_allowed` 为假）；否则 `optimize` 时 ≥ 20 MB 或 ≥ 30 万行为大文件。各级关闭的功能由
  `disabled_features(SizeClass)` 给出（`ReducedFeature`，只有数据，界面字符串由 #22/#27 登记）。
- **内存映射**（`mapped.rs`，`MappedText`）：UTF-8（含 BOM）直接 `memmap2` 映射；编码按显式指定 → BOM →
  `files.encoding`，**不做全文猜测**；非 UTF-8 编码只能整体解码进内存。二进制判定与换行符统计只看头部 1 MiB。
  不合法 UTF-8 字节逐字节显示为 `?`（字节长度不变，坐标仍对应）；映射内容被外部改写时只会读到新字节，不会 panic。
  Windows 上被映射的文件不能被其他进程截断（系统拒绝），可以改写、追加、重命名。
- **分块行索引**（`line_index.rs`）：每 256 行一个 `u64` 锚点，块内 `u32` 相对偏移（跨 4 GiB 的块退回绝对值表）。
  65,536 行一段，写满即冻结为 `Arc` 段。打开时同步索引头部 1 MiB，其余由 `rove-line-index` 线程按 32 MiB 分块
  扫描（`memchr`，64 KiB 子块先判断有无 CR）。`lines()` 的 `complete == false` 表示最终行数未知；未索引的行返回
  `NotReady`，越界返回 `OutOfBounds`。`wait_for_lines` / `wait_complete` 供“转到行”等待（界面在 #22）。
- **增量**：`MappedText::reload()` 用“大小 + 修改时间 + 头部 4 KiB 与旧末尾 4 KiB 的校验”判断：`Unchanged`、
  `Appended`（新快照共享冻结段、只扫描新增部分；旧快照末尾单独的 CR 与新内容开头的 LF 合成 CRLF）、
  `Replaced`（截断或中间改写，整体重建）。跟随文件末尾（R66）基于它实现。

验证：`cargo test -p rove-buffer`（`tests/large_file.rs` 覆盖“打开超大文件 → 只读原因 → 进度 → 转到行”、
真值表、追加/改写/截断、边界用例、单行 100 MB）；性能：`cargo xtask bench --filter huge/`（`xtask/benches/huge_file.rs`，
1 GB 用例需要本地 `big1g.txt`）。

## 20. C# 索引生命周期与状态（#36）

位置：核心逻辑在 `crates/rove-lang-cs/src/lifecycle/`（不依赖 gpui，`lib.rs` 只追加 `pub mod lifecycle;`）；
界面在 `crates/rove-ui/src/index_status.rs`，装配在 `skeleton/workbench.rs` 末尾的 `#36` 段。
`rove-lang-cs` 新增依赖 `rove-config`（读 `rove.index.*`），见 §2 的依赖图。

### 缓存目录布局（#37 共用）

```text
<缓存根>\                      rove.index.cacheDir；为空时 %LOCALAPPDATA%\Rove\index（必须是绝对路径）
  <小写文件夹名>-<16 位哈希>\    每个项目一个目录；哈希是规范化项目根路径（去 \?\、反斜杠、不分大小写）的 FNV-1a-64
    project.json              项目路径、最近打开时间（淘汰依据）、最近一次构建记录（世代、头部配置哈希、输入指纹、文件数、符号数）
    l1\l1-decls.bin           L1 快速声明索引：目录由 ProjectCache::l1_dir() 给出，文件名由 #37 固定
    l2\<16 位十六进制世代>.rvcsidx   L2 语义索引；最大的世代是当前索引，其余是待删除的旧世代
    l2\*.tmp                  索引进程写入中的临时文件（超过 1 小时视为崩溃遗留，打开时清理）
    logs\indexer.log          索引日志；超过 1 MiB 轮转为 indexer.1.log … indexer.3.log
```

- **#37 的接入方式**：`ProjectCache::new(&cache_root, &project_root).l1_dir()` 就是传给 L1 持久化接口的目录；
  生命周期在打开项目时已经创建好它（`rove.index.enabled = false` 或工作区没有 C# 代码时不创建任何目录）。
  淘汰按项目目录整体删除，L1 随之删除，不需要单独处理。
- **原子替换**：Windows 上正在被内存映射的文件不能被覆盖，所以新索引写到新世代文件（索引进程自己先写
  同目录临时文件再改名），切换是把 `IndexLifecycle::snapshot()` 里的 `Arc<IndexSnapshot>` 换成新的；
  已经拿到旧快照的查询继续读旧映射，不会读到半截文件。旧快照最后一个持有者放手时删除旧世代文件
  （Rust 的 `remove_file` 在 Windows 上使用 POSIX 删除语义，映射中的文件名也能立即移除，数据在解除映射前有效）。

### 生命周期（`IndexLifecycle`，每个项目一个后台线程）

| 步骤 | 内容 |
|---|---|
| 打开 | `rove.index.enabled = false` → `Disabled`；扫描项目输入（根目录 `*.csproj`，`Assets/`、`Packages/` 下的 `*.asmdef`/`*.asmref` 内容与 `*.cs` 路径），没有 C# → `NoCSharp`（条目不显示）；创建缓存目录、写 `project.json`（同时验证可写，失败 → `Failed(CacheDir)`）、淘汰 |
| 有效性检查 | ① magic 与格式版本 ② 索引进程版本（`validity::EXPECTED_INDEXER_VERSION`，测试对照 `RoveIndexer.csproj`）③ 项目配置：头部配置哈希与构建记录一致且输入指纹与磁盘一致 ④ 抽样文件哈希：`max(200, 2%)` 个文件比对 xxHash64，命中率 < 90% 重建，否则只把不一致的文件标记过期。①–③ 同步完成后立即发布索引（`Ready`），④ 在预读之后的后台进行 |
| 预读 | `prefetch::prefetch`：`FILE_FLAG_SEQUENTIAL_SCAN` 全速顺序读整个文件、普通 I/O 优先级、可取消（关项目即停）；刚构建的索引跳过。限速与低优先级两种策略保留给基准，实测在 HDD 上都更差（数据见 `prefetch.rs` 注释与 #36 的 PR） |
| 构建 | `process::locate_indexer`（唯一的组件查找函数，规则见 §34）→ `ProcessBuilder` 拉起 `rove-indexer-cs <项目> <新世代路径>`，stdout/stderr 逐行写日志并解析进度（阶段行与 `files: N`；将来的 `progress <done> <total>` 直接采用）；失败重试一次，组件缺失 → `NotInstalled`，组件不兼容 → `Incompatible`（不重试，见 §34），其余 → `Failed`；已知对不上的旧索引不再提供（降级到 L1/L0） |
| 文件变化 | `files_changed(&[SourceChange])`（界面从 #15 的 `FileEvent` 转换）：源文件先标记过期（`is_stale` 供 #38 降级为近似），再转发给常驻索引进程增量更新（#35，状态 `Updating`，覆盖层写入后清除过期标记；没有常驻组件时停留在过期），项目配置文件变化全量重建，监听溢出重新检查。见 §29 |
| 淘汰 | `CachePolicy` 默认 10 个项目、总量 5 GiB（依据见 `evict.rs`），按 `project.json` 的最近打开时间从旧到新删除，当前项目永远保留，只删除带 `project.json` 的项目目录 |

状态机 `IndexState`：`Checking`、`NoCSharp`、`Disabled`、`NotInstalled`、`Building { progress, eta, rebuilding }`、`Ready`、
`Updating { files }`、`Failed(Failure)`；所有迁移只在后台线程里发生，订阅 `changes()` 拿到 `IndexStatus`（状态 + 详情数据 + 修订号）。
项目配置哈希的 Rust 侧判定见 `inputs.rs` 的模块注释（头部哈希由 C# 计算，Rust 无法逐字节重算，所以在构建记录里绑定头部哈希与输入指纹）。

### 界面（UX_SPEC §2.1）

- 登记：状态栏条目 `rove.status.index`（`StatusLeft`，order 40，在问题计数右侧，默认命令 `rove.index.showDetails`）、
  面板视图 `rove.index.details`（“C# 索引”详情，工厂 key 同 ID，已加入装配校验的 `KNOWN_FACTORIES`）；文案全部是 `index.*` 字符串。
- 命令：`rove.index.rebuild` / `showDetails` / `openLog` 由 #14 的 `rove_commands::builtin` 登记，这里提供 GPUI action
  （`index_status::action_for`，已接入 `keys::action_for`，因此命令面板可见），`Workbench::run_invocation` 也直接处理命令 ID。
- 失败通知：进入 `Failed` 时经 `shell::view::report(…, Reason::Global(Error), …)` 报一次，key 为 `rove.index.failed.<类别>`，
  同因去重与勿扰由反馈层负责；按钮“查看详情”“重试”的动作 ID 就是命令 ID。恢复到 `Ready` 时收起失败通知。
- 设置热加载：`rove.index.enabled` / `rove.index.cacheDir` 变化时关闭旧生命周期并按新设置重新打开。

验证：`cargo test -p rove-lang-cs --test lifecycle`（打开 → 检查 → 预读 → 就绪 → 文件变更 → 标记过期；四类失效各自重建；
原子替换；进程缺失 / 启动失败 / 非零退出 / 崩溃；禁用与自定义缓存目录；淘汰）与 `cargo test -p rove-ui index_tests`
（五种状态 + 已禁用 + 不显示的条目文字、悬停与点击，通知去重，命令面板，真实生命周期的失败与重试）。
预读基准：`cargo run --release -p rove-lang-cs --example index_prefetch_bench -- <index> <scratch-dir>`（无缓冲复制制造冷缓存，
对比不预读 / 预读 / 限速预读），数据见 #36 的 PR。

## 21. 语法高亮的性能、增量解析与编辑器接线（#94）

位置：`crates/rove-syntax/src/highlight/`（#9 的实现之上），接线在 `crates/rove-ui/src/editors/syntax.rs`、
`editors/mod.rs`（打开与释放）和 `skeleton/workbench.rs`（后台任务与重新着色）。`contracts.rs` 的冻结契约未改动。

| 模块 | 内容 |
|---|---|
| `source.rs` | 解析输入：从 `TextSource::visit_chunks` 读一次，存成至多 64 KiB 的共享分片；tree-sitter 的读取回调和查询的 `TextProvider` 直接读分片。编辑只重建受影响的分片，其余分片与上一版共享，增量路径不再复制全文 |
| `csharp_parts.rs` | C# 分区并行：词法扫描识别注释、普通/逐字/插值字符串（含嵌套插值洞）、字符、`@标识符` 与条件指令（`#if/#elif/#else/#endif` 必须在同一花括号深度配对），只在没有括号、方括号和条件块未闭合的成员边界切分。容器取“唯一的一对花括号”逐层下探（`namespace N { class C { … } }`），多顶层类型时取编译单元。每个分区带真实的头尾作为 included ranges 解析，校验无错误且直接成员都是独立声明后，在同一个工作线程里查询并排序 capture。线程来自 `WorkerPool`：默认是进程共享池（硬件线程数，最多 16），并发文档按剩余线程分配；`TreeSitterSyntax::with_parallelism(n)` 用私有池（测试据此与其他分析隔离）。工作线程 panic 转成 `NotReady`，panic 信息写到 stderr（前缀 `rove-syntax:`） |
| `rows.rs` | 行表：一次扫描按 LF/CRLF/单独 CR 切分，再与 `TextSource::line_range` 核对行数和 64 个抽样行，不一致时逐行读取；编辑时只重读编辑附近的行，其后的行按字节差平移 |
| `documents.rs` | `DocumentHighlighter`（编辑器持有的接缝：`analyze(document, text, language, edit, visible, cancel)` 与 `close(document)`）；`SyntaxDocuments` 按 `DocumentId` 各自保存基线，`Stateless` 把任意 `SyntaxService`（如 `EmptySyntaxService`）包装成同一接口 |
| `mod.rs` | 编辑：只核对编辑区及前后 256 字节和总长度（`EDIT_CONTEXT`），编辑点的 tree-sitter 坐标从旧树就近节点推算，不再从头扫描。整树基线做常规增量解析；分区基线只重解析包含编辑的那个分区（编辑落在头尾或分区边界时整文件重建），变化行取该分区新旧树的 changed ranges。Markdown 把扫描行扩到完整的行内区域（`inline`、表格单元格）和代码围栏，只重新注入这些区域。`DocumentSyntax::with_visible(lines)` 在没有基线时只给可见行生成 token（结果标为 partial，不作为编辑基线）；对同一文本再分析一次时复用其解析结果补全其余行。`ROVE_SYNTAX_TRACE=1` 时每次分析在 stderr 打印读取、解析、行表、查询、token 生成的分段耗时 |
| `settings.rs` | `[csharp]` 行长：提供方对语言作用域和全局作用域返回同一个值时视为没有语言级值，保留内置的 2500（VS Code 的默认语言覆盖高于用户全局值）；用户的 `[csharp]` 值优先 |

接线：

- `FileDocuments::open` 把共享设置实例传给 `language_for_path`，`files.associations` 在正式打开路径生效（例如 `*.asset` → JSON）。
- `Workbench` 装配 `SyntaxDocuments::new(TreeSitterSyntax::default().with_settings(settings))`，与其他模块共用 #104 的设置实例。
- 打开：`SizeClass::Huge` 不高亮；小于 256 KiB（`editors::syntax::INLINE_BYTES`）的普通文档在标签就绪前分析完；
  更大的文档先就绪显示文本，`finish` 后启动后台分析：先分析当前视口附近的行，再补全全文。结果只应用到仍显示该文档、
  且文本版本与结果一致的标签，然后 `refresh_syntax` 重新给显示它的代码视图着色。不支持的语言按纯文本打开，不让打开失败。
- 生命周期：文档的最后一个标签释放时（关闭、被预览标签替换、换成别的文档）取消它正在进行的分析，并 `close` 它的基线；
  之后才完成的结果被丢弃。同一文档的不同版本、不同文档的相同版本号互不影响。
- 应用内目前没有产生 `TextEdit` 的编辑入口（编辑交互属于 M3 的 #46）；编辑入口接入时把 `edit` 传给
  `DocumentHighlighter::analyze` 即可走增量路径，按文档管理基线与过期结果的处理已经在上面这一层完成。
- 对 #20 的接口：token 仍按 `SyntaxSnapshot::tokens(line)` 读取，scope 由外到内；新增的只有“快照会在后台到达”：
  显示中的文档收到新快照时，工作台会更新 `SyntaxColors` 并失效语法装饰层。partial 快照中未覆盖的行返回空 token。

验证：`cargo test -p rove-syntax`（`tests/incremental.rs`：分区增量与整文件解析逐 token 一致、混合换行（#92）、
Markdown 局部更新与回退、可见区域优先、按文档基线、设置层；`csharp_parts` 单元测试覆盖 Unity 特征分区、不安全边界与分区重解析）；
`cargo test -p rove-ui syntax_tests`（真实窗口：大文件先显示后着色、版本不符与关闭后的结果、`files.associations`、
主题按选择器特异性取色）。性能：`cargo xtask bench --filter syntax/ --check-budgets`，用 `RopeText` 计入缓冲区读取与取行开销。

## 22. 文档符号、大纲、面包屑与转到符号（#26）

位置：核心层 `crates/rove-syntax/src/symbols/`（不依赖 gpui，`lib.rs` 只追加 `pub mod symbols;`，不改 `contracts.rs`）；
界面 `crates/rove-ui/src/outline/`；骨架装配 `crates/rove-ui/src/skeleton/workbench/outline_host.rs`（`workbench.rs` 只追加字段、
渲染调用与 action 监听，`editor_panes.rs` 在标签栏与正文之间插入面包屑）。

**符号数据（供 #24 `@`、R54 粘性滚动、R65 上下文篮子复用）**

| 入口 | 用途 |
|---|---|
| `symbols::availability(language, size)` | 是否提供符号：`Unsupported`（不是五种语言）/ `TooLarge`（`SizeClass::Huge`）/ `Available`；UI 线程 O(1) |
| `symbols::SymbolService::analyze(text, language, size, previous, cancel)` | 后台 `Task<SymbolUpdate>`：读 `TextSource` → tree-sitter 完整解析 → 提取 → 与上一版比较；内容相同的子树复用上一版的 `Arc<[Symbol]>`，`changed` 列出变化/新增节点，`removed` 列出消失的键 |
| `symbols::extract(language, text, cancel)` | 同步提取契约 `Symbol` 树（后台或测试） |
| `symbols::DocumentSymbols` | 先序编号 `SymbolId`、父子、深度、容器名（VS Code `containerName`）、跨版本稳定键（父键 + 种类 + 名称 + 同名序号）；`path_at` / `path_from` 按位置二分查找符号路径（O(深度 · log 兄弟数)，`path_from` 复用上一条路径） |
| `symbols::kinds` | 契约 `SymbolKind` 没有的 VS Code 种类走 `SymbolKind::Other(名称)`：`enumMember`、`operator`、`string`、`number`、`boolean`、`null`、`document`（YAML 多文档） |

语言：C#（命名空间含文件范围命名空间、类/结构/接口/枚举/记录/委托、字段（多变量拆开）、常量、属性、索引器 `this[int]`、
事件、构造/析构函数、方法 `Move(Vector3, float)`、运算符；`#if` 分支照常列出；未闭合的类型从 ERROR 节点恢复）、Markdown
（`#`–`######` 与 setext，按层级嵌套，区间是整节）、JSON/JSONC（键 + 值种类，数组元素以下标命名）、YAML（同 JSON；多文档各成
子树，界面显示“文档 N”）、XML（元素名）。数据文件最多 `DATA_SYMBOL_LIMIT`（5 万）个符号，超出时大纲提示只显示前若干个。
增量：只读阅读器阶段没有编辑，符号在后台对新快照完整解析；“只重建受影响的子树”体现为按子树比较后复用未变化的子树并报告变化节点。

**界面**

- 大纲小节：`outline::registry` 以 `Placement::Sidebar { container: workbench.view.explorer }`、`order = 20`、工厂 `outline`
  登记（文件夹 10 之后）。树与键盘导航复用 #18 的 `ListView`（`find_mode = Filter`：直接输入即过滤）；节点 ID 是符号稳定键，
  新版本到达后焦点、选择、滚动与用户的展开/折叠保留。遵循 `outline.collapseItems`、`outline.icons`、`outline.show*`（隐藏的种类连同
  子树隐藏）；图标颜色取 `symbolIcon.*`。小节顶部工具条：跟随光标（`outline.followCursor`，视图开关，不是设置项）、全部折叠
  （`outline.collapse`）。单击/回车经 `OpenDocuments`（`OpenSource::Outline`）定位到符号名称起点；回车后焦点回到编辑器。
  空状态：不支持的文件与无编辑器显示「活动编辑器无法提供大纲信息。」，超大文件显示原因，首次分析时显示加载中。
- 面包屑：每个编辑器组标签栏下方；文件路径段 + 光标所在符号路径，遵循 `breadcrumbs.enabled/filePath/symbolPath/icons/show*/
  symbolSortOrder/symbolPathSeparator`（`copyPath` 使用分隔符）。点击段或 `breadcrumbs.selectFocused` 弹出选择器
  （`breadcrumbPicker.background`）：符号段列同级符号，文件段列所在目录（文件夹逐层进入，文件经 `OpenSource::Breadcrumb` 打开）。
  命令 `breadcrumbs.focus/focusAndSelect/focusNext/focusPrevious/...WithPicker/revealFocused/revealFocusedFromTreeAside/
  selectFocused/selectEditor/toggleToOn/toggle/copyPath`，默认键位全部来自 VS Code 默认表；上下文键 `breadcrumbsPossible`、
  `breadcrumbsVisible`（窗口根 key context）、`breadcrumbsActive`（面包屑栏及其选择器），`config.breadcrumbs.enabled` 发布在
  #14 的全局上下文键服务上。
- 转到文件中的符号：`workbench.action.gotoSymbol`（`Ctrl+Shift+O`）用 #18 的 `QuickInputView`，`QuickPickConfig::filter = None`，
  候选与排序由 `outline::goto::symbol_picks` 计算（无查询按位置；有查询按分数、同分按位置；`:` 前缀按种类分组）。
  **#24 接入 `@` 时调用同一个函数并传入自己的 `fuzzyScorer`**，两处结果与顺序即一致。

光标移动时面包屑与跟随光标只做 `path_from`，不遍历整棵树；基准 `cargo xtask bench --filter outline/`
（`perf/budgets.toml` 的 `breadcrumb_cursor_move_5000_lines`，< 1 ms）。

验证：`cargo test -p rove-syntax symbols`（五种语言含嵌套、空文件、语法错误；索引、路径、增量、取消、超大文件）、
`cargo test -p rove-ui outline`（设置读取、过滤、面包屑段、选择器排序、转到符号排序、登记完整性，以及
`skeleton/workbench/outline_tests.rs` 的真实窗口入口测试：打开 `.cs` → 大纲节点与面包屑文本、跟随光标、键盘定位、
`Ctrl+Shift+O`、面包屑键位与选择器、设置热加载）。

## 23. C# L1 快速声明索引（#37）

位置：`crates/rove-lang-cs/src/l1/`（`lib.rs` 只追加 `pub mod l1;`），核心层，不依赖 gpui；`query.rs` 的冻结契约未改动，L1 只复用其中的 `QueryResult`、`Precision`、`Target`、`WorkspaceSymbol`。
直接依赖与 #9 相同版本的 `tree-sitter` / `tree-sitter-c-sharp`（同一个 grammar，不经过高亮层）；**不依赖 `rove-workspace`**（依赖图不变，`rove-workspace` 只是 dev-dependency）。

| 模块 | 内容 |
|---|---|
| `extract.rs` | 单文件声明提取：命名空间（含文件级 `namespace X;`）、class/struct/interface/enum/record/delegate（嵌套、泛型参数个数）、方法/属性/字段/事件/构造/析构/索引器/运算符/枚举成员、using（`global`、`static`、别名）；只走声明容器和 `#if` 分支，不进方法体 |
| `mod.rs` | `L1Index`：并行扫描（线程数可配）、分批并入（查询在批间可用）、`partial` 聚合（同一完整名 + 参数个数 = 一个符号、多个位置）、增量更新、进度与 `is_scanned` |
| `resolve.rs` | 类型引用解析：外层类型的嵌套类型 → 由内到外的命名空间（`namespace A.B` 含隐含的 `A`）+ 各层 using / using static / using 别名 → 全局 using；同层多个匹配全部返回 |
| `persist.rs` | `l1-decls.bin`：只写被引用的名字，带版本与整体校验，损坏即作废并全量构建 |
| `fuzzy.rs` | `SymbolScorer` 打分接口 + 最简 `SimpleScorer`（#24 合并前的占位） |
| `names.rs` | 共享名称表（同名声明只存一份字符串）、FxHash、内容哈希 |

接线（由装配处完成，L1 不遍历目录、不读 `.gitignore`、不联网）：

- 文件列表：从 #15 的 `Workspace::files` 分页取出，交给 `L1Index::build(files, cancel)`（非 `.cs` 自动忽略，列表外的旧文件移除）。根目录用 `Folder::root()`（带长路径前缀）。
- 文件事件：`FileEvent::Created/Changed → L1Change::Upserted`、`Removed → Removed`、`Renamed → Renamed`；`RescanRequired` 时重新取列表调用 `build`。`apply_changes` 只解析变化的文件，修改时间变了但内容哈希相同的不重新解析。示例见 `tests/l1_workspace.rs`。
- 缓存目录：`L1Options::cache_dir`，由 #36 定义布局后传入（与 L2 索引同一个项目目录，文件名固定 `l1-decls.bin`）。`build` 首次调用先载入缓存（载入即可查询，`is_scanned` 在核对磁盘后才为 true）；`build_and_save` 结束时保存；增量更新后由调用方择机 `save()`。
- 模糊匹配：#24 合并后，装配处把 VS Code `fuzzyScorer` 的移植包成 `SymbolScorer` 传给 `L1Options::with_scorer`，L1 不另写打分规则。

查询与精确度（全部同步，必须在后台线程调用）：

| 入口 | 结果 | 精确度 |
|---|---|---|
| `workspace_symbols(query, limit, cancel)` | `L1Symbol`（名字、容器、种类、参数个数、全部声明位置）；`to_workspace_symbols()` 转成契约类型，partial 每个位置一条 | `Syntax` |
| `resolve_type(file, position, text, cancel)` | `text` 是引用处写出的类型（`List<int>`、`Dictionary<,>`、`A.B`、`global::X`），按参数个数区分重名泛型；同一符号的位置中文件名相同、目录不同的视为源码副本（备份、迁移目录），只保留与引用处目录公共前缀最长的那份 | `Approximate` |
| `member_candidates(name, cancel)` | 同名成员（重载各自一条） | **一律** `Approximate` |
| `find_type(qualified)` / `progress()` / `is_scanned(path)` / `stats()` / `generation()` | 按完整名查类型、构建进度、文件是否已扫描、规模与常驻内存估算、索引代次（= 结果的 `index_generation`） | — |

`reason` 统一为 `ApproximationReason::Other("l1-declaration-index")`；何时用 L1、是否改写原因由 #38 决定。

边界（`LANGUAGE_INTELLIGENCE` §2.1）：不做绑定和类型推断，不看继承链（基类里的嵌套类型查不到），不处理 `#if`（两侧声明都收集，由 #38 决定是否使用；例外：`#if` 把类型头和 `{` 分在两个分支、整段解析失败的文件，会只取每个 `#if` 的第一个分支重新解析一次，无错才采用），不处理特性名省略 `Attribute` 后缀。非 UTF-8 且无 BOM 的文件按有损 UTF-8 解码：同一行中非 ASCII 字节之后的列可能偏移。

与 #26 大纲的关系：大纲和面包屑使用 `rove-syntax` 的文档符号（单个打开文件、随编辑增量更新）；L1 是全工作区的磁盘声明表（跨文件、持久化）。两者都基于 tree-sitter C# grammar，但数据各自独立，不互相替代：打开文件里的结构以 #26 为准，跨文件查找以 L1 为准。

验证：`cargo test -p rove-lang-cs`（`src/l1/tests.rs`：partial 三文件聚合、嵌套/泛型重名/record/文件级命名空间、using static 与别名、成员种类与“一律近似”、持久化与增量的扫描计数、缓存损坏回退、BOM/UTF-16 列；`tests/l1_workspace.rs`：工作区扫描 → 符号搜索 → 类型跳转 → 文件事件增量 → 重新打开不重扫，以及中型/大型 Unity 项目往返）。准确率：`cargo run --release -p rove-lang-cs --example nav_check -- --source l1`（#39 测试集的 `type_ref` 分类即验收口径；其余分类需要语义信息，不由 L1 回答）。性能：`cargo xtask bench --filter l1/`（预算 `l1_*`），真实项目数字用 `cargo run --release -p rove-lang-cs --example l1_bench -- <项目>`。

## 24. 编辑核心模型（#46）

位置：`crates/rove-buffer/src/edit/`（核心层，不依赖 gpui；`lib.rs` 只追加 `pub mod edit;`，不改 `contracts.rs` 与 #1 的契约）。
R46（编辑交互与输入法）、R47（当前文件替换）、R48（保存）、R62（全局替换）都不自己改文本，一律经过 `EditSession` / `EditBuffer`。
本 issue 不登记命令、界面字符串，也没有 `SKELETON(...)` 标记（骨架是只读的）。

| 模块 | 内容 |
|---|---|
| `transaction.rs` | `Transaction`：多区间、按位置排序、重叠报 `InvalidInput`；区间都用事务前的坐标，从后往前应用。`EditBuffer`：rope 写时复制，每次事务产出新的 `RopeText` 快照（新 `Version`）与 `AppliedEdit`（`InputEdit` 按应用顺序、逐个 `TextEdit`、`coalesced()` 合并成一个 `TextEdit`、`LineChanges` 按行失效、反向事务）。插入的换行统一为主换行符，其余行尾不动；超大文件 / 有只读原因时返回 `Unsupported` |
| `history.rs` | `EditKind` 与 VS Code 的撤销分组规则（连续输入合并、空格与 VS Code 一致、输入/向左删/向右删之间断开、其他编辑前后断开），另加 `UNDO_GROUP_TIMEOUT`（1 s）；撤销栈只存正反事务与编辑前后的选区。光标撤销栈独立，最多 50 条，文本变化即清空 |
| `selection.rs` | `Selection`（anchor/active 字节偏移 + 期望可见列）、`Selections`（下标 0 为主选区，`normalize` 按 `multiCursorLimit` 截断、按 `multiCursorMergeOverlapping` 合并）；左右按字素簇、上下按可见列（Tab 到制表位、全角两列） |
| `word.rs` | VS Code `cursorWordOperations.ts` 的移植：`move_word`（`cursorWord*`、`cursorWordPart*`、无障碍版本）、`delete_word`（`deleteWord*`、`deleteWordPart*`）、`delete_inside_word`、`word_at` / `select_word`（按 `editor.wordSeparators`）、`word_at_with_pattern`（接 #12 `CompiledLanguageConfiguration::word_at`，语言级 `wordPattern`） |
| `clipboard.rs` | 复制 / 剪切 / 粘贴的文本语义：`emptySelectionClipboard`、多选区分段、`multiCursorPaste` 的 spread/full；系统剪贴板读写在 R46 |
| `rewrite.rs` | 缩进转换（只改前导空白）与换行符统一（一次事务、一次撤销） |
| `session.rs` | `EditSession`：`edit`（任意事务，R47/R62）、`edit_each`（每个选区一处替换）、`type_text`、`delete_left/right`、`move_word`、`delete_word`、`undo/redo`（恢复编辑前 / 后的选区）、`cursor_undo/redo`、`copy/cut/paste`、`convert_indentation`、`change_eol`。时间由调用方传入 |
| `settings.rs` | `editor.wordSeparators`、`wordSegmenterLocales`、`emptySelectionClipboard`、`multiCursorMergeOverlapping`、`multiCursorLimit`、`multiCursorPaste`、`tabSize`、`indentSize` 的登记（已登记则跳过）与 `EditOptions::read` |

接线提示：

- 语法：把 `AppliedEdit::coalesced(&snapshot)` 交给 #94 的 `DocumentHighlighter::analyze` 走增量路径；多区间事务也可逐个使用 `text_edits` / `input_edits`（顺序即应用顺序）。
- 视图：`LineChanges` 的 `old`/`new` 行区间与 `line_delta` 直接用于按行失效（§16 的装饰协议）。
- 撤销 / 重做返回的 `AppliedEdit` 与普通编辑一样需要通知语法与视图。
- `editor.wordSegmenterLocales`（`Intl.Segmenter` 词典分词）由 `rove-buffer` 的 cargo feature `intl-segmenter` 提供（ICU4X `icu_segmenter`，默认关闭，开启后可执行文件约 +2.4 MB，只要中日文词典约 +2.0 MB）；未开启时设置被忽略、等同默认值 `[]`。
- 与 VS Code 的差异：自动闭合对的“一起删除”判定属于 R52，`DeleteWordContext::auto_closing_pair` 只执行结果；撤销分组多了超时规则。

验证：`cargo test -p rove-buffer`——`tests/edit_word_operations.rs`（VS Code `wordOperations` / `wordPartOperations` 用例逐例对拍；`wordSegmenterLocales: 'ja'` 两例需 `--features intl-segmenter`）、
`tests/edit_word_pattern.rs`（Markdown / XML `wordPattern` 与 #12 用例对齐）、`tests/edit_core.rs`（入口“打开 → 编辑 → 撤销 → 变更通知与 InputEdit”、事务、撤销分组四种情况、选区、剪贴板、改写事务、混合换行符字节级比对）、
`tests/edit_proptest.rs`（随机多区间事务 / 输入 / 删除 / 撤销后全部撤销逐字节回到初始文本、全部重做回到撤销前）。性能：`cargo xtask bench --filter edit/ --check-budgets`（预算 `edit_insert_char_100k_lines`，p99 < 100 µs）。

## 25. C# 查询路由与未保存修改的位置映射（#38）

位置：`crates/rove-lang-cs/src/router/`（`lib.rs` 只追加 `pub mod router;`），核心层，不依赖 gpui；`query.rs` 的冻结契约未改动。
`rove-lang-cs` 新增直接依赖 `rove-buffer`（`TextSource` 快照，见 §2 的依赖图；此前已经经 `rove-syntax` 间接依赖）。

| 模块 | 内容 |
|---|---|
| `mod.rs` | `CsRouter`：实现 `CsQueries`（#40、#41、#42、R59 的唯一数据入口）；`SemanticSource`（#36 的快照、状态、过期文件、重建请求；`IndexLifecycle` 已实现，`ManualSemantic` 供测试/基准/`nav_check`）；`navigate_detailed` 逐条精确度 |
| `mapping.rs` | 位置映射纯函数：`EditMap` 是按位置排序、两两不相邻的修改块（旧/新两侧的半开区间），`push(LineEdit)` 按编辑记录累积，`old_to_new` / `new_to_old` 一次二分查找；行级修改判定 `line_modified_old/new` |
| `buffers.rs` | `BufferStates`：打开中的缓冲区相对索引版本的状态（基线内容哈希、当前磁盘哈希、编辑表、文本快照），装配处在打开 / 编辑 / 重新载入 / 保存 / 关闭时调用 |
| `l0.rs` | L0 兜底：当前文件语法树上的作用域查找（局部变量、参数、局部函数、lambda 参数、foreach/for/using/catch 变量、`out var`/模式变量、LINQ 范围变量），最内层优先 |
| `testing.rs` | 测试与基准用的 RVCSIDX v2 写出器（`TestIndex`）与示例项目（`sample_index`），`#[doc(hidden)]` |

路由（`LANGUAGE_INTELLIGENCE` §3.2）：

1. 没有 L2 快照 → 回退；原因按 #36 的状态：构建中/检查中 `IndexNotReady`（界面用 `CsRouter::semantic_status()` 的百分比）、未安装 `ComponentUnavailable`、已禁用 `IndexDisabled`、失败 `Other("index-failed")`。
2. 文件不在索引里 → 回退（`Other("file-not-indexed")`）。
3. 文件状态：打开的缓冲区按“基线哈希 = 索引文件表里的哈希”判定编辑记录有效；“当前磁盘哈希 = 索引哈希”（保存后已重建）时丢弃编辑记录；都不等则**无法映射**。未打开的文件按 #36 的 `is_stale`。版本比对只是查表（索引文件表的哈希 + 缓冲区登记时算好的哈希），查询路径上不算哈希。
4. 查询位置先映射回索引坐标：落在修改块里、或所在行被修改过 → 回退（`ModifiedRegion`）；无法映射 → 回退，并在磁盘内容与索引不同时向 #36 请求重建（`files_changed(Changed)`，由 #35 的常驻进程增量更新；未保存的编辑不请求，索引进程只读磁盘）。同一（文件, 内容哈希）只请求一次。
5. 映射后的位置落在 `#if` 不生效的行区间里（L2 不收集这些代码）→ 回退（`Other("inactive-preprocessor-region")`），不把“没有出现位置”当成精确的空结果。
6. L2 结果逐条反向映射到当前坐标：干净文件原样；脏缓冲区里未修改的行精确平移；落在修改行上的结果尽力平移并**标为近似**；无法映射的文件原样给出并标为近似。不可信的快照（`trusted = false`，配置变化后重建中）所有结果为近似。
7. 回退：L0 先认局部名字（结果“语法”）；否则定义/类型定义问 L1：类型名解析（`resolve_type`），没有再列同名成员（`member_candidates`），结果“近似”；其他查询返回空结果。

**修改区判定粒度：行级。** 旧文本中被任何修改块触及的行都算修改过；整行插入/删除（块的起止都在行首）不触及相邻行，所以“在开头插入 10 行”后其余行全部精确。选行级而非语句级：同一行上改类型、修饰符、实参都可能改变绑定，行级都能覆盖；语句级要语法树，跨行语句会把远处的行一起降级。已知代价：其他行的修改（如新增遮蔽字段的局部变量）可能改变本行绑定而不被发现，属于 `LANGUAGE_INTELLIGENCE` §4 代价第 2 条，保存并重建后恢复。

精确度与契约：`navigate_detailed` 返回 `RoutedTarget { target, precision, reason, role }`，**逐条**标记（`role`：定义 / 引用 / L1 候选 / 元数据）。冻结的 `CsQueries::navigate` 只有批级精确度，按契约取最弱（精确 > 近似 > 语法）。空结果 + `Exact` 表示“确实没有”；空结果 + 非精确表示“不能确定”，`reason` 说明原因。

各查询：`navigate`（定义；类型定义仅对类型符号，否则 `Unsupported`——索引不记录符号的类型；引用含定义处；实现 = 类型的派生类型或成员的实现/重写，传递闭包；派生/基类型；`Overrides` = 本成员重写或实现的成员）、`hover`（索引签名，否则显示名 + XML 文档；L0 回退给出声明文本）、`file_symbols`（当前文本上 #26 的语法大纲，`Syntax`）、`workspace_symbols`（L1，`Syntax`；没有 L1 时 `NotReady`）、`inactive_regions`（L2 的不生效行区间，映射到当前坐标）、`semantic_tokens`（`Unsupported`，属于 R58）。
外部符号（没有源码定义）指向 `Resource::Virtual("rove-cs-metadata:<程序集|文档注释 ID>")`（`router::METADATA_SCHEME`，`metadata_symbol_key` 取回符号键），由 #42 按 #34 的元数据视图打开。

装配（#40 接入时完成；本 PR 不改 UI/app）：`CsRouter::new(lifecycle, buffers).with_l1(l1).with_root(项目根)`；文档打开时 `BufferStates::opened(path, text, hash_bytes(磁盘原始字节))`，外部修改重新载入时 `reloaded`，M3 编辑事务 `edited`，保存 `saved`，关闭 `closed`。所有查询在后台执行器上调用。

验证：`cargo test -p rove-lang-cs router`（`mapping` 属性测试：随机编辑后未修改行逐字节对应、正反映射互逆；路由单元测试覆盖插入 10 行仍精确、修改所在行降级、反向映射混合干净文件与脏缓冲区、四条降级路径、L0 局部名字/lambda/局部函数/遮蔽、逐条精确度）与 `cargo test -p rove-lang-cs --test router`（真实 `IndexLifecycle`：打开 → 精确 → 编辑 → 近似 → 保存并重建 → 精确；组件未安装时 L1/L0 回退）。准确率：`cargo run -p rove-lang-cs --example nav_check -- --source router`，与 `--source l2` 逐例一致。性能：`cargo xtask bench --filter cs_router/`（合成索引，预算 `router_definition_*`）；真实索引端到端 `cs_index/router_*`（预算 `goto_definition` / `find_references`，需要 `--cs-index`）。

## 26. 外部符号元数据视图（#34）

位置：生成在 `tools/rove-indexer-cs/MetadataViews.cs`（登记、生成、XML 文档读取、报告统计），挂接点是 `ProjectBinder.NoteExternal`（绑定时登记）、`Program.cs` 的 `metadata_views` 阶段与 `IndexWriter.MetadataSections`（写段 11、13）。读取在 `crates/rove-lang-cs/src/index.rs`。格式与生成规则见 [CS_INDEX_FORMAT](CS_INDEX_FORMAT.md)「元数据视图」一节。`query.rs` 的冻结契约未改动。#34 没有对应的 `SKELETON(...)` 标记，元数据视图这种文档类型的界面登记由 #42 负责。

流程：

1. 绑定期间，遇到外部符号（不在源码中、程序集不属于任何项目）时，只登记它所在的最外层类型。登记内容是程序集标识、DLL 引用和类型的元数据名，每个项目每个类型只登记一次。同时把类型链上的每一层放进符号表，供读取端沿 container 找到最外层类型。
2. 流水线结束、各项目的编译都已释放后，用缓存的 DLL 引用建一个只含元数据的编译（导入全部成员），按程序集标识解析类型并并行生成视图。同名程序集的不同版本若在共用编译里冲突，会单独建一个编译。此时符号表已完整，所以只为表中存在的成员写位置记录。生成与写索引的前几步（符号编号、出现位置打包）同时进行，写入端到需要视图时才等待。XML 文档文件流式读取，只保留被请求类型的条目。
3. 写索引时按最终符号序号输出视图记录、成员记录与签名，并统计报告。

不中断索引的情况：DLL 被占用时，先以共享方式重读，仍失败则跳过该引用，项目照常编译；缺少 `.xml` 文档时只有签名；XML 损坏或无法读取时同样只有签名；某个类型解析不到或生成失败时，跳过这个视图；视图自检有语法错误时照常写出。以上情况都会在 stderr 打出 `warning:`，并计入报告的 `metadata.warnings`。

| 读取接口（`CsIndex`） | 用途 |
|---|---|
| `metadata_definitions(symbol)` | 外部符号的定义位置（视图序号 + 行、UTF-8 列、长度）；源码符号或没有视图的符号返回空。同名程序集的多个版本各返回一个位置 |
| `metadata_view_text(view)` / `metadata_view_records()` / `metadata_views_of(type_symbol)` / `metadata_view_members(view)` | 视图正文（含 `#region` 文件头，当作 C# 文件显示）、视图列表、某类型的视图、视图内有位置记录的成员 |
| `signature(symbol)` / `xml_doc(symbol)` | 声明签名（段 11）；原始 XML 文档文本（取自视图中成员上方的 `///` 行） |

报告（`--report` 的 `metadata`）：视图数、正文字节数、带文档成员的比例、自检有语法错误的视图数、外部符号引用数与落空数（`external_miss_ratio`）、按符号种类的落空分布、落空最多的 40 个符号、最大的 20 个视图、警告样本。

验证：`cargo test -p rove-lang-cs`（`index_tests.rs` 的段 13 解析与校验）；`cargo test -p rove-lang-cs --lib -- --ignored`（`roslyn_writer_generates_metadata_views`：真实索引进程 + 临时构建的第三方库 1.0/2.0，覆盖缺 `.xml`、DLL 被独占锁住、同名不同版本、嵌套类型、泛型约束、确定性输出）；设置 `ROVE_TEST_UNITY_MEDIUM` 后，`cargo test -p rove-lang-cs --test metadata_views` 在真实项目上抽查 `Transform.position`、`List<T>.Add`、第三方 DLL 和去重。准确率：`nav_check` 的 `external` 分类现在按「符号在视图里有定义位置」判定。


## 27. 当前文件查找条（#21）

位置：`crates/rove-ui/src/find/`。只依赖 #1 的冻结契约（`TextSource`、`Settings`、`Theme`、登记表）、#12 的 JavaScript 正则兼容层（`rove-syntax::language_config::CompiledRegex`）与 #18 的输入框和历史；不修改契约。`lib.rs` 只追加 `pub mod find;`。

| 文件 | 内容 | GPUI |
|---|---|---|
| `engine.rs` | 查询编译（`Searcher`）与按“整行块”流式扫描：`search_all`（全量，带进度回调与匹配数上限）、`find_next`（从某位置起的上一个 / 下一个，含循环、范围）。普通文本走 memchr / 忽略大小写正则，JavaScript 正则复用 #12（含 `
` 跨行匹配）；全词按 `editor.wordSeparators` 判定（VS Code `isValidMatch`）；`
` 在搜索文本里规整为 `
`，列号仍是源行 UTF-8 字节列 | 否 |
| `state.rs` | `FindModel`：查找文本、选项、范围、匹配列表、当前项、校验错误、搜索状态机（每次改动取消上一次、过期结果按代次丢弃）；`CountLabel`（无结果 / 第 N 项共 M 项 / 超过上限 / 已找到 N 处 · 已搜索 P%）；`plan_open`（种子与 `autoFindInSelection` 的决定） | 否 |
| `registry.rs` | 命令（VS Code ID）、界面字符串、颜色键、`FindColors`、`FindAction`（GPUI action） | 是（仅 action） |
| `host.rs` | `FindHost`：包住一个 `CodeView`，在右上角浮出查找条（输入框 + 区分大小写 / 全词 / 正则三个开关 + 计数 + 在选区内查找 + 上一个 / 下一个 / 关闭），后台搜索、装饰层、导航、历史、`editor.find.*`；`FindShared`：窗口内共享的选项、上次查找文本与历史 | 是 |

**接入**：每个编辑器组的每个代码视图一个 `FindHost`（`CodeSlot::find`，`editor_panes.rs` 绘制它而不是裸 `CodeView`）。选项开关、上次的查找文本与历史在窗口内共享（`Workbench::find_shared`），所以切换文件、切换组、再次打开后保持；历史按 `editor.find.history = workspace` 存到 `%APPDATA%\Rove\workspaceStorage\<哈希>\find-history.json`（后台写，`never` 时不记）。命令：`actions.find`、`actions.findWithSelection`、`closeFindWidget`、`editor.action.{next,previous}{Match,SelectionMatch}FindAction`、`toggleFind{CaseSensitive,Regex,WholeWord,InSelection}`，默认键位全部来自 #14 的 VS Code 默认键位表；查找条在 key context 上发布 `findWidgetVisible`、`findInputFocussed` 与 `editorFocus`（VS Code 里 `editorFocus` 包含编辑器内的小组件），输入框的 `↑` / `↓` 走 #18 的 `history.showPrevious` / `showNext`。**不登记**任何替换命令（`replaceOne`、`replaceAll`、`startFindReplaceAction`、`togglePreserveCase`）与 `selectAllMatches`；`ctrl+h` 按下没有任何反应。

**装饰**：匹配是代码视图的一个装饰层（优先级 `priority::FIND_MATCH`）。层里只存匹配列表的引用，`decorate(line)` 在某行进入视口或预取范围时二分定位本行的匹配，所以 1 GB 文件上几十万个匹配也只为可见行生成装饰。当前匹配用 `editor.findMatch*`，其他匹配用 `editor.findMatchHighlight*`（两套键不混用），在选区内查找时范围用 `editor.findRangeHighlight*`。导航到的匹配通过 `set_selection(.., Reveal::CenterIfOutside)` 显示。`editor.find.addExtraSpaceOnTop` 经 `CodeView::set_top_inset` 在第一行之上留出查找条的高度。

**线程与取消**：输入、点击只登记请求；搜索在后台执行器按块（约 1 MiB）进行，每块检查取消并回报进度，结果经代次校验后回到界面。匹配列表完整（未超过 19999 个、文本版本未变）时“上一个 / 下一个”在内存里二分；列表被截断或过期时回到引擎在后台查找。多行正则需要整个范围一次读入，超过 128 MiB 返回 `Unsupported`（界面提示“文件太大”）。

**给 #22 / #48 的扩展点**：

- #22（超大文件）：`engine::search_all` 已按块回报 `Progress{found, scanned_bytes, total_bytes}`，`SearchOutput::partial` 标明行索引未建完只扫了已知行；`FindModel` 的 `SearchState::Searching{found, progress}` 与 `CountLabel::Searching` 已能显示“已找到 N 处 · 已搜索 37%”（文本不到 8 MiB 时不显示进度）。#22 只需换流式入口或在 `FindHost` 里按文件级别隐藏替换按钮，状态机与界面不用改。
- #48（替换）：匹配列表、当前项、范围与 `Searcher` 都在可独立测试的 `FindModel` 里；替换按钮 / 输入框与命令在 `host.rs` 的 `render_widget` 与 `execute` 追加，查找条布局的第一列（展开替换的箭头）留给它，不需要改状态机。

验证：`cargo test -p rove-ui find`（引擎、模型、登记的单元测试）与 `cargo test -p rove-ui find_tests`（`skeleton/workbench/find_tests.rs`，真实 GPUI 窗口、只通过键位驱动：`ctrl+f` 种子、输入即查找与光标跟随、`enter` / `f3` / `ctrl+f3`、循环开关、四个开关与跨文件保持、非法正则、在选区内查找与 `autoFindInSelection`、历史、计数、替换入口不存在）。性能：`cargo xtask bench --filter find/`（10 万行输入一个字符到计数更新，预算 `find_input_char_100k_lines`）与 `cargo xtask bench --filter scroll/find_visible`（查找条打开时的帧预算，对照 `scroll/frame_cpu_cache_hit`）。

## 28. 快速打开与命令面板（#24）

核心层 `rove-fuzzy`（不依赖 gpui）逐条移植 VS Code 的 `fuzzyScorer.ts`（`scoreFuzzy`、`scoreFuzzy2`、
`scoreItemFuzzy`、`compareItemsByFuzzyScore`、`prepareQuery`）及其用到的 `filters.ts` / `comparers.ts` 片段，并把
`fuzzyScorer.test.ts` 的 68 个用例一对一移植成单元测试（1 个校验 VS Code 内部 32 位哈希缓存键的用例因实现不同只留
`#[ignore]` 空壳）。字符串按 UTF-16 码元处理，区间与位置是码元下标，界面用 `utf16_to_byte_ranges` 转成字节区间。
来源与许可见 `THIRD_PARTY_NOTICES.md`。**排序上任何“更合理”的偏离都算 bug**；已知的有意近似只有
`compare_file_names`（无 ICU，数字按数值、忽略大小写）与 `matchesWords` 不做重音折叠。

`rove-ui::quick_access`（见模块文档）：

| 模块 | 内容 | GPUI |
|---|---|---|
| `parse` | 文本开头的 `>` `:` `@` `#` 前缀与 `文件:行:列` / `文件(行,列)` 后缀 | 否 |
| `files` | `FileIndex`（`Workspace::files` 分页读入，按 `FileEvent` 增量更新，`RescanRequired` 重建）、`Search`（按键间前缀剪枝、历史文件加权、最多 512 条）、`Picks` 行与高亮 | 否 |
| `commands` | 命令面板：已实现命令、`WORD_FILTER`、最近使用分组、重复标签显示命令 ID | 否 |
| `lines` | 转到行/列的解析（负数从末尾数起，越界只提示） | 否 |
| `matcher` | `scoreFuzzy2` 作为 #18 的 `Matcher`（`@` 符号与 #26 的 `goto::symbol_picks` 共用） | 否 |
| `registry` | 命令 `workbench.action.quickOpen` / `showCommands` / `gotoLine`、界面字符串、`QuickAccessAction`、`search.quickOpen.*` 与 `workbench.quickOpen.*` 的读取、命令面板不列出的内部命令 | 仅 action |

- **候选与排序**：label = 文件名，description = 相对目录，path = 相对路径（分隔符 `\`，与 VS Code 在 Windows 上一致）。
  排序规则全部来自 `rove-fuzzy`；为了让每次按键在十几万个候选上也守住 16 ms，打分的数据布局与 VS Code 的 JS 实现不同，
  但结果必须逐位相同（`rove-fuzzy` 的测试逐项对照）：
  - 纯 ASCII 的路径（绝大多数）依次追加在一整块连续的字节里（`FileIndex::arena`），字符集合的 64 位掩码放在紧凑的数组里
    （`Hot`，每个候选 32 字节）：查询里出现的字符，候选里必须都有，否则几纳秒内否决；其余候选（非 ASCII 文件名）用预先算好
    UTF-16 与小写形式的 `Target`，走同一套泛型打分代码（`Text` trait：`Wide` / `Ascii`）。
  - 打分只产生 `Summary`（分数与匹配区间的首尾，不分配）；比较器只用得到分数、首尾、label/description/路径长度，
    字符串只在整数部分全部相同时才取（`compare_keys`）。排名确定后才给最终的前 512 名生成完整的高亮区间。
  - 查询是上一次查询的延长（且没有引号）时，只在上一次的匹配集合里重新打分；候选集变化（`FileIndex::generation`）时作废。
  - 候选超过 1.2 万个时在 rayon 线程池里分块并行，各块共享“第 512 名的分数下限”（原子变量），最后归并各块的前 512 名。
  - 前 512 名已经全是 label 级结果（分数 ≥ `LABEL_SCORE_THRESHOLD`）且查询只有一段时，只在 description 里匹配的候选排不进去：
    不再为它们跑矩阵，只用顺序检查判断“有没有匹配”（供下一次按键剪枝）。
  - 只保留最好的 512 条，用二分插入而不是 `sort`（VS Code 的比较规则不保证严格全序，`sort` 可能 panic）。
- **历史**：窗口内最近打开的文件（最近的在前，最多 100 个，不持久化）。空查询只列历史；有查询时历史中匹配的排在前面，
  `search.quickOpen.history.filterSortOrder` 为 `default` 时按相关度、`recency` 时按最近打开；文件结果不重复历史里的文件；
  `search.quickOpen.includeHistory = false` 时没有历史分组。
- **命令面板**：只列已实现的命令（`keys::is_implemented`，即有模块 action 或命令表里有处理器），列表/快速输入/通知
  等内部导航命令不列出（`registry::hidden_in_palette`）；键位取 `KeybindingTable::preferred_keys`（用户自定义优先，
  否则第一条默认键位）；分“最近使用”和“其他命令”两组。运行命令：关闭浮层、焦点还给编辑器后派发该命令的 GPUI action
  （没有 action 的命令派发 `RunCommand`）。
- **宿主**（`skeleton/workbench/quick_access_host.rs`）：创建 #18 的 `QuickInputView`（`filter = None`，自己过滤与排序），
  `QuickInputTextChanged` 同步重算列表；行的高亮与右侧键位文本经 #18 新增的 `RowDecor`（`ActiveInput::decor`）传入；
  `Ctrl+Enter`（`quickInput.accept` 带 `ctrl`）在旁边打开；工作区文件列表在扫描完成后后台建索引，未就绪时立即可用并在
  列表底部显示“正在加载文件列表…”；`workbench.quickOpen.closeOnFocusLost` 与 `preserveInput` 按设置读取。
  打开与定位经 #17 的 `OpenDocuments`（`OpenSource::QuickOpen` / `GoToLine`）。命令中心点击调用
  `workbench.action.quickOpen`（#16 的 `run_command` → `run_external_command`）。
- **没有的**：`#` 工作区符号（R59）只显示“尚不可用”并提示 `Ctrl+P`；命令的 `when`/前置条件没有数据模型，命令面板不按
  `when` 过滤；最近打开的文件不持久化；`@` 在 `文件名@符号` 的组合写法不支持。

验证：`cargo test -p rove-fuzzy`（移植的 VS Code 用例，另有与稠密矩阵参考实现、`Target` 路径的随机对照）；`cargo test -p rove-ui quick_access`（前缀解析、转到行、命令面板、
索引增量、剪枝与完整扫描等价、并行与单线程等价、优化后的整条流水线与朴素 VS Code 实现等价、历史）；`skeleton/workbench/quick_access_tests.rs`（真实 GPUI 窗口：`Ctrl+P` → 输入 → 选中
→ 文档已打开并定位、四个前缀、命令面板与键位、`Ctrl+Enter`、`Esc`、命令中心入口、设置）。性能：
`cargo xtask bench --filter quickopen/`（预算 `quickopen_*`）。

## 29. C# 增量索引：常驻模式、单文件重绑、覆盖层（#35）

位置：索引进程 `tools/rove-indexer-cs/`（`Server.cs` 协议与调度、`Incremental.cs` 引擎、`Impact.cs` 影响分析与声明指纹、`Overlay.cs` 覆盖层段与日志、`BaseIndex.cs` 读基础索引、`SelfTest.cs`）；Rust 侧 `crates/rove-lang-cs/src/overlay.rs`（日志读取、合并视图 `L2View`，`lib.rs` 只追加 `pub mod overlay;`）与 `lifecycle/resident.rs`（拉起常驻进程、消息解析）。格式与协议见 [CS_INDEX_FORMAT](CS_INDEX_FORMAT.md)「覆盖层」「常驻模式与协议」。`query.rs` 的冻结契约未改动；本 issue 没有 `SKELETON(...)` 标记。

三方边界（issue #35 的约定）：本 issue 定消息内容；#36 定何时启停（`IndexLifecycle` 在第一次源文件变化时拉起，项目关闭、全量重建、索引不可信时关闭，进程空闲自行退出）；#43 定版本如何协商（第一条消息 `hello` 带 `protocol`、`protocol_min` 与 `indexer_version`，按协议区间取交集，规则见 §34）。

| 环节 | 内容 |
|---|---|
| 转发 | `files_changed` 标记过期并把源文件路径交给后台线程；后台线程按需拉起 `rove-indexer-cs --serve`（`IndexBuilder::incremental()`，`ProcessBuilder::default()` 开启，测试替身默认关闭），发 `changed`，状态 `Updating { files }` |
| 重绑 | 进程按项目预热编译（依赖项目用元数据镜像），`ReplaceSyntaxTree` 只重绑变化的文件，结果追加为覆盖层段，回复段位置 |
| 合并 | 生命周期读出日志的新段，`Overlay::build` 与基础索引合并，发布新的 `IndexSnapshot`（同一世代共享基础映射，`IndexSnapshot::view()` 是查询视图），清除已满足的过期标记 |
| 影响分析 | 声明签名或可见性变了 → 引用者进后台队列；新名字 → 可见项目内文本查找；后台段到达时同样合并发布 |
| 阈值 | 覆盖层 ≥ 256 个文件或日志 ≥ 32 MiB（`LifecycleConfig::overlay`）→ 后台全量重建，旧索引 + 覆盖层照常可查询；重建期间的变化排队，新世代就绪后转发 |
| 结构变化 | `.csproj` / `.asmdef` / `.asmref`：不做增量，全量重建（进程收到时回复 `rebuild_required`） |
| 打开项目 | 读出当前世代配套的日志（上次会话落盘的增量）；抽样比对按覆盖层的哈希，不一致的文件转发给常驻进程 |

#38 的接线：`IndexHolder::view()` 返回 `L2View`（基础索引 + 覆盖层），路由的全部 L2 查询改走视图；文件在索引中的版本取 `L2View::file_hash`（覆盖层重绑过的文件是覆盖层里的哈希），所以保存并增量更新后缓冲区直接回到“干净”。`SemanticSource::request_update` 经 `files_changed(Changed)` 进入增量更新。

验证：`cargo test -p rove-lang-cs --lib overlay`（合并规则：替换文件、删除不留悬空位置、新文件、后写优先、关系归属、局部键、日志截断与校验）；`cargo test -p rove-lang-cs --test incremental -- --nocapture`（真实常驻进程：生命周期“启动 → 通知变更 → 查询得到新结果 → 改签名后后台更新引用 → 新增 / 重命名 / 删除 → `.csproj` 全量重建 → 关闭”，协议层的被独占文件重试、空闲修剪与退出、全量索引消息、`--self-test`；需要先 `dotnet build tools/rove-indexer-cs -c Release`，否则跳过）；同一测试文件里的 `overlay_limit_triggers_background_rebuild`（假常驻进程：覆盖层达到阈值触发后台全量重建，重建期间旧索引 + 覆盖层可查询，不依赖索引进程）。合并后的查询复杂度：`cargo xtask bench --filter cs_overlay/`（段数 0 → 1024）。真实项目的一致性与延迟：`cargo run --release -p rove-lang-cs --example incr_check`（见 AGENTS.md §8.3）。

## 30. 保存与外部修改（#49）

核心在 `crates/rove-buffer/src/save/`（不依赖 gpui；`lib.rs` 只追加 `pub mod save;`，不改 #1 的契约），界面层在
`crates/rove-ui/src/save/`（`lib.rs` 只追加 `pub mod save;`）。#46 的 `EditSession` 只追加了三个方法：`history_marker`
（撤销单元身份，用于脏标记）、`edit_joined`（保存时的变换并入最后一个撤销单元）、`replace_from_disk`（重载 / 还原，
只替换首尾公共部分之间的区间，选区自然保留，可撤销）。`rove-buffer` 新增 Windows 目标依赖 `windows-sys`
（`Win32_Storage_FileSystem`：`ReplaceFileW`、临时文件的删除处置）。

| 模块 | 内容 |
|---|---|
| `save/encode.rs` | 文本 → 磁盘字节：BOM + 原编码（UTF-16 自编码，`encoding_rs` 没有 UTF-16 编码器）；无法编码字符的计数与 `?` 替换 |
| `save/write.rs` | `write_atomic`：同目录临时文件（创建即设“关闭即删除”，强杀后由系统清理）→ 刷盘 → `ReplaceFileW`（保留属性、创建时间、ACL）/ 目标不存在时重命名；共享冲突、锁冲突、映射中、无法移除被替换文件 → `Busy` 并自动重试一次；只读属性 → `ReadOnly`（可清除后写）；不提供提权写入；故障注入点 `FailPoint` |
| `save/stamp.rs` | `DiskStamp`（长度、修改时间、FNV-1a 内容哈希）：长度或时间变了才读内容，以哈希为准 |
| `save/transforms.rs` | 保存时的变换（VS Code 保存参与者的顺序）：去行尾空白（只认空格 / Tab，可跳过字符串 / 正则与指定行）→ 去末尾多余空行 → 补末尾换行 |
| `save/watch.rs` | `SelfWrites`（路径 + 时间窗 + 内容哈希三者结合判定自写）、`ChangeCoalescer`（按路径去抖，安静期 100 ms、最长等待 1 s） |
| `save/settings.rs` | `files.eol`、`insertFinalNewline`、`trimFinalNewlines`、`trimTrailingWhitespace(InRegexAndStrings)`、`saveConflictResolution`、`autoSave` / `autoSaveDelay` / `autoSaveWorkspaceFilesOnly` / `autoSaveWhenNoErrors` 的登记（已登记则跳过）与 `SaveOptions::read`；`auto_save_due` 给出 VS Code 的自动保存判定（没有问题提供方时视为无错误） |
| `save/file.rs` | `TextFile`：编辑会话 + 路径（或未命名名字）+ 编码 / BOM + 保存点 + 磁盘标记 + 删除 / 外部修改状态。保存三步：`begin_save`（UI 线程：变换、编码）→ `SaveJob::run`（后台：冲突检测 + 原子写入）→ `complete_save`；`disk_changed` / `revert` / `keep_local_changes`；`untitled` 按 `files.encoding` / `files.eol` 新建 |
| `rove-ui/src/save/registry.rs` | 命令（`workbench.action.files.save` / `saveAs` / `saveAll`（含 `ctrl+k s` 的短 ID `saveAll`）/ `saveFiles` / `saveWithoutFormatting`、`workbench.files.action.saveAllInGroup` / `revertLocalChanges` / `acceptLocalChanges`、`workbench.action.files.revert`、`workbench.action.editor.changeEOL`）、界面字符串、反馈键与动作 ID |
| `rove-ui/src/save/controller.rs` | `SaveController`（不依赖 GPUI）：`SaveOutcome` / `DiskChange` → #16 的五级反馈，反馈按钮的后续动作，关闭确认（`set_hot_exit` 是 #50 的开关点），外部修改的合并、自写去重与提示条 |

**语义**：

- **原样写回**：文本快照保留原始行终止符，混合换行保存后仍混合；编码与 BOM 按打开时写回（合法输入的解码再编码逐字节可逆），保存路径上没有格式化。
- **脏标记**：当前撤销单元身份 ≠ 保存点即为已修改；撤销回保存点重新变为未修改。未命名文档有内容或被编辑过即为已修改。
- **保存冲突**：保存前比较磁盘与最后一次已知的标记；`files.saveConflictResolution = overwriteFileOnDisk` 或选择“覆盖”时不检测。文件已被删除时直接新建写入。
- **外部修改**：事件先按路径合并，再排除自写；磁盘内容等于已知内容 → 无事发生；本地无修改 → 静默重载（`replace_from_disk`，选区映射保持，编码与换行符重新识别）；本地有修改 → 提示条（只弹一次，不堆积）；文件不存在 → “已删除”，内容保留。“保留本地修改”以这次外部修改为新基准，之后保存不再报冲突。
- **反馈**（`UX_SPEC` §6.1 / §6.4）：占用、只读属性、保存冲突、其他写入失败是第 4 级通知，键带文档 ID，同因再次触发只更新次数；“比较”在对比视图（P2）前隐藏；无法编码的字符与关闭确认是第 5 级模态；外部修改是第 3 级提示条（不可“不再提示”）。

**装配**：设置经 `app_settings` 登记（`rove-buffer-save`）；命令经 `keys::default_commands` 登记、字符串经
`appearance::strings` 登记。当前代码视图只读，窗口里没有可保存的修改，所以命令只登记、**暂不**接入
`keys::action_for`（不进命令面板）。#47 接入可编辑视图时：每个可编辑文档建一个 `TextFile` 交给窗口的
`SaveController`；`SaveAction` 分发给控制器；`Effect::Saved` 时清除标签脏标记并调用 #38 的
`BufferStates::saved(path, hash)`（保存事件的唯一出口，#40 负责装配）；`Effect::Reloaded` 时把新快照交给代码视图与
语法（`BufferStates::reloaded`）；工作区 `FileEvent` 经 `note_change` → `due` → 后台读盘 → `apply_disk`。
#50（热退出）用 `set_hot_exit` 跳过关闭确认，`SaveJob::bytes` 可用于备份；#52、#62 经 `TextFile::session_mut`
编辑、经 `write_atomic` / `TextFile::save` 写盘。

验证：`cargo test -p rove-buffer --test save`（入口集成测试：五类文件编码往返字节级比对、混合换行与统一换行一次撤销、
撤销回保存点、无法编码字符三条路径、强杀写入中的子进程后原文件完整且无临时残留、持有独占句柄的辅助进程（自动重试一次后
报告 / 重试间隔内释放则静默成功 / 释放后“重试”成功）、只读属性与“覆盖”、保存冲突三条路径、静默重载保持光标、提示条两个选项、
删除后保存重建、外部脚本连续改写 100 次、连续保存 20 次零误报、保存时变换参数化且与用户改动同一撤销单元、字符串内行尾空白、
自动保存判定、未命名文档、另存为）；`cargo test -p rove-buffer --lib save`（编码、写入、变换、去重与去抖的单元测试）；
`cargo test -p rove-ui --lib save::`（控制器：各反馈的级别与按钮、按钮后续动作、关闭确认三条路径与热退出开关、提示条不堆积、
自写零误报、“编辑 → 保存 → 外部修改 → 重载”）。

## 31. C# 导航交互（#40）

位置：`crates/rove-ui/src/nav/`（`lib.rs` 只追加 `pub mod nav;`）与窗口装配 `crates/rove-ui/src/skeleton/workbench/nav_host.rs`。只消费 #1 的冻结契约（`CsQueries` 经 #38 的 `CsRouter`、`OpenDocuments`、`FeedbackService`、登记表）与 #19 的代码视图接口，不修改契约；GPUI 类型只在 `rove-ui`。

| 文件 | 内容 | GPUI |
|---|---|---|
| `nav/registry.rs` | 命令（VS Code ID：`revealDefinition`、`revealDefinitionAside`、`goToDeclaration`、`goToImplementation`、`gotoNextSymbolFromResult(.cancel)`、`showHover`、8 个卡片内滚动命令、两个悬停详细程度命令、`wordHighlight.next/prev/trigger`）、界面字符串、颜色键（`editorHoverWidget.*`、`editorLink.activeForeground`）、上下文键名、`NavAction` | 仅 action |
| `nav/settings.rs` | `editor.gotoLocation.multiple*`（含已弃用的 `multiple`）、`editor.definitionLinkOpensInPeek`、`editor.hover.*`、`editor.maxTokenizationLineLength`、`editor.occurrencesHighlight*` 按 `csharp` 语言作用域读取 | 否 |
| `nav/model.rs` | 查询结果 → 呈现方式（`plan`：空状态 / 只有元数据 / 单个候选 / 多候选，逐条精确度取最弱）、速览标题标记、符号导航模式、近似提示去重、语义同名高亮区间（按行二分、`F7` 循环） | 否 |
| `nav/hover.rs` | 卡片内容（签名、XML 文档注释 → 段落、定义预览、近似标记、长行提示）与卡片内滚动 | 否 |
| `nav/semantic.rs` | `LiveSemantic`（可替换的 `SemanticSource`）、`OpenBuffers`（打开中的 C# 文档 → `BufferStates` 的打开 / 重新载入 / 关闭） | 否 |
| `nav/view.rs` | 悬停卡片的绘制 | 是 |
| `skeleton/workbench/nav_host.rs` | 窗口装配：命令、后台查询与过期丢弃、跳转、符号导航模式、悬停与 `Ctrl+悬停`、装饰层、上下文键、L1 构建与增量、`F12` 计时 | 是 |

**#38 的装配**（§25“装配”一段）：`CsRouter::new(LiveSemantic, BufferStates).with_root(项目根)` 每个窗口建一次；#36 的生命周期换掉时（`attach_index`）只换 `LiveSemantic` 里的来源（`IndexStatusController` 改为持有 `Arc<IndexLifecycle>`，`shared()` 取出）；应用入口在工作区扫描完成后在后台建 L1（`rove.index.enabled` 时写到 #36 缓存目录的 `l1/`），文件事件转成 `L1Change` 增量更新，然后 `with_l1` 重建路由（`BufferStates` 沿用）。`BufferStates` 的打开 / 重新载入 / 关闭由 `sync_nav` 每帧比较打开中的 C# 标签得出，读盘算原始字节的哈希在后台；**编辑（M3 编辑事务）与保存（#49）的装配处调用 `Workbench::cs_buffers()` 的 `edited` / `saved`**。所有查询在后台执行器上执行，回到窗口时按请求代次与文档版本丢弃过期结果。

**代码视图的追加接口**（#19 模块内只追加，不改既有行为）：事件 `CodeViewEvent::{PointerMoved, PointerLeft, LinkClicked}`（文本区上的指针位置与 Ctrl、`Ctrl+点击`，修饰键变化时在原位置重发）、`set_link_cursor`（手形光标）、`set_semantic_word_highlights(Option<bool>)`（`Some(has)`：停用 #20 的文本单词高亮，`hasWordHighlights` 取 `has`；`None`：回到文本模式；选区相似匹配不受影响）、`word_range_at`（与同名高亮同一套分词规则）。

**行为要点**

- **上下文键**：焦点组活动标签是 C# 时在编辑器区域发布 `editorHasDefinitionProvider` / `DeclarationProvider` / `ImplementationProvider` / `ReferenceProvider` / `HoverProvider`（D2：其他语言一个都不发布，`F12` 的 when 不成立，命令直接执行也什么都不做）；符号导航模式发布 `hasSymbols`；卡片可见发布 `editorHoverVisible`，卡片有焦点时卡片自己的 key context 是 `editorHoverFocused`。
- **跳转**一律经打开文档服务（`OpenSource::CodeNavigation`，目标组 = 发起的组，`revealDefinitionAside` 为 `Beside`），插入符放到符号名起点，目标行 `Reveal::CenterIfOutside`；跳转前的位置由 #17 自动压入导航历史（跨文件、跨标签、跨分屏，目标标签已关闭时后退会重新打开）。
- **多候选**按 `editor.gotoLocation.multiple*`：`peek` 交给 `Workbench::open_peek(PeekRequest)`——**#41 的接入点**，合并前只记录请求并返回 false，调用方按 `goto` 处理（#134 已接上，见 §35）；`goto` 跳到第一个并进入符号导航模式（`f12` 循环、`escape` 退出、状态栏“第 N / 共 M 个符号”）；`gotoAndPeek` 先跳再交给速览。速览标题的近似标记由 `PeekRequest::annotation` 传入（精确为 `None`，近似为“近似 · N 个候选”；查找引用用 `model::references_annotation`，近似为“近似”）。`Ctrl+点击` 在 `editor.definitionLinkOpensInPeek` 为真时总是交给速览。
- **外部符号**（只有 `rove-cs-metadata:` 目标）：`Workbench::open_metadata`——**#42 的接入点**，合并前按 UX_SPEC §4.1 末行在状态栏提示“未找到 `X` 的元数据”，不打开空标签（#42 已接上，见 §35）。
- **近似结果**（UX_SPEC §2.2 逐行）：单个候选跳转 + 状态栏 3 秒“近似结果：语义索引未就绪”（同一目标位置每次会话只提示一次）；多候选与引用的速览标题标记；悬停底部灰色“近似结果”；同名高亮退化为 #20 的文本高亮且无提示；精确结果没有任何标记。空状态在光标处提示“未找到 `X` 的定义 / 声明 / 实现”，近似时追加“（语义索引构建中）”。查询失败（未就绪、版本过期、资源不支持）按“近似的空结果”处理。
- **悬停**：鼠标停在 C# 标识符上 `editor.hover.delay` 后在后台查询（`hover_detailed`），300 ms 内没有结果显示“正在加载…”；`editor.hover.enabled = off` 时只有 `ctrl+k ctrl+i`；`sticky` 时离开单词等 `hidingDelay`、指针在卡片上不收起，否则立即收起；`above` 决定先上后下（`gpui::anchored` 放不下时翻转）；长行（超过 `editor.maxTokenizationLineLength`）在 `showLongLineWarning` 时追加提示。第二次 `ctrl+k ctrl+i` 把焦点移进卡片，方向键 / 翻页键 / `ctrl+home|end` 滚动卡片，`escape` 收起并把焦点还给编辑器。悬停详细程度只有一级（两个命令只登记）。
- **`Ctrl+悬停`**：标识符加 `editorLink.activeForeground` 下划线与前景色、手形光标，并在 `delay` 后弹出定义预览（定义处开始最多 8 行源码，打开中的缓冲区优先，否则读盘）；松开 Ctrl（`on_modifiers_changed`）立即收起。
- **语义同名高亮**：只在 C#、`SizeClass::Normal` 的文档上接管；光标仍在当前高亮的某一处时不重查，否则按 `editor.occurrencesHighlightDelay` 去抖后在后台查询引用、只保留本文件的结果；装饰层优先级 `SYMBOL_HIGHLIGHT`，结果变化时只让新旧两组涉及的行失效（超过 64 行才整层失效），绘制时按行二分。索引的出现位置只区分定义与引用，所以**声明处按“写”（`wordHighlightStrong*`）、其余按“读”（`wordHighlight*`）**；真正的读 / 写区分要等索引记录访问方式。光标离开旧符号时旧高亮立即消失，模式（语义 / 文本）保持到新结果到达，不闪烁；索引快照代次或状态变化时静默重查（`nav_index_changed`）。`F7` / `Shift+F7` 在语义区间（文本模式下按整词扫描本文件）之间循环；`wordHighlight.trigger` 在 `occurrencesHighlight = off` 时也高亮一次。概览标尺的 `editorOverviewRuler.wordHighlight*` 由 R55 绘制，位置集合在装饰层状态里。

**性能**：`F12` → 目标行画面完成 = 后台查询（索引侧，预算 `goto_definition` / `router_definition_*`）+ 打开文档服务对已打开文件的切换与定位 + 一帧绘制。`cargo xtask bench --filter nav/`（`xtask/benches/nav.rs`，真实窗口、2 万行目标文件、200 个不同目标区域）输出 `nav/f12_to_paint`（预算 `goto_definition_e2e`，p99 < 50 ms）、`nav/f12_to_frame`、`nav/f12_query`。`ROVE_NAV_TRACE=1` 运行 release 版时每次跳转在 stdout 输出 `NAV_F12_TO_PAINT_MS <总> QUERY_MS <查询>`。同名高亮不进入绘制关键路径（查询在后台，装饰按行缓存）。

验证：`cargo test -p rove-ui nav`（`nav::*` 单元测试：登记、设置逐项、呈现决策、符号导航、去重、高亮区间、XML 文档、语义来源与缓冲区同步；`skeleton::workbench::nav_tests` 入口集成测试：真实 GPUI 窗口 + 用 #38 的测试写出器现场生成的 RVCSIDX v2 索引——`F12` → 文档已打开并定位、历史后退 / 前进 / 重新打开已关闭的标签、`ctrl+k f12` 跨分屏、`ctrl+f12` 在抽象方法 / 虚方法 / 接口上、`partial` 多定义与 `hasSymbols`、`Ctrl+悬停` 下划线与预览、`Ctrl+点击`、`definitionLinkOpensInPeek`、悬停的 `delay` / `enabled` / `sticky` 与卡片焦点、`Init` / `Update` / `Dispose` / `Get` / `Value` 的语义高亮数量与 `F7`、近似呈现的五种场景、空状态、非 C# 文件、缓冲区登记）。真实项目上的界面抽查：`cargo test -p rove-ui ui_nav_cases -- --ignored --nocapture`（`skeleton/workbench/nav_cases_tests.rs`：需要 `ROVE_TEST_UNITY_MEDIUM` 与中型项目的 L2 索引 `target/nav-check/medium.idx` 或 `ROVE_NAV_INDEX_MEDIUM`，缺任何一个都跳过；从 #39 的 `tests/nav-cases/medium-*.toml` 按 var、重载、扩展方法、泛型、partial、接口实现、Unity API 每类取 5 个，在真实工作台里打开文件、放插入符、按 `f12`，读回落点并与期望定义比较，结果写到 `target/nav-check/ui-nav-cases-medium.md`）；手工核对用 `cargo run -p rove-app -- <ROVE_TEST_UNITY_MEDIUM>`。

## 32. 全局搜索（#25）

引擎在核心层 `crates/rove-search`（不依赖 gpui；**不读取 `.gitignore`**，运行时不联网），视图在 `crates/rove-ui/src/search/`。
只依赖冻结契约（`Settings`、`Theme`、登记表、#17 的 `OpenDocuments`、#18 的 `ListModel`/`ListView`/`TextInput`）与
#15 的 `RuleSet` / 设置键、#7 的编码识别（`rove_buffer::open_bytes`），不修改契约。

| 位置 | 内容 | GPUI |
|---|---|---|
| `rove-search::query` | `SearchQuery`；匹配器编译（`grep-regex`：字面量 / 正则、全词、大小写、`search.smartCase`，正则含 `
` 时多行）；“包含 / 排除的文件”按 VS Code `expandGlobalGlob` 展开（`./` 相对根，其余加 `**/`，花括号内逗号不分隔） | 否 |
| `rove-search::settings` | `search.*` 登记（与 `register_builtin` 默认值一致）；`SearchConfig`（引擎）、`ViewSettings`（视图）。`search.useIgnoreFiles` / `useParentIgnoreFiles` / `useGlobalIgnoreFiles` 不登记、不读取 | 否 |
| `rove-search::engine` | `search(root, query, config, cancel, on_batch) -> SearchSummary` | 否 |
| `rove-ui::search::model` | 结果模型：按文件分组、`search.sortOrder`、`search.collapseResults`、`search.action.remove`；展平快照 `ResultRows` 实现 `ListModel` | 否 |
| `rove-ui::search::registry` | `search.action.*`、`toggleSearch*`、`search.focus.*` 命令、字符串、颜色键、`workbench.view.search` 容器里的结果小节、`SearchAction` | 仅 action |
| `rove-ui::search::view` | `SearchView`：查询框 + 三个开关、包含 / 排除、使用排除设置、进度条、摘要 / 上限 / 空状态、虚拟化结果列表 | 是 |
| `skeleton/workbench/search_host.rs` | 窗口装配：绘制小节、转发 `SearchAction`、经 `OpenDocuments`（`OpenSource::Search`，带选区）打开、发布 `searchViewletVisible` / `hasSearchResult`、`findInFiles` 后把焦点交给查询框并用编辑器单行选中内容作种子 | 是 |

**文件集合**：与 #15 的扫描逐项相同的 walker 设置（全部忽略文件开关关闭）+ 同一套 `RuleSet` 合并语义：`files.exclude` 叠在
`rove.files.extraExclude` 之上（`RuleSet::merged`）再加 `search.exclude`；“使用排除设置”关闭时三者都不生效，视图里的
“排除的文件”始终生效。搜索自己遍历而不读工作区快照，因为 `search.followSymlinks`（默认 true）与“不使用排除设置”都需要快照
之外的文件；剪枝时顺带记录“是否有目录只因 `rove.files.extraExclude` 被排除”（`SearchSummary::excluded_by_rove_defaults`），
空状态据此显示“已排除 Unity 生成的目录。”与“包含排除的文件再搜索一次”（关闭“使用排除设置”重搜）。

**并行、流式与取消**：`ignore` 的并行 walker 在工作线程里直接搜索（`search.ripgrep.maxThreads`，0 自动）；每个有匹配的文件整体
送回调用线程，第一批立即交出、之后约 30 ms 一批；视图经通道收到后按代次丢弃过期批次。取消令牌检查点：每个目录项、每个文件、
每 256 KiB 块、每处匹配，`search` 返回时线程已全部结束（`std::thread::scope`），测试断言取消到返回 < 50 ms。
`search.maxResults` 按匹配处数原子扣减，恰好截断并报告 `limit_hit`。

**编码与二进制**：先读开头 8 KiB，含 NUL（且无 UTF-16 BOM）即按二进制跳过、不读剩余内容；UTF-8 BOM 去掉、UTF-16 BOM 转码；
纯 ASCII 字面量查询先在原始字节上判定有无匹配（ASCII 兼容编码解码后不会凭空出现 ASCII 匹配），没有就结束；
否则合法 UTF-8 原样搜索，非 UTF-8 交给 #7 的编码识别（`files.autoGuessEncoding`、候选 GBK），识别失败按原始字节搜索。
大于 16 MiB 的文件内存映射、不做 UTF-8 校验。结果的行号从 0 开始，区间是解码后该行的 UTF-8 字节列（与编辑器坐标一致）；
预览去掉缩进，超长行只保留第一处匹配附近约 1000 字节。

**视图行为**：`search.searchOnType` + `searchOnTypeDebouncePeriod` 防抖，回车立即搜索，切换开关立即重搜；非法正则不发起搜索、
输入框标红。文件行单击 / 回车展开折叠，匹配行单击预览打开（保留焦点）、回车固定打开、`ctrl+enter` 在侧边打开；`f4` / `shift+f4`
循环（必要时展开折叠的文件）并保留焦点预览；`delete` 消除；左右方向键在展平列表上换成文件行展开 / 折叠（`capture_action`）。
上下文键：`searchViewletFocus`、`searchState`、`inputBoxFocus`、`searchInputBoxFocus`、`fileMatchOrMatchFocus`、`fileMatchFocus`、
`matchFocus`、`firstMatchFocus`、`fileMatchOrFolderMatchWithResourceFocus`、`folderMatchWithResourceFocus`（列表模式没有文件夹节点，
文件行上也发布，`shift+alt+f` 限制到它所在的文件夹）。

**有意的差异与未做**：替换（R61）、搜索编辑器（P2）、`searchWithAI` 不登记；“查看排除设置”打开共用的 `settings.json`
（Rove 没有设置界面）；`search.sortOrder = modified` 按 `default` 排（结果不带修改时间）；`search.decorations.colors` 没有
可着色的来源（没有 SCM / 问题装饰）；包含 / 排除输入框总是显示（VS Code 默认收在 `…` 后）；多行匹配只在起始行显示与选中。

验证：`cargo test -p rove-search`（`tests/search_it.rs`：分组与列、正则 / 全词 / 大小写、`search.smartCase`、包含 / 排除 glob、
排除设置合并与“只因 Rove 默认排除”、`.gitignore` / `.git/info/exclude` / `.ignore` 不生效、`search.maxResults` 截断、GBK / UTF-16 /
BOM / 二进制、分块行号、多行正则、大型 Unity 项目计时（release））与 `tests/cancellation.rs`（独立二进制，取消到全部线程结束 < 50 ms）；`cargo test -p rove-ui search`（模型、登记、
`skeleton/workbench/search_tests.rs` 真实 GPUI 窗口：`Ctrl+Shift+F` → 输入 → 回车 → 分组与首条位置、`f4` 打开并选中、`.md` 打开源码、
`alt+c/w/r`、`delete`、空状态与再搜索一次）。性能：`cargo xtask bench --filter search/`（预算 `search_*`）。

## 33. 速览窗口（#41）

位置：`crates/rove-ui/src/peek/`（`lib.rs` 只追加 `pub mod peek;`）；窗口装配在 `skeleton/workbench/peek_host.rs`。不修改 #1 的契约；GPUI 类型只出现在 `widget.rs`、`zone.rs` 与 `registry.rs` 的 action。

| 模块 | 内容 | GPUI |
|---|---|---|
| `model.rs` | `PeekResults`：一次排序分组得到“文件 → 计数 + 偏移”的索引（4.7 万条只排一次，不为每条结果建界面节点）；展开规则 `initially_expanded`（总数 ≤ 100 全部展开，否则只展开第一个文件）；结果间导航（跨文件循环）；标题与近似标记（精确无标记、多候选定义“近似 · N 个候选”、其他“近似”，`Marker::Custom` 由 #40 传入） | 否 |
| `source.rs` | `PeekQueries`（#38 `CsRouter::navigate_detailed` / `hover_detailed`；冻结契约 `CsQueries` 经 `BatchQueries` 适配）、**能力开关** `Availability`（实现关系只在 L2 就绪时可用，`router_availability` 是唯一判定处）、后台流水线 `run`（查询 → 分组 → 读初始展开文件的片段 → 外部符号的悬停签名） | 否 |
| `snippets.rs` | 结果行的单行片段：只为**展开的文件**读取，文本经 `Documents`（已打开的文档复用同一份快照） | 否 |
| `results_tree.rs` | 结果树数据源（#18 `TreeSource`）：文件节点直接来自分组索引；结果行只在文件展开时生成，片段未读好时返回 `Pending`，宿主后台读完后 `set_children` | 否 |
| `external.rs` | 外部符号片段的登记点 `ExternalSnippets`：保底实现只显示 `外部符号：<签名>`；#42 合并后调用 `register` 注册元数据视图的片段提供方，速览代码不需要改动 | 否 |
| `registry.rs` | 12 条命令（VS Code ID）、界面字符串、16 个 `peekView*` 颜色键（粘性滚动两个键只登记，随 R54 接入）、上下文键名、`PeekSettings`（`editor.stablePeek`、`editor.peekWidgetDefaultFocus`）、一条附加默认键位、`PeekAction` | 仅 action |
| `widget.rs` | `PeekWidget`：标题栏、左侧只读预览（一个完整的 `CodeView`：同一套整形缓存、语法着色与主题，背景与行号栏经主题适配映射到 `peekViewEditor*`）、右侧结果树（`ListView` + `TreeModel`，选择色映射到 `peekViewResult.*`）、底边与中缝拖动、窗口栈（嵌套速览） | 是 |
| `zone.rs` | `zone_overlay`：在代码视图 prepaint 之后读取同一帧的嵌入区域位置，把元素布局并摆进去（滚动无一帧滞后，坐标与折叠共用代码视图的映射） | 是 |

- **挂载**：窗口不是浮层，而是 #19 的“行下方嵌入区域”：宿主 `set_view_zone(插入符行, 18 行高)`，再用 `zone_overlay` 把窗口作为代码视图之后的兄弟元素加入同一个外框（`editor_panes.rs` 的 `code_viewport` 结果经 `peek_viewport` 包一层）；`peek::reveal_zone` 滚动使锚定行与整个窗口尽量可见。嵌套速览在上一层预览的嵌入区域里再挂一层。
- **上下文键**：宿主外框发布 `referenceSearchVisible`（窗口打开时）与 `editorHas{Reference,Definition,Implementation}Provider`（C# 文档且有查询服务时）；窗口根发布 `referenceSearchVisible`；预览外框发布 `inReferenceSearchEditor`、`isInEmbeddedEditor`；`config.editor.stablePeek` 发布到全局上下文键（四条 `closeReferenceSearch` 键位共用一处判定，窗口内的“能否关闭”同样只有 `PeekSettings::can_close_by_escape` 一处）。
- **键位**：全部来自 #14 生成的 VS Code 默认键位表（导航一节 14 行逐条一致，`peek_keybindings_match_the_navigation_table` 校验）。附加一条：`alt+f12` → `editor.action.peekDefinition`，when `editorHasDefinitionProvider && editorTextFocus && inReferenceSearchEditor`——VS Code 表里那一条带 `!inReferenceSearchEditor`，预览里按不出来，而 UX_SPEC §2.3 要求在预览里 `Alt+F12` 嵌套速览。`F12` 在窗口打开期间是 `goToNextReference`（与键位表、VS Code 一致），预览里的“继续跳转”经 `Alt+F12` 嵌套速览与 #40 的 `Ctrl+点击`。
- **打开结果**：`Enter`（`revealReference`）、结果行双击、预览内容双击发 `PeekEvent::Open`，宿主关闭速览并经 #17 的 `OpenDocuments` 打开（`OpenSource::CodeNavigation`、带选区，所以 `.md` 也打开源码并定位）；`Ctrl+Enter`（`openReferenceToSide`）在旁边的组打开并保留速览。`editor.stablePeek` 为 true 时 `Esc` 与预览双击都不关闭。
- **空结果与降级**：查询前先问 `Availability`，实现关系不可用时不开窗口、光标处提示“未找到实现”；没有查询服务或结果为空时关闭窗口，按 UX_SPEC §6.3 在光标处提示“未找到“X”的引用”，不精确的空结果追加“（语义索引构建中）”。窗口立即出现，300 ms 内没有结果时显示“正在加载…”。
- **生命周期**：关闭（实体释放）时取消全部后台工作，结果数据、片段、预览文档与预览代码视图随之释放；代码视图换了文档时宿主关闭它上面的速览。
- **查询路由**：每次打开速览时取 #40 装配的 `Workbench::cs_router()`（带 L1、同步打开中的缓冲区，未保存修改的位置经映射；#40 换掉路由后下一次速览自然是新的，打开期间的嵌套速览沿用打开时那个）。测试可用 `Workbench::set_peek_queries` 换成替身。外部调用方要在已有结果上直接打开速览时走 `Workbench::open_peek_with`（`PeekRequest::preset` 带结果、`marker` 带文案）。

验证：`cargo test -p rove-ui peek`（数据层单元测试 + `skeleton/workbench/peek_tests.rs` 的 17 个真实 GPUI 窗口入口测试（只经键位驱动）与 1 个键位表核对：三个入口、分组与计数、100 / 101 展开规则、`↑↓` / `Enter` / `Ctrl+Enter` / `F4` / `Shift+F4` / `F12` / `Ctrl+K F2` / `Esc` / `Shift+Esc`、`editor.stablePeek`、`editor.peekWidgetDefaultFocus` 两个取值、嵌套速览逐层返回与上下文键、近似标记、外部符号保底与注册提供方、空结果与实现不可用的提示、`.md` 打开源码、加载提示、切换到的文件打开失败时预览回滚并可重试（#143）、反复开关 100 次后结果与预览全部释放、经 #40 的正式路由速览时未保存修改的位置经映射、键位表逐条一致）。性能：`cargo xtask bench --filter peek/`（真实窗口：28,671 处引用打开到结果树首次绘制，预算 `peek_open_to_first_paint_28k`；4.7 万条全部展开后结果树每帧 CPU，预算 `peek_tree_scroll_frame_47k`；开关 100 次前后的工作集 / 私有字节打印在 stdout）。

## 34. 索引组件打包与版本握手（#43）

位置：`crates/rove-lang-cs/src/lifecycle/component.rs`（安装位置、发现规则、兼容判定，`lifecycle/mod.rs` 只追加 `pub mod component;`）；握手在 `lifecycle/resident.rs`（`hello` 的协议区间）；索引进程侧是 `tools/rove-indexer-cs/ComponentInfo.cs`（`--version`）与 `Server.cs`（协议区间）；打包脚本在 `scripts/components/`。`query.rs` 的冻结契约未改动。#43 没有对应的 `SKELETON(...)` 标记：它替换的是 #36 里 `locate_indexer` 的“环境变量或开发路径”临时查找。规则与版本对应表写在 [CS_INDEX_FORMAT](CS_INDEX_FORMAT.md)「组件、协议与索引格式的版本」「组件包与安装约定」，这里只记代码如何接缝。

| 环节 | 内容 |
|---|---|
| 发现 | `component::discover(&Roots)`：环境变量 `ROVE_INDEXER_CS`（开发覆盖，不看清单）→ `%LOCALAPPDATA%\Rove\components\indexer-cs\<版本>\`（`ROVE_COMPONENTS_DIR` 可替换 `components` 目录）→ `<rove.exe 目录>\components\indexer-cs\<版本>\`；每个版本目录要有 `rove-indexer-cs.exe` 与 `component.json`；在兼容的候选里选版本最高者，版本相同时靠前的目录优先。不启动任何进程。`process::locate_indexer` 只是 `locate_in(&Roots::current())` |
| 兼容判定 | `component::check(&Manifest)`：索引格式必须等于 `validity::EXPECTED_FORMAT`，协议区间与 `resident::PROTOCOL_MIN..=PROTOCOL` 必须有交集（`negotiate_protocol`）；不兼容时 `Incompatible { direction: ComponentTooOld / ComponentTooNew, aspect, component_version, detail }` |
| 状态与降级 | 没有组件 → `IndexState::NotInstalled`（状态栏“C# 基础导航”，不是失败通知）；有组件但都不兼容 → `IndexState::Incompatible(..)`（状态栏“C# 组件需更新”，不重试）；两者对路由都是 `SemanticStatus::NotInstalled`（近似结果，原因 `ComponentUnavailable`），导航降级到 L1/L0。已有的有效缓存索引不丢：构建时发现不兼容而缓存仍可信 → `Ready` + `IndexInfo::component_issue`；常驻进程 `hello` 协议对不上 → `ResidentEvent::Incompatible`，放弃该进程（源文件变化只标过期），同样记在 `component_issue` |
| 索引头部版本 | `IndexBuilder::component_version()`（`ProcessBuilder` 取已安装组件的清单版本）与缓存索引头部的索引进程版本必须相等，否则重建；版本未知（环境变量覆盖、测试替身）时用 `validity::EXPECTED_INDEXER_VERSION` |
| 界面 | `rove-ui::index_status`：状态栏条目、悬停与详情页（未安装时的安装方法、不兼容时更新哪一边）全部是 `index.*` 字符串，在 `STRINGS` 里登记；点击仍走 `rove.index.showDetails` |

验证：`cargo test -p rove-lang-cs --test component`（发现与选择、未安装、三种不兼容方向且不重试、`hello` 协议区间不相交被拒绝与相交被接受、缓存索引在常驻握手失败时保持可用）；`cargo test -p rove-lang-cs --lib lifecycle::component`（清单解析、协议协商、候选选择的单元测试）；`cargo test -p rove-lang-cs --test incremental protocol_version_negotiation_and_component_info`（真实进程的 `--version`、`hello` 区间与不支持协议的 `initialize` 被拒绝，需先 `dotnet build`）；`cargo test -p rove-ui index_status`（未安装、不兼容的状态栏文字、悬停、详情页内容）。组件包的打包与干净机器验证见 `scripts/components/` 与 [CS_INDEX_FORMAT](CS_INDEX_FORMAT.md)。

## 35. 元数据视图界面（#42）与多候选速览接线（#134）

**元数据视图是一份普通的只读文档**：资源 `Resource::Virtual("rove-metadata:<最外层类型的全局符号键>")`（键形如 `UnityEngine.CoreModule|T:UnityEngine.Transform`，取自索引，不用显示标题；嵌套类型落在外层类型的视图里，泛型用文档注释 ID，所以“同一个类型”的判定稳定）。正文是 #34 生成的签名 + XML 文档注释，经文档加载器的虚拟来源（`skeleton/text.rs` 的 `VirtualSource`，`FileDocuments::with_virtual_source`）交给 #17 的打开文档服务与 #19 的代码视图，只读原因为 `ReadOnlyReason::Metadata`（标签锁形图标、输入时光标处提示；“在此会话中切换只读”无效）。标签复用、预览 / 固定、导航历史、面包屑、`Ctrl+Shift+O`、`#region` 折叠、C# 语法与主题全部沿用既有实现。**没有单独注册一种编辑器类型**（issue 的风险提示即建议做成普通文档）：归属在这里登记，编辑器类型解析仍是文本。

| 位置 | 内容 | GPUI |
|---|---|---|
| `rove-lang-cs::metadata_view` | `MetadataViews`（语义来源 + `rove.metadataView.showDocs`）：符号键 → 视图位置（`locate`）、类型键 → 内容（`content`：正文、标题含泛型参数、程序集、DLL 路径）、`showDocs = false` 时去掉 `///` 行与显示 / 原始行号换算、按名字找有视图的外部类型（按快照代次缓存） | 否 |
| `rove-lang-cs::router::metadata` | 视图内的查询点（显示坐标）→ 符号：先按成员位置记录，再按名字解析（唯一外部类型为精确，多个或 L1 源码类型为近似），然后交给与源码相同的 `l2_navigate`，所以视图内 `Shift+F12` 的条数就是同一个符号在源码里的引用数 | 否 |
| `rove-ui::metadata` | `MetadataLoader`（`VirtualSource`，记下标题与 DLL 路径）、`MetadataSnippets`（#41 `ExternalSnippets` 的提供方）、界面字符串、标签悬停提示 | 仅 tooltip |
| `skeleton/workbench/metadata_host.rs` | `open_metadata_targets`（#40 的接入点：定位 → `OpenDocuments`，找不到时状态栏提示）、`showDocs` 与索引快照代次变化时重新渲染（`EditorService::replace_document`，光标与滚动换算）、标签标题 `Transform [元数据]` | 是 |

没有新增命令、设置项或颜色键（`rove.metadataView.showDocs` 由 #13 登记）。面包屑的文件路径段对元数据视图只有程序集名。元数据视图不做本文件的语义同名高亮。测试写出器（`router::testing`）新增 `TestView` 与 `metadata_sample_index`。

**#134**：`Workbench::open_peek` 不再是空实现。#40 的 `PeekRequest`（目标列表、近似标记、符号名）登记为待办，下一帧 `sync_nav`（手里有 `Window`）转成 #41 的 `PeekRequest`（`preset` 带结果）调用 `show_peek`；`peek` / `goto` / `gotoAndPeek` 按 `editor.gotoLocation.multiple*`（`gotoAndPeek` 等目标显示且插入符到位后再在那里开）。单个候选仍直接跳转。#40 / #41 的“合并前”说明以此为准。

验证：`cargo test -p rove-lang-cs metadata`、`cargo test -p rove-ui metadata_`（窗口级：`F12` 打开 / 定位 / 只读 / 标签复用 / 泛型与嵌套 / 找不到 / `showDocs` / 面包屑 / 视图内导航与 `Alt+←` / `Alt+F12` / `Shift+F12` / 索引替换 / 预览片段）与 `nav_tests`、`peek_tests`（多候选速览）。

## 36. 资源管理器（#23）

位置：`crates/rove-ui/src/explorer/`（`lib.rs` 只追加 `pub mod explorer;`）；窗口装配在 `skeleton/workbench/explorer_host.rs`。不修改 #1 的契约；GPUI 类型只出现在 `view.rs` 与 `registry.rs` 的 action。只读：文件操作（重命名、删除、复制路径、新建、拖放）属于 R62，一律不登记。

| 模块 | 内容 | GPUI |
|---|---|---|
| `model.rs` | `ExplorerSettings`（`explorer.compactFolders` / `sortOrder*` / `autoReveal` / `autoRevealExclude`、`workbench.editor.decorations.*`）；VS Code 的自然排序（数字按数值、大小写只在其余相同时决定顺序，`sortOrderLexicographicOptions` 四个取值）；树数据源 `ExplorerSource`（`TreeSource`）：展开时才向 #15 要子项、排序、紧凑文件夹（沿“只有一个子目录”的链压缩成一个节点）、读取失败的提示行；`plan_refresh`（一批文件事件 → 需要刷新的节点）；`reveal_plan`（路径 → 要展开的节点与目标节点） | 否 |
| `decorations.rs` | 装饰接口 `DecorationProvider` 与两类来源：只读标记（#15 的 `Workspace::rules`，含 `rove.files.extraReadonlyInclude`）、错误 / 警告计数（`ProblemDecorations`，问题面板 R60 合并前为空，文件夹显示其下的总数）；**不接 Git**（`gitDecoration.*` 静默忽略） | 否 |
| `registry.rs` | 命令（`explorer.openAndPassFocus`、`explorer.openToSide`、`filesExplorer.openFilePreserveFocus`、`first/last/next/previousCompressedFolder`、`workbench.files.action.showActiveFileInExplorer`）、界面字符串、上下文键名、`ExplorerAction` | 仅 action |
| `view.rs` | `ExplorerView`：#18 `ListView` 的包装——行渲染（twistie、按扩展名区分的 codicon 图标、紧凑文件夹分段、锁 / 计数徽标）、命令、`filesExplorerFocus` 等上下文键、空状态、定位、文件事件的增量应用 | 是 |

- **节点就是路径**：节点 ID 永远是工作区路径；紧凑文件夹链的 ID 是**链首目录**的路径、标签是 `a/b/c`。选中、定位、过滤、装饰都按路径解析（`chain_paths`、`reveal_plan`），不依赖行下标；链首作 ID 使“链尾继续变长或在下面断开”时上层展开状态不变。
- **打开根目录不等扫描**：宿主用 #15 的 `Folder::start`（根目录已列出就返回），资源管理器立即显示；其余目录经文件事件到达。需要完整文件列表的消费者（快速打开、C# 快速索引、关闭已删除文件的标签订阅）在 `wait_until_complete` 之后再装配（`workspace_scanned`）。`files.exclude` / `rove.files.*` 变化触发重新扫描，展开状态与选中项按路径保留。
- **增量更新，不重建整棵树**：宿主把一批（风暴时合并后的）文件事件交给 `ExplorerView::files_changed`；`plan_refresh` 只给出形状变了的节点——链不变时只刷新链首的子项，链变了（单子目录变多 / 变少）刷新它所在的节点；只处理已显示的目录；`Changed` 不触发任何刷新，`RescanRequired` 整棵重新装填。装填走 #18 的 `TreeModel::set_children`，它按 ID 还原展开状态，`ListWidget::update_tree` 按 ID 还原焦点与选中。
- **紧凑文件夹导航**：压缩节点内部的焦点段由 `ExplorerView` 保存（聚焦时从最后一段开始，与 VS Code 一致）；`home` / `end` / `left` / `right` 的默认键位带 `explorerViewletCompressed*Focus` 条件，包装元素的 `KeyContext` 发布这三个键：在第一段按 `left`、最后一段按 `right` 时条件不成立，落回 #18 的 `list.collapse` / `list.expand`。鼠标点击某一段暂不单独聚焦（#18 的行不带监听器，行内命中要改树组件，留给后续）。
- **打开文件**：`ListEvent::Open`（单击 = 预览且保持焦点、双击 / `list.select` = 正式标签）与三个命令（`enter` 正式并移交焦点、`ctrl+enter` 在侧边、`space` 预览并保持焦点）都转成 `ExplorerEvent::Open`，宿主经 #17 的 `OpenDocuments` 打开（`OpenSource::Explorer`；`.md` 的编辑器类型由 #17 解析：从文件树进入预览，D6）。
- **定位当前文件**：宿主每帧比较活动标签，变了就 `auto_reveal`（`explorer.autoReveal`：true / false / `focusNoScroll`；`autoRevealExclude` 用 #15 的 `RuleSet`）；目标已是唯一选中项时不再动（由树自己打开的文件不会让焦点抖动）；`workbench.files.action.showActiveFileInExplorer` 无视设置、打开侧边栏并滚动到目标；`ctrl+shift+e` 后焦点交给树（`sync_explorer`）。
- **过滤**：`list.find` 与 `workbench.list.defaultFindMode` / `defaultFindMatchType` 原样来自 #18（过滤时命中节点的父级自动展开、清空后还原）；作用在已加载的节点上。
- **空状态与失败**：没有工作区时 `ExplorerView::set_workspace(None)` 显示“尚未打开文件夹。”与“打开文件夹”按钮（按钮发出 `workbench.action.files.openFolder`，对话框由 #28 / #16 提供）；目录读取失败显示一行原因（#15 的快照只区分“存在 / 不存在”，所以权限与路径过长在现有契约下都表现为“已不存在或无法读取”）。
- **没有右键菜单**：本 issue 没有可放进菜单的已实现项，文件操作菜单属于 R62，所以不弹任何菜单（也就不存在未实现的文件操作项）。
- **已知限制**：`explorer.sortOrder = modified` 需要修改时间，#15 不提供，按默认顺序；`explorer.expandSingleFolderWorkspaces` 针对多根工作区，Rove 单根且不显示根行；行高沿用骨架的 24 px。

验证：`cargo test -p rove-ui explorer`（数据模型与装饰的单元测试；`skeleton/workbench/explorer_tests.rs` 真实 GPUI 窗口 + 真实文件监听的入口测试：根目录立即显示与默认排除、`extraExclude` 改动后重新扫描、紧凑文件夹与 `home` / `end` / `left` / `right`、`explorer.compactFolders = false`、单击预览 / 双击正式 / `space` / `enter` / `ctrl+enter`、`.md` 的编辑器类型、`autoReveal` 与 `autoRevealExclude`、命令“在资源管理器中显示活动文件”、`ctrl+shift+e` 焦点、过滤与还原、脚本一次创建 / 删除 500 个文件后树与磁盘一致且展开与选中保留、重命名 / 删除已展开的目录、只读装饰、空状态）。性能：`cargo test -p rove-ui --release --test explorer_perf -- --nocapture`（合成的 1 万个文件 / 1 万个文件夹，以及 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 的最大目录；实测数据见 PR）。

## 37. 代码视图的编辑交互与输入法（#47）

位置：`crates/rove-ui/src/code_view/edit.rs`（`code_view/mod.rs` 只追加 `pub mod edit;`）与窗口装配
`crates/rove-ui/src/skeleton/workbench/edit_host.rs`。只用 #1 的冻结契约、#46 的 `EditSession`、#49 的 `TextFile` /
`SaveController`；不修改契约。GPUI 类型只在 `rove-ui`。`rove-ui` 新增 Windows 目标依赖 `windows-sys`
（`Win32_UI_Input_Ime`、`Win32_UI_Input_KeyboardAndMouse`、`Win32_Globalization`：`ImmNotifyIME`）。

| 文件 | 内容 |
|---|---|
| `code_view/edit.rs` | `EditHost`（宿主借出会话）、`CodeViewEvent::Edited` / `EditNotice`（事务、合成的 `TextEdit`）、编辑命令、`EntityInputHandler`、`Composition`、行号映射（`old_to_new` / `new_to_old`）、`combine_edits` / `combine_spans` |
| `code_view/{view,element}.rs` | 追加：编辑入口与组合串字段、回车、组合行内嵌渲染与 `editor.compositionBorder` 下划线、改写模式插入符、`window.handle_input`、插入符变化时 `invalidate_character_coordinates` |
| `code_view/{cache,decorations,highlight}.rs` | 追加：编辑后按行平移（`Lru::rekey` / `remove`、`DecorationStore::remap`、`SyntaxLayer::pending`：新快照到达前用旧 token 画新文本） |
| `editors/{mod,syntax}.rs` | 追加：`replace_document`（编辑后的新快照换进标签，身份不变）、`set_dirty`、`open_ready`（未命名文档）、`force_close_document`；`Highlights::edited` / `resume`（分析进行中不取消，完成后按累积编辑合成一个增量编辑再分析） |
| `skeleton/workbench/edit_host.rs` | 窗口装配：每个可编辑文档后台建 `TextFile` 挂到 `SaveController`、编辑事件同步（其他视图、语法、`FileDocuments` 缓存、`BufferStates::edited`、脏点）、保存命令、`Effect` 反映、外部修改读盘、关闭确认、`ctrl+n` |

**编辑链路**：视图不持有文本模型。命令先把视图选区写进会话（`set_selections`，光标撤销与撤销分组以会话为准），执行
会话的事务，再用新快照增量更新本视图：行缓存与装饰按 `LineChanges` 平移，只有被改的行重建并在事件处理里立即整形
（下一帧不画占位，行高不变）；语法旧结果经 `SyntaxLayer::pending` 映射到新行号继续着色，新快照到达后按变化行替换
（不闪烁）。`Edited` 的宿主处理：新快照换进打开文档服务（所有标签与 `FileDocuments` 缓存，文档身份不变）→ 同一文档
的其他视图 `apply_edits`（选区按事务映射）→ #94 `DocumentHighlighter::analyze(edit)` 后台增量解析（**不在输入的
同一帧里调用 tree-sitter**）→ #38 `BufferStates::edited`（C# 文档；`OpenBuffers::note_edited` 让每帧的比较不把编辑
当成重载）→ 标签脏点（撤销回保存点即干净）。

**输入法**（GPUI Windows 后端的调用路径见 `vendor/gpui/src/platform/windows/events.rs`）：所有范围参数为 `None`，
偏移是 UTF-16；视图把“组合串（或选区）所在的行”作为输入法看到的文档（`ImeRegion`），组合内光标按组合串自身换算
（ROVE_PATCHES 的第二轮评审要求）。组合串**不写进缓冲区**：没有事务、不进撤销栈、不触发重解析；提交时
`push_undo_stop` 前后各一次，整串是一个撤销单元。取消（空组合串、空提交）不删选区；点击别处与失焦先在原位置提交，
再 `ImmNotifyIME(CPS_CANCEL)` 让输入法丢掉自己的组合状态（不留残片、不重复提交）；组合中按到的 `undo` 只取消组合。
只读文档上不进入组合态（组合开始时在光标处提示一次原因）。改写模式下组合串按整串覆盖其后同样多的字素簇。

**命令**（`code_view::commands::COMMANDS` 追加，默认键位来自 #14 的 VS Code 表）：`lineBreakInsert`、`tab` / `outdent`、
`editor.action.indentLines` / `outdentLines`、`deleteLeft` / `deleteRight`、`deleteWord*`（含 `Start` / `End` / `Part`）、
`deleteAllLeft` / `deleteAllRight`、`cursorWord*` 与 `*Select`（含 `Part`、`PartStart`、无障碍版本）、`cursorLineStart` /
`cursorLineEnd` 与 `*Select`、`undo` / `redo` / `cursorUndo` / `cursorRedo`、选择定位点四个命令（上下文键
`selectionAnchorSet`）、`insertLineBefore` / `insertLineAfter`、剪切 / 粘贴（无选区整行剪切经剪贴板元数据
`rove.wholeLine` 粘贴到当前行之上）、`toggleOvertypeInsertMode`、`indentationToSpaces` / `indentationToTabs`、`type` /
`default:type`（带参数，经命令桥 → 窗口 → 聚焦的视图）。回车没有键位：视图在 `on_key_down` 里换行并保持当前行缩进
（自动缩进规则属于 R52），`editor.trimAutoWhitespace` 时只有空白的行在再次回车时清空。设置项追加 `editor.insertSpaces`、
`useTabStops`、`stickyTabStops`、`trimAutoWhitespace`、`overtypeCursorStyle`、`overtypeOnPaste`；颜色键追加
`editor.compositionBorder`。单词导航与 `cursorLineStart/End` 在只读文档上同样可用。

**保存装配**（§30“装配”一段的清单）：`SaveAction` 经 `keys::action_for` 分发到窗口（`changeEOL` 带参数，走命令桥）；
保存的变换与编码在 UI 线程（`begin_save`），冲突检测与原子写入在后台（`SaveJob::run`），变换改了文本时先把新快照交给
视图；`Effect::Saved` 清脏点并调用 `BufferStates::saved`；`Effect::Reloaded` / 还原 / 换行符统一把会话的新快照换进文档与
所有视图（`set_text(keep_view_state)`：滚动位置不变，选区取会话映射后的位置）；`Effect::Deleted` 在标签上显示
“已删除”（`save.label.deleted`）；工作区 `FileEvent` → `note_change` → 安静期后后台 `DiskStamp::read` → `apply_disk`。
关闭有未保存修改的标签（标签上的按钮、`closeActiveEditor`）与窗口关闭按钮（`edit_host::attach_close_confirmation`，
替换 `Shell::attach_window` 登记的回调并先做它原来的布局记录）走 #49 的“保存 / 不保存 / 取消”。`ctrl+n`
（`workbench.action.files.newUntitledFile`）新建 `Untitled-N`（`files.encoding` / `files.eol`），首次保存走另存为
（`prompt_for_new_path`）。#50 的热退出仍经 `SaveController::set_hot_exit` 接管关闭确认。

尚未完成（见 PR）：状态栏 `changeEditorIndentation` 选择器属于 #27（条目尚未合并），两个缩进转换命令已登记、可从命令
面板执行；按键到上屏的 p99 基准与输入法实机验收见 PR 描述。

验证：`cargo test -p rove-ui --test code_view_edit`（入口：输入一个字符 → 缓冲内容、编辑事件与 `InputEdit`、插入符、
撤销重做；编辑基础命令参数化；CJK / 驼峰 / 下划线 / emoji（ZWJ）混排行的单词导航与 #46 逐步一致；光标撤销与选择定位点；
混合换行的剪切粘贴字节级比对；改写模式（输入、行尾、粘贴、输入法整串覆盖）；组合串不进缓冲区、一次撤销整串、取消不删
选区、组合中途点击在原位置提交、组合中的撤销；只读拒绝且不进入组合态）、`cargo test -p rove-ui code_view::edit`
（行号映射、增量编辑合成、UTF-16 区域换算）、`cargo test -p rove-ui edit_tests`（真实窗口：编辑 → `ctrl+s` → 外部修改
静默重载且滚动不变 → 本地有修改时提示条 → 删除后“已删除”；C# 编辑与保存登记到 `BufferStates`；`ctrl+n`；标签与窗口的
关闭确认）。

## 38. Markdown 预览（#29）

位置：`crates/rove-ui/src/markdown/`（`lib.rs` 只追加 `pub mod markdown;`）、打包资源 `crates/rove-ui/assets/markdown/`、
窗口装配 `crates/rove-ui/src/skeleton/workbench/markdown_host.rs`。按 [M0 WebView2 报告](M0_WEBVIEW_SPIKE_REPORT.md) §5 的
推荐方案实现（Hwnd + 相交隐藏），并补上报告 §5.1 列出的全部缺口；不修改 #1 的契约（`documents.rs` 的
`Presentation`/`OpenSource`、`contributions.rs` 的 `Placement::EditorTitle` 均为已有登记点）。
网页内核只用在这里，且只在第一次有预览要显示时创建。

| 文件 | 内容 | GPUI |
|---|---|---|
| `markdown/render.rs` | Markdown → HTML：`pulldown-cmark`（GFM 表格、任务列表、删除线、脚注、提示块、`$` 数学、YAML front matter），自写 HTML 输出给块级元素加 `class="code-line" data-line=<源码行>`（与 VS Code 预览、`markdown.css` 相同）；`ammonia` 白名单净化（去掉脚本、事件属性、`iframe`、`javascript:`，`style` 只留不能带 `url()` 的属性）；资源地址改写：本地 → `https://<盘符>.rove-file.local/…`，远程（含 `//host`、UNC）未放行时换成占位图并计数；`markdown.preview.breaks/linkify/typographer/frontMatter`、`markdown.math.enabled` | 否 |
| `markdown/page.rs` | 预览页（CSP：`script-src 'nonce-<随机>'`、`connect-src 'none'`）与打包资源表（`build.rs` 构建时 DEFLATE 压缩后嵌入，请求时解压；约 4.3 MB → 见 PR 的体积数据）；主题消息：扫描打包 CSS 里的 `--vscode-*` 变量，按 `textLink-foreground` → `textLink.foreground` 从当前主题取色，加上 `markdown.preview.font*` 与 `editor.font*` | 否 |
| `markdown/registry.rs` | 命令（`markdown.togglePreview`、`showPreview`、`showPreviewToSide`、`showSource`、`preview.refresh`）、界面字符串、颜色键（含 `markdownAlert.*`）、编辑器标题栏按钮（`Placement::EditorTitle`，`when` 用 `activeCustomEditorId` / `resourceLangId`）、`MarkdownAction` | action |
| `markdown/backend.rs` | 后端接口 `Backend` 与事件 `BackendEvent`；`FakeBackend`（GPUI 测试窗口没有 HWND）、`UnavailableBackend`（非 Windows） | 类型 |
| `markdown/webview.rs` | WebView2 后端（`webview2-com` 0.38）：环境参数关闭后台联网（组件更新、SmartScreen、同步、崩溃上报…），`IsReputationCheckingRequired=false`；全部资源请求经 `WebResourceRequested`（打包资源从内存返回、本地文件走盘符映射、远程未放行 403）；拒绝顶层 / 子框架导航、新窗口、下载、权限、外部协议、基本认证；显式 `CoInitializeEx`（STA）；`RasterizationScale` 由宿主给；隐藏时 `TrySuspend`；回调不持有借用调 COM；网页里的 Ctrl/Alt 组合、F 键、Esc 交回 Rove 键位服务 | 是 |
| `markdown/mod.rs` | `prepare_hwnd_hosting()`：设 `GPUI_DISABLE_DIRECT_COMPOSITION=1`（#6 报告：DirectComposition 的 topmost 合成目标会盖住子 HWND），`skeleton::launch` 在创建应用前调用 | 否 |

**打开方式（D6）**：`workbench.editorAssociations` 的 Rove 默认值 `{"*.md": "vscode.markdown.preview.editor"}` 由
#17 登记（只在用户未设置时生效）；文件树、快速打开 → 预览；带位置或选区的打开（搜索结果、跳转、引用、转到行、大纲、面包屑）
→ 源码（#17 的 `EditorTypes::resolve`）。没有运行时时 `EditorTypes::set_preview_available(false)`：关联到预览的文件改为源码，
显式要求预览返回 `Unsupported`。`EditorService::reopen_as` 把一个标签就地换成另一种编辑器（切换、双击），文档不重新读盘。

**实例生命周期**（`markdown_host.rs`）：每帧选“拥有者”（聚焦组优先的、可见的预览标签），其余可见预览显示一行提示、单击接管；
第一次有拥有者时才 `Backend::create`（异步，期间显示标题 + 正文灰条的骨架）；拥有者消失 → 隐藏并挂起；所有预览标签都关闭 →
`rove.markdown.webviewIdleTimeout` 秒的 GPUI 定时任务（不依赖渲染帧），到期在前台任务里 `Close()` 并释放 environment，浏览器进程退出。
弹出层（快速打开 / 命令面板、转到符号、菜单、模态框、通知、悬停卡片、拖动标签）打开时隐藏实例（`Render` 末尾判定，画布在 prepaint 读）。
WebView2 的回调只往通道里送 `BackendEvent`，由窗口的前台任务在正常 GPUI 上下文里处理。

**通信**：`{version:1,type,…}`。宿主 → 页面：`render`（`seq`、`documentId`、净化后的 HTML、`markdown.styles`、`markdown.math.macros`、
初始行或片段）、`theme`、`scrollTo`、`selection`（`markEditorSelection`）、`fragment`；页面 → 宿主：`ready`、`rendered`（统计）、`scroll`、
`dblclick`、`link`、`focusExit`。旧 `seq` / 别的 `documentId` 的消息丢弃；预览 → 编辑器的滚动带回声抑制。

**链接**：`#片段` 页内滚动；相对 `.md` 按 `markdown.preview.openMarkdownLinks` / `markdown.links.openLocation` 打开（片段在渲染后滚动）；
其他本地文件以源码打开；`http(s)` / `mailto` 先弹安全确认模态框，确认后交给系统浏览器。

验证：`cargo test -p rove-ui markdown`（`markdown::*` 单元测试：渲染与行号、净化、远程计数与放行、UNC、链接分类、选项、front matter、
锚点、键转发、资源与 CSP、主题变量与颜色登记；`skeleton::workbench::markdown_tests` 入口测试：打开 `.md` → 预览 → `ctrl+shift+v` 源码 →
切回、`ctrl+k v` 并排与标题栏按钮、用户 `editorAssociations`、带位置打开为源码、没有运行时只通知一次、远程放行、主题不重建、
命令面板隐藏实例、空闲回收与超时前复用、双击定位、双向滚动同步）。真实 WebView2：`cargo run -p rove-ui --example markdown_probe`
（见 AGENTS §8.5）。不做：`markdown.preview.toggleLock` / `showLockedPreviewToSide`（Rove 的预览总是绑定一个文件）、
`showPreviewSecuritySelector`（放行走提示条）、预览内 `Ctrl+F`（P1，WebView2 Find API 留给后续）、预览中的大纲联动（#26 的大纲
目前只跟随文本编辑器）。

## 39. 着色黄金测试（#11）

位置：数据在 `tests/golden/`（VS Code `vscode-colorize-tests` 的 `colorize-fixtures/` 与
`colorize-results/`，原样复制）与 `tests/golden/thresholds.toml`（阈值，本仓库自己的文件）；
比对器是 `rove-syntax` 的 example `golden_colors`（`config.rs` 读阈值、`data.rs` 读数据并对齐到
字节区间、`check.rs` 跑链路与逐字符比对、`report.rs` 出报告），判定测试
`crates/rove-syntax/tests/golden_colors.rs` 用 `#[path]` 引入同一份模块。

链路：fixture → `rove_buffer::RopeText` → #9 `TreeSitterSyntax::analyze` 的 scope →
#10 `ResolvedTheme::token_style` 的前景色 → 与 `colorize-results` 的期望颜色逐字符比较。
**比颜色不比 scope**；六个受阈值约束的主题 + 四个只统计的主题。报告写 `out/golden/`（已 gitignore）。
口径、当前一致率、每一类差异的原因与为对齐所做的查询改动见 [GOLDEN_COLORS.md](GOLDEN_COLORS.md)。

本 issue 为了向 VS Code 的 scope 层级看齐，改了 `crates/rove-syntax/src/highlight/queries/` 下的
C# / YAML / XML / Markdown 查询，并在 `highlight/grammar.rs` 里加了两处：围栏代码块注入时补
`meta.embedded.block.<语言>` 外层 scope；独占一行的 C# 注释把捕获补齐到行首空白。
`contracts.rs` 的冻结契约与 #10 的匹配算法都没有改动。本 issue 没有对应的 `SKELETON(...)` 标记
（`syntax` 与 `theme` 两处分别由 #9、#10 负责）。

验证：`cargo test -p rove-syntax --test golden_colors`（并入 `cargo test --workspace`）；
本地排查用 `cargo run -p rove-syntax --example golden_colors`（见 AGENTS §8.6）。
## 40. 欢迎页、启动参数与首选项入口（#28）

位置：`crates/rove-ui/src/welcome/`（`lib.rs` 只追加 `pub mod welcome;`）、窗口装配
`crates/rove-ui/src/skeleton/workbench/welcome_host.rs`、启动参数解析在 `crates/rove-app/src/main.rs`。
不修改 #1 的契约：欢迎页复用 #17 的编辑器类型与打开文档服务，提示条复用 #16 的第 3 级反馈，
快速选择复用 #18，写设置复用 #13 的保留格式写入。#2 没有 `SKELETON(welcome)` 标记（骨架不含欢迎页）。

| 文件 | 内容 | GPUI |
|---|---|---|
| `welcome/registry.rs` | 命令（`workbench.action.files.openFile` / `files.openFolder` / `openRecent` / `openSettings` / `openGlobalKeybindings` / `selectTheme` / `showWelcomePage`）、界面字符串、颜色键（`welcomePage.background` / `tileBackground` / `tileHoverBackground`）、齿轮菜单项、`WelcomeAction` | 仅 action |
| `welcome/recent.rs` | 最近打开的文件夹与文件：大小写不敏感去重、上限 20、JSON 读写 | 否 |
| `welcome/startup.rs` | `rove.exe <路径>` 的分类（文件 / 文件夹 / 不存在）、长路径（> 260 字符）转 `\\?\` 前缀、相对路径与 `.` `..` 规整 | 否 |
| `welcome/themes.rs` | 颜色主题选择器的候选与「预览 / 还原 / 确认」状态机 | 否 |
| `welcome/view.rs` | 欢迎页与水印的内容模型：「开始」两项、「最近」行、水印行（快捷键由 #14 解析）、`workbench.startupEditor` 的读写 | 否 |
| `skeleton/workbench/welcome_host.rs` | 窗口装配：虚拟文档来源、命令实现、浮层、共用文件提示条、绘制 | 是 |

**欢迎页**是一种编辑器类型 `rove.welcomePage`（#17 的 `EditorTypes` 里与 Markdown 预览并列登记），资源是虚拟文档
`rove-welcome`（`skeleton/text.rs` 的 `VirtualSource` 链，正文为空，绘制全在宿主）。它不受 `workbench.editorAssociations`
影响，用户改不掉。`launch` 装配好窗口后调用一次 `open_startup_editor`：`workbench.startupEditor == "welcomePage"` 就打开；
**没有打开文件夹的空窗口总是打开**（UX_SPEC §6.3）。窗口构造函数本身不打开任何编辑器（测试与嵌入场景保持干净）。

**空窗口**：`Workbench::empty`（根路径为空）。资源管理器保持 #23 的「尚未打开文件夹。」+「打开文件夹」空状态，
不扫描工作区，快速打开不再显示「正在加载文件列表…」。打开文件夹时**一个窗口一个文件夹**（FEATURES §6）：
新文件夹装进一个新窗口，当前窗口是空窗口且 `window.openFoldersInNewWindow != "on"` 时新窗口出来后关掉它。

**启动参数**：`main.rs` 先拆 `--vscode-user-dir`（#13），其余参数交给 `welcome::startup::plan`。第一个文件夹是本窗口的
工作区，其余文件夹各开一个窗口，文件在本窗口打开；不存在或无权限的路径照常开标签，内容区是 #17 的错误占位页
（UX_SPEC §6.4），不弹模态框。不提供 `code` 那样的 shell 命令行工具。

**首选项入口**：齿轮菜单（`MenuId::Manage`）四项 = #16 的命令面板 + 本 issue 的设置、键盘快捷方式、颜色主题。
`Ctrl+,` / `Ctrl+K Ctrl+S` 打开**与 VS Code 共用的那一份**文件（位置由 #13 按 `--vscode-user-dir` / `ROVE_VSCODE_USER_DIR` /
`%APPDATA%\Code\User` 解析），标签就绪后用 `Reason::FileContext` 报一条第 3 级提示条；「可关闭」「不再提示」与
`%APPDATA%\Rove\globalState.json` 的持久化都是 #16 反馈层的既有能力，**绝不写进共用的 `settings.json`**。
不登记 `workbench.action.openSettings2`（图形化设置界面是 R70）、演练与 `welcome.showNewFileEntries`。

**颜色主题**：`Ctrl+K Ctrl+T` 的快速选择列出 #10 目录里的内置主题（扩展主题是 R68）。移动选中项时调用新增的
`ThemeService::preview`（只换内存中的快照、发布修订号，**不写文件**），`Esc` 或失去焦点调 `reload()` 按设置重建即还原，
`Enter` 才通过 `Settings::write` 写一次 `workbench.colorTheme`（保留注释、格式与键顺序，写入前备份）。
接受事件在浮层关闭之前同步落到 `Welcome::accepted`，所以关闭时分得清「确认」与「取消」。

**状态位置**：最近列表在 `%APPDATA%\Rove\recent.json`（`LayoutStore::recent_path`），「不再提示」在
`%APPDATA%\Rove\globalState.json`；都不写项目目录、不写共用设置。

验证：`cargo test -p rove-ui welcome`（`welcome::*` 单元测试 + `skeleton::workbench::welcome_tests` 入口测试：
空窗口 → 欢迎页 + 资源管理器空状态、`startupEditor` 开关与「只改一个键、注释与格式不变」、最近列表的完整路径 /
移除 / 重启后仍在 / `Ctrl+R` 快速选择、启动参数的文件与错误占位页、共用提示条只出现一次与「不再提示」、
主题预览 → `Esc` 还原 → `Enter` 写入、水印跟随用户 `keybindings.json`、齿轮菜单四项）；
`cargo test -p rove-theme preview`（预览不写设置、`reload` 还原、未知主题保留当前快照）。

