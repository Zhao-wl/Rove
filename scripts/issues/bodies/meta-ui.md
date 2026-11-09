## 背景

阅读 Unity 代码时，有很大一部分跳转落在源码之外：`transform.position`、`List<T>.Add`、第三方 DLL 的 API。VS Code 的 C# Dev Kit 在这种情况下反编译出完整源码；Rove 的做法不同——[[idx-meta]] 在建索引时就把外部符号的**签名和 XML 文档注释**写进索引的元数据段，不做反编译（`UX_SPEC.md` §4.1、`LANGUAGE_INTELLIGENCE.md` §3.3 的“索引内容”表）。

本 issue 负责把这些预生成的内容**变成一个真正的编辑器标签**：只读、带 C# 高亮、带面包屑、能在里面继续跳转，而且 `Shift+F12` 能列出工作区里所有用到这个 API 的位置——这一条是 Rove 相对 C# Dev Kit 的增强（§4.1 表格中标注为“Rove 的增强”），因为引用数据本来就在预计算索引里。

[[nav-ui]] 已经把“目标是外部符号”这一分支接到本 issue：在本 issue 合并前它显示状态栏临时提示，合并后改为打开元数据视图。

必读：`UX_SPEC.md` §4.1（逐行对照表）、§3.4（只读状态）、§6.1（反馈分级）、§6.3，`LANGUAGE_INTELLIGENCE.md` §3.3，`FEATURES.md` §5、§9，`SETTINGS.md`（`rove.metadataView.showDocs`，P1，默认 true），`THEME_COLORS.md`（Editor Groups & Tabs、Editor colors）。

## 目标

### 1. 注册一种只读的自定义文档类型

- 通过 [[groups]] 的“编辑器类型登记 + 解析”机制注册元数据视图这一种文档，用一个专用的虚拟路径（例如 `rove-metadata:UnityEngine.CoreModule/UnityEngine.Transform`）作为文档标识，使打开文档服务的既有语义（查找已打开的文档、按位置定位、预览 / 固定标签、标签状态恢复）**原样可用**，不另造一套打开路径。
- 内容由 [[idx-meta]] 产出、经 [[router]] 的查询接口取得，以一个只读 `TextSource`（[[contracts]]）的形式交给 [[codeview]]，不落盘、不进文件监听。
- 文档模型中的只读原因取“元数据视图”（`UX_SPEC.md` §3.4 列出的只读来源之一）：标签与编辑器标题显示锁形图标，尝试输入时在光标处提示（第 2 级反馈，复用 [[codeview-hl]] 的只读表现，不自己造浮层）。“在此会话中切换只读”对元数据视图**无效**。

### 2. 标签标题与内容呈现

- 标题 `Transform [元数据]`（§4.1：VS Code 是 `Transform [从元数据]`），带锁形图标；悬停提示显示 DLL 完整路径。
- 文件头是一个**默认展开的 `#region`**：程序集名、版本、DLL 完整路径（照搬 C# Dev Kit 的 `#region 程序集 UnityEngine.CoreModule, Version=…`）。区域的折叠依赖 [[codeview-hl]] 的 `#region` 标记折叠，不在本 issue 另写折叠逻辑。
- 正文为签名 + XML 文档注释（以 `///` 形式），**没有方法体**。`rove.metadataView.showDocs` 为 false 时隐藏文档注释；设置热切换后当前标签立即重新渲染。
- 高亮走 [[syntax]] 的 C# 语法与 [[theme]]，与普通 `.cs` 文件完全一致——元数据视图不得有自己的一套配色。

### 3. 面包屑

`UnityEngine.CoreModule › UnityEngine › Transform › position`（§4.1），行为与源码文件的面包屑一致。本 issue 把元数据文档的符号树按 [[contracts]] 的符号树类型提供给 [[outline]]，由 [[outline]] 绘制面包屑与大纲小节；`Ctrl+Shift+O`（转到文件中的符号）因此在元数据视图中自然可用。

### 4. 在视图内部继续导航

- `F12`（`editor.action.revealDefinition`）跳到其他类型，目标可能是源码文件，也可能是另一个元数据视图；`Alt+F12`（`editor.action.peekDefinition`）在视图内开速览（[[peek]]）。
- **`Shift+F12`（`editor.action.goToReferences`）列出工作区中所有使用这个 API 的位置**，以速览窗口呈现（D3）——例如“项目里哪些地方用了 `Transform.position`”。
- 因此本 issue 必须在元数据视图激活时设置 `editorHasDefinitionProvider` / `editorHasReferenceProvider` 上下文键（规则与 [[nav-ui]] 对齐），并把视图内的位置 → 符号解析交给 [[router]]；视图内的跳转同样入导航历史（`Alt+←` 能退回源码）。

### 5. 标签复用与嵌套类型

- **同一个类型只开一个标签**：再次跳到同一类型的其他成员时只滚动定位，不新开标签，也不把已固定的标签变回预览标签（§4.1）。
- 嵌套类型（如 `Dictionary<TKey,TValue>.Enumerator`）打开**外层类型**的视图并定位到嵌套类型的那一段。
- 泛型类型的标题与路径用索引中的规范名称，保证“同一个类型”的判定是稳定的键，而不是显示用的字符串。

### 6. 找不到元数据

组件未安装、索引未就绪、或索引中没有这个符号时：状态栏临时提示“未找到 `Xxx` 的元数据”（第 1 级反馈，3 秒后消失），**不打开空标签**（§4.1 末行）。这与 [[nav-ui]] 在本 issue 合并前的降级文案一致，合并后由本 issue 统一发出。

### 7. 登记与替换

新增命令、设置项、颜色键、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。[[skeleton]] 中没有元数据视图的最简实现，本 issue 不删除 `SKELETON(...)` 标记，但要保持其端到端测试通过，并在 `docs/ARCHITECTURE.md` 中登记这一文档类型的归属。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 元数据内容的生成、签名与文档注释的格式化、索引中的元数据段（[[idx-meta]]、[[idx-format]]）。
- 反编译（明确不做，§4.1）、保存与另存为、编辑。
- `F12` / `Ctrl+点击` / 悬停 / 导航历史 / 同名高亮的实现本身（[[nav-ui]]）、速览窗口（[[peek]]）、查询路由（[[router]]）。
- 索引状态栏条目与“安装语义组件”的说明页（[[idx-life]]）。
- 元数据视图中的语义高亮（[[semantic]]，M4）与粘性滚动（[[sticky]]，M4）。

## 验收标准

- [ ] 从 `transform.position` 按 `F12` 打开 `Transform [元数据]`，并定位到 `position` 属性；标签带锁形图标，尝试输入时光标处提示只读原因
- [ ] 文件头 `#region` 默认展开，内含程序集名、版本、DLL 完整路径；可折叠
- [ ] 正文只有签名与文档注释，没有方法体；高亮与同主题下的 `.cs` 文件一致（附截图）
- [ ] `rove.metadataView.showDocs` 设为 false 后文档注释隐藏，改回后恢复，无需重开标签
- [ ] 面包屑显示 `程序集 › 命名空间 › 类型 › 成员`；`Ctrl+Shift+O` 可在视图内转到成员
- [ ] 视图内 `F12` 可跳到其他类型（至少一次落到源码、一次落到另一个元数据视图）；`Alt+F12` 可开速览；`Alt+←` 能退回源码
- [ ] 视图内 `Shift+F12` 列出工作区中使用该 API 的全部位置，计数与 [[router]] 的引用查询结果一致（在 `ROVE_TEST_UNITY_LARGE` 上对 `Transform.position` 实测并记录条数）
- [ ] 标签复用：连续跳到同一类型的三个不同成员，始终只有一个标签，且只滚动定位
- [ ] 嵌套类型跳转打开外层类型并定位到嵌套类型；泛型类型（`List<T>`）标题与定位正确
- [ ] 索引中没有的符号：状态栏临时提示“未找到 `Xxx` 的元数据”，没有空标签产生
- [ ] 新增的设置项、界面字符串、文档类型都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增集成测试（跳转 → 断言元数据文档已打开、定位到成员、只读、标签复用）

## 风险与提示

- 最省事也最正确的做法是**把元数据视图做成一种普通文档**：只读 `TextSource` + 已登记的编辑器类型。一旦为它单独写一条打开路径，标签复用、标签状态恢复、面包屑、导航历史都要再实现一遍，而且会和 [[groups]] 的语义不一致。
- “同一个类型只开一个标签”的判定键必须是索引中的符号 ID 或规范名称。用显示标题做键，`List<T>` 与 `List<String>` 会被当成两个类型，或者反过来被错误合并。
- `Shift+F12` 在元数据视图中列出工作区用法是本 issue 的亮点功能，也是最容易做慢的地方：`Transform.position` 这类 API 在大型 Unity 项目里的引用数可能上万，走的是 [[peek]] 的 > 100 折叠规则，不要在本 issue 里另写一个列表。
- 文档注释里可能有 `<see cref="…"/>` 这类标签。首版按纯文本展示（与 C# Dev Kit 的 `///` 形式一致）即可，不要顺手做成可点击链接——那属于另一个 issue 的范围，想做要先改 `UX_SPEC.md` §4.1。
- 元数据内容来自索引，索引重建时内容可能变化。标签要能在索引原子替换后刷新（订阅 [[idx-life]] 的索引版本变更），而不是一直显示旧内容。
