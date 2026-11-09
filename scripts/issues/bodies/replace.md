## 背景

[[findwidget]] 已经按 VS Code 实现了浮动查找条，并且**刻意为替换留好了位置**：布局与状态机预留了替换行、替换相关的命令一律没有注册、超大文件级别下隐藏替换按钮的能力已经做好（见该 issue 第 5 节与“不包含”）。本 issue 把这块空位填上。

替换是 M3 中最小但最容易被用户立刻察觉的一块：`Ctrl+H` 是肌肉记忆，“全部替换”之后按一次 `Ctrl+Z` 就要全部回去——这依赖 [[edit-core]] 的“一次多区间事务 = 一个撤销单元”。

必读：`FEATURES.md` §4（查找 / 替换一行，P0，含保留大小写），`UX_SPEC.md` §3.1（超大文件关闭替换）、§3.2（超大文件查找条中隐藏替换按钮），`docs/compat/KEYBINDINGS.md`（查找替换一节），`SETTINGS.md`（`editor.find.replaceHistory`），`THEME_COLORS.md`（Editor widget colors、Input control）。

## 目标

### 1. 替换界面

- 查找条左侧的展开箭头切换替换行的显示；替换行包含替换输入框、“替换”“全部替换”两个按钮和“保留大小写”开关，外观与位置照搬 VS Code。
- 颜色键复用 [[findwidget]] 已登记的 `editorWidget.*`、`input.*`、`inputOption.active*`；本 issue 不新增颜色键。
- 上下文键 `replaceInputFocussed`（[[findwidget]] 明确没有注册，由本 issue 补上）。

### 2. 命令与键位

- `editor.action.startFindReplaceAction`（`ctrl+h`，when `editorFocus || editorIsOpen`）：打开查找条并展开替换行，种子文本规则与 `ctrl+f` 一致。
- `editor.action.replaceOne`（`ctrl+shift+1`，以及替换输入框内的 `enter`，when `editorFocus && findWidgetVisible && replaceInputFocussed`）。
- `editor.action.replaceAll`（`ctrl+alt+enter`，when `editorFocus && findWidgetVisible`）。
- `togglePreserveCase`（`alt+p`，when `editorFocus`）。
- 替换历史：`editor.find.replaceHistory`（默认 `workspace`），用 [[lists]] 的输入历史导航，与查找历史各自独立。

### 3. 替换语义

- 替换“当前匹配”后自动前进到下一个匹配，匹配列表与计数同步更新（查找模型来自 [[findwidget]]，本 issue 不重新实现匹配查找）。
- 正则替换支持捕获组引用 `$1`…`$9`、`$0`、`$&` 与 `$$` 转义，语义与 VS Code（JavaScript 正则）一致；非法替换串按 VS Code 的处理方式原样插入，不报错。
- **保留大小写**（`togglePreserveCase`）照搬 VS Code 的规则：全大写 → 替换串全大写、首字母大写 → 首字母大写、其余保持替换串原样；与正则模式可同时开启。
- 在“在选区内查找”范围生效时，替换只作用于该范围。

### 4. 全部替换 = 一次撤销

把全部匹配收集成**一组区间**，交给 [[edit-core]] 做一次多区间事务；撤销一次即全部回退，光标与选区回到替换前。替换过程中不逐条刷新界面，完成后一次性更新匹配计数与装饰；替换后按 VS Code 把匹配清零并保留查找串。

### 5. 可写性判定

替换只在**可写文档**上可用：超大文件（`UX_SPEC.md` §3.1，只读）、元数据视图、`files.readonly*` 与 `rove.files.extraReadonlyInclude` 命中的文件，按 `UX_SPEC.md` §1.2“未实现 / 不可用的项直接不显示”隐藏整个替换行（不是显示为灰色），并且 `ctrl+h` 只打开查找条、不展开替换行。判定来自 [[workspace]] 的只读规则与文档的大小级别，本 issue 不自己探测。

### 6. 登记与替换

新增命令、设置项、界面字符串通过 [[contracts]] 的登记机制加入，不得写死。[[skeleton]] 中没有本 issue 对应的 `SKELETON(...)` 标记，本 issue 只在 `docs/ARCHITECTURE.md` 中新增 / 更新自己的一行。不得在本 PR 中修改 [[contracts]] 与 [[findwidget]] 的查找模型接口（需要扩展时走契约变更流程单独提 PR）。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 查找条本身、查找模型、四个查找开关、匹配计数与导航（[[findwidget]]）。
- 跨文件的全局替换与替换预览（[[replace-all]]，M4）：`workbench.action.replaceInFiles`（`ctrl+shift+h`）、`search.action.replace*`、`closeReplaceInFilesWidget` 一律不在本 issue 注册。
- `editor.action.selectAllMatches`（`alt+enter`，需要多选区，[[multicursor]]）。
- 超大文件的流式查找与提示条（[[largefile-ui]]）；概览标尺上的匹配标记（[[minimap]]）。
- 格式化、缩进转换等其他“批量改写”（[[structure]]、[[edit-ui]]）。

## 验收标准

- [ ] `ctrl+h` 打开查找条并展开替换行；`ctrl+f` 打开时替换行默认收起，展开状态在同一窗口内保持
- [ ] `ctrl+shift+1` / 替换框内 `enter` 替换当前匹配并前进；`ctrl+alt+enter` 全部替换；`alt+p` 切换保留大小写
- [ ] **全部替换后一次 `ctrl+z` 完全回退**，文本与替换前逐字节相同（含 CRLF 与混合换行符文件）；再一次 `ctrl+y` 重做同样完整
- [ ] 正则替换用例：`$1` 捕获组、`$&`、`$$`、跨行 `\n`、非法替换串；与 VS Code 在同一组样例上结果一致（样例写入测试数据）
- [ ] 保留大小写用例：全大写、首字母大写、全小写、混合大小写四种源文本各一例
- [ ] “在选区内查找”开启时替换不越界；替换后选区范围正确收缩 / 扩张
- [ ] 10 万行文件中替换 **≥ 5000 处**：界面不卡顿（无长于两帧的掉帧）、完成后计数正确、可一次撤销（附耗时数据）
- [ ] 只读与超大文件：替换行不显示，`ctrl+h` 不展开替换行，替换命令执行后无任何修改
- [ ] 新增的命令、设置项、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；新增替换入口的集成测试（替换一处、全部替换、撤销各一个断言）

## 风险与提示

- **不要自己再写一遍匹配查找**。[[findwidget]] 的查找模型（匹配列表、当前索引、范围限制）被设计成可独立测试的部分，本 issue 只消费它；两套匹配逻辑一定会在正则边界上分叉。
- 全部替换的区间顺序必须从后往前应用（或由 [[edit-core]] 统一处理平移）。零宽匹配（如 `^`、`\b`、`a*`）会产生长度为 0 的区间，VS Code 对此有明确行为，必须对齐并写测试，否则容易死循环。
- 替换 5000 处时不要每替换一处就提交一次事务——那会产生 5000 个撤销单元，并击穿帧预算。
- “保留大小写”在正则模式下的行为细节容易偏差：VS Code 是按**匹配到的文本**而不是按查找串判断大小写形态。
- 替换行的显示 / 隐藏是 `UX_SPEC.md` §1.2 的“不显示而不是禁用”原则的一次具体应用：只读文件上宁可没有入口，也不要给一个按下去没反应的按钮。
