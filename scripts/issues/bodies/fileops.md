## 背景

[[explorer]] 把资源管理器做成了**只读**的文件树，并明确把文件操作留给本 issue（见该 issue 的「不包含」）。`FEATURES.md` §6 把「资源管理器中的文件操作」单列为照搬、P1：新建、重命名、删除（移到回收站）、复制路径、在系统资源管理器中打开。

这是 Rove 里**第一处会改动用户磁盘的功能**，而用户打开的往往是 Unity 工程这种随时可能被别的进程写入的目录。所以确认语义、回收站语义、撤销、只读规则、与文件监听的配合都必须与 VS Code 一致，不能自创。

必读：`FEATURES.md` §6（文件操作一行）、§13 规则 5；`UX_SPEC.md` §3.4（只读的五个来源）、§6.1（反馈分级：确认对话框是第 5 级，只用于会丢数据的情况）、§6.4（错误处理）；`docs/compat/KEYBINDINGS.md`「文件」一节与 §7 附录「P0 · 文件」；`docs/compat/SETTINGS.md`「资源管理器」一节（`explorer.*`）与「文件」一节（`files.enableTrash`）；`THEME_COLORS.md`（Lists and trees）；`PROJECT_CHARTER.md` §7（长路径、大小写不敏感路径）。

## 目标

### 1. 新建文件与文件夹

- 命令：`explorer.newFile`、`explorer.newFolder`、`workbench.files.action.createFileFromExplorer`、`workbench.files.action.createFolderFromExplorer`（均无默认键位，`KEYBINDINGS.md` §7 附录「P0 · 文件」），以及 `workbench.files.action.refreshFilesExplorer`。
- 交互照搬 VS Code：在目标目录下插入一个**就地编辑的空行**，键入名称后 `enter` 创建、`escape` 取消；名称中带 `/` 时自动创建中间目录；重名、非法字符、保留名（`CON`、`PRN` 等 Windows 设备名）在输入框下方给出一句话原因并阻止提交。
- 创建成功后通过 [[groups]] 的打开文档服务打开新文件（文件夹不打开），并保持树的选中项。

### 2. 重命名（就地编辑）

- `renameFile`（`f2`，when `filesExplorerFocus && foldersViewVisible && !explorerResourceIsRoot && !explorerResourceReadonly`）。
- 就地编辑框复用 [[lists]] 的树节点内编辑能力；默认选中不含扩展名的部分（与 VS Code 一致）。
- **只改大小写**（`Foo.cs` → `foo.cs`）必须成功：Windows 路径大小写不敏感，直接 `rename` 会被当成同名；按 VS Code 的做法走中间名两步重命名。
- 重命名已打开的文件时，标签、面包屑、导航历史、[[basket]] 的条目位置都要跟到新路径（通过 [[workspace]] 的文件事件，不各自探测）。

### 3. 删除到回收站

- `moveFileToTrash`（`delete`，when 含 `explorerResourceMoveableToTrash`）与 `deleteFile`（`shift+delete` 永久删除，以及回收站不可用时的 `delete`）。
- `files.enableTrash`（默认 true）为 true 时走 Windows 回收站（`IFileOperation` / `SHFileOperation`，带 `FOF_ALLOWUNDO`），为 false 时永久删除。回收站不可用（网络盘、超长路径、文件过大）时按 VS Code：降级为永久删除，并在确认框中写明「此文件无法移动到回收站，将被永久删除」。
- `explorer.confirmDelete`（默认 true）决定是否弹确认框；**永久删除一律确认**，且确认文案区分「移到回收站」与「永久删除」。确认框是 §6.1 第 5 级模态框，带「不再询问」（勾选后写入 `explorer.confirmDelete=false`，通过 [[settings]] 的保留格式写入）。
- 删除多选项时合并为一次确认与一次操作，文案给出数量。

### 4. 复制路径与在系统资源管理器中显示

- `copyFilePath`（`shift+alt+c`）、`copyRelativeFilePath`（`ctrl+k ctrl+shift+c`）：两条都有 `editorFocus` 与 `!editorFocus` 两份键位登记，作用对象分别是活动编辑器与树中的选中项。
- `workbench.action.files.copyPathOfActiveFile`（`ctrl+k p`，P0）同样在本 issue 登记。
- 分隔符遵循 `explorer.copyPathSeparator` / `explorer.copyRelativePathSeparator`（`auto` 在 Windows 上是 `\`）；多选时每行一条。
- `revealFileInOS`（`shift+alt+r`）与 `workbench.action.files.revealActiveFileInWindows`（`ctrl+k r`，P0）：调用 `explorer.exe /select,`，路径正确转义，**不经过 shell 字符串拼接**。

### 5. 剪切、复制、粘贴文件

`filesExplorer.copy`（`ctrl+c`）、`filesExplorer.cut`（`ctrl+x`）、`filesExplorer.paste`（`ctrl+v`）、`filesExplorer.cancelCut`（`escape`），上下文键 `explorerResourceCut`（剪切后的节点变淡，用 `list.deemphasizedForeground`）。与系统剪贴板互通（`CF_HDROP`），从资源管理器复制进来的文件按 `explorer.confirmPasteNative`（默认 true）确认。粘贴重名时按 `explorer.incrementalNaming`（`simple` / `smart` / `disabled`）生成新名称，三种取值都要有测试。

### 6. 拖放移动与撤销

- `explorer.enableDragAndDrop`（默认 true）在树内部拖动即移动；跨目录移动前按 `explorer.confirmDragAndDrop`（默认 true）确认。拖放的事件与绘制来自 [[lists]]，本 issue 只实现「移动文件」这一业务语义。
- 从系统资源管理器拖入时按 `explorer.autoOpenDroppedFile`（默认 true）决定是否打开。
- `explorer.enableUndo`（默认 true）与 `explorer.confirmUndo`（默认 `default`）：新建、重命名、移动、粘贴、删除到回收站都可以用 `undo` 撤销（回收站删除通过还原实现）；**永久删除不可撤销**，确认框中写明。撤销栈只记录本 issue 发起的操作，不与编辑器的撤销栈混用。

### 7. 只读规则与文件监听

- 操作前查询 [[workspace]] 的只读规则（含 `rove.files.extraReadonlyInclude` 的 `Library/PackageCache/**`）：只读资源的重命名、删除、粘贴入口全部禁用（上下文键 `explorerResourceReadonly`），不是先做再报错。
- 所有变更都**依赖 [[workspace]] 的文件事件刷新树**，不在操作后手工改树的数据，避免与外部改动的事件打架。
- 失败路径按 `UX_SPEC.md` §6.4：被占用（Unity 正在导入）、无权限、路径过长，各给出通知与「重试」，文案先说发生了什么再说能做什么。

### 8. 登记与替换

新增命令、设置项的读取、颜色键、界面字符串、右键菜单项全部通过 [[contracts]] 与 [[shell]] 的登记机制加入，不得写死；[[explorer]] 的右键菜单在本 issue 中补齐这些项。[[skeleton]] 中没有本 issue 对应的 `SKELETON(...)` 标记，只在 `docs/ARCHITECTURE.md` 中新增自己的一行。不得在本 PR 中修改 [[contracts]] 的接口。GPUI 类型只出现在 `rove-ui` / `rove-app`。

## 不包含

- 文件树本身、紧凑文件夹、过滤、装饰（[[explorer]]）；树组件与拖放绘制（[[lists]]）；文件事件与只读规则（[[workspace]]）。
- **无标题文档**：`workbench.action.files.newUntitledFile`（`ctrl+n`，P0）与 `workbench.action.files.newFile`（P1）由 [[save]] 负责（未命名文档）。本 issue 只做磁盘上的新建。
- `explorer.openWith`（P0，选择打开方式）：属于编辑器类型解析，归 [[groups]] 的登记表；本 issue 不注册。
- `explorer.download` / `explorer.upload`：仅在 Web / 远程场景有意义，与 `FEATURES.md` §10「不做远程」冲突，本 issue 不实现（需要在 `rules.py` 中改判为「不做」）。
- `filesExplorer.findInFolder`（`shift+alt+f`，随 [[search]]）、文件嵌套（P2）、本地历史（P2）、比较文件（P2）。
- Unity `.meta` 文件的联动：重命名或删除 `.cs` 时**不**自动处理同名 `.meta`（交给 Unity 自己处理，VS Code 也是如此），不要自作主张。

## 验收标准

- [ ] 新建文件 / 文件夹：就地输入、带 `/` 的多级名称、重名与非法字符、Windows 保留名各有测试；新建文件后自动打开
- [ ] 重命名：`f2` 就地编辑、默认选中不含扩展名部分；**只改大小写**（`Foo.cs` → `foo.cs`）成功且树与标签都更新
- [ ] 重命名已打开的文件后，标签、面包屑、导航历史指向新路径；[[basket]] 中该文件的条目仍然有效（[[basket]] 合并前断言路径更新事件已发出）
- [ ] 删除：`files.enableTrash=true` 时文件进入回收站（测试中实际检查回收站并还原）；为 false 时永久删除；`shift+delete` 永久删除且始终确认
- [ ] `explorer.confirmDelete=false` 时不弹确认框；确认框中的「不再询问」写回设置后共用文件的注释与格式不变
- [ ] 多选删除只弹一次确认框，文案含数量；回收站不可用时降级为永久删除并在文案中说明
- [ ] 复制路径与复制相对路径：`auto` 下为 `\`，改为 `/` 后生效；多选时每行一条；`ctrl+k p` 对活动编辑器生效
- [ ] 在系统资源管理器中显示：目标文件被选中；路径含空格、中文、`,` 时都正确
- [ ] 剪切 / 复制 / 粘贴：树内部与系统剪贴板双向互通；`explorer.incrementalNaming` 的三种取值各有测试；`escape` 取消剪切后节点恢复正常颜色
- [ ] 拖放移动文件生效，`explorer.confirmDragAndDrop=true` 时先确认；`explorer.enableDragAndDrop=false` 时拖动无效
- [ ] 撤销：新建、重命名、移动、粘贴、回收站删除都能用 `undo` 撤回，并与编辑器撤销栈互不影响
- [ ] `Library/PackageCache/**` 下的节点，重命名 / 删除 / 粘贴入口全部禁用（而不是执行后报错）
- [ ] 长路径（> 260 字符）与被占用的文件各有一条失败路径测试，提示文案符合 §6.4
- [ ] 外部进程在操作期间改动同一目录时，树最终与磁盘一致（脚本并发创建 / 删除 200 个文件）
- [ ] [[skeleton]] 的端到端测试仍然通过；新增「新建 → 重命名 → 删除到回收站 → 撤销」的集成测试
- [ ] `docs/ARCHITECTURE.md` 的对照表已更新；本 PR 没有修改 [[contracts]] 的接口

## 风险与提示

- **真正会删用户文件**。所有涉及删除的测试都必须在临时目录里跑，严禁指向 `ROVE_TEST_UNITY_*`；回收站相关的测试要在断言后清理。实现时优先用带 `FOF_ALLOWUNDO` 的 `IFileOperation`，不要自己实现「移动到 `$Recycle.Bin`」。
- 与文件监听的竞争是最容易出 bug 的地方：操作成功 → 自己改树 → 监听事件又来一次，会出现节点闪烁或重复。统一走「只由事件改树」的单向数据流。
- 大小写重命名、长路径、UNC 路径三项在 CI 的容器环境里可能表现不同，`PROJECT_CHARTER.md` §7 已把它们列为 Windows 专项风险；测不了的要在 PR 中如实写明。
- 确认框的分级很容易用错：删除要模态（§6.1 第 5 级，会丢数据），而拖放移动在 VS Code 里也是确认框——照搬，不要自行降级为提示条。
