## 背景

`Ctrl+P` 是阅读代码时最高频的入口，它的**排序结果是用户的肌肉记忆**：同样的输入在 Rove 里排出不一样的第一条，就等于迁移失败。所以这里不能用 `nucleo` 之类的现成库近似（早期草稿中的 `nucleo` 方案作废），必须把 VS Code 的打分算法逐条移植过来。

必读：`FEATURES.md` §6、§11，`UX_SPEC.md` §1.2（命令中心）、§1.7（命令面板只列已实现命令）、§6.2（加载）、§6.3（空状态），`docs/compat/KEYBINDINGS.md`、`SETTINGS.md`。

性能基准是大型 Unity 测试项目（`ROVE_TEST_UNITY_LARGE`，约 2.1 万个 `.cs` 文件外加其他文件）：**每次按键到列表刷新 < 16 ms**。

## 目标

### 1. 移植 VS Code 的模糊匹配算法（核心层）

- 移植 `src/vs/base/common/fuzzyScorer.ts`：`scoreFuzzy`、`scoreFuzzy2`、`scoreItemFuzzy`（label 与 description/路径分开打分再合并）、`compareItemsByFuzzyScore`，以及匹配字符的位置（供列表高亮）。
- **同时移植它的单元测试**（`fuzzyScorer.test.ts`）作为行为一致性测试，逐条对照（`PLAN.md`「与 VS Code 的行为一致性测试」）。
- 算法放在核心层 crate（归属以 [[contracts]] 的 `docs/ARCHITECTURE.md` 为准），不得依赖 gpui；来源与 MIT 许可登记到 `THIRD_PARTY_NOTICES.md`。
- 候选集来自 [[workspace]] 的文件列表，文件增删事件触发增量更新。

### 2. 快速打开（`Ctrl+P`）

- 命令 `workbench.action.quickOpen`（`ctrl+p`、`ctrl+e`），通过 [[keys]] 的命令登记表注册，不得硬编码键位。
- 结果项显示文件名 + 相对路径，匹配字符高亮；颜色取自登记过的 `list.filterMatchBackground`、`list.highlightForeground`、`quickInput*` 等颜色键。
- 最近打开文件加权：遵循 `search.quickOpen.includeHistory`、`search.quickOpen.history.filterSortOrder`、`search.quickOpen.includeSymbols`（默认 false）、`workbench.quickOpen.closeOnFocusLost`、`workbench.quickOpen.preserveInput`。
- 工作区扫描未完成时**立即可用**，列表底部显示「正在加载文件列表…」（UX_SPEC §6.2）。
- 打开文件走 [[groups]] 的打开文档服务；从这里打开 `.md` 时传递“来源是文件树 / 快速打开”的语义，由 [[markdown]] 决定打开预览（UX_SPEC §4.3）。

### 3. 四个前缀

| 前缀 | 行为 | 说明 |
|---|---|---|
| `:` | 转到行、列 | 与 `workbench.action.gotoLine`（`ctrl+g`）共用实现，支持 `文件:行:列` |
| `@` | 文件中的符号 | 符号数据来自 [[outline]]；[[outline]] 尚未合并或当前文件类型没有符号时，显示「活动编辑器无法提供大纲信息。」（UX_SPEC §6.3），不报错、不留空白 |
| `#` | 工作区符号 | 需要 L1 索引与 [[nav-extra]]，M1 中还没有：列表中显示一行「工作区符号搜索尚不可用」，按 §6.3 的空状态措辞风格写，并提示改用 `Ctrl+P` |
| `>` | 命令 | 与命令面板同一套结果 |

### 4. 命令面板（`Ctrl+Shift+P` / `F1`）

- `workbench.action.showCommands`；**只列出已登记且已实现的命令**（UX_SPEC §1.7），命令 ID 一律沿用 VS Code。
- 每条显示 [[keys]] 解析出的实际键位，包含用户在共用 `keybindings.json` 中的覆盖。
- 分「最近使用」和「其他命令」两组；`when` 不满足的命令直接不显示，不置灰（UX_SPEC §1.2）。

### 5. 命令中心

标题栏中间的命令中心点击后打开快速打开（UX_SPEC §1.2）。标题栏由 [[shell]] 绘制，本 issue 只提供它调用的入口并登记这个行为。

### 6. 组件与登记

浮层、列表、键盘导航全部复用 [[lists]] 的快速输入组件。新增的命令、设置项、颜色键、界面字符串一律通过 [[contracts]] 的登记机制加入，不写死。替换 [[skeleton]] 中的 `SKELETON(quickopen)`（骨架里 `workbench.action.quickOpen` 只弹“未实现”提示），删除标记并更新 `docs/ARCHITECTURE.md` 的对照表。

## 不包含

- 工作区符号搜索（`#`、`Ctrl+T` / `workbench.action.showAllSymbols`）：[[nav-extra]]。
- 编辑器选择器（`Ctrl+Tab`）、视图选择器（`Ctrl+Q`）属于 [[groups]] 与 [[shell]]；最近打开（`Ctrl+R` / `workbench.action.openRecent`）属于 [[welcome]]。
- 全局搜索：[[search]]。大纲视图与面包屑：[[outline]]。
- 修改 [[contracts]] 的接口：需要改时按契约变更流程单独提小 PR，不在本 PR 中顺手改。

## 验收标准

- [ ] `fuzzyScorer.ts` 的单元测试全部移植并通过（PR 中给出条目数；跳过的用例逐条说明原因）
- [ ] `ROVE_TEST_UNITY_LARGE` 上每次按键到列表刷新 < 16 ms（p99，附测量数据，基准接入 [[perf]]）
- [ ] 四个前缀都可用；`@` 无数据与 `#` 不可用时的降级提示符合 UX_SPEC §6.3
- [ ] 命令面板只列出已实现的命令，显示的键位与共用 `keybindings.json` 中的覆盖一致
- [ ] 点击标题栏命令中心可以打开快速打开
- [ ] 全键盘可用：`↑↓`、`Enter`、`Ctrl+Enter`（在旁边打开）、`Esc`
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] `SKELETON(quickopen)` 已删除，`ARCHITECTURE.md` 的对照表已更新
- [ ] [[skeleton]] 的端到端测试仍然通过；另加覆盖本 issue 入口的集成测试（`Ctrl+P` → 输入 → 选中 → 断言文档已打开并定位）
- [ ] 无运行时网络访问；不读取 `.gitignore`

## 风险与提示

- label/description 双段打分加上 VS Code 的打分缓存是性能关键：2 万多个候选项下必须复用上一次按键的结果做前缀剪枝，否则 16 ms 预算守不住。
- `@` 前缀的符号数据来自 [[outline]]（已列为前置依赖）。当前文件类型没有符号时，按 UX_SPEC §6.3 显示空状态。
- 排序上任何“我觉得这样更合理”的偏离都算 bug（UX_SPEC §0.1：没写到的地方一律与 VS Code 一致）。
