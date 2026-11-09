## 背景

`FEATURES.md` §6 对全局替换只写了一句话，但这句话定了全部基调：**带预览，可以一次撤销**。跨文件批量改写是 Rove 里唯一一个“一次操作能同时写几十个文件”的功能，它必须比其他任何功能都更保守：先让人看清楚要改什么，改完还能整体回退。

[[search]] 已经把搜索引擎、流式结果、按文件分组的结果树做好了，并且**刻意没有注册任何替换相关的命令与界面元素**（见该 issue“不包含”一节）。本 issue 在它之上补齐替换输入框、替换预览与批量应用。

写入一律经 [[save]] 的写入器（M3 已合并）：编码、BOM、混合换行符原样保持，原子写入，占用与只读按 `UX_SPEC.md` §6.4 处理。本 issue **不自己写文件**。

必读：`FEATURES.md` §6（全局替换一行），`UX_SPEC.md` §3.1（文件大小分级：超大文件只读、关闭替换）、§3.4（只读来源）、§6.1（反馈分级）、§6.4，`docs/compat/KEYBINDINGS.md`（搜索一节中的 `search.action.replace*` 与 `workbench.action.replaceInFiles`、查找替换一节中的 `toggleSearchPreserveCase`），`SETTINGS.md`（`search.useReplacePreview`、`search.maxResults`、`search.smartCase`）。

## 目标

### 1. 替换界面

- 搜索视图顶部的替换输入框（由搜索框左侧的展开箭头切换），带“全部替换”按钮与“保留大小写”开关，外观照搬 VS Code。
- 命令：`workbench.action.replaceInFiles`（`ctrl+shift+h`，打开搜索视图并展开替换框）、`closeReplaceInFilesWidget`（`escape`，when `replaceInputBoxFocus && searchViewletVisible`）、`toggleSearchPreserveCase`（`alt+p`，when `searchViewletFocus`）。
- 上下文键：`replaceActive`（替换框有内容）、`replaceInputBoxFocus`、`isEditableItem`（结果项所属文件可写）。
- 颜色键复用 [[search]] 与 [[lists]] 已登记的 `input.*`、`inputOption.active*`、`list.*`，不新增颜色键。

### 2. 预览

- `search.useReplacePreview`（默认 true）：结果树中的每一处匹配直接显示“旧文本 → 新文本”的行内对照（删除线 + 新内容），不需要打开文件就能看清。
- 单条结果的操作：`search.action.replace`（`ctrl+shift+1`，替换这一处）、`search.action.replaceAllInFile`（`ctrl+shift+1` / `ctrl+shift+enter`，when `fileMatchFocus`）、`search.action.replaceAllInFolder`（同键位，when `folderMatchFocus`）。
- 用户可以先用 `search.action.remove`（[[search]] 已实现）从结果树里剔除不想改的项，被剔除的项不参与替换。
- 应用前在按钮旁显示汇总：“将在 N 个文件中替换 M 处”。

### 3. 批量应用与一次撤销

- `search.action.replaceAll`（`ctrl+alt+enter`，when `replaceActive && searchViewletVisible && !findWidgetVisible`）：按文件分组，每个文件一次多区间事务（[[edit-core]]），全部文件作为**一个批量编辑单元**记录。
- 撤销：应用后按 `UX_SPEC.md` §6.1 第 1 级在状态栏提示“已在 N 个文件中替换 M 处”，并提供一次性的整体撤销入口（照搬 VS Code 的“撤销”按钮语义）。撤销要同时覆盖**已打开**与**未打开**的文件：未打开的文件按批量编辑记录回写原内容。
- 已在编辑器中打开且有未保存修改的文件：替换作用在缓冲上并保持脏状态，不强制保存；未打开的文件由 [[save]] 的写入器直接写盘。
- 失败处理：某个文件写入失败（占用、只读属性）时**不回滚已成功的部分**，而是在汇总中列出失败文件并保留它们在结果树中，用户可重试；按 `UX_SPEC.md` §6.4 的“占用 / 只读属性”分支给出通知，同一原因只弹一次。

### 4. 跳过不可写的文件

- 只读（`files.readonly*`、`rove.files.extraReadonlyInclude` 命中的 `Library/PackageCache/**`）与超大文件（≥ `rove.largeFile.threshold`，`UX_SPEC.md` §3.1 明确关闭替换）**不参与替换**：结果树中这些文件的替换操作不可用（`isEditableItem` 为 false），预览仍然显示。
- 应用后给出一句汇总：“已跳过 N 个只读或过大的文件”，点击可在结果树中定位它们。这是第 1 级反馈，不弹通知。

### 5. 替换语义

正则捕获组（`$1`…`$9`、`$0`、`$&`、`$$`）与保留大小写的规则，与 [[replace]] 在当前文件内的实现**完全一致**——复用同一份实现，不写第二套。查找侧遵循 [[search]] 已实现的 `search.smartCase`、正则、全词、include / exclude 与 `search.maxResults` 截断（截断时禁止“全部替换”，并说明原因）。

### 6. 登记与替换

新增命令、设置项、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。[[skeleton]] 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行。不得在本 PR 中修改 [[contracts]]、[[search]]、[[save]] 的接口（需要扩展时走契约变更流程单独提 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 搜索引擎、流式结果、结果树、三个查找开关、include / exclude、`search.action.remove` 等（[[search]]）。
- 当前文件内的替换条（[[replace]]，M3）；本 issue 复用它的替换语义实现。
- 文件写入、编码保持、原子写入、占用重试、保存冲突（[[save]]）。
- 搜索编辑器与其中的替换（`searchEditor`、`selectAllSearchEditorMatches` 等，P2）。
- 重命名重构（`FEATURES.md` §4：**不做**）——跨文件改名请用全局替换，界面上不提供任何“重命名符号”入口。
- 本地历史作为误操作的安全网（P2）。
- `addCursorsAtSearchResults`（从搜索结果放多光标，归 [[multicursor]]）。

## 验收标准

- [ ] `Ctrl+Shift+H` 打开搜索视图并展开替换框；`Escape` 关闭替换框但保留搜索结果
- [ ] 预览：`search.useReplacePreview` 为 true 时结果树显示“旧 → 新”行内对照，为 false 时不显示；预览内容与实际替换结果逐条一致
- [ ] 单条替换、按文件替换、按文件夹替换、全部替换四个入口各有用例；被 `search.action.remove` 剔除的项不参与替换
- [ ] **一次撤销**：在 ≥ 10 个文件中替换后整体撤销，**全部文件逐字节回到替换前**（含未打开的文件）
- [ ] 编码保持：GBK + CRLF、UTF-8 带 BOM、混合换行符三类文件参与同一次批量替换，除匹配处外字节完全一致
- [ ] 已打开且有未保存修改的文件：替换后仍为脏状态、未被强制保存，内容正确
- [ ] 跳过规则：`Library/PackageCache/**` 下的文件与超大文件不被修改，`isEditableItem` 为 false，应用后出现“已跳过 N 个只读或过大的文件”的汇总
- [ ] 失败处理：用持有独占句柄的辅助进程锁住其中一个文件，断言其余文件替换成功、失败文件保留在结果树中、同一原因只弹一次通知
- [ ] `search.maxResults` 截断时“全部替换”被禁用并说明原因
- [ ] 正则捕获组与保留大小写的结果与 [[replace]] 在同一组样例上**完全一致**（交叉断言，样例共用）
- [ ] 在大型 Unity 项目（`ROVE_TEST_UNITY_LARGE`）上替换一个常见标识符：预览可滚动不卡顿，应用过程可取消（`UX_SPEC.md` §6.2：超过 1 秒的操作必须可取消），附耗时数据
- [ ] 新增的命令、设置项、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增全局替换入口的集成测试

## 风险与提示

- **这是整个项目里最容易造成数据损失的功能**。宁可少做一个入口，也不要让“全部替换”在用户没看过预览的情况下跑起来。失败时不回滚已成功的部分是刻意的选择（回滚本身也可能失败，反而更难解释），但必须把失败清单摆在用户面前。
- 跨文件撤销要靠**自己记录的批量编辑**，不能指望各文件的撤销栈：未打开的文件根本没有撤销栈，而打开过又关闭的文件撤销栈已经没了。
- 不要在替换时把未打开的文件全部读进内存。按文件流式处理，并复用 [[search]] 已有的“先按字节快速判定编码、只有疑似非 UTF-8 才走完整识别”的优化（见该 issue 的风险提示），否则大型项目上内存会爆。
- 替换语义必须与 [[replace]] 共用实现。两处各写一遍，一定会在零宽匹配与保留大小写上分叉，而用户会在同一个项目里交替使用这两个入口。
- KEYBINDINGS 中这组命令的优先级并不统一（`workbench.action.replaceInFiles` 是 P1，而 `closeReplaceInFilesWidget`、`toggleSearchPreserveCase` 继承了搜索区域的 P0）。以 `FEATURES.md` §6 的“全局替换 P1”为准，整组功能一起在 M4 交付。
