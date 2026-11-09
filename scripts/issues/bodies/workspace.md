## 背景

文件树**不遵守 `.gitignore`**，只为性能排除大目录（`PROJECT_CHARTER` §4.1、`FEATURES` §6）。Unity 项目一直在变：M0 验证期间，中型项目在 1 小时内改动了 790 个文件（`M0_LANG_SPIKE_REPORT`），所以文件监听必须能承受批量变更和缓冲区溢出。另外 Unity 编辑器或杀毒软件会独占打开文件，读取必须共享锁容忍。

实现放在 `rove-workspace`，在 [[contracts]] 定义的工作区接口（文件列表与目录树模型、文件事件流、排除与只读规则查询）背后；本 PR 不修改这些接口。替换 [[skeleton]] 中的 `SKELETON(workspace)` 标记（"递归列出文件，只排除 `.git`、`Library`、`Temp`，不监听变化"），并更新 `docs/ARCHITECTURE.md` 的对照表。

## 目标

### 1. 并行扫描

用 `ignore` crate 的并行 walker，但**把 gitignore 相关的开关全部关掉**（`git_ignore`、`git_global`、`git_exclude`、`parents`、`ignore`）。产出扁平文件列表和目录树模型；目录展开时再加载子节点。资源管理器要能立即显示根目录、快速打开要能在扫描完成前就使用（`UX_SPEC` §6.2），所以扫描结果必须是流式可见的，而不是扫完才给。

### 2. 排除规则

`files.exclude` 与 `rove.files.extraExclude`（默认 `{"**/Library": true, "**/Temp": true, "**/obj": true, "**/Logs": true}`）**合并生效**；`rove.*` 是附加规则，不改写用户的 VS Code 设置（`FEATURES` §13 规则 5）。`search.useIgnoreFiles` 整体忽略（`SETTINGS.md` §3）。监听另有 `files.watcherExclude`。

glob 语义与 VS Code 一致：`**`、`*`、`?`、`{a,b}`、`[...]`、以及前导 `**/` 的隐含匹配。`{"when": "$(basename).ts"}` 形式的兄弟文件条件首版可以只支持布尔值，但要**显式记录这个限制**并保证遇到对象形式时不崩溃。

### 3. 文件监听

基于 `notify`（Windows 上是 `ReadDirectoryChangesW`）：

- 去抖与合并（同一文件的多次写合并成一次）；
- **缓冲区溢出**时对受影响的目录做一次重新扫描，并通过事件流告知"发生过溢出"；
- 对外暴露变更事件流：新增、删除、修改、重命名（Windows 的重命名是成对事件，要还原成一次 rename）；
- 遵循 `files.watcherExclude`，不监听被排除的目录（Unity 的 `Library/` 变化极其频繁，监听它会直接把事件队列打爆）。

### 4. 只读规则

按 `UX_SPEC` §3.4 求值并给出**原因**：

1. `files.readonlyFromPermissions`（磁盘只读属性）
2. `files.readonlyInclude` / `files.readonlyExclude`（glob，exclude 优先级高于 include，与 VS Code 一致）
3. `rove.files.extraReadonlyInclude`（默认 `{"**/Library/PackageCache/**": true}`，因为 Unity 会覆盖这个目录）

原因要能转成用户可见的说明（例如"Unity 包缓存目录，修改会被覆盖"），界面在 [[codeview-hl]] / [[statusbar]]。超大文件与元数据视图的只读来源不在本 issue（分别是 [[largefile]] 与 [[meta-ui]]），但查询接口要能把多个来源合起来表达。

### 5. 路径处理

Windows 长路径（> 260 字符，`\\?\` 前缀）、大小写不敏感比较（但**显示时保留磁盘上的大小写**）、工作区内统一用 `/` 分隔的相对路径作为标识（[[contracts]] 的工作区内路径类型）。

### 6. 共享锁容忍的读取

以 `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` 打开文件，失败时重试；被独占时返回明确的错误类型，供 `UX_SPEC` §6.4 的"无法打开文件"占位页使用。

## 不包含

- 全局文本搜索（[[search]]）、模糊匹配打分（[[quickopen]]）。
- 资源管理器界面与紧凑文件夹（[[explorer]]）、文件操作（[[fileops]]）。
- 外部修改的重载与保存冲突（[[save]]）。
- 多根工作区（`FEATURES` §6 明确不做：一个窗口一个文件夹）。

## 验收标准

- [ ] 基准：大型项目（`ROVE_TEST_UNITY_LARGE`，约 2 万个 `.cs` 文件）全量扫描 < 1 s（热缓存），接入 [[perf]]
- [ ] 扫描是流式的：根目录在首帧即可显示，不等待全量完成
- [ ] 测试：1 秒内批量新增或删除 1000 个文件后，事件流的最终状态与磁盘一致
- [ ] 测试：人为制造事件溢出后能恢复（重新扫描受影响目录），并对外报出"发生过溢出"
- [ ] 测试：重命名被还原成一次 rename 事件而不是 删除 + 新增
- [ ] 测试：工作区里存在 `.gitignore` 时，被它忽略的文件仍然出现在文件列表中
- [ ] 测试：`files.exclude` 与 `rove.files.extraExclude` 的合并，含用户显式把 `**/Library` 设为 `false` 的情况
- [ ] 只读规则的真值表测试：权限只读、include、exclude 覆盖 include、`extraReadonlyInclude` 默认值，每条都返回正确的原因
- [ ] 测试：长路径（> 260 字符）可扫描可读取；大小写不同的同一路径被认为相同，但显示保留磁盘大小写
- [ ] 测试：文件被独占打开时读取返回明确错误，不 panic
- [ ] `SKELETON(workspace)` 标记已删除，`docs/ARCHITECTURE.md` 的对照表已更新
- [ ] 新增集成测试覆盖"扫描 → 排除生效 → 修改文件 → 收到事件 → 只读查询"；[[skeleton]] 的端到端测试继续通过
- [ ] `cargo tree -p rove-workspace -i gpui` 为空

## 风险与提示

- `ignore` crate 的默认行为**就是**读 gitignore。关开关很容易漏一个，所以把"存在 `.gitignore` 时结果不变"写成测试，比在代码里小心更可靠。
- 监听 Unity 项目时，`Library/` 和 `Temp/` 的事件量能淹没一切。`files.watcherExclude` 必须在**注册监听之前**生效，而不是收到事件后再过滤。
- 大小写不敏感比较不要简单用 `to_lowercase()`：Windows 的比较规则是按 Unicode 大小写折叠的，且某些字符（土耳其语 i 等）会出问题。用一致的折叠实现并写进文档。
- 排除规则的求值在扫描热路径上，每个文件都要过一遍。把 glob 预编译成一个匹配器集合，不要在循环里现编译。
