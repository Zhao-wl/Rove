## 背景

D4 决定与 VS Code 共用 `keybindings.json`，`FEATURES` §7 进一步要求 **Rove 的命令 ID 直接采用 VS Code 的命令 ID**（如 `editor.action.revealDefinition`），否则用户的自定义键位无法原样生效。`docs/compat/KEYBINDINGS.md` 已经把 VS Code 1.145.2 的 1269 条默认键位逐条判定完毕（实现 P0 435 条、P1 82 条、P2 74 条），并列出了 Rove 专有命令（§5）与需要支持的 `when` 上下文键（§6）。

实现放在 `rove-commands`，在 [[contracts]] 定义的命令登记、上下文键与键位解析接口背后；本 PR 不修改这些接口。替换 [[skeleton]] 中的 `SKELETON(keys)` 标记（"登记 2–3 个命令，用硬编码的键位触发"），并更新 `docs/ARCHITECTURE.md` 的对照表。

## 目标

### 1. 共用 `keybindings.json`

与 `settings.json` 同目录（目录解析复用 [[settings]]：环境变量 `ROVE_VSCODE_USER_DIR` 或启动参数 `--vscode-user-dir`）。JSONC 解析、热加载。命令 ID 在 Rove 中不存在时**忽略该条**，不报错（`FEATURES` §13 规则 7）。

### 2. 键表达式解析

- 修饰键 + 主键：`ctrl+shift+p`、`alt+f12`；
- **和弦**：`ctrl+k ctrl+s`，含中间态（按下第一段后的待定状态）与超时 / 取消行为，与 VS Code 一致；
- **Windows 虚拟键名**：`oem_2`、`oem_5`、`oem_comma`、`oem_period`、`numpad_*` 等（导出文件里大量出现，与键盘布局有关）；
- 按 VS Code 的 `keyboard.dispatch` 语义解析（默认按 `code` 派发），保持与 VS Code 一致（`KEYBINDINGS.md` §1）。

### 3. `when` 表达式解析器与求值器

- 运算符：`&&`、`||`、`!`、`==`、`!=`、`>=` 等比较、`=~`（**正则字面量**，如 `resource =~ /\.md$/`）、`in`、括号；
- **不认识的上下文键一律视为 false**（`FEATURES` §13 规则 7）；
- 提供上下文键的设置与订阅接口（[[contracts]]），至少实现 `KEYBINDINGS.md` §6 中落在 Rove 功能范围内的上下文键（`editorTextFocus`、`editorFocus`、`editorReadonly`、`inQuickOpen`、`listFocus`、`filesExplorerFocus`、`foldingEnabled`、`findWidgetVisible`、`searchViewletVisible`、`breadcrumbsVisible`、`referenceSearchVisible` 等）；属于不做功能的上下文键不实现，自然求值为 false。

### 4. 默认键位表

从 `docs/compat/KEYBINDINGS.md` 的数据生成（P0 行先行，P1/P2 随对应功能启用）。**最好直接扩展 `scripts/compat/`**：从 `docs/compat/vscode/default-keybindings.json` 按 `scripts/compat/rules.py` 的判定过滤，生成一份机器可读的默认键位表（键、命令、when、优先级）进仓库，并在 CI 中校验生成物与源数据一致。435 条不要手抄。

### 5. 命令登记表

- 命令 ID 一律沿用 VS Code；**全部通过 [[contracts]] 的命令登记机制注册**，不允许写死；
- Rove 专有命令按 `KEYBINDINGS.md` §5 登记（`rove.contextBasket.add` = `ctrl+k b` / `editorTextFocus`、`rove.contextBasket.focus` = `ctrl+shift+alt+b`、`rove.editor.followTail`、`rove.editor.openLongLine`、`rove.json.minify`、`rove.structure.copy*`、`rove.index.*` 等）。新增 Rove 命令时必须与 VS Code 默认键位做冲突检查，并避开 `ctrl+alt+a`；
- 本 issue 只提供注册、参数传递与分发，命令的**实现**在各自 issue 中。未实现的命令不进命令面板（`UX_SPEC` §1.7），因此登记时要能区分"已登记未实现"与"已实现"。

### 6. 按键 → 命令的解析

同一个键有多条规则时按 VS Code 的规则裁决：后定义优先、用户键位覆盖默认键位、`"command": "-xxx"` 形式移除某条默认键位、`when` 不满足时继续往下找。

## 不包含

- 命令面板与快速打开界面（[[quickopen]]）、图形化键位编辑器（P2）。
- GPUI 的实际键盘事件接入与焦点管理（[[skeleton]]、[[shell]]）。
- 各命令的具体行为。

## 验收标准

- [ ] 键表达式解析有单元测试：修饰键组合、和弦、`oem_*` 虚拟键名、大小写、非法写法的报错
- [ ] 和弦的中间态、超时、按 `Esc` 取消有测试
- [ ] `when` 表达式求值有一组移植自 VS Code 的用例，覆盖 `&&`/`||`/`!`/比较/`=~` 正则字面量/`in`/括号/优先级
- [ ] 未知上下文键求值为 false，有测试
- [ ] 用户键位覆盖默认、`-command` 移除默认、同键多规则的裁决顺序，各有测试
- [ ] 默认键位表由脚本从 `docs/compat/vscode/default-keybindings.json` 生成，CI 校验生成物与源数据一致
- [ ] P0 的 435 条默认键位全部进入默认键位表，并有一个对照 `KEYBINDINGS.md` 的测试防止漏项
- [ ] `KEYBINDINGS.md` §5 的 Rove 专有命令全部登记，且有一个"与 VS Code 默认键位无冲突"的自动检查
- [ ] 命令 ID 不存在时忽略该条键位，不产生用户可见错误
- [ ] `SKELETON(keys)` 标记已删除，`docs/ARCHITECTURE.md` 的对照表已更新
- [ ] 新增集成测试覆盖"读 keybindings.json → 按下某个键 → 分发到命令"；[[skeleton]] 的端到端测试继续通过
- [ ] `cargo tree -p rove-commands -i gpui` 为空

## 风险与提示

- **`KEYBINDINGS.md` §6 的上下文键清单里混进了 `c`、`A`、`Users`、`zhaowenlong`、`AppData`、`Roaming`、`Code`、`User`、`keybindings.json` 这些片段**。它们来自把 `resource =~ /c:/Users/.../AppData/Roaming/Code/User/keybindings.json/` 这样的**正则字面量**按分隔符切开的结果。两个结论：①`when` 解析器必须先把 `/.../ ` 正则字面量当成一个整体 token；②生成脚本已经修正（先去掉字符串与正则字面量再提取上下文键），`KEYBINDINGS.md` §6 已重新生成。`when` 解析器必须支持 `=~` 运算符。
- `keyboard.dispatch` 在非 US 布局上会影响 `oem_*` 键的实际按键。Rove 首发只做 Windows，建议与 VS Code 的默认行为完全一致，并在 `ROVE` 侧不额外做布局推断；若实测发现中文键盘上某些键对不上，在 PR 中如实记录而不是自行"修正"。
- 命令登记表会被几乎所有界面 issue 追加。登记入口要稳定、追加友好（不要求集中在一个大数组里），否则会成为合并冲突的热点。
