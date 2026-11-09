# R25 全局搜索：搜索引擎（search.* 语义、流式结果、取消、GBK）与搜索视图

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M1 只读阅读器 | 3 困难 | workspace | 第 4 批 | R15（工作区）、R07（文本缓冲）、R18（通用列表、树与快速输入组件）、R17（编辑器组与标签） | R31（【集成验收】M1）、R61（全局替换） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

全局搜索是阅读代码时仅次于快速打开的入口。本 issue 是一个**纵向条目**：搜索引擎只服务搜索视图这一个界面，所以引擎和界面放在一起做，避免接口在两个 PR 之间来回调整。

必读：`FEATURES.md` §6，`UX_SPEC.md` §6.2（进度与取消）、§6.3（空状态），`docs/compat/SETTINGS.md`（`search.*`）、`KEYBINDINGS.md`（`search.action.*`）、`THEME_COLORS.md`。

性能基准（大型 Unity 测试项目 `ROVE_TEST_UNITY_LARGE`）：**首批结果 < 100 ms，常见标识符的全量搜索 < 2 s（热缓存）**。

## 目标

### 1. 搜索引擎（核心层，不依赖 gpui）

- 基于 `grep-searcher` + `grep-regex`，在 R15（工作区） 给出的文件集合上并行搜索；线程数遵循 `search.ripgrep.maxThreads`（0 为自动）。
- 排除规则：`search.exclude` + `files.exclude` + Rove 的附加排除 `rove.files.extraExclude`（合并生效，不改写用户的 VS Code 设置，FEATURES §13 规则 5）；`search.followSymlinks`。
- **`search.useIgnoreFiles`、`search.useParentIgnoreFiles`、`search.useGlobalIgnoreFiles` 一律忽略**：Rove 从不读取 `.gitignore`（FEATURES §6，SETTINGS.md §3）。
- 查询选项：正则、大小写敏感、全词匹配，以及 `search.smartCase`（全小写时不区分大小写）。
- 结果以**流**的形式按文件分组返回：文件路径、每处匹配的行号、行文本预览、匹配区间（UTF-8 字节区间，供界面高亮）。
- 取消：用户继续输入或点击取消时，**50 ms 内**所有工作线程停止（`search.searchOnType`、`search.searchOnTypeDebouncePeriod` 控制何时发起下一次）。
- 编码：通过 R07（文本缓冲） 的编码识别读取，GBK 文件中的中文必须能搜到（`files.autoGuessEncoding` 在 Rove 默认为 true）。
- 结果上限 `search.maxResults`（默认 20000），达到上限时停止并向界面报告“已达到上限”。

### 2. 搜索视图（侧边栏）

- 活动栏中的搜索视图（`workbench.view.search`）；`Ctrl+Shift+F` / `workbench.action.findInFiles`；`search.mode` 为 `view` 时在侧边栏显示（搜索编辑器是 P2，不做）。
- 查询框 + 三个开关：`toggleSearchCaseSensitive`（`alt+c`）、`toggleSearchWholeWord`（`alt+w`）、`toggleSearchRegex`（`alt+r`）；“包含的文件”“排除的文件”两个 glob 输入框，以及“使用排除设置”开关。
- 结果按文件分组，可折叠（`search.collapseResults`），显示匹配计数徽章（`search.decorations.badges`、`search.decorations.colors`、`search.showLineNumbers`、`search.sortOrder`）；列表与虚拟化来自 R18（通用列表、树与快速输入组件）。
- 键位（全部沿用 VS Code 的命令 ID 与默认键）：`search.action.openResult`（`enter`）、`openResultToSide`（`ctrl+enter`）、`focusNextSearchResult`（`f4`）/ `focusPreviousSearchResult`（`shift+f4`）、`remove`（`delete`）、`copyMatch`（`ctrl+c`）、`copyPath`（`shift+alt+c`）、`restrictSearchToFolder`（`shift+alt+f`）、`cancel`（`escape`）、`clearSearchResults`、`focusSearchFromResults`。
- 打开结果走 R17（编辑器组与标签） 的打开文档服务，并选中匹配区间；`.md` 从搜索结果打开的是**源码并定位**，不是预览（UX_SPEC §4.3）。
- 反馈：视图顶部进度条、结果随流逐步出现、随时可取消（UX_SPEC §6.2）。
- 空状态（UX_SPEC §6.3）：「未找到结果。」+「查看排除设置」链接；当 Rove 的默认排除规则影响了结果时，额外显示「已排除 Unity 生成的目录」和「包含排除的文件再搜索一次」。
- 颜色键全部取自登记表（`editor.findMatchHighlightBackground`、`search.resultsInfoForeground`、`list.*` 等）。

### 3. 登记与替换

命令、设置项、颜色键、界面字符串、视图本身都通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不写死。替换 R02（【接口先行 A2】行走骨架） 中与搜索相关的 `SKELETON(search)` 标记（如有），删除标记并更新 `docs/ARCHITECTURE.md` 的对照表。

## 不包含

- **文件中替换**：`search.action.replace*`、`workbench.action.replaceInFiles`（`Ctrl+Shift+H`）、替换输入框与替换开关，全部属于 R61（全局替换）（M4）。本 issue 中替换相关的界面元素直接不显示（UX_SPEC §1.2）。
- 当前文件查找条（`Ctrl+F`）：R21（当前文件查找条）；超大文件中的流式查找：R22（超大文件与超长行的界面）。
- 搜索编辑器（P2）、`search.action.searchWithAI`（Rove 不做 AI）、本地历史、问题面板。
- 从搜索结果加入上下文篮子：R65（上下文篮子）。
- 修改 R01（【接口先行 A1】架构与接口契约） 的接口（需要改时走契约变更流程，单独提小 PR）。

## 验收标准

- [ ] `ROVE_TEST_UNITY_LARGE` 上搜索一个常见标识符：首批结果 < 100 ms，全部完成 < 2 s（热缓存，附测量数据，基准接入 R04（性能基准与预算守护））
- [ ] 取消后 50 ms 内所有工作线程停止（测试断言）
- [ ] 结果超过 1 万条时列表滚动依然流畅（虚拟化）
- [ ] 测试覆盖：正则、全词、`search.smartCase`、include/exclude glob、`search.maxResults` 截断、GBK 文件中的中文
- [ ] 仓库中存在 `.gitignore` 时搜索结果不受其影响（显式测试）
- [ ] 空状态与「已排除 Unity 生成的目录」「包含排除的文件再搜索一次」符合 UX_SPEC §6.3
- [ ] 点击结果在编辑器中定位并选中匹配；`.md` 结果打开的是源码
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；另加覆盖本 issue 入口的集成测试（输入查询 → 断言结果分组与首条位置）
- [ ] 无运行时网络访问

## 风险与提示

- 取消的 50 ms 预算要求把取消令牌传到每个工作线程的内层循环，而不是只在文件之间检查。
- 编码识别对搜索是额外开销：先按字节快速判定（BOM、是否合法 UTF-8），只有疑似非 UTF-8 的文件才走 R07（文本缓冲） 的完整识别，否则 2 s 预算守不住。
- `search.exclude` 与 `rove.files.extraExclude` 的合并语义要和 R15（工作区） 保持同一套实现，不要在搜索里另写一份 glob 匹配。
