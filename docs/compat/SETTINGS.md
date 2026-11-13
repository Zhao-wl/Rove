# 设置兼容清单（阶段 0.3b）

> 版本：v1.0（自动生成，2026-11-09 已确认判定规则） · 来源：VS Code 1.145.2（Windows，中文界面导出）的默认设置 · 生成脚本：`scripts/compat/gen_compat_docs.py` · 判定规则：`scripts/compat/rules.py`

## 1. 说明

- Rove 与 VS Code 共用 `settings.json`（D4）。下表中“支持”的设置项，Rove 必须遵循 VS Code 的语义和默认值；“不支持”的设置项会被静默忽略（FEATURES §13 规则 2）。
- “支持”指 Rove 会读取并遵循该设置项。功能本身的优先级决定它何时生效：例如 `editor.minimap.*` 标为 P1，是因为缩略图是 P1 功能。
- 用户安装的扩展贡献的设置项已排除。对判定有异议时，修改 `rules.py` 后重新生成。

## 2. 统计

| 判定 | 条数 |
|---|---|
| 支持 P0 | 372 |
| 支持 P1 | 119 |
| 支持 P2 | 162 |
| 不支持（忽略） | 1113 |
| 扩展贡献（排除） | 645 |
| **合计** | 2411 |

## 3. Rove 默认值与 VS Code 不同的设置项

这些默认值**只在用户没有设置该项时生效**，并且不会写入共用的设置文件（FEATURES §13 规则 5）。

| 设置项 | VS Code 默认值 | Rove 默认值 | 原因 |
|---|---|---|---|
| `files.autoGuessEncoding` | `false` | `true` | Unity 项目中常见 GBK 文件（UX_SPEC §3.5） |
| `files.candidateGuessEncodings` | `[]` | `["gbk"]` | 同上 |
| `workbench.editorAssociations` | `{}` | `{"*.md": "vscode.markdown.preview.editor"}` | `.md` 默认以预览打开（D6） |
| `markdown.preview.doubleClickToSwitchToEditor` | `false` | `true` | `.md` 默认以预览打开（D6），双击切换到源码是主要的编辑入口（UX_SPEC §4.3） |

### Rove 支持的 VS Code 隐藏设置

以下设置项是 VS Code 的隐藏设置，不出现在默认设置导出中，但 Rove 读取并遵循它们。

| 优先级 | 设置项 | 默认值 | 说明 |
|---|---|---|---|
| P0 | `editor.stopRenderingLineAfter` | `10000` | 超长行在多少个字符之后停止渲染（UX_SPEC §3.3） |

## 4. Rove 专有设置项（`rove.*`）

| 优先级 | 设置项 | 默认值 | 说明 |
|---|---|---|---|
| P0 | `rove.largeFile.threshold` | `67108864` | 超大文件（只读、内存映射）的阈值，单位字节，默认 64 MB（UX_SPEC §3.1） |
| P0 | `rove.files.extraExclude` | `{"**/Library": true, "**/Temp": true, "**/obj": true, "**/Logs": true}` | 附加的排除规则，与 `files.exclude` 合并生效（FEATURES §13 规则 5） |
| P0 | `rove.files.extraReadonlyInclude` | `{"**/Library/PackageCache/**": true}` | 附加的只读规则，与 `files.readonlyInclude` 合并生效（UX_SPEC §3.4） |
| P0 | `rove.index.cacheDir` | `""` | C# 索引缓存目录；为空时使用 `%LOCALAPPDATA%\Rove\index` |
| P0 | `rove.index.enabled` | `true` | 是否启用 C# 语义索引 |
| P1 | `rove.metadataView.showDocs` | `true` | 元数据视图中是否显示文档注释（UX_SPEC §4.1） |
| P0 | `rove.markdown.webviewIdleTimeout` | `300` | 所有 Markdown 预览关闭后，多少秒销毁 WebView2 实例（UX_SPEC §4.3） |
| P0 | `rove.markdown.allowRemoteResources` | `false` | 旧版兼容键，预览始终拦截远程资源（UX_SPEC §4.3） |
| P1 | `rove.contextBasket.referenceTemplate` | `"@{path} (lines {start}-{end})"` | “复制为引用”的格式（UX_SPEC §5.3） |
| P1 | `rove.contextBasket.contextLines` | `3` | 从速览或搜索结果加入时，额外带上的前后行数（UX_SPEC §5.1） |
| P1 | `rove.locale` | `"auto"` | 界面语言：`auto`（跟随系统）、`zh-cn`、`en`。只读参考共用用户目录下 `argv.json` 的 `locale`，从不写入它 |
| P1 | `rove.contextBasket.clearAfterCopy` | `false` | 复制后是否清空篮子（UX_SPEC §5.3） |

## 5. 支持的 VS Code 设置项（按功能区）

### Markdown（18）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `markdown.links.openLocation` | `"currentGroup"` | 控制应在哪里打开 Markdown 文件中的链接。 |
| P0 | `markdown.math.enabled` | `true` | Markdown Math |
| P0 | `markdown.math.macros` | `{}` | 自定义宏的集合。每个宏都是键值对，其中键是新的命令名称，值是宏的扩展。 |
| P0 | `markdown.preferredMdPathExtensionStyle` | `"auto"` | 控制是否为指向 Markdown 文件的链接添加文件扩展名(例如“.md”)。通过工具(如路径完成或文件重命名)添加文件路径时，将使用此设… |
| P0 | `markdown.preview.breaks` | `false` | 预览 |
| P0 | `markdown.preview.doubleClickToSwitchToEditor` | `false` | 在 Markdown 预览中双击以切换到编辑器。 |
| P0 | `markdown.preview.fontFamily` | `"-apple-system, BlinkMacSystemFont, 'Segoe WPC', 'Segoe UI'…` | 控制 Markdown 预览中使用的字体系列。 |
| P0 | `markdown.preview.fontSize` | `14` | 控制 Markdown 预览中使用的字号(以像素为单位)。 |
| P0 | `markdown.preview.frontMatter` | `"table"` | 控制 Markdown 文件开头的 YAML frontmatter (由 `---` 分隔)在预览中的渲染方式。 |
| P0 | `markdown.preview.lineHeight` | `1.6` | 控制 Markdown 预览中使用的行高。此数值与字号相关。 |
| P0 | `markdown.preview.linkify` | `true` | 将类似于 URL 的文本转换为 Markdown 预览中的链接。 |
| P0 | `markdown.preview.markEditorSelection` | `false` | 在 Markdown 预览中标记当前的编辑器选定内容。 |
| P0 | `markdown.preview.openMarkdownLinks` | `"inPreview"` | 控制如何打开 Markdown 预览中其他 Markdown 文件的链接。 |
| P0 | `markdown.preview.scrollEditorWithPreview` | `true` | 滚动 Markdown 预览时，更新其编辑器视图。 |
| P0 | `markdown.preview.scrollPreviewWithEditor` | `true` | 滚动 Markdown 编辑器时，更新其预览视图。 |
| P0 | `markdown.preview.typographer` | `false` | 在 Markdown 预览中启用或一些与语言无关的替换和引文美化。 |
| P0 | `markdown.styles` | `[]` | 要从 Markdown 预览使用的 CSS 样式表的 URL 或本地路径的列表。相对路径解释为相对于资源管理器中打开的文件夹。如果没有打开… |
| P0 | `markdown.trace.server` | `"off"` | 跟踪 VS Code 和 Markdown 语言服务器之间的通信。 |

### 大纲与面包屑（65）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `breadcrumbs.enabled` | `true` | 导航路径 |
| P0 | `breadcrumbs.filePath` | `"on"` | 控制是否及如何在“导航路径”视图中显示文件路径。 |
| P0 | `breadcrumbs.icons` | `true` | 使用图标渲染面包屑导航项。 |
| P0 | `breadcrumbs.showArrays` | `true` | 启用后，痕迹导航栏将显示“数组”符号。 |
| P0 | `breadcrumbs.showBooleans` | `true` | 启用后，痕迹导航栏将显示“布尔”符号。 |
| P0 | `breadcrumbs.showClasses` | `true` | 启用后，痕迹导航栏显示“类”符号。 |
| P0 | `breadcrumbs.showConstants` | `true` | 启用后，痕迹导航栏将显示“常量”符号。 |
| P0 | `breadcrumbs.showConstructors` | `true` | 启用后，痕迹符将显示“构造函数”符号。 |
| P0 | `breadcrumbs.showEditorType` | `true` | 控制痕迹导航栏是否显示下拉列表，以便在可打开当前文件的编辑器(例如，文本编辑器和自定义编辑器)之间切换。仅当有更专用的编辑器可用时，才会显… |
| P0 | `breadcrumbs.showEnumMembers` | `true` | 启用后，痕迹导航栏将显示 "enumMember" 符号。 |
| P0 | `breadcrumbs.showEnums` | `true` | 启用后，痕迹导航栏将显示“枚举”符号。 |
| P0 | `breadcrumbs.showEvents` | `true` | 启用后，痕迹导航栏将显示“事件”符号。 |
| P0 | `breadcrumbs.showFields` | `true` | 启用后，痕迹导航栏将显示“字段”符号。 |
| P0 | `breadcrumbs.showFiles` | `true` | 启用后，痕迹导航栏将显示“文件”符号。 |
| P0 | `breadcrumbs.showFunctions` | `true` | 启用后，痕迹导航栏将显示“函数”符号。 |
| P0 | `breadcrumbs.showInUntitledFiles` | `false` | Controls whether breadcrumbs are shown for untitled files. |
| P0 | `breadcrumbs.showInterfaces` | `true` | 启用后，痕迹导航栏将显示“接口”符号。 |
| P0 | `breadcrumbs.showKeys` | `true` | 启用后，痕迹导航栏将显示“键”符号。 |
| P0 | `breadcrumbs.showMethods` | `true` | 启用后，痕迹导航栏将显示“方法”符号。 |
| P0 | `breadcrumbs.showModules` | `true` | 启用后，痕迹导航栏将显示“模块”符号。 |
| P0 | `breadcrumbs.showNamespaces` | `true` | 启用后，痕迹导航栏将显示“命名空间”符号。 |
| P0 | `breadcrumbs.showNull` | `true` | 启用后，痕迹导航栏将显示 "null" 符号。 |
| P0 | `breadcrumbs.showNumbers` | `true` | 启用后，痕迹导航栏将显示“数字”符号。 |
| P0 | `breadcrumbs.showObjects` | `true` | 启用后，痕迹导航栏将显示“对象”符号。 |
| P0 | `breadcrumbs.showOperators` | `true` | 启用后，痕迹导航栏将显示“运算符”符号。 |
| P0 | `breadcrumbs.showPackages` | `true` | 启用后，痕迹导航栏将显示“包”符号。 |
| P0 | `breadcrumbs.showProperties` | `true` | 启用后，痕迹导航栏将显示“属性”符号。 |
| P0 | `breadcrumbs.showStrings` | `true` | 启用后，痕迹导航栏将显示“字符串”符号。 |
| P0 | `breadcrumbs.showStructs` | `true` | 启用后，痕迹导航栏将显示“结构”符号。 |
| P0 | `breadcrumbs.showTypeParameters` | `true` | 启用后，痕迹导航栏将显示 "typeParameter" 符号。 |
| P0 | `breadcrumbs.showVariables` | `true` | 启用后，痕迹导航栏将显示“变量”符号。 |
| P0 | `breadcrumbs.symbolPath` | `"on"` | 控制是否及如何在“导航路径”视图中显示符号。 |
| P0 | `breadcrumbs.symbolPathSeparator` | `"."` | 复制痕迹导航符号路径时使用的分隔符。 |
| P0 | `breadcrumbs.symbolSortOrder` | `"position"` | 控制“导航路径”大纲视图中符号的排序方式。 |
| P0 | `outline.collapseItems` | `"alwaysExpand"` | 大纲 |
| P0 | `outline.icons` | `true` | 使用图标呈现大纲元素。 |
| P0 | `outline.problems.badges` | `true` | 对大纲元素的错误和警告使用锁屏提醒。被关闭时由 `problems.visibility` 覆盖。 |
| P0 | `outline.problems.colors` | `true` | 对大纲元素的错误和警告使用颜色。被关闭时由 `problems.visibility` 覆盖。 |
| P0 | `outline.problems.enabled` | `true` | 显示大纲元素上的错误和警告。被关闭时由 `problems.visibility` 覆盖。 |
| P0 | `outline.showArrays` | `true` | 启用后，大纲将显示 `array` 符号。 |
| P0 | `outline.showBooleans` | `true` | 启用后，大纲将显示 `boolean` 符号。 |
| P0 | `outline.showClasses` | `true` | 启用后，大纲将显示 `class` 符号。 |
| P0 | `outline.showConstants` | `true` | 启用后，大纲将显示`constant`符号。 |
| P0 | `outline.showConstructors` | `true` | 启用大纲时，大纲将显示 `constructor` 符号。 |
| P0 | `outline.showEnumMembers` | `true` | 启用后，大纲将显示 `enumMember` 符号。 |
| P0 | `outline.showEnums` | `true` | 启用后，大纲将显示 `enum` 符号。 |
| P0 | `outline.showEvents` | `true` | 启用后，大纲将显示 `event` 符号。 |
| P0 | `outline.showFields` | `true` | 启用时，大纲将显示 `field`符号。 |
| P0 | `outline.showFiles` | `true` | 启用后，大纲将显示 `file` 符号。 |
| P0 | `outline.showFunctions` | `true` | 启用时，大纲将显示 `function` 符号。 |
| P0 | `outline.showInterfaces` | `true` | 启用后，大纲将显示 `interface` 符号。 |
| P0 | `outline.showKeys` | `true` | 启用后，大纲将显示 `key`符号。 |
| P0 | `outline.showMethods` | `true` | 启用后，大纲将显示 `method` 符号。 |
| P0 | `outline.showModules` | `true` | 启用后，大纲将显示 `module` 符号。 |
| P0 | `outline.showNamespaces` | `true` | 启用后，大纲将显示 `namespace` 符号。 |
| P0 | `outline.showNull` | `true` | 启用后，大纲将显示 `null` 符号。 |
| P0 | `outline.showNumbers` | `true` | 启用后，大纲将显示 `number` 符号。 |
| P0 | `outline.showObjects` | `true` | 启用后，大纲将显示 `object` 符号。 |
| P0 | `outline.showOperators` | `true` | 启用时，大纲显示 `operator` 符号。 |
| P0 | `outline.showPackages` | `true` | 启用后，大纲将显示 `package` 符号。 |
| P0 | `outline.showProperties` | `true` | 启用后，大纲将显示 `property` 符号。 |
| P0 | `outline.showStrings` | `true` | 启用后，大纲将显示 `string` 符号。 |
| P0 | `outline.showStructs` | `true` | 启用后，大纲将显示`struct` 符号。 |
| P0 | `outline.showTypeParameters` | `true` | 启用后，大纲将显示 `typeParameter` 符号。 |
| P0 | `outline.showVariables` | `true` | 启用后，大纲将显示 `variable` 符号。 |

### 工作台（104）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `workbench.activityBar.autoHide` | `false` | 控制当只有一个视图容器要显示时，活动栏是否自动隐藏。当 `workbench.activityBar.location` 设置为 `top… |
| P0 | `workbench.activityBar.compact` | `false` | 控制活动栏是否使用图标较小、宽度减少的紧凑布局。仅当 `workbench.activityBar.location` 设置为 `defa… |
| P0 | `workbench.activityBar.location` | `"default"` | 控制活动栏相对于主边栏和辅助侧栏的位置。 |
| P0 | `workbench.colorCustomizations` | `{}` | 覆盖当前所选颜色主题的颜色。 |
| P0 | `workbench.colorTheme` | `"Dark 2026"` | 指定未启用 `window.autoDetectColorScheme` 时在工作台中使用颜色主题。 |
| P0 | `workbench.commandPalette.history` | `50` | 控制命令面板中保留最近使用命令的数量。设置为 0 时禁用命令历史功能。 |
| P0 | `workbench.commandPalette.preserveInput` | `false` | 当再次打开命令面板时，控制是否恢复上一次输入的内容。 |
| P0 | `workbench.diffEditorAssociations` | `{}` | 将 [glob 模式](https://aka.ms/vscode-glob-patterns)配置为编辑器以打开差异视图(例如 `"*.… |
| P0 | `workbench.editor.alwaysShowEditorActions` | `false` | 控制是否始终显示编辑器操作(即使编辑器组不处于活动状态)。 |
| P0 | `workbench.editor.centeredLayoutAutoResize` | `true` | 如果在居中布局中打开了超过一组编辑器，控制是否自动将宽度调整为最大宽度值。当回到只打开了一组编辑器的状态，将自动将宽度调整为原始的居中宽度… |
| P0 | `workbench.editor.centeredLayoutFixedWidth` | `false` | 调整窗口大小时，居中布局尝试维持常量宽度的控件。 |
| P0 | `workbench.editor.closeEmptyGroups` | `true` | 控制编辑器组中最后一个选项卡关闭时这个空组的行为。若启用，将自动关闭空组。若禁用，空组仍将保留在网格布局中。 |
| P0 | `workbench.editor.closeOnFileDelete` | `false` | 控制在会话期间显示已打开文件的编辑器是否应在被其他进程删除或重命名时自动关闭。禁用此功能将使编辑器在此类事件中保持打开状态。请注意，从应用… |
| P0 | `workbench.editor.customLabels.enabled` | `true` | 控制是否应应用自定义工作台编辑器标签。 |
| P0 | `workbench.editor.customLabels.patterns` | `{}` | 控制编辑器标签的呈现。每个 __Item__ 都是与文件路径匹配的模式。同时支持相对文件路径和绝对文件路径。相对路径必须包含 WORKSP… |
| P0 | `workbench.editor.decorations.badges` | `true` | 控制编辑器文件修饰是否应使用徽章。 |
| P0 | `workbench.editor.decorations.colors` | `true` | 控制编辑器文件修饰是否应使用颜色。 |
| P0 | `workbench.editor.defaultBinaryEditor` | `""` | 检测为二进制文件的默认编辑器。如果未定义，将向用户显示选取器。 |
| P0 | `workbench.editor.doubleClickTabToToggleEditorGroupSizes` | `"expand"` | 控制双击选项卡时编辑器组的大小调整方式。如果未将 `workbench.editor.showTabs` 设置为 `multiple`，则… |
| P0 | `workbench.editor.editorActionsLocation` | `"default"` | 控制显示编辑器操作的位置。 |
| P0 | `workbench.editor.enablePreview` | `true` | 控制在编辑器打开时是否使用预览模式。每个编辑器组最多只能有一个预览模式编辑器。此编辑器在其选项卡或标题标签和“打开编辑器”视图中以斜体显示… |
| P0 | `workbench.editor.enablePreviewFromCodeNavigation` | `false` | 控制当从编辑器开始进行代码导航时，编辑器是否保持为预览状态。预览编辑器不会保持打开状态，在将其显式设置为保持打开(通过双击或编辑)前将会重… |
| P0 | `workbench.editor.enablePreviewFromQuickOpen` | `false` | 控制通过 Quick Open 打开的编辑器是否显示为预览编辑器。预览编辑器不会保持打开状态，在将其显式设置为保持打开(通过双击或编辑)前… |
| P0 | `workbench.editor.focusRecentEditorAfterClose` | `true` | 控制是按最近使用的顺序还是从左到右关闭编辑器。 |
| P0 | `workbench.editor.hiddenEditorTypes` | `[]` | Configure editor types that are hidden from the editor type picker. T… |
| P0 | `workbench.editor.highlightModifiedTabs` | `false` | 控制是否在具有未保存更改的编辑器的选项卡上绘制顶部边框。如果未将 `workbench.editor.showTabs` 设置为 mult… |
| P0 | `workbench.editor.labelFormat` | `"default"` | 控制编辑器标签的格式。 |
| P0 | `workbench.editor.limit.enabled` | `false` | 控制打开的编辑器数是否应受限制。启用后，最近使用较少的编辑器将关闭，以为新打开的编辑器腾出空间。 |
| P0 | `workbench.editor.limit.excludeDirty` | `false` | 控制打开的编辑器的最大数目是否应排除脏编辑器以计入配置的限制。 |
| P0 | `workbench.editor.limit.perEditorGroup` | `false` | 控制最大打开的编辑器的限制是否应应用于每个编辑器组或所有编辑器组。 |
| P0 | `workbench.editor.limit.value` | `10` | 控制打开编辑器的最大数量。使用 `workbench.editor.limit.perEditorGroup` 设置控制每个编辑器组或跨所… |
| P0 | `workbench.editor.mouseBackForwardToNavigate` | `true` | 允许使用鼠标按钮四和五执行“返回”和“前进”命令。 |
| P0 | `workbench.editor.navigationScope` | `"default"` | 控制编辑器中“返回”和“前进”等命令的历史导航范围。 |
| P0 | `workbench.editor.openPositioning` | `"right"` | 控制编辑器打开的位置。选择 `left` 或 `right` 可分别在当前活动编辑器的左侧或右侧打开。选择 `first` 或 `last… |
| P0 | `workbench.editor.openSideBySideDirection` | `"right"` | 控制编辑器在并排打开时(例如从资源管理器)出现的默认位置。默认在当前活动编辑器右侧打开。若更改为 "down"，则在当前活动编辑器下方打开… |
| P0 | `workbench.editor.pinnedTabSizing` | `"normal"` | 控制固定的编辑器选项卡的大小。固定的选项卡排在所有打开的选项卡的开头，并且在取消固定之前，通常不会关闭。如果未将 `workbench.e… |
| P0 | `workbench.editor.pinnedTabsOnSeparateRow` | `false` | 启用后，在所有其他选项卡上方的单独行中显示固定的选项卡。如果未将 `workbench.editor.showTabs` 设置为 `mul… |
| P0 | `workbench.editor.preventPinnedEditorClose` | `"keyboardAndMouse"` | 控制在使用键盘或鼠标中键单击关闭时是否关闭固定的编辑器。 |
| P0 | `workbench.editor.restoreEditors` | `true` | 控制启动时是否还原编辑器。禁用时，仅还原上次会话中未保存的编辑器。 |
| P0 | `workbench.editor.restoreViewState` | `true` | 关闭编辑器后重新打开时，将还原最后的编辑器视图状态(例如滚动位置)。编辑器视图状态按编辑器组存储，并在组关闭时被放弃。使用 `workbe… |
| P0 | `workbench.editor.revealIfOpen` | `false` | 控制是否在打开的任何可见组中显示编辑器。如果禁用，编辑器将优先在当前活动的编辑器组中打开。如果启用，将显示在已打开的编辑器，而不是在当前活… |
| P0 | `workbench.editor.scrollToSwitchTabs` | `false` | 控制在滚动到选项卡上方时是否打开这些选项卡。默认情况下，选项卡仅在鼠标滚动时呈现，但不打开。可通过在滚动时按住 Shift 键来更改滚动期… |
| P0 | `workbench.editor.sharedViewState` | `false` | 跨所有编辑器组保留最新的编辑器视图状态(例如滚动位置等)并在未找到编辑器组的特定编辑器视图状态时进行还原。 |
| P0 | `workbench.editor.showIcons` | `true` | 控制是否在打开的编辑器中显示图标。这要求同时启用文件图标主题。 |
| P0 | `workbench.editor.showTabIndex` | `false` | 启用后，将显示选项卡索引。如果未将 `workbench.editor.showTabs` 设置为 `multiple`，则会忽略此值。 |
| P0 | `workbench.editor.showTabs` | `"multiple"` | 控制打开的编辑器是否显示为单个选项卡还是一个大选项卡，或者标题区域是否应显示。 |
| P0 | `workbench.editor.splitInGroupLayout` | `"horizontal"` | 控制在编辑器组中垂直或水平拆分编辑器时的布局。 |
| P0 | `workbench.editor.splitOnDragAndDrop` | `true` | 通过将编辑器或文件放到编辑器区域的边缘，控制是否可以由拖放操作拆分编辑器组。 |
| P0 | `workbench.editor.splitSizing` | `"auto"` | 拆分编辑器组时控制编辑器组大小。 |
| P0 | `workbench.editor.tabActionCloseVisibility` | `true` | 控制选项卡关闭操作按钮的可见性。 |
| P0 | `workbench.editor.tabActionLocation` | `"right"` | 控制编辑器选项卡操作按钮（关闭、取消固定）的位置。如果未将 `workbench.editor.showTabs` 设置为 `multip… |
| P0 | `workbench.editor.tabActionReserveSpace` | `true` | Controls whether Modern UI editor tabs always reserve space for tab a… |
| P0 | `workbench.editor.tabActionUnpinVisibility` | `true` | 控制选项卡取消固定操作按钮的可见性。 |
| P0 | `workbench.editor.tabSizing` | `"fit"` | 控制编辑器选项卡的大小。如果未将 `workbench.editor.showTabs` 设置为 `multiple`，则会忽略此值。 |
| P0 | `workbench.editor.tabSizingFixedMaxWidth` | `160` | 控制将 `workbench.editor.tabSizing` 大小设置为 `fixed` 时选项卡的最大宽度。 |
| P0 | `workbench.editor.tabSizingFixedMinWidth` | `50` | 控制将 `workbench.editor.tabSizing` 大小设置为 `fixed` 时选项卡的最小宽度。 |
| P0 | `workbench.editor.titleScrollbarSizing` | `"default"` | 控制编辑器标题区域中用于选项卡和面包屑的滚动条的高度。 |
| P0 | `workbench.editor.titleScrollbarVisibility` | `"auto"` | 控制编辑器标题区域中用于选项卡和面包屑的滚动条的可见性。 |
| P0 | `workbench.editor.untitled.labelFormat` | `"content"` | 控制无标题编辑器的标签格式。 |
| P0 | `workbench.editor.wrapTabs` | `false` | 控制当超出可用空间时，选项卡是否应在多行之间换行，或者是否应显示滚动条。如果未将 `workbench.editor.showTabs` … |
| P0 | `workbench.editorAssociations` | `{}` | 将 [glob 模式](https://aka.ms/vscode-glob-patterns)配置到编辑器(例如 `"*.hex": "… |
| P0 | `workbench.hover.delay` | `500` | 控制为工作台项显示悬停之前的延迟时间(以毫秒为单位)(例如，一些扩展提供了树状视图项)。已经可见的项可能需要刷新，然后才会反映出此设置更改。 |
| P0 | `workbench.hover.reducedDelay` | `500` | 控制在特定场景下显示悬停时的延迟(以毫秒为单位)，以便更快地反馈。 |
| P0 | `workbench.iconTheme` | `"vs-seti"` | 指定工作台中使用的文件图标主题；若指定为 "null"，则不显示任何文件图标。 |
| P0 | `workbench.layoutControl.enabled` | `true` | 控制布局控件是否显示在自定义标题栏中。仅当 `window.customTitleBarVisibility` 设置为 `never` 时… |
| P0 | `workbench.list.defaultFindMatchType` | `"fuzzy"` | 控制在工作台中搜索列表和树时使用的匹配类型。 |
| P0 | `workbench.list.defaultFindMode` | `"highlight"` | 控制工作台中列表和树的默认查找模式。 |
| P0 | `workbench.list.fastScrollSensitivity` | `5` | 按下"Alt"时滚动速度倍增。 |
| P0 | `workbench.list.horizontalScrolling` | `false` | 控制工作台上的列表和树是否支持水平滚动。警告: 打开此设置会影响性能。 |
| P0 | `workbench.list.keyboardNavigation` | `"highlight"` | 请改用 "workbench.list.defaultFindMode" 和 "workbench.list.typeNavigation… |
| P0 | `workbench.list.mouseWheelScrollSensitivity` | `1` | 对鼠标滚轮滚动事件的 `deltaX` 和 `deltaY` 乘上的系数。 |
| P0 | `workbench.list.multiSelectModifier` | `"ctrlCmd"` | 在通过鼠标多选树和列表条目时使用的修改键 (例如“资源管理器”、“打开的编辑器”和“源代码管理”视图)。“在侧边打开”功能所需的鼠标动作 … |
| P0 | `workbench.list.openMode` | `"singleClick"` | 控制如何使用鼠标打开树和列表中的项(若支持)。请注意，如果此设置不适用，某些树和列表可能会选择忽略它。 |
| P0 | `workbench.list.scrollByPage` | `false` | 控制在滚动条中单击时是否逐页单击。 |
| P0 | `workbench.list.smoothScrolling` | `false` | 控制列表和树是否具有平滑滚动效果。 |
| P0 | `workbench.list.typeNavigationMode` | `"automatic"` | 控制类型导航在工作台的列表和树中的工作方式。如果设置为`trigger`，则在运行 `list.triggerTypeNavigation… |
| P0 | `workbench.notifications.position` | `"bottom-right"` | 控制通知 Toast 和通知中心的位置。 |
| P0 | `workbench.notifications.showInTitleBar` | `true` | 控制标题栏中通知按钮的可见性。仅当通知位于右上角时适用。 |
| P0 | `workbench.panel.defaultLocation` | `"bottom"` | 控制新工作区中面板(终端、调试控制台、输出、问题)的默认位置。它可以显示在编辑器区域的底部、顶部、右侧或左侧。 |
| P0 | `workbench.panel.showLabels` | `true` | 控制面板标题中的活动项是显示为标签还是图标。 |
| P0 | `workbench.preferredDarkColorTheme` | `"Dark 2026"` | 指定当系统颜色模式为深色并启用 `window.autoDetectColorScheme` 时的颜色主题。 |
| P0 | `workbench.preferredHighContrastColorTheme` | `"Default High Contrast"` | 指定在高对比度深色模式下启用 `window.autoDetectHighContrast` 时的颜色主题。 |
| P0 | `workbench.preferredHighContrastLightColorTheme` | `"Default High Contrast Light"` | 指定在高对比度浅色模式下启用 `window.autoDetectHighContrast` 时的颜色主题。 |
| P0 | `workbench.preferredLightColorTheme` | `"Light 2026"` | 指定当系统颜色模式为浅色并启用 `window.autoDetectColorScheme` 时的颜色主题。 |
| P0 | `workbench.quickOpen.closeOnFocusLost` | `true` | 控制 Quick Open 是否在其失去焦点时自动关闭。 |
| P0 | `workbench.quickOpen.preserveInput` | `false` | 在打开 Quick Open 视图时，控制是否自动恢复上一次输入的值。 |
| P0 | `workbench.sash.autoHideGrippers` | `true` | Controls whether sash grippers between views and editors hide when th… |
| P0 | `workbench.sash.autoHideGrippersDelay` | `2000` | Controls how long to wait in milliseconds without mouse movement befo… |
| P0 | `workbench.sash.size` | `4` | 控制视图/编辑器之间拖动区域的反馈区域大小(以像素为单位)。如果你认为很难使用鼠标调整视图的大小，请将该值调大。 |
| P0 | `workbench.sideBar.location` | `"left"` | 控制主边栏和活动栏的位置。它们可以显示在工作台的左侧或右侧。辅助边栏将显示在工作台的另一侧。 |
| P0 | `workbench.startupEditor` | `"welcomePage"` | 在没有从上一会话中恢复出信息的情况下，控制启动时显示的编辑器。 |
| P0 | `workbench.statusBar.visible` | `true` | 控制工作台底部状态栏的可见性。 |
| P0 | `workbench.tree.enableStickyScroll` | `true` | 控制是否在树中启用粘性滚动。 |
| P0 | `workbench.tree.expandMode` | `"singleClick"` | 控制在单击文件夹名称时如何扩展树文件夹。请注意，如果不适用，某些树和列表可能会选择忽略此设置。 |
| P0 | `workbench.tree.indent` | `8` | 控制树缩进(以像素为单位)。 |
| P0 | `workbench.tree.renderIndentGuides` | `"onHover"` | 控制树是否应呈现缩进参考线。 |
| P0 | `workbench.tree.stickyScrollMaxItemCount` | `7` | 控制启用 `workbench.tree.enableStickyScroll` 时树中显示的粘滞元素数。 |
| P0 | `workbench.view.alwaysShowHeaderActions` | `false` | 控制是否显示视图头部的操作项。视图头部操作项可以一直，或是仅当聚焦到和悬停在视图上时显示。 |
| P0 | `workbench.welcomePage.preferReducedMotion` | `false` | 已弃用，请使用全局 `workbench.reduceMotion`。 |
| P2 | `workbench.localHistory.enabled` | `true` | 控制是否启用本地文件历史记录。启用后，所保存编辑器文件内容将存储到备份位置，以便稍后可以还原或查看内容。更改此设置不会影响现有本地文件历史… |
| P2 | `workbench.localHistory.exclude` | `{}` | 配置路径或 [glob 模式](https://aka.ms/vscode-glob-patterns)以排除本地文件历史记录中的文件。g… |
| P2 | `workbench.localHistory.maxFileEntries` | `50` | 控制每个文件的最大本地文件历史记录条目数。当文件的本地文件历史记录条目数超过此数目时，将丢弃最早的条目。 |
| P2 | `workbench.localHistory.maxFileSize` | `256` | 控制考虑用于本地历史记录的文件最大大小(KB)。较大的文件将不会添加到本地历史记录中。更改此设置不会影响现有本地文件历史记录条目。 |
| P2 | `workbench.localHistory.mergeWindow` | `10` | 配置时间间隔(以秒为单位)，在此间隔期间，本地文件历史记录中的最后一个条目将替换为正在添加的条目。这有助于减少所添加的条目总数，例如启用自… |

### 搜索（31）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `search.actionsPosition` | `"right"` | 搜索 |
| P0 | `search.collapseResults` | `"alwaysExpand"` | 控制是折叠还是展开搜索结果。 |
| P0 | `search.decorations.badges` | `true` | 控制搜索文件修饰是否应使用徽章。 |
| P0 | `search.decorations.colors` | `true` | 控制搜索文件修饰是否应使用颜色。 |
| P0 | `search.exclude` | `{"**/node_modules": true, "**/bower_components": true, "**/…` | Configure [glob patterns](https://code.visualstudio.com/docs/editor/c… |
| P0 | `search.followSymlinks` | `true` | Controls whether to follow symlinks while searching. |
| P0 | `search.maxResults` | `20000` | Controls the maximum number of search results, this can be set to `nu… |
| P0 | `search.mode` | `"view"` | 控制新的“搜索: 在文件中查找”和“在文件夹中查找”操作发生的位置: 是在搜索视图中，还是在搜索编辑器中。 |
| P0 | `search.quickAccess.preserveInput` | `false` | 在打开快速搜索视图时，控制是否自动恢复上一次输入的值。 |
| P0 | `search.quickOpen.history.filterSortOrder` | `"default"` | 控制在快速打开中筛选时编辑器历史记录的排序顺序。 |
| P0 | `search.quickOpen.includeHistory` | `true` | 是否在 Quick Open 的文件结果中包含最近打开的文件。 |
| P0 | `search.quickOpen.includeSymbols` | `false` | 控制 Quick Open 文件结果中是否包括全局符号搜索的结果。 |
| P0 | `search.ripgrep.maxThreads` | `0` | Number of threads to use for searching. When set to 0, the engine aut… |
| P0 | `search.searchOnType` | `true` | 在键入时搜索所有文件。 |
| P0 | `search.searchOnTypeDebouncePeriod` | `300` | 启用 `search.searchOnType` 时，控制键入的字符与开始搜索之间的超时(以毫秒为单位)。禁用 `search.searc… |
| P0 | `search.seedOnFocus` | `false` | 聚焦搜索视图时，将搜索查询更新为编辑器的所选文本。单击时或触发 "workbench.views.search.focus" 命令时会发生… |
| P0 | `search.seedWithNearestWord` | `false` | 当活动编辑器没有选定内容时，从离光标最近的字词开始进行种子设定搜索。 |
| P0 | `search.showLineNumbers` | `false` | 控制是否显示搜索结果所在的行号。 |
| P0 | `search.smartCase` | `false` | Search case-insensitively if the pattern is all lowercase, otherwise,… |
| P0 | `search.sortOrder` | `"default"` | Controls sorting order of search results. |
| P0 | `search.useReplacePreview` | `true` | 控制在选择或替换匹配项时是否打开“替换预览”视图。 |
| P2 | `search.defaultViewMode` | `"list"` | 控制默认搜索结果视图模式。 |
| P2 | `search.experimental.closedNotebookRichContentResults` | `false` | 显示已关闭笔记本的笔记本编辑器中丰富的内容结果。请在更改此设置后刷新搜索结果。 |
| P2 | `search.experimental.useIgnoreFilesInFindFiles` | `false` | 启用后，旧版 `findFiles` 扩展 API 会遵循用户的 `search.useIgnoreFiles` 设置，而非始终忽略 `.… |
| P2 | `search.searchEditor.defaultNumberOfContextLines` | `1` | 创建新的搜索编辑器时要使用的周围上下文行的默认数目。如果使用 "#search.searchEditor.reusePriorSearch… |
| P2 | `search.searchEditor.doubleClickBehaviour` | `"goToLocation"` | 配置在搜索编辑器中双击结果的效果。 |
| P2 | `search.searchEditor.focusResultsOnSearch` | `false` | 触发搜索时，聚焦搜索编辑器结果，而不是搜索编辑器输入。 |
| P2 | `search.searchEditor.reusePriorSearchConfiguration` | `false` | 启用后，新的搜索编辑器将重用之前打开的搜索编辑器的包含、排除和标志。 |
| P2 | `search.searchEditor.singleClickBehaviour` | `"default"` | 配置在搜索编辑器中单击结果的效果。 |
| P2 | `search.searchView.keywordSuggestions` | `false` | 在搜索视图中启用关键字建议。 |
| P2 | `search.searchView.semanticSearchBehavior` | `"manual"` | 控制搜索视图中显示的语义搜索结果的行为。 |

### 文件（25）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `files.associations` | `{}` | 文件 |
| P0 | `files.autoGuessEncoding` | `false` | 启用后，编辑器将尝试在打开文件时猜测字符集编码。还可以按语言配置此设置。请注意，文本搜索不遵守此设置。仅遵守 `files.encodin… |
| P0 | `files.candidateGuessEncodings` | `[]` | 编辑器应尝试按其列出顺序进行猜测的字符集编码的列表。如果无法确定，则遵循 `files.encoding` |
| P0 | `files.encoding` | `"utf8"` | 在读取和写入文件时使用的默认字符集编码。可以按语言对此项进行配置。 |
| P0 | `files.eol` | `"auto"` | 默认行尾字符。 |
| P0 | `files.exclude` | `{"**/.git": true, "**/.svn": true, "**/.hg": true, "**/.jj"…` | 配置 [glob 模式](https://aka.ms/vscode-glob-patterns)以排除文件和文件夹。例如，文件资源管理器… |
| P0 | `files.readonlyExclude` | `{}` | 配置路径或 [glob 模式](https://aka.ms/vscode-glob-patterns)，以避免在该路径或模式因 `fil… |
| P0 | `files.readonlyFromPermissions` | `false` | 当文件权限指示为只读时，将文件标记为只读。可以通过 `files.readonlyInclude` 和 `files.readonlyEx… |
| P0 | `files.readonlyInclude` | `{}` | 配置路径或 [glob 模式](https://aka.ms/vscode-glob-patterns)以标记为只读。glob 模式的计算… |
| P0 | `files.watcherExclude` | `{".git/objects/**": true, ".git/subtree-cache/**": true, ".…` | 配置要从文件监视中排除的路径或 [glob 模式](https://aka.ms/vscode-glob-patterns)。路径可以相对… |
| P0 | `files.watcherInclude` | `[]` | 配置额外路径以监视工作区内的更改。默认情况下，将以递归方式监视所有工作区文件夹，但符号链接的文件夹除外。可以显式添加绝对路径或相对路径，以… |
| P1 | `files.autoSave` | `"off"` | 控制具有未保存更改的编辑器的 [自动保存](https://code.visualstudio.com/docs/editor/codeb… |
| P1 | `files.autoSaveDelay` | `1000` | 控制自动保存具有未保存更改的编辑器之前的延迟(以毫秒为单位)。只有当 `files.autoSave` 设置为 `afterDelay` … |
| P1 | `files.autoSaveWhenNoErrors` | `false` | 启用后，会将编辑器的 [自动保存](https://code.visualstudio.com/docs/editor/codebasic… |
| P1 | `files.autoSaveWorkspaceFilesOnly` | `false` | 启用后，会将编辑器的 [自动保存](https://code.visualstudio.com/docs/editor/codebasic… |
| P1 | `files.defaultLanguage` | `""` | 分配给新文件的默认语言标识符。如果配置为 "${activeEditorLanguage}"，将使用当前活动文本编辑器(如果有)的语言标识… |
| P1 | `files.dialog.defaultPath` | `""` | 文件对话框的默认路径，用于替代用户的主路径。仅在缺少特定于上下文的路径时使用，例如最近打开的文件或文件夹。 |
| P1 | `files.enableTrash` | `true` | 在删除文件或文件夹时，将它们移动到操作系统的“废纸篓”中 (Windows 为“回收站”)。禁用此设置将永久删除文件或文件夹。 |
| P1 | `files.hotExit` | `"onExit"` | [热退出](https://aka.ms/vscode-hot-exit)控制是否在会话之间记住未保存的文件，从而允许在退出编辑器时跳过保… |
| P1 | `files.insertFinalNewline` | `false` | 启用后，保存文件时在文件末尾插入一个最终新行。 |
| P1 | `files.restoreUndoStack` | `true` | 重新打开文件后，还原撤消堆栈。 |
| P1 | `files.saveConflictResolution` | `"askUser"` | 当文件保存到磁盘上并被另一个程序更改时，可能会发生保存冲突。 为了防止数据丢失，要求用户将编辑器中的更改与磁盘上的版本进行比较。 仅当经常… |
| P1 | `files.trimFinalNewlines` | `false` | 启用后，保存文件时将删除在最终新行后的所有新行。 |
| P1 | `files.trimTrailingWhitespace` | `false` | 启用后，将在保存文件时删除行尾的空格。 |
| P1 | `files.trimTrailingWhitespaceInRegexAndStrings` | `true` | 启用后，将在保存时或执行 "editor.action.trimTrailingWhitespace" 时从多行字符串中移除尾随空格，并将… |

### 窗口（30）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `window.border` | `"default"` | 窗口 |
| P0 | `window.commandCenter` | `true` | 将命令启动器与窗口标题一起显示。仅当 `window.customTitleBarVisibility` 设置为 `never` 时，此设… |
| P0 | `window.controlsStyle` | `"native"` | 将窗口控件的外观调整为操作系统原生样式、自定义绘制或隐藏状态。更改需要完全重启才能应用。 |
| P0 | `window.customTitleBarVisibility` | `"auto"` | 调整何时应自定义标题栏。在全屏模式下使用 `windowed` 时，可隐藏自定义标题栏。当 `window.titleBarStyle` … |
| P0 | `window.density.editorTabHeight` | `"default"` | 控制编辑器选项卡的高度。当 `workbench.editor.showTabs` 未设置为 `multiple`时，也适用于标题控件栏。 |
| P0 | `window.density.layout` | `"default"` | Controls the spacing density of the workbench layout. Only applies wh… |
| P0 | `window.dialogStyle` | `"native"` | 将对话框的外观调整为操作系统或自定义的本机样式。 |
| P0 | `window.menuStyle` | `"inherit"` | 将菜单样式调整为操作系统本机、自定义或从 `window.titleBarStyle` 中定义的标题栏样式继承。这也会影响上下文菜单外观。… |
| P0 | `window.newWindowProfile` | `null` | 指定打开新窗口时要使用的配置文件。如果提供了配置文件名称，则新窗口将使用该配置文件。如果未提供配置文件名称，则新窗口将使用活动窗口的配置文… |
| P0 | `window.openWithoutArgumentsInNewWindow` | `"on"` | 在另一实例无参启动时，控制是打开新的空窗口或是聚焦到最后运行的实例。 |
| P0 | `window.restoreFullscreen` | `false` | 控制在全屏模式下退出时窗口是否应还原为全屏模式。 |
| P0 | `window.title` | `"${dirty}${activeEditorShort}${separator}${rootName}${separ…` | 基于打开的工作区或活动编辑器等当前上下文控制窗口标题。根据上下文替换变量: |
| P0 | `window.titleBarStyle` | `"custom"` | 将窗口标题栏的外观调整为操作系统本机或自定义。更改需要完全重启才能应用。 |
| P0 | `window.titleSeparator` | `" - "` | `window.title` 使用的分隔符。 |
| P1 | `window.autoDetectColorScheme` | `false` | 如果已启用，将根据系统颜色模式自动选择颜色主题。如果系统颜色模式为深色，则使用 `workbench.preferredDarkColor… |
| P1 | `window.autoDetectHighContrast` | `true` | 如果已启用，则在操作系统使用高对比度主题时，将自动更改为高对比度主题。要使用的高对比度主题是由“`workbench.preferredH… |
| P1 | `window.closeWhenEmpty` | `false` | 控制关闭最后一个编辑器时是否还应关闭该窗口。此设置仅适用于不显示文件夹的窗口。 |
| P1 | `window.confirmBeforeClose` | `"never"` | 控制是否在关闭窗口或退出应用程序之前显示确认对话框。 |
| P1 | `window.confirmSaveUntitledWorkspace` | `true` | 控制切换到另一个工作区时是否显示确认对话框，以询问是保存还是放弃窗口中打开的无标题工作区。禁用确认对话框将始终放弃无标题工作区。 |
| P1 | `window.customMenuBarAltFocus` | `true` | 控制是否通过按 Alt 键聚焦菜单栏。此设置对使用 Alt 键切换菜单栏没有任何影响。 |
| P1 | `window.doubleClickIconToClose` | `false` | 如果启用，当双击标题栏中的应用程序图标时，此设置将关闭窗口。该窗口将无法使用图标拖动。仅当 `window.titleBarStyle` … |
| P1 | `window.enableMenuBarMnemonics` | `true` | 控制是否可通过 Alt 键快捷键打开主菜单。如果禁用助记符，则可将这些 Alt 键快捷键绑定到编辑器命令。 |
| P1 | `window.menuBarVisibility` | `"classic"` | 控制菜单栏的可见性。“切换”设置表示菜单栏处于隐藏状态，只需按一下 Alt 键即可显示。“精简”设置会将菜单移到边栏中。 |
| P1 | `window.newWindowDimensions` | `"default"` | 控制在至少已打开一个窗口时打开新窗口的维度。请注意，此设置对打开的第一个窗口没有影响。第一个窗口将始终还原为你在关闭之前退出窗口时的大小和… |
| P1 | `window.openFilesInNewWindow` | `"off"` | 控制是否应在使用命令行或文件对话框时在新窗口中打开文件。 |
| P1 | `window.openFoldersInNewWindow` | `"default"` | 控制打开文件夹时是在新窗口打开还是替换上一个活动窗口。 |
| P1 | `window.restoreWindows` | `"all"` | 控制其中的窗口和编辑器在打开时的还原方式。 |
| P1 | `window.systemColorTheme` | `"default"` | 设置本机 UI 元素的颜色模式，例如本机对话框、菜单和标题栏。即使在浅色模式下配置 OS，也可以为窗口选择深色系统颜色主题。还可以配置为根… |
| P1 | `window.zoomLevel` | `0` | 调整所有窗口的默认缩放级别。大于“0”的每个增量（例如“1”）或小于“0”的每个增量（例如“-1”）表示放大或缩小“20%”。还可以输入小… |
| P1 | `window.zoomPerWindow` | `true` | 控制“放大”和“缩小”命令是将缩放级别应用于所有窗口还是仅应用于活动窗口。请参阅 `window.zoomLevel` 以配置所有窗口的默… |

### 编辑器（173）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `editor.allowVariableFonts` | `true` | 控制是否允许在编辑器中使用可变字体。 |
| P0 | `editor.allowVariableFontsInAccessibilityMode` | `false` | 控制是否允许在辅助功能模式下在编辑器中使用可变字体。 |
| P0 | `editor.allowVariableLineHeights` | `true` | 控制是否允许在编辑器中使用可变行高。 |
| P0 | `editor.cursorBlinking` | `"blink"` | 控制光标的动画样式。 |
| P0 | `editor.cursorHeight` | `0` | 当 `editor.cursorStyle` 设置为 `line` 时，控制光标的高度。光标的最大高度取决于行高。 |
| P0 | `editor.cursorStyle` | `"line"` | 在插入输入模式下控制光标样式。 |
| P0 | `editor.cursorSurroundingLines` | `0` | 控制光标周围可见的前置行(最小值为 0)和尾随行(最小值为 1)的最小数目。在其他一些编辑器中称为 “scrollOff” 或 “scro… |
| P0 | `editor.cursorSurroundingLinesStyle` | `"default"` | 控制何时应强制执行 `editor.cursorSurroundingLines`。 |
| P0 | `editor.cursorWidth` | `0` | 当 `editor.cursorStyle` 设置为 `line` 时，控制光标的宽度。 |
| P0 | `editor.doubleClickSelectsBlock` | `true` | 控制双击方括号或引号旁边是否选择其中的内容。 |
| P0 | `editor.emptySelectionClipboard` | `true` | 控制在没有选择内容时进行复制是否复制当前行。 |
| P0 | `editor.fastScrollSensitivity` | `5` | 按下"Alt"时滚动速度倍增。 |
| P0 | `editor.find.addExtraSpaceOnTop` | `true` | 控制 "查找小部件" 是否应在编辑器顶部添加额外的行。如果为 true, 则可以在 "查找小工具" 可见时滚动到第一行之外。 |
| P0 | `editor.find.autoFindInSelection` | `"never"` | 控制自动打开“在选定内容中查找”的条件。 |
| P0 | `editor.find.closeOnResult` | `false` | 控制在显式查找导航命令定位到结果后，查找小组件是否关闭。 |
| P0 | `editor.find.cursorMoveOnType` | `true` | 控制在键入时光标是否应跳转以查找匹配项。 |
| P0 | `editor.find.findOnType` | `true` | 控制在键入时是否应搜索“查找”小组件。 |
| P0 | `editor.find.history` | `"workspace"` | 控制如何存储查找小组件历史记录 |
| P0 | `editor.find.loop` | `true` | 控制在找不到其他匹配项时，是否自动从开头(或结尾)重新开始搜索。 |
| P0 | `editor.find.replaceHistory` | `"workspace"` | 控制如何存储替换小组件历史记录 |
| P0 | `editor.find.seedSearchStringFromSelection` | `"always"` | 控制是否将编辑器选中内容作为搜索词填入到查找小组件中。 |
| P0 | `editor.folding` | `true` | 控制编辑器是否启用了代码折叠。 |
| P0 | `editor.foldingHighlight` | `true` | 控制编辑器是否应突出显示折叠范围。 |
| P0 | `editor.foldingMaximumRegions` | `5000` | 可折叠区域的最大数量。如果当前源具有大量可折叠区域，那么增加此值可能会导致编辑器的响应速度变慢。 |
| P0 | `editor.foldingStrategy` | `"auto"` | 控制计算折叠范围的策略。 |
| P0 | `editor.fontFamily` | `"Consolas, 'Courier New', monospace"` | 控制字体系列。 |
| P0 | `editor.fontLigatures` | `false` | 配置字体连字或字体特性。可以是用于启用/禁用连字的布尔值，或用于设置 CSS "font-feature-settings" 属性值的字符… |
| P0 | `editor.fontSize` | `14` | 控制字体大小(像素)。 |
| P0 | `editor.fontVariations` | `false` | 配置字体变体。可以是用于启用/禁用从 font-weight 到 font-variation-settings 的转换的布尔值，也可以是… |
| P0 | `editor.fontWeight` | `"normal"` | 控制字体粗细。接受关键字“正常”和“加粗”，或者接受介于 1 至 1000 之间的数字。 |
| P0 | `editor.fullwidthCharacterWidth` | `"font"` | Controls the width used to render full-width characters. |
| P0 | `editor.glyphMargin` | `true` | 控制编辑器是否应呈现垂直字形边距。字形边距最常用于调试。 |
| P0 | `editor.gotoLocation.multiple` | `null` | 此设置已弃用，请改用单独的设置，如"editor.editor.gotoLocation.multipleDefinitions"或"ed… |
| P0 | `editor.gotoLocation.multipleDeclarations` | `"peek"` | 控制存在多个目标位置时"转到声明"命令的行为。 |
| P0 | `editor.gotoLocation.multipleDefinitions` | `"peek"` | 控制存在多个目标位置时"转到定义"命令的行为。 |
| P0 | `editor.gotoLocation.multipleImplementations` | `"peek"` | 控制存在多个目标位置时"转到实现"命令的行为。 |
| P0 | `editor.gotoLocation.multipleReferences` | `"peek"` | 控制存在多个目标位置时"转到引用"命令的行为。 |
| P0 | `editor.gotoLocation.multipleTypeDefinitions` | `"peek"` | 控制存在多个目标位置时"转到类型定义"命令的行为。 |
| P0 | `editor.guides.highlightActiveBracketPair` | `true` | 控制编辑器是否应突出显示活动的括号对。 |
| P0 | `editor.guides.highlightActiveIndentation` | `true` | 控制是否突出显示编辑器中活动的缩进参考线。 |
| P0 | `editor.guides.indentation` | `true` | 控制编辑器是否显示缩进参考线。 |
| P0 | `editor.hover.above` | `true` | 如果有空间，首选在线条上方显示悬停。 |
| P0 | `editor.hover.delay` | `300` | 控制显示悬停提示前的等待时间 (毫秒)。 |
| P0 | `editor.hover.enabled` | `"on"` | 控制是否显示悬停提示。 |
| P0 | `editor.hover.hidingDelay` | `300` | 控制隐藏悬停提示前的延迟时间(毫秒)。需要启用 `editor.hover.sticky`。 |
| P0 | `editor.hover.showLongLineWarning` | `true` | 控制是否显示长行警告悬停提示，例如在跳过标记化或暂停渲染时。 |
| P0 | `editor.hover.sticky` | `true` | 控制当鼠标移动到悬停提示上时，其是否保持可见。 |
| P0 | `editor.inertialScroll` | `false` | 为滚动设置惯性 - 最适合用于 Linux 上的触摸板。 |
| P0 | `editor.language.brackets` | `null` | 定义增加和减少缩进的括号。 |
| P0 | `editor.language.colorizedBracketPairs` | `null` | 如果启用方括号对着色，则按照其嵌套级别定义已着色的方括号对。 |
| P0 | `editor.largeFileOptimizations` | `true` | 对大型文件进行特殊处理，禁用某些内存密集型功能。 |
| P0 | `editor.letterSpacing` | `0` | 控制字母间距(像素)。 |
| P0 | `editor.lineHeight` | `0` | 控制行高。 |
| P0 | `editor.lineNumbers` | `"on"` | 控制行号的显示。 |
| P0 | `editor.matchBrackets` | `"always"` | 突出显示匹配的括号。 |
| P0 | `editor.maxTokenizationLineLength` | `20000` | 由于性能原因，超过这个长度的行将不会被标记 |
| P0 | `editor.mouseWheelScrollSensitivity` | `1` | 对鼠标滚轮滚动事件的 `deltaX` 和 `deltaY` 乘上的系数。 |
| P0 | `editor.mouseWheelZoom` | `false` | 按住 `Ctrl` 键并滚动鼠标滚轮时对编辑器字体大小进行缩放。 |
| P0 | `editor.occurrencesHighlight` | `"singleFile"` | 控制是否应突出显示在打开的文件中的出现次数。 |
| P0 | `editor.occurrencesHighlightFromAllProviders` | `false` | Controls whether single-document highlights from all matching provide… |
| P0 | `editor.overtypeCursorStyle` | `"block"` | 在改写输入模式中控制光标样式。 |
| P0 | `editor.overtypeOnPaste` | `true` | 控制粘贴是否改写。 |
| P0 | `editor.padding.bottom` | `0` | 控制编辑器的底边和最后一行之间的间距量。 |
| P0 | `editor.padding.top` | `0` | 控制编辑器的顶边和第一行之间的间距量。 |
| P0 | `editor.peekWidgetDefaultFocus` | `"tree"` | 控制是将焦点放在内联编辑器上还是放在预览小部件中的树上。 |
| P0 | `editor.renderFinalNewline` | `"on"` | 当文件以换行符结束时, 呈现最后一行的行号。 |
| P0 | `editor.renderLineHighlight` | `"line"` | 控制编辑器的当前行进行高亮显示的方式。 |
| P0 | `editor.renderLineHighlightOnlyWhenFocus` | `false` | 控制编辑器是否仅在焦点在编辑器时突出显示当前行。 |
| P0 | `editor.renderWhitespace` | `"selection"` | 控制编辑器在空白字符上显示符号的方式。 |
| P0 | `editor.roundedSelection` | `true` | 控制选区是否有圆角。 |
| P0 | `editor.scrollBeyondLastColumn` | `4` | 控制编辑器水平滚动时可以超过范围的字符数。 |
| P0 | `editor.scrollBeyondLastLine` | `true` | 控制编辑器是否可以滚动到最后一行之后。 |
| P0 | `editor.scrollOnMiddleClick` | `false` | 控制在按下中间按钮时编辑器是否滚动。 |
| P0 | `editor.scrollPredominantAxis` | `true` | 同时垂直和水平滚动时，仅沿主轴滚动。在触控板上垂直滚动时，可防止水平漂移。 |
| P0 | `editor.scrollbar.horizontal` | `"auto"` | 控制水平滚动条的可见性。 |
| P0 | `editor.scrollbar.horizontalScrollbarSize` | `12` | 水平滚动条的高度。 |
| P0 | `editor.scrollbar.ignoreHorizontalScrollbarInContentHeight` | `false` | 设置后，水平滚动条将不会增加编辑器内容的大小。 |
| P0 | `editor.scrollbar.scrollByPage` | `false` | 控制单击按页滚动还是跳转到单击位置。 |
| P0 | `editor.scrollbar.vertical` | `"auto"` | 控制垂直滚动条的可见性。 |
| P0 | `editor.scrollbar.verticalScrollbarSize` | `14` | 垂直滚动条的宽度。 |
| P0 | `editor.selectedTextMatchMode` | `"findOptions"` | Controls how occurrences of selected text are matched when selecting … |
| P0 | `editor.selectionHighlight` | `true` | 控制编辑器是否应突出显示与所选内容类似的匹配项。 |
| P0 | `editor.selectionHighlightMaxLength` | `200` | Controls how many characters can be in the selection before similar m… |
| P0 | `editor.selectionHighlightMultiline` | `false` | 控制编辑器是否应突出显示跨多行的选择匹配项。 |
| P0 | `editor.showFoldingControls` | `"mouseover"` | 控制何时显示行号槽上的折叠控件。 |
| P0 | `editor.stablePeek` | `false` | 保持速览编辑器处于打开状态，即使双击其中的内容或者点击 `Escape` 键也是如此。 |
| P0 | `editor.stickyTabStops` | `false` | 在使用空格进行缩进时模拟制表符的选择行为。所选内容将始终使用制表符停止位。 |
| P0 | `editor.tokenColorCustomizations` | `{}` | 替代当前所选颜色主题中的编辑器语法颜色和字形。 |
| P0 | `editor.trimWhitespaceOnDelete` | `false` | 控制在删除新行时编辑器是否还会删除下一行的缩进空格。 |
| P0 | `editor.unfoldOnClickAfterEndOfLine` | `false` | 控制单击已折叠的行后面的空内容是否会展开该行。 |
| P0 | `editor.wordBreak` | `"normal"` | 控制中文/日语/韩语(CJK)文本使用的断字规则。 |
| P0 | `editor.wordSegmenterLocales` | `[]` | 执行与字词相关的导航或操作时用于分词的区域设置。指定要识别的字词的 BCP 47 语言标记(如 ja、zh-CN、zh-Hant-TW 等… |
| P0 | `editor.wordSeparators` | `"`~!@#$%^&*()-=+[{]}\\\|;:'\",.<>/?"` | 执行单词相关的导航或操作时作为单词分隔符的字符。 |
| P0 | `editor.wrapOnEscapedLineFeeds` | `false` | 控制在启用 `editor.wordWrap` 时文本 `\n` 是否应触发 wordWrap。 |
| P1 | `editor.autoClosingBrackets` | `"languageDefined"` | 控制编辑器是否在左括号后自动插入右括号。 |
| P1 | `editor.autoClosingComments` | `"languageDefined"` | 控制在用户添加打开注释后编辑器是否应自动关闭注释。 |
| P1 | `editor.autoClosingDelete` | `"auto"` | 控制在删除时编辑器是否应删除相邻的右引号或右方括号。 |
| P1 | `editor.autoClosingOvertype` | `"auto"` | 控制编辑器是否应改写右引号或右括号。 |
| P1 | `editor.autoClosingQuotes` | `"languageDefined"` | 控制编辑器是否在左引号后自动插入右引号。 |
| P1 | `editor.autoIndent` | `"full"` | 控制编辑器是否应在用户键入、粘贴、移动或缩进行时自动调整缩进。 |
| P1 | `editor.autoIndentOnPaste` | `false` | 控制编辑器是否应自动缩进粘贴的内容。 |
| P1 | `editor.autoIndentOnPasteWithinString` | `true` | 控制编辑器是否应在粘贴到字符串中时自动缩进粘贴的内容。当 autoIndentOnPaste 为 true 时，此操作将会生效。 |
| P1 | `editor.autoSurround` | `"languageDefined"` | 控制在键入引号或方括号时，编辑器是否应自动将所选内容括起来。 |
| P1 | `editor.bracketPairColorization.enabled` | `true` | 控制是否启用括号对着色。请使用 `workbench.colorCustomizations` 重写括号突出显示颜色。 |
| P1 | `editor.bracketPairColorization.independentColorPoolPerBracketType` | `false` | 控制每个方括号类型是否具有自己的独立颜色池。 |
| P1 | `editor.columnSelection` | `false` | 启用使用鼠标和键进行列选择。 |
| P1 | `editor.comments.ignoreEmptyLines` | `true` | 控制在对行注释执行切换、添加或删除操作时，是否应忽略空行。 |
| P1 | `editor.comments.insertSpace` | `true` | 控制在注释时是否插入空格字符。 |
| P1 | `editor.cursorSmoothCaretAnimation` | `"off"` | 控制是否启用平滑插入动画。 |
| P1 | `editor.defaultFormatter` | `null` | 定义一个默认格式化程序, 该格式化程序优先于所有其他格式化程序设置。必须是提供格式化程序的扩展的标识符。 |
| P1 | `editor.definitionLinkOpensInPeek` | `false` | 控制"转到定义"鼠标手势是否始终打开预览小部件。 |
| P1 | `editor.detectIndentation` | `true` | 控制在基于文件内容打开文件时是否自动检测 `editor.tabSize` 和 `editor.insertSpaces`。 |
| P1 | `editor.formatOnPaste` | `false` | 控制编辑器是否自动格式化粘贴的内容。格式化程序必须可用，并且能针对文档中的某一范围进行格式化。 |
| P1 | `editor.formatOnSave` | `false` | 保存时设置文件格式。格式化程序必须可用，并且不得关闭编辑器。当 `files.autoSave` 设置为 `afterDelay` 时，仅… |
| P1 | `editor.formatOnSaveMode` | `"file"` | 控制在保存时设置格式是设置整个文件格式还是仅设置修改内容的格式。仅当 "#editor.formatOnSave#" 处于启用状态时适用。 |
| P1 | `editor.guides.bracketPairs` | `false` | 控制是否启用括号对指南。 |
| P1 | `editor.guides.bracketPairsHorizontal` | `"active"` | 控制是否启用水平括号对指南。 |
| P1 | `editor.hideCursorInOverviewRuler` | `false` | 控制是否在概览标尺中隐藏光标。 |
| P1 | `editor.indentSize` | `"tabSize"` | 用于缩进或 `"tabSize"` 的空格数，可使用 `editor.tabSize` 中的值。当 `editor.detectInden… |
| P1 | `editor.insertSpaces` | `true` | 按 `Tab` 时插入空格。当 `editor.detectIndentation` 打开时，将根据文件内容替代此设置。 |
| P1 | `editor.links` | `true` | 控制是否在编辑器中检测链接并使其可被点击。 |
| P1 | `editor.minimap.autohide` | `"none"` | 控制是否自动隐藏缩略图。 |
| P1 | `editor.minimap.enabled` | `true` | 控制是否显示缩略图。 |
| P1 | `editor.minimap.markSectionHeaderRegex` | `"\\bMARK:\\s*(?<separator>-?)\\s*(?<label>.*)$"` | 定义用于在注释中查找节标头的正则表达式。正则表达式必须包含命名匹配组“label”，(写为 “(？<label>.+)”) 封装节标头，否… |
| P1 | `editor.minimap.maxColumn` | `120` | 限制缩略图的宽度，控制其最多显示的列数。 |
| P1 | `editor.minimap.scale` | `1` | 在迷你地图中绘制的内容比例: 1、2 或 3。 |
| P1 | `editor.minimap.sectionHeaderFontSize` | `9` | 控制缩略图中节标题的字号。 |
| P1 | `editor.minimap.sectionHeaderLetterSpacing` | `1` | 控制节标头字符之间的空间量(以像素为单位)。这有助于提高小字体大小的标题的可读性。 |
| P1 | `editor.minimap.showMarkSectionHeaders` | `true` | 控制 MARK: 命令是否在缩略图中显示为节标题。 |
| P1 | `editor.minimap.showRegionSectionHeaders` | `true` | 控制命名区域是否在缩略图中显示为节标题。 |
| P1 | `editor.minimap.showSlider` | `"mouseover"` | 控制何时显示迷你地图滑块。 |
| P1 | `editor.minimap.side` | `"right"` | 控制在哪一侧显示缩略图。 |
| P1 | `editor.minimap.size` | `"proportional"` | 控制迷你地图的大小。 |
| P1 | `editor.multiCursorLimit` | `10000` | 控制一次可以在活动编辑器中显示的最大游标数。 |
| P1 | `editor.multiCursorMergeOverlapping` | `true` | 当多个光标重叠时进行合并。 |
| P1 | `editor.multiCursorModifier` | `"alt"` | 用于使用鼠标添加多个游标的修饰符。“转到定义”和“打开链接”鼠标手势将进行调整，使其不与 [多光标修饰符](https://code.vi… |
| P1 | `editor.multiCursorPaste` | `"spread"` | 控制粘贴时粘贴文本的行计数与光标计数相匹配。 |
| P1 | `editor.overviewRulerBorder` | `true` | 控制是否在概览标尺周围绘制边框。 |
| P1 | `editor.rulers` | `[]` | 在一定数量的等宽字符后显示垂直标尺。输入多个值，显示多个标尺。若数组为空，则不绘制标尺。 |
| P1 | `editor.semanticHighlighting.enabled` | `"configuredByTheme"` | 控制是否为支持它的语言显示语义突出显示。 |
| P1 | `editor.semanticTokenColorCustomizations` | `{}` | 从当前所选颜色主题重写编辑器语义标记颜色和样式。 |
| P1 | `editor.smartSelect.selectLeadingAndTrailingWhitespace` | `true` | 是否应始终选择前导和尾随空格。 |
| P1 | `editor.smartSelect.selectSubwords` | `true` | 是否应选择子字(如“fooBar”或“foo_bar”中的“foo”)。 |
| P1 | `editor.smoothScrolling` | `false` | 控制编辑器是否使用动画滚动。 |
| P1 | `editor.stickyScroll.defaultModel` | `"outlineModel"` | 定义用于确定要粘贴的行的模型。如果大纲模型不存在，它将回退到回退到缩进模型的折叠提供程序模型上。在所有三种情况下都遵循此顺序。 |
| P1 | `editor.stickyScroll.enabled` | `true` | 在编辑器顶部的滚动过程中显示嵌套的当前作用域。 |
| P1 | `editor.stickyScroll.maxLineCount` | `5` | 定义要显示的最大粘滞行数。 |
| P1 | `editor.stickyScroll.scrollWithEditor` | `true` | 使用编辑器的水平滚动条启用粘滞滚动。 |
| P1 | `editor.tabSize` | `4` | 一个制表符等于的空格数。当 `editor.detectIndentation` 打开时，将根据文件内容替代此设置。 |
| P1 | `editor.trimAutoWhitespace` | `true` | 删除自动插入的尾随空白符号。 |
| P1 | `editor.useTabStops` | `true` | 空格和制表符的插入和删除与制表位对齐。 |
| P1 | `editor.wordWrap` | `"off"` | 控制折行的方式。 |
| P1 | `editor.wordWrapColumn` | `80` | 在 `editor.wordWrap` 为 `wordWrapColumn` 或 `bounded` 时，控制编辑器的折行列。 |
| P1 | `editor.wordWrapIndicator` | `false` | Controls whether an indicator is rendered at the wrapping column of l… |
| P1 | `editor.wrappingIndent` | `"same"` | 控制折行的缩进。 |
| P1 | `editor.wrappingStrategy` | `"simple"` | 控制计算换行位置的算法。请注意，在辅助功能模式下，高级版将用于提供最佳体验。 |
| P2 | `editor.accessibilityPageSize` | `500` | 控制编辑器中可由屏幕阅读器一次读出的行数。我们检测到屏幕阅读器时，会自动将默认值设置为 500。警告: 如果行数大于默认值，可能会影响性能。 |
| P2 | `editor.accessibilitySupport` | `"auto"` | 控制 UI 是否应在已针对屏幕阅读器进行优化的模式下运行。 |
| P2 | `editor.copyWithSyntaxHighlighting` | `true` | 控制在复制时是否同时复制语法高亮。 |
| P2 | `editor.dragAndDrop` | `true` | 控制在编辑器中是否允许通过拖放来移动选中内容。 |
| P2 | `editor.mouseMiddleClickAction` | `"default"` | 控制编辑器中单击鼠标中键时的操作。 |
| P2 | `editor.renderControlCharacters` | `true` | 控制编辑器是否显示控制字符。 |
| P2 | `editor.renderRichScreenReaderContent` | `false` | 启用 `editor.editContext` 设置时是否呈现丰富的屏幕阅读器内容。 |
| P2 | `editor.tabFocusMode` | `false` | 控制编辑器是接收选项卡还是将其延迟到工作台进行导航。 |
| P2 | `editor.unicodeHighlight.allowedCharacters` | `{}` | 定义未突出显示的允许字符。 |
| P2 | `editor.unicodeHighlight.allowedLocales` | `{"_os": true, "_vscode": true}` | 未突出显示在允许区域设置中常见的 Unicode 字符。 |
| P2 | `editor.unicodeHighlight.ambiguousCharacters` | `true` | 控制是否突出显示可能与基本 ASCII 字符混淆的字符，但当前用户区域设置中常见的字符除外。 |
| P2 | `editor.unicodeHighlight.includeComments` | `"inUntrustedWorkspace"` | 控制注释中的字符是否也应进行 Unicode 突出显示。 |
| P2 | `editor.unicodeHighlight.includeStrings` | `true` | 控制字符串中的字符是否也应进行 Unicode 突出显示。 |
| P2 | `editor.unicodeHighlight.invisibleCharacters` | `true` | 控制是否突出显示仅保留空格或完全没有宽度的字符。 |
| P2 | `editor.unicodeHighlight.nonBasicASCII` | `"inUntrustedWorkspace"` | 控制是否突出显示所有非基本 ASCII 字符。只有介于 U+0020 到 U+007E 之间的字符、制表符、换行符和回车符才被视为基本 A… |
| P2 | `editor.unusualLineTerminators` | `"prompt"` | 删除可能导致问题的异常行终止符。 |
| P2 | `workbench.editor.dragToOpenWindow` | `true` | 控制是否可将编辑器拖出窗口以在新窗口中打开。长按 Alt 键，同时拖动以动态切换。 |

### 语言级覆盖（32）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `[ansible][azure-pipelines][css][dockerfile][dockercompose][html][json][jsonc][less][postcss][scss][stylus][vue][yaml]` | `{"gitlens.codeLens.scopes": ["document"]}` | 默认语言配置替代 |
| P0 | `[chatagent]` | `{"editor.insertSpaces": true, "editor.tabSize": 2, "editor.…` | 配置要为 chatagent 替代的设置。 |
| P0 | `[clojure]` | `{"diffEditor.ignoreTrimWhitespace": false}` | 配置要为 clojure 替代的设置。 |
| P0 | `[coffeescript]` | `{"diffEditor.ignoreTrimWhitespace": false, "editor.defaultC…` | 配置要为 coffeescript 替代的设置。 |
| P0 | `[csharp]` | `{"editor.maxTokenizationLineLength": 2500}` | 配置要为 csharp 替代的设置。 |
| P0 | `[css]` | `{"editor.suggest.insertMode": "replace"}` | 配置要为 css 替代的设置。 |
| P0 | `[dockercompose]` | `{"editor.insertSpaces": true, "editor.tabSize": 2, "editor.…` | 配置要为 dockercompose 替代的设置。 |
| P0 | `[dockerfile]` | `{"editor.quickSuggestions": {"strings": true}}` | 配置要为 dockerfile 替代的设置。 |
| P0 | `[fsharp]` | `{"diffEditor.ignoreTrimWhitespace": false}` | 配置要为 fsharp 替代的设置。 |
| P0 | `[go]` | `{"editor.insertSpaces": false}` | 配置要为 go 替代的设置。 |
| P0 | `[handlebars]` | `{"editor.suggest.insertMode": "replace"}` | 配置要为 handlebars 替代的设置。 |
| P0 | `[html]` | `{"editor.suggest.insertMode": "replace"}` | 配置要为 html 替代的设置。 |
| P0 | `[instructions]` | `{"editor.insertSpaces": true, "editor.tabSize": 2, "editor.…` | 配置要为 instructions 替代的设置。 |
| P0 | `[jade]` | `{"diffEditor.ignoreTrimWhitespace": false}` | 配置要为 jade 替代的设置。 |
| P0 | `[javascript]` | `{"editor.maxTokenizationLineLength": 2500}` | 配置要为 javascript 替代的设置。 |
| P0 | `[json]` | `{"editor.quickSuggestions": {"strings": true}, "editor.sugg…` | 配置要为 json 替代的设置。 |
| P0 | `[jsonc]` | `{"editor.quickSuggestions": {"strings": true}, "editor.sugg…` | 配置要为 jsonc 替代的设置。 |
| P0 | `[julia]` | `{"editor.defaultColorDecorators": "never"}` | 配置要为 julia 替代的设置。 |
| P0 | `[less]` | `{"editor.suggest.insertMode": "replace"}` | 配置要为 less 替代的设置。 |
| P0 | `[makefile]` | `{"editor.insertSpaces": false}` | 配置要为 makefile 替代的设置。 |
| P0 | `[markdown]` | `{"editor.unicodeHighlight.ambiguousCharacters": false, "edi…` | 配置要为 markdown 替代的设置。 |
| P0 | `[plaintext]` | `{"editor.unicodeHighlight.ambiguousCharacters": false, "edi…` | 配置要为 plaintext 替代的设置。 |
| P0 | `[prompt]` | `{"editor.insertSpaces": true, "editor.tabSize": 2, "editor.…` | 配置要为 prompt 替代的设置。 |
| P0 | `[python]` | `{"diffEditor.ignoreTrimWhitespace": false, "editor.defaultC…` | 配置要为 python 替代的设置。 |
| P0 | `[ruby]` | `{"editor.defaultColorDecorators": "never"}` | 配置要为 ruby 替代的设置。 |
| P0 | `[scss]` | `{"editor.suggest.insertMode": "replace"}` | 配置要为 scss 替代的设置。 |
| P0 | `[search-result]` | `{"editor.lineNumbers": "off"}` | 配置要为 search-result 替代的设置。 |
| P0 | `[shellscript]` | `{"files.eol": "\n", "editor.defaultColorDecorators": "never…` | 配置要为 shellscript 替代的设置。 |
| P0 | `[skill]` | `{"editor.insertSpaces": true, "editor.tabSize": 2, "editor.…` | 配置要为 skill 替代的设置。 |
| P0 | `[snippets]` | `{"editor.quickSuggestions": {"strings": true}, "editor.sugg…` | 配置要为 snippets 替代的设置。 |
| P0 | `[xaml]` | `{"editor.wordBasedSuggestions": "off"}` | 配置要为 xaml 替代的设置。 |
| P0 | `[yaml]` | `{"editor.insertSpaces": true, "editor.tabSize": 2, "editor.…` | 配置要为 yaml 替代的设置。 |

### 资源管理器（23）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `explorer.autoOpenDroppedFile` | `true` | 文件资源管理器 |
| P0 | `explorer.autoReveal` | `true` | 控制资源管理器是否应在打开文件时自动显示并选择文件。 |
| P0 | `explorer.autoRevealExclude` | `{"**/node_modules": true, "**/bower_components": true}` | 配置路径或 [glob 模式](https://aka.ms/vscode-glob-patterns)，以便在打开文件和文件夹时避免在资… |
| P0 | `explorer.compactFolders` | `true` | 控制资源管理器是否应以紧凑形式呈现文件夹。在此形式中，单个子文件夹将被压缩在组合的树元素中。例如，对 Java 包结构非常有用。 |
| P0 | `explorer.confirmDelete` | `true` | 控制在删除文件和文件夹时资源管理器是否应要求进行确认。 |
| P0 | `explorer.confirmDragAndDrop` | `true` | 控制资源管理器是否应在通过拖放移动文件或文件夹时要求确认。 |
| P0 | `explorer.confirmPasteNative` | `true` | 控制在粘贴本机文件和文件夹时资源管理器是否应要求进行确认。 |
| P0 | `explorer.confirmUndo` | `"default"` | 控制资源管理器是否应在撤消时要求确认。 |
| P0 | `explorer.copyPathSeparator` | `"auto"` | 复制文件路径时使用的路径分隔字符。 |
| P0 | `explorer.copyRelativePathSeparator` | `"auto"` | 复制相对文件路径时使用的路径分隔字符。 |
| P0 | `explorer.enableDragAndDrop` | `true` | 控制资源管理器是否应允许通过拖放来移动文件和文件夹。此设置仅影响从浏览器内部进行拖放。 |
| P0 | `explorer.enableUndo` | `true` | 控制资源管理器是否应支持撤消文件和文件夹操作。 |
| P0 | `explorer.expandSingleFolderWorkspaces` | `true` | 控制资源管理器是否应在初始化期间展开仅包含一个文件夹的多根工作区 |
| P0 | `explorer.incrementalNaming` | `"simple"` | 选择在粘贴同名资源管理器项时要使用的重命名方式。 |
| P0 | `explorer.sortOrder` | `"default"` | 控制资源管理器中文件和文件夹基于属性的排序。启用 `explorer.fileNesting.enabled` 后，还控制嵌套文件的排序。 |
| P0 | `explorer.sortOrderLexicographicOptions` | `"default"` | 在资源管理器中控制文件和文件夹名称的词典排序。 |
| P0 | `explorer.sortOrderReverse` | `false` | 控制是否应颠倒文件和文件夹的排序顺序。 |
| P2 | `explorer.fileNesting.enabled` | `false` | 控制是否已在资源管理器中启用文件嵌套。文件嵌套允许目录中的相关文件在单个父文件下以可视方式组合在一起。 |
| P2 | `explorer.fileNesting.expand` | `true` | 控制是否自动扩展文件嵌套。要使此操作生效，必须设置 `explorer.fileNesting.enabled`。 |
| P2 | `explorer.fileNesting.patterns` | `{"*.ts": "${capture}.js", "*.js": "${capture}.js.map, ${cap…` | 控制资源管理器中的文件嵌套。必须设置 `explorer.fileNesting.enabled` 才能让它生效。每个 __Item__ … |
| P2 | `explorer.openEditors.minVisible` | `0` | “打开编辑器”窗格中预分配的编辑器槽数下限。如果设置为 0，则“打开编辑器”窗格将根据编辑器数量动态重设大小。 |
| P2 | `explorer.openEditors.sortOrder` | `"editorOrder"` | 控制编辑器在“打开编辑器”窗格中的排序顺序。 |
| P2 | `explorer.openEditors.visible` | `9` | “打开编辑器”窗格中显示的初始编辑器数上限。超过此限制将显示滚动条，并允许调整窗格大小以显示更多项目。 |

### 键位（1）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P0 | `keyboard.mapAltGrToCtrlAlt` | `false` | 键盘 |

### JSON（4）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P1 | `json.format.enable` | `true` | 启用或禁用默认 JSON 格式化程序。 |
| P1 | `json.format.keepLines` | `false` | 设置格式时保留所有现有新行。 |
| P1 | `json.maxItemsComputed` | `5000` | 计算的大纲符号和折叠区域的最大数量(因性能原因而受限)。 |
| P1 | `json.trace.server` | `"off"` | 跟踪 VS Code 和 JSON 语言服务器之间的通信。 |

### 图片预览（2）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P1 | `mediaPreview.video.autoPlay` | `false` | Media Preview |
| P1 | `mediaPreview.video.loop` | `false` | Loop videos over again automatically. |

### 导航（1）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P1 | `references.preferredLocation` | `"peek"` | 引用搜索视图 |

### 布局（8）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P1 | `zenMode.centerLayout` | `true` | 禅模式 |
| P1 | `zenMode.fullScreen` | `true` | 控制在打开禅模式时是否将工作台切换到全屏。 |
| P1 | `zenMode.hideActivityBar` | `true` | 控制在打开禅模式时是否隐藏工作台左侧或右侧的活动栏。 |
| P1 | `zenMode.hideLineNumbers` | `true` | 控制在打开禅模式时是否隐藏编辑器行号。 |
| P1 | `zenMode.hideStatusBar` | `true` | 控制在打开禅模式时是否隐藏工作台底部的状态栏。 |
| P1 | `zenMode.restore` | `true` | 控制在禅模式下退出的窗口是否应还原为禅模式。 |
| P1 | `zenMode.showTabs` | `"multiple"` | 控制打开禅模式是应该显示多个编辑器选项卡、单个编辑器选项卡还是完全隐藏编辑器标题区域。 |
| P1 | `zenMode.silentNotifications` | `true` | 控制在禅模式下是否应启用通知请勿打扰模式。如果为 true，则仅弹出错误通知。 |

### 设置（6）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P1 | `workbench.settings.alwaysShowAdvancedSettings` | `false` | 控制是否始终在设置编辑器中显示高级设置，而无需使用 `@tag:advanced` 筛选器。 |
| P1 | `workbench.settings.editor` | `"ui"` | 确定默认情况下要使用的“设置”编辑器。 |
| P1 | `workbench.settings.openDefaultKeybindings` | `false` | 控制在打开按键绑定设置时是否同时打开显示所有默认按键绑定的编辑器。 |
| P1 | `workbench.settings.openDefaultSettings` | `false` | 控制在打开设置时是否同时打开显示所有默认设置的编辑器。 |
| P1 | `workbench.settings.settingsSearchTocBehavior` | `"filter"` | 控制设置编辑器的目录在搜索时的行为。如果在“设置”编辑器中更改此设置，则更改后的设置将在修改搜索查询后生效。 |
| P1 | `workbench.settings.useSplitJSON` | `false` | 控制在将设置编辑为 json 时是否使用拆分 json 编辑器。 |

### 问题（6）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P1 | `problems.autoReveal` | `true` | 问题 |
| P1 | `problems.decorations.enabled` | `true` | 在文件和文件夹上显示错误和警告。被关闭时由 `problems.visibility` 覆盖。 |
| P1 | `problems.defaultViewMode` | `"tree"` | 控制“问题”视图的默认视图模式。 |
| P1 | `problems.showCurrentInStatus` | `false` | 启用后，状态栏中将显示当前问题。 |
| P1 | `problems.sortOrder` | `"severity"` | 控制问题导航的顺序。 |
| P1 | `problems.visibility` | `true` | 控制问题是否在整个编辑器和工作台中可见。 |

### 对比（22）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P2 | `diffEditor.codeLens` | `false` | 编辑器 |
| P2 | `diffEditor.diffAlgorithm` | `"advanced"` | - legacy: 使用旧差异算法。 |
| P2 | `diffEditor.experimental.showEmptyDecorations` | `true` | 控制差异编辑器是否显示空修饰，以查看插入或删除字符的位置。 |
| P2 | `diffEditor.experimental.showMoves` | `false` | 控制差异编辑器是否应显示检测到的代码移动。 |
| P2 | `diffEditor.experimental.useTrueInlineView` | `false` | 如果已启用并且编辑器使用内联视图，则将以内联方式呈现字词更改。 |
| P2 | `diffEditor.hideOriginalLineNumbers` | `null` | Controls whether original line numbers are hidden in inline diffs. Wh… |
| P2 | `diffEditor.hideUnchangedRegions.contextLineCount` | `3` | 控制在比较未改变的区域时使用多少行作为上下文。 |
| P2 | `diffEditor.hideUnchangedRegions.enabled` | `false` | 控制差异编辑器是否显示未更改的区域。 |
| P2 | `diffEditor.hideUnchangedRegions.minimumLineCount` | `3` | 控制将多少行用作未更改区域的最小值。 |
| P2 | `diffEditor.hideUnchangedRegions.revealLineCount` | `20` | 控制用于未更改区域的行数。 |
| P2 | `diffEditor.ignoreTrimWhitespace` | `true` | 启用后，差异编辑器将忽略前导空格或尾随空格中的更改。 |
| P2 | `diffEditor.maxComputationTime` | `5000` | 超时(以毫秒为单位)，之后将取消差异计算。使用0表示没有超时。 |
| P2 | `diffEditor.maxFileSize` | `50` | 要为其计算差异的最大文件大小(MB)。使用 0 表示无限制。 |
| P2 | `diffEditor.renderGutterMenu` | `true` | 启用后，差异编辑器将显示用于还原和阶段操作的特殊装订线。 |
| P2 | `diffEditor.renderIndicators` | `true` | 控制差异编辑器是否为添加/删除的更改显示 +/- 指示符号。 |
| P2 | `diffEditor.renderMarginRevertIcon` | `true` | 启用后，差异编辑器会在其字形边距中显示箭头以还原更改。 |
| P2 | `diffEditor.renderSideBySide` | `true` | 控制差异编辑器的显示方式是并排还是内联。 |
| P2 | `diffEditor.renderSideBySideInlineBreakpoint` | `900` | 如果差异编辑器宽度小于此值，则使用内联视图。 |
| P2 | `diffEditor.useInlineViewWhenSpaceIsLimited` | `true` | 如果启用并且编辑器宽度太小，则使用内联视图。 |
| P2 | `diffEditor.wordWrap` | `"inherit"` | - off: 永不换行。 |
| P2 | `multiDiffEditor.experimental.fileList` | `false` | Multi Diff Editor |
| P2 | `multiDiffEditor.experimental.variant` | `"noCards"` | Controls the experimental variant of the multi diff editor. |

### 无障碍（102）

| 优先级 | 设置项 | VS Code 默认值 | 说明 |
|---|---|---|---|
| P2 | `accessibility.accessibleView.closeOnKeyPress` | `true` | 辅助功能 |
| P2 | `accessibility.chat.showCheckmarks` | `false` | 工作台 |
| P2 | `accessibility.debugWatchVariableAnnouncements` | `true` | 控制是否应在调试监视视图中公布变量更改。 |
| P2 | `accessibility.dimUnfocused.enabled` | `false` | 是否将未聚焦的编辑器和终端变暗，这可以更清楚地说明键入的输入将转到何处。这适用于大多数编辑器，但使用 iframe(如笔记本和扩展 Web… |
| P2 | `accessibility.dimUnfocused.opacity` | `0.75` | 不透明度分数 (0.2 到 1.0) 用于未聚焦的编辑器和终端。这仅在启用 `accessibility.dimUnfocused.ena… |
| P2 | `accessibility.hideAccessibleView` | `false` | 控制是否隐藏辅助视图。 |
| P2 | `accessibility.openChatEditedFiles` | `false` | 控制在聊天代理对文件应用编辑后是否应打开文件。 |
| P2 | `accessibility.replEditor.autoFocusReplExecution` | `"input"` | 控制执行代码时是否应自动将焦点发送到 REPL。 |
| P2 | `accessibility.replEditor.readLastExecutionOutput` | `true` | 控制是否将宣布来自本机 REPL 中执行的输出。 |
| P2 | `accessibility.signalOptions.debouncePositionChanges` | `false` | 是否应停用位置更改 |
| P2 | `accessibility.signalOptions.experimental.delays.errorAtPosition` | `{}` |  |
| P2 | `accessibility.signalOptions.experimental.delays.general` | `{}` | Delays for all signals besides error and warning at position |
| P2 | `accessibility.signalOptions.experimental.delays.warningAtPosition` | `{}` |  |
| P2 | `accessibility.signalOptions.volume` | `70` | 声音音量百分比(0-100)。 |
| P2 | `accessibility.signals.chatEditModifiedFile` | `{"sound": "auto"}` | 在显示包含聊天编辑更改的文件时播放声音/音频提示 |
| P2 | `accessibility.signals.chatRequestSent` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 - 声音(音频提示)和/或公告(警报) - 发出聊天请求时。 |
| P2 | `accessibility.signals.chatResponseReceived` | `{"sound": "auto"}` | 收到响应时播放声音/音频提示。 |
| P2 | `accessibility.signals.chatUserActionRequired` | `{"sound": "auto", "announcement": "auto"}` | 当聊天中需要用户操作时，播放信号 - 声音(音频提示)和/或通知(警报)。 |
| P2 | `accessibility.signals.clear` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 - 声音(音频提示)和/或公告(警报) - 清除功能(例如，终端、调试控制台或输出通道)时。 |
| P2 | `accessibility.signals.codeActionApplied` | `{"sound": "auto"}` | 应用代码操作后播放声音/音频提示。 |
| P2 | `accessibility.signals.codeActionTriggered` | `{"sound": "auto"}` | 播放声音/音频提示 - 触发代码操作时。 |
| P2 | `accessibility.signals.confetti` | `{"sound": "auto"}` | Plays a joyful sound / audio cue when confetti is shown. |
| P2 | `accessibility.signals.diffLineDeleted` | `{"sound": "auto"}` | 当焦点在可访问差异查看器模式下移动到删除行或下一个/上一个更改时播放声音/音频提示。 |
| P2 | `accessibility.signals.diffLineInserted` | `{"sound": "auto"}` | 当焦点在可访问差异查看器模式下移动到插入行或下一个/上一个更改时播放声音/音频提示。 |
| P2 | `accessibility.signals.diffLineModified` | `{"sound": "auto"}` | 当焦点在可访问差异查看器模式下移动到修改行或下一个/上一个更改时播放声音/音频提示。 |
| P2 | `accessibility.signals.editsKept` | `{"sound": "auto", "announcement": "auto"}` | 保存编辑后播放信号 - 声音(音频提示)和/或通知(警报)。 |
| P2 | `accessibility.signals.editsUndone` | `{"sound": "auto", "announcement": "auto"}` | 撤消编辑后播放信号 - 声音(音频提示)和/或通知(警报)。 |
| P2 | `accessibility.signals.format` | `{"sound": "never", "announcement": "never"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 设置文件或笔记本的格式时。 |
| P2 | `accessibility.signals.lineHasBreakpoint` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 活动行出现断点时。 |
| P2 | `accessibility.signals.lineHasError` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 活动行出现错误时。 |
| P2 | `accessibility.signals.lineHasFoldedArea` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 - 声音(音频提示)和/或公告(警报) - 活动行包含可展开的折叠区域时。 |
| P2 | `accessibility.signals.lineHasInlineSuggestion` | `{"sound": "auto"}` | 活动行包含内联建议时播放声音/音频提示。 |
| P2 | `accessibility.signals.lineHasWarning` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 活动行出现警告时。 |
| P2 | `accessibility.signals.nextEditSuggestion` | `{"sound": "auto", "announcement": "auto"}` | 当有下一个编辑建议时，播放信号音/音频提示和/或公告(警报)。 |
| P2 | `accessibility.signals.noInlayHints` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 尝试读取带有包含无内嵌提示的内嵌提示的行时。 |
| P2 | `accessibility.signals.notebookCellCompleted` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 成功完成笔记本单元格执行时。 |
| P2 | `accessibility.signals.notebookCellFailed` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 笔记本单元格执行失败时。 |
| P2 | `accessibility.signals.onDebugBreak` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 调试程序在断点处停止时。 |
| P2 | `accessibility.signals.positionHasError` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 活动行出现警告时。 |
| P2 | `accessibility.signals.positionHasWarning` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 活动行出现警告时。 |
| P2 | `accessibility.signals.progress` | `{"sound": "auto", "announcement": "off"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 进度过程中发生循环时。 |
| P2 | `accessibility.signals.save` | `{"sound": "never", "announcement": "never"}` | 播放信号 - 声音(音频提示)和/或公告(警报) - 保存文件时。 |
| P2 | `accessibility.signals.taskCompleted` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 - 声音(音频提示)和/或公告(警报) - 完成任务时。 |
| P2 | `accessibility.signals.taskFailed` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 任务失败(非零退出代码)时。 |
| P2 | `accessibility.signals.terminalBell` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 终端铃声响铃时。 |
| P2 | `accessibility.signals.terminalCommandFailed` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 - 终端命令失败(非零退出代码)或在可访问视图中导航到具有此类退出代码的命令时。 |
| P2 | `accessibility.signals.terminalCommandSucceeded` | `{"sound": "auto", "announcement": "auto"}` | 当终端命令成功(零退出代码)或在可访问视图中导航到具有此类退出代码的命令时，播放信号 - 声音(音频提示)和/或公告(警报)。 |
| P2 | `accessibility.signals.terminalQuickFix` | `{"sound": "auto", "announcement": "auto"}` | 播放信号 -声音(音频提示)和/或公告(警报) - 终端快速修复可用时。 |
| P2 | `accessibility.signals.voiceModeStarted` | `{"sound": "on", "announcement": "auto"}` | Plays a signal - sound (audio cue) and/or announcement (alert) - when… |
| P2 | `accessibility.signals.voiceModeStopped` | `{"sound": "on", "announcement": "auto"}` | Plays a signal - sound (audio cue) and/or announcement (alert) - when… |
| P2 | `accessibility.signals.voiceRecordingStarted` | `{"sound": "on"}` | 语音录制开始时播放声音/音频提示。 |
| P2 | `accessibility.signals.voiceRecordingStopped` | `{"sound": "on"}` | 语音录制停止时播放声音/音频提示。 |
| P2 | `accessibility.underlineLinks` | `false` | 控制是否应在工作台中向链接添加下划线。 |
| P2 | `accessibility.verboseChatProgressUpdates` | `true` | 控制是否应在聊天请求进行过程中发出详细的进度公告，包括 <search term> 的搜索文本(具有 X 个结果)、创建的文件 <file… |
| P2 | `accessibility.verbosity.agentsWindowExtensions` | `true` | Provide information about how to access accessibility help when manag… |
| P2 | `accessibility.verbosity.automations` | `true` | Provide information about how to use Automations management views, in… |
| P2 | `accessibility.verbosity.browserElementCommenting` | `true` | Provide information about how to access element commenting accessibil… |
| P2 | `accessibility.verbosity.chatPetAchievements` | `true` | Provide information about how to access chat pet achievements accessi… |
| P2 | `accessibility.verbosity.chatPetJumpGame` | `true` | Provide information about how to access accessibility help when Blobb… |
| P2 | `accessibility.verbosity.chatQuestionCarousel` | `true` | 提供有关如何导航和与聊天问题轮播交互的信息，包括在适用时如何聚焦终端。 |
| P2 | `accessibility.verbosity.comments` | `true` | 提供有关可在注释小组件或包含注释的文件中执行的操作的信息。 |
| P2 | `accessibility.verbosity.connectionDiagnostics` | `true` | Provide information about how to access connection diagnostics access… |
| P2 | `accessibility.verbosity.customizationDiscovery` | `true` | Provide information about how to access accessibility help when disco… |
| P2 | `accessibility.verbosity.customizationMigrations` | `true` | Provide information about how to access accessibility help for the cu… |
| P2 | `accessibility.verbosity.debug` | `true` | 提供有关在聚焦调试控制台或运行和调试 viewlet 时如何访问调试控制台辅助功能帮助对话框的信息。请注意，需要重新加载窗口才能使其生效。 |
| P2 | `accessibility.verbosity.diffEditor` | `true` | 提供有关如何在聚焦差异编辑器时在其中导航更改的信息。 |
| P2 | `accessibility.verbosity.diffEditorActive` | `true` | 指示差异编辑器何时成为活动编辑器。 |
| P2 | `accessibility.verbosity.emptyEditorHint` | `true` | 在空文本编辑器中提供有关相关操作的信息。 |
| P2 | `accessibility.verbosity.emptyFileEditorSearch` | `true` | Provide information about how to use file search in the empty Files e… |
| P2 | `accessibility.verbosity.find` | `true` | 提供有关在以查找输入为焦点时如何访问查找辅助功能帮助菜单的信息。 |
| P2 | `accessibility.verbosity.hover` | `true` | 提供有关如何在辅助视图中打开悬停的信息。 |
| P2 | `accessibility.verbosity.inboxNotifications` | `true` | Provide information about how to navigate and act on prioritized noti… |
| P2 | `accessibility.verbosity.inlineChat` | `true` | 提供有关如何访问内联编辑器聊天辅助功能帮助菜单的信息，并提供如何在聚焦输入时使用该功能的提示。 |
| P2 | `accessibility.verbosity.inlineCompletions` | `true` | 提供有关如何访问内联完成悬停和辅助视图的信息。 |
| P2 | `accessibility.verbosity.keybindingsEditor` | `true` | 提供有关在行聚焦时如何在键盘绑定编辑器中更改键盘绑定以及如何导航到结果表的信息。 |
| P2 | `accessibility.verbosity.mcpServerConfiguration` | `true` | Provide information about how to access accessibility help when an MC… |
| P2 | `accessibility.verbosity.multiDiffEditorFileList` | `true` | Provide information about how to navigate the changed files list in t… |
| P2 | `accessibility.verbosity.newSessionWelcome` | `true` | Announce the welcome message when a new session composer is shown. |
| P2 | `accessibility.verbosity.notebook` | `true` | 提供有关在笔记本单元格聚焦时如何聚焦单元格容器或内部编辑器的信息。 |
| P2 | `accessibility.verbosity.notification` | `true` | 提供有关如何在辅助视图中打开通知的信息。 |
| P2 | `accessibility.verbosity.panelChat` | `true` | 提供有关如何在聚焦聊天输入时访问聊天帮助菜单的信息。 |
| P2 | `accessibility.verbosity.releaseNotes` | `true` | Provide information about how to access release notes accessibility h… |
| P2 | `accessibility.verbosity.remotes` | `true` | Provide information about how to access accessibility help when manag… |
| P2 | `accessibility.verbosity.replEditor` | `true` | 提供有关在 REPL 编辑器处于焦点状态下如何访问 REPL 编辑器辅助功能帮助菜单的信息。 |
| P2 | `accessibility.verbosity.sessionCanvas` | `true` | Provide information about how to access canvas accessibility help whe… |
| P2 | `accessibility.verbosity.sessionWorktreeCleanup` | `true` | Provide information about how to use the Manage Agent Session Storage… |
| P2 | `accessibility.verbosity.sessionsChanges` | `true` | 提供信息说明在焦点位于“更改”视图时如何访问“更改”视图辅助功能帮助菜单。 |
| P2 | `accessibility.verbosity.sessionsChat` | `true` | 提供有关在聊天输入框聚焦时如何访问智能体窗口辅助功能帮助菜单的信息。 |
| P2 | `accessibility.verbosity.sessionsGetStarted` | `true` | Provide information about how to access accessibility help for the Ge… |
| P2 | `accessibility.verbosity.sessionsListNotification` | `true` | Provide information about how to access accessibility help for the Un… |
| P2 | `accessibility.verbosity.sourceControl` | `true` | 提供有关在以输入为焦点时如何访问源代码管理辅助功能帮助菜单的信息。 |
| P2 | `accessibility.verbosity.survey` | `true` | 提供有关如何导航问卷编辑器窗格并与其交互的信息。 |
| P2 | `accessibility.verbosity.terminal` | `true` | 提供有关如何在聚焦终端时访问终端辅助功能帮助菜单的信息。 |
| P2 | `accessibility.verbosity.terminalChatOutput` | `true` | 介绍如何在辅助视图中打开聊天终端输出。 |
| P2 | `accessibility.verbosity.walkthrough` | `true` | 提供有关如何在辅助视图中打开演练的信息。 |
| P2 | `accessibility.windowTitleOptimized` | `true` | 控制在屏幕阅读器模式下是否应针对屏幕阅读器优化 `window.title`。启用后，窗口标题会将 `activeEditorState`… |
| P2 | `screencastMode.fontSize` | `56` | 屏幕录制模式 |
| P2 | `screencastMode.keyboardOptions` | `{"showKeys": true, "showKeybindings": true, "showCommands":…` | 用于在屏幕录制模式下自定义键盘覆盖的选项。 |
| P2 | `screencastMode.keyboardOverlayTimeout` | `800` | 控制屏幕录制模式下键盘覆盖显示的时长(以毫秒为单位)。 |
| P2 | `screencastMode.mouseIndicatorColor` | `"#FF0000"` | 控制屏幕录制模式下鼠标指示器的十六进制(#RGB、#RGBA、#RRGGBB 或 #RRGGBBAA)的颜色。 |
| P2 | `screencastMode.mouseIndicatorSize` | `20` | 控制屏幕录制模式下鼠标光标的大小(以像素为单位)。 |
| P2 | `screencastMode.verticalOffset` | `20` | 控制屏幕录制模式叠加的垂直偏移,从底部作为工作台高度的百分比。 |

## 6. 不支持的设置项（按原因）

<details><summary>JSON：5 项（json 5）</summary>

`json.colorDecorators.enable`, `json.schemaDownload.enable`, `json.schemaDownload.trustedDomains`, `json.schemas`, `json.validate.enable`

</details>

<details><summary>Markdown：29 项（markdown 29）</summary>

`markdown.copyFiles.destination`, `markdown.copyFiles.overwriteBehavior`, `markdown.editor.drop.copyIntoWorkspace`, `markdown.editor.drop.enabled`, `markdown.editor.filePaste.audioSnippet`, `markdown.editor.filePaste.copyIntoWorkspace`, `markdown.editor.filePaste.enabled`, `markdown.editor.filePaste.videoSnippet`, `markdown.editor.highlightActiveBlock`, `markdown.editor.pasteUrlAsFormattedLink.enabled`, `markdown.editor.quickSuggestions`, `markdown.editor.renderWhitespace`, `markdown.editor.updateLinksOnPaste.enabled`, `markdown.experimental.richLinks.enabled`, `markdown.occurrencesHighlight.enabled`, `markdown.server.log`, `markdown.suggest.paths.enabled`, `markdown.suggest.paths.includeWorkspaceHeaderCompletions`, `markdown.updateLinksOnFileMove.enableForDirectories`, `markdown.updateLinksOnFileMove.enabled`, `markdown.updateLinksOnFileMove.include`, `markdown.validate.duplicateLinkDefinitions.enabled`, `markdown.validate.enabled`, `markdown.validate.fileLinks.enabled`, `markdown.validate.fileLinks.markdownFragmentLinks`, `markdown.validate.fragmentLinks.enabled`, `markdown.validate.ignoredLinks`, `markdown.validate.referenceLinks.enabled`, `markdown.validate.unusedLinkDefinitions.enabled`

</details>

<details><summary>不做的功能域：887 项（chat 281, terminal 136, notebook 56, debug 47, css 34, less 32, scss 32, scm 31, html 24, remote 22, extensions 19, testing 16）</summary>

`agents.voice.agentProgress`, `agents.voice.enabled`, `agents.voice.handsFree`, `agents.voice.language`, `agents.voice.liveTranscript`, `agents.voice.sessionAwareness`, `agents.voice.showButton`, `agents.voice.showTranscript`, `agents.voice.speakResponses`, `agents.voice.turn.silenceMs`, `agents.voice.turn.stopPhrases`, `agents.voice.useBYOKVoiceModel`, `agents.voice.voice`, `application.experimental.rendererProfiling`, `chat.agent.codeBlockProgress`, `chat.agent.collapseCompletedResponses`, `chat.agent.enabled`, `chat.agent.maxRequests`, `chat.agent.networkFilter`, `chat.agent.sandbox.advanced.runtime`, `chat.agent.sandbox.allowUnsandboxedCommands`, `chat.agent.sandbox.credentials.authenticategh`, `chat.agent.sandbox.credentials.authenticategit`, `chat.agent.sandbox.enabled`, `chat.agent.sandbox.fileSystem.allowDevToolAccess`, `chat.agent.sandbox.fileSystem.linux`, `chat.agent.sandbox.fileSystem.mac`, `chat.agent.sandbox.fileSystem.userConfiguredPaths`, `chat.agent.sandbox.lspServers`, `chat.agent.sandbox.mcpServers`, `chat.agent.sandbox.network.allowLocalNetwork`, `chat.agent.sandbox.network.allowNetwork`, `chat.agent.sandbox.network.allowedDomains`, `chat.agent.sandbox.network.deniedDomains`, `chat.agent.sandbox.retryWithAllowNetworkRequests`, `chat.agent.terminal.allowList`, `chat.agent.terminal.autoApprove`, `chat.agent.terminal.denyList`, `chat.agent.thinking.collapsedTools`, `chat.agent.thinking.generateTitles`, `chat.agent.thinking.phrases`, `chat.agent.thinking.terminalTools`, `chat.agent.thinkingStyle`, `chat.agentFilesLocations`, `chat.agentHost.agentDebugLog.enabled`, `chat.agentHost.agentDebugLog.maxEventsInMemory`, `chat.agentHost.agentOrchestrationLimits`, `chat.agentHost.ahpJsonlLoggingEnabled`, `chat.agentHost.byokModels.enabled`, `chat.agentHost.claudeAgent.enabled`, `chat.agentHost.cloudSandbox.autoConnectOnOpen`, `chat.agentHost.cloudSandbox.earlyPreparation`, `chat.agentHost.cloudSandbox.enabled`, `chat.agentHost.cloudSandbox.sessionFeed.enabled`, `chat.agentHost.codexAgent.enabled`, `chat.agentHost.computerUse.enabled`, `chat.agentHost.copilot.executionSubagent.enabled`, `chat.agentHost.copilot.executionSubagent.model`, `chat.agentHost.copilot.includeCoAuthoredBy`, `chat.agentHost.copilot.modelCapabilityOverrides`, `chat.agentHost.copilot.toolSearch.deferThreshold`, `chat.agentHost.copilot.toolSearch.enabled`, `chat.agentHost.copilotSdk.logLevel`, `chat.agentHost.customTerminalTool.enabled`, `chat.agentHost.debugLogs.defaultExportLocation`, `chat.agentHost.defaultCloneDirectory`, `chat.agentHost.experimental.autoAttachPullRequests`, `chat.agentHost.experimental.markdownPlanRichLinks`, `chat.agentHost.experimental.mcpToolRouting`, `chat.agentHost.experimental.overlapProviderPreparation`, `chat.agentHost.experimental.prewarmSessions`, `chat.agentHost.experimental.titleGeneration`, `chat.agentHost.experimentalMissionControl.useLocalCredentials`, `chat.agentHost.githubEnvironments.enabled`, `chat.agentHost.githubMcpServer.enabled`, `chat.agentHost.moveChats.enabled`, `chat.agentHost.multipleChatProviders.enabled`, `chat.agentHost.opus48Prompt.enabled`, `chat.agentHost.otel.captureContent`, `chat.agentHost.otel.dbSpanExporter.enabled`, `chat.agentHost.otel.enabled`, `chat.agentHost.otel.exporterType`, `chat.agentHost.otel.otlpEndpoint`, `chat.agentHost.otel.outfile`, `chat.agentHost.remoteConnections`, `chat.agentHost.shellTool.initScript.enabled`, `chat.agentHost.systemProxy.enabled`, `chat.agentMerge.addressReviews`, `chat.agentMerge.enabled`, `chat.agentMerge.fixCI`, `chat.agentMerge.mergeMethod`, `chat.agentMerge.mergePullRequest`, `chat.agentMerge.replyAttribution`, `chat.agentMerge.resolveConflicts`, `chat.agentSessionProjection.enabled`, `chat.agentSessions.autoDeleteMarkedAsDoneMergedSessionsAfterDays`, `chat.agentSessions.autoMarkAsDoneMergedSessionsAfterDays`, `chat.agentSessions.migrateLegacyCopilotCli`, `chat.agentSessions.openInEditor`, `chat.agentSessions.showExternal`, `chat.agentSkillsLocations`, `chat.agentsControl.enabled`, `chat.agentsHandoffTip.delaySeconds`, `chat.agentsHandoffTip.mode`, `chat.agentsParallelWorkBanner.enabled`, `chat.agentsWindowBanner.developerMode`, `chat.agentsWindowBanner.enabled`, `chat.agentsWindowBanner.revealCurrentSession`, `chat.artifactIntegrations.enabled`, `chat.artifactIntegrations.githubPullRequests.autoMarkReadyIgnoredChecks`, `chat.artifactTools.enabled`, `chat.artifacts.enabled`, `chat.artifacts.rules.byFilePath`, `chat.artifacts.rules.byMemoryFilePath`, `chat.artifacts.rules.byMimeType`, `chat.autoReply`, `chat.autopilot.advanced.enabled`, `chat.btwTip.enabled`, `chat.byokUtilityModelDefault`, `chat.canvases.enabled`, `chat.checkpoints.enabled`, `chat.checkpoints.showFileChanges`, `chat.closedPromoNotification`, `chat.contextUsage.enabled`, `chat.copilot.claudeAdvisor.enabled`, `chat.copilot.dynamicAutoTiers.enabled`, `chat.copilot.exploreAgent.enabled`, `chat.copilot.hydraFusion.enabled`, `chat.copilot.imageGeneration.enabled`, `chat.copilot.imageGeneration.model`, `chat.copilot.imageGeneration.watermark`, `chat.copilot.localModel.enabled`, `chat.copilot.localModel.llamaServerPath`, `chat.copilot.localModel.useInAuto`, `chat.copilot.memory.enabled`, `chat.copilot.memory.local.enabled`, `chat.copilot.searchSubagent.enabled`, `chat.copilot.searchSubagent.model`, `chat.copilot.skillCharBudget`, `chat.copilot.stabilityOrderedPrompt.enabled`, `chat.copilotCliCommand.enabled`, `chat.copilotHarnessIntroduction.mode`, `chat.customizations.copilotConnectors.enabled`, `chat.customizations.locationsMigration.enabled`, `chat.customizations.marketplace.enabled`, `chat.customizations.marketplace.sources.publicFeed.enabled`, `chat.customizations.mcpServerMigration.enabled`, `chat.customizations.migration.enabled`, `chat.customizations.promptMigration.enabled`, `chat.customizations.structuredPreview.enabled`, `chat.customizations.userDataMigration.enabled`, `chat.defaultConfiguration`, `chat.defaultModel`, `chat.defaultToCopilotHarness`, `chat.detectParticipant.enabled`, `chat.disableAIFeatures`, `chat.editRequests`, `chat.editing.autoAcceptDelay`, `chat.editing.confirmEditRequestRemoval`, `chat.editing.confirmEditRequestRetry`, `chat.editing.explainChanges.enabled`, `chat.editing.openChangedFileInDiffEditor`, `chat.editing.revealNextChangeOnResolve`, `chat.editor.codex.preferAgentHost`, `chat.editor.fontFamily`, `chat.editor.fontSize`, `chat.editor.fontWeight`, `chat.editor.lineHeight`, `chat.editor.localAgent.enabled`, `chat.editor.preferCopilotHarness`, `chat.editor.wordWrap`, `chat.editor.workingSet.enabled`, `chat.editorAssociations`, `chat.exitAfterDelegation`, `chat.experimental.collectInstructionsInExtension`, `chat.experimental.detectParticipant.enabled`, `chat.experimental.incrementalRendering.animationStyle`, `chat.experimental.incrementalRendering.buffering`, `chat.experimental.incrementalRendering.enabled`, `chat.experimental.openInAgentsWindow.transferDraft`, `chat.experimental.richLinks.enabled`, `chat.experimental.sessionArchiveActionWording`, `chat.experimental.sessionStateIndicator.enabled`, `chat.experimental.stickyScroll.enabled`, `chat.experimental.useSkillAdherencePrompt`, `chat.experimental.workspaceSnapshot`, `chat.experimentalModePermissionsPicker`, `chat.experimentalSessionsWindowOverride`, `chat.exploreAgent.defaultModel`, `chat.extensionTools.enabled`, `chat.extensionUnification.enabled`, `chat.fontFamily`, `chat.fontSize`, `chat.growthNotification.enabled`, `chat.harnessSwitchFeedbackSurvey.enabled`, `chat.hookFilesLocations`, `chat.implicitContext.enabled`, `chat.implicitContext.includeActiveEditor`, `chat.implicitContext.suggestedContext`, `chat.includeApplyingInstructions`, `chat.includeReferencedInstructions`, `chat.inlineAttachments.enabled`, `chat.inlineChat.agentHost.enabled`, `chat.inlineReferences.style`, `chat.instructionsFilesLocations`, `chat.math.enabled`, `chat.mcp.access`, `chat.mcp.apps.enabled`, `chat.mcp.assisted.nuget.enabled`, `chat.mcp.autostart`, `chat.mcp.collisionBehavior`, `chat.mcp.discovery.enabled`, `chat.mcp.gallery.enabled`, `chat.mcp.serverSampling`, `chat.mcp.workspaceRootConfig.enabled`, `chat.microsoftAuthentication.enabled`, `chat.modeFilesLocations`, `chat.newSession.defaultMode`, `chat.notifyWindowOnConfirmation`, `chat.notifyWindowOnResponseReceived`, `chat.pasteAsAttachmentThreshold`, `chat.permissions.default`, `chat.persistentProgress`, `chat.persistentProgressVerbosity`, `chat.planAgent.defaultModel`, `chat.pluginLocations`, `chat.plugins.enabled`, `chat.plugins.enabledPlugins`, `chat.plugins.marketplaces`, `chat.progressBorder.enabled`, `chat.promptFilesLocations`, `chat.promptFilesRecommendations`, `chat.remoteAgentHostsEnabled`, `chat.requestQueuing.defaultAction`, `chat.restoreLastPanelSession`, `chat.saveBeforeSend`, `chat.sessionSync.enabled`, `chat.sessionSync.excludeRepositories`, `chat.stickyScroll.enabled`, `chat.subagents.allowInvocationsFromSubagents`, `chat.subagents.showCreditUsage`, `chat.terminal.agentHost.enabled`, `chat.tips.enabled`, `chat.titleBar.openInAgentsWindow.enabled`, `chat.titleBar.openInAgentsWindow.richHover`, `chat.titleBar.signIn.enabled`, `chat.tools.autoExpandFailures`, `chat.tools.compressOutput.enabled`, `chat.tools.confirmationCarousel.enabled`, `chat.tools.edits.autoApprove`, `chat.tools.eligibleForAutoApproval`, `chat.tools.global.autoApprove`, `chat.tools.riskAssessment.enabled`, `chat.tools.riskAssessment.model`, `chat.tools.terminal.autoApprove`, `chat.tools.terminal.autoApproveWorkspaceNpmScripts`, `chat.tools.terminal.backgroundNotifications`, `chat.tools.terminal.blockDetectedFileWrites`, `chat.tools.terminal.enableAutoApprove`, `chat.tools.terminal.enforceTimeoutFromModel`, `chat.tools.terminal.idlePollInterval`, `chat.tools.terminal.idleSilenceTimeoutMs`, `chat.tools.terminal.ignoreDefaultAutoApproveRules`, `chat.tools.terminal.outputCompaction`, `chat.tools.terminal.outputDeltas`, `chat.tools.terminal.outputLocation`, `chat.tools.terminal.outputReflow`, `chat.tools.terminal.preventShellHistory`, `chat.tools.terminal.shellIntegrationTimeout`, `chat.tools.terminal.simpleCollapsible`, `chat.tools.terminal.terminalProfile.linux`, `chat.tools.terminal.terminalProfile.osx`, `chat.tools.terminal.terminalProfile.windows`, `chat.tools.todos.showWidget`, `chat.tools.urls.autoApprove`, `chat.undoRequests.restoreInput`, `chat.unifiedAgentsBar.enabled`, `chat.upvoteAnimation`, `chat.useAgentSkills`, `chat.useAgentsMdFile`, `chat.useClaudeHooks`, `chat.useClaudeMdFile`, `chat.useCustomizationsInParentRepositories`, `chat.useHooks`, `chat.useNestedAgentsMdFiles`, `chat.utilityModel`, `chat.utilitySmallModel`, `chat.verbose`, `chat.viewProgressBadge.enabled`, `chat.viewSessions.enabled`, `chat.viewSessions.orientation`, `chat.welcomePage.agentsWindowGraphic`, `chat.welcomePage.agentsWindowSectionStyle`, `chat.welcomePage.signIn.enabled`, `chat.workspaceRemoteAgentHosts`, `comments.collapseOnResolve`, `comments.maxHeight`, `comments.openPanel`, `comments.openView`, `comments.thread.confirmOnCollapse`, `comments.useRelativeTime`, `comments.visible`, `css.completion.completePropertyWithSemicolon`, `css.completion.triggerPropertyValueCompletion`, `css.customData`, `css.format.braceStyle`, `css.format.enable`, `css.format.maxPreserveNewLines`, `css.format.newlineBetweenRules`, `css.format.newlineBetweenSelectors`, `css.format.preserveNewLines`, `css.format.spaceAroundSelectorSeparator`, `css.hover.documentation`, `css.hover.references`, `css.lint.argumentsInColorFunction`, `css.lint.boxModel`, `css.lint.compatibleVendorPrefixes`, `css.lint.duplicateProperties`, `css.lint.emptyRules`, `css.lint.float`, `css.lint.fontFaceProperties`, `css.lint.hexColorLength`, `css.lint.idSelector`, `css.lint.ieHack`, `css.lint.importStatement`, `css.lint.important`, `css.lint.propertyIgnoredDueToDisplay`, `css.lint.universalSelector`, `css.lint.unknownAtRules`, `css.lint.unknownProperties`, `css.lint.unknownVendorSpecificProperties`, `css.lint.validProperties`, `css.lint.vendorPrefix`, `css.lint.zeroUnits`, `css.trace.server`, `css.validate`, `debug.allowBreakpointsEverywhere`, `debug.autoExpandLazyVariables`, `debug.breakpointsView.presentation`, `debug.closeReadonlyTabsOnEnd`, `debug.confirmOnExit`, `debug.console.acceptSuggestionOnEnter`, `debug.console.closeOnEnd`, `debug.console.collapseIdenticalLines`, `debug.console.fontFamily`, `debug.console.fontSize`, `debug.console.historySuggestions`, `debug.console.lineHeight`, `debug.console.maximumLines`, `debug.console.wordWrap`, `debug.disassemblyView.showSourceCode`, `debug.enableStatusBarColor`, `debug.focusEditorOnBreak`, `debug.focusWindowOnBreak`, `debug.gutterMiddleClickAction`, `debug.hideLauncherWhileDebugging`, `debug.hideSlowPreLaunchWarning`, `debug.inlineValues`, `debug.internalConsoleOptions`, `debug.javascript.autoAttachFilter`, `debug.javascript.autoAttachSmartPattern`, `debug.javascript.automaticallyTunnelRemoteServer`, `debug.javascript.breakOnConditionalError`, `debug.javascript.codelens.npmScripts`, `debug.javascript.debugByLinkOptions`, `debug.javascript.defaultRuntimeExecutable`, `debug.javascript.enableNetworkView`, `debug.javascript.pickAndAttachOptions`, `debug.javascript.resolveDebugEntrypoint`, `debug.javascript.resourceRequestOptions`, `debug.javascript.terminalOptions`, `debug.javascript.unmapMissingSources`, `debug.onTaskErrors`, `debug.openDebug`, `debug.openExplorerOnEnd`, `debug.saveBeforeStart`, `debug.showBreakpointsInOverviewRuler`, `debug.showInStatusBar`, `debug.showInlineBreakpointCandidates`, `debug.showSubSessionsInToolBar`, `debug.showVariableTypes`, `debug.terminal.clearBeforeReusing`, `debug.toolBarLocation`, `dictation.enabled`, `dictation.experimental.llmCleanup`, `dictation.experimental.llmCleanupModel`, `dictation.model`, `dictation.showButton`, `dictation.showTranscript`, `emmet.excludeLanguages`, `emmet.extensionsPath`, `emmet.includeLanguages`, `emmet.optimizeStylesheetParsing`, `emmet.preferences`, `emmet.showAbbreviationSuggestions`, `emmet.showExpandedAbbreviation`, `emmet.showSuggestionsAsSnippets`, `emmet.syntaxProfiles`, `emmet.triggerExpansionOnTab`, `emmet.useInlineCompletions`, `emmet.variables`, `extensions.allowOpenInModalEditor`, `extensions.allowed`, `extensions.autoCheckUpdates`, `extensions.autoUpdate`, `extensions.autoUpdateDelay`, `extensions.closeExtensionDetailsOnViewChange`, `extensions.confirmedUriHandlerExtensionIds`, `extensions.experimental.affinity`, `extensions.experimental.deferredStartupFinishedActivation`, `extensions.experimental.issueQuickAccess`, `extensions.ignoreRecommendations`, `extensions.requestTimeout`, `extensions.showRecommendationsOnlyOnDemand`, `extensions.supportAgentsWindow`, `extensions.supportNodeGlobalNavigator`, `extensions.supportUntrustedWorkspaces`, `extensions.supportVirtualWorkspaces`, `extensions.verifySignature`, `extensions.webWorker`, `github-authentication.preferDeviceCodeFlow`, `github-authentication.useElectronFetch`, `github-enterprise.uri`, `github-enterprise.uris`, `github.branchProtection`, `github.copilot.chat.agent.terminal.allowList`, `github.copilot.chat.agent.terminal.denyList`, `github.copilot.chat.autoModeTierOverride`, `github.gitAuthentication`, `github.gitProtocol`, `github.showAvatar`, `grunt.autoDetect`, `gulp.autoDetect`, `html.autoClosingTags`, `html.autoCreateQuotes`, `html.completion.attributeDefaultValue`, `html.customData`, `html.format.contentUnformatted`, `html.format.enable`, `html.format.extraLiners`, `html.format.indentHandlebars`, `html.format.indentInnerHtml`, `html.format.maxPreserveNewLines`, `html.format.preserveNewLines`, `html.format.templating`, `html.format.unformatted`, `html.format.unformattedContentDelimiter`, `html.format.wrapAttributes`, `html.format.wrapAttributesIndentSize`, `html.format.wrapLineLength`, `html.hover.documentation`, `html.hover.references`, `html.suggest.hideEndTagSuggestions`, `html.suggest.html5`, `html.trace.server`, `html.validate.scripts`, `html.validate.styles`, `http.electronFetch`, `http.experimental.networkInterfaceCheckInterval`, `http.experimental.systemCertificatesV2`, `http.fetchAdditionalSupport`, `http.noProxy`, `http.proxy`, `http.proxyAuthorization`, `http.proxyKerberosServicePrincipal`, `http.proxyStrictSSL`, `http.proxySupport`, `http.systemCertificates`, `http.systemCertificatesNode`, `http.useLocalProxyConfiguration`, `http.webSocketAdditionalSupport`, `imageCarousel.chat.enabled`, `imageCarousel.explorerContextMenu.enabled`, `inlineChat.affordance`, `inlineChat.askInChat`, `inlineChat.defaultModel`, `inlineChat.fixDiagnostics`, `inlineChat.notebookAgent`, `interactiveWindow.alwaysScrollOnNewCell`, `interactiveWindow.collapseCellInputCode`, `interactiveWindow.executeWithShiftEnter`, `interactiveWindow.promptToSaveOnClose`, `interactiveWindow.showExecutionHint`, `ipynb.experimental.serialization`, `ipynb.pasteImagesAsAttachments.enabled`, `issueReporter.additionalRepositories`, `issueReporter.wizard.enabled`, `issueReporter.wizard.fullWorkspaceScan`, `issueReporter.wizard.openLocation`, `jake.autoDetect`, `launch`, `less.completion.completePropertyWithSemicolon`, `less.completion.triggerPropertyValueCompletion`, `less.format.braceStyle`, `less.format.enable`, `less.format.maxPreserveNewLines`, `less.format.newlineBetweenRules`, `less.format.newlineBetweenSelectors`, `less.format.preserveNewLines`, `less.format.spaceAroundSelectorSeparator`, `less.hover.documentation`, `less.hover.references`, `less.lint.argumentsInColorFunction`, `less.lint.boxModel`, `less.lint.compatibleVendorPrefixes`, `less.lint.duplicateProperties`, `less.lint.emptyRules`, `less.lint.float`, `less.lint.fontFaceProperties`, `less.lint.hexColorLength`, `less.lint.idSelector`, `less.lint.ieHack`, `less.lint.importStatement`, `less.lint.important`, `less.lint.propertyIgnoredDueToDisplay`, `less.lint.universalSelector`, `less.lint.unknownAtRules`, `less.lint.unknownProperties`, `less.lint.unknownVendorSpecificProperties`, `less.lint.validProperties`, `less.lint.vendorPrefix`, `less.lint.zeroUnits`, `less.validate`, `merge-conflict.autoNavigateNextConflict.enabled`, `merge-conflict.codeLens.enabled`, `merge-conflict.decorators.enabled`, `merge-conflict.diffViewPosition`, `mergeEditor.diffAlgorithm`, `mergeEditor.showDeletionMarkers`, `microsoft-authentication.implementation`, `network.meteredConnection`, `notebook.backup.sizeLimit`, `notebook.breadcrumbs.showCodeCells`, `notebook.cellExecutionTimeVerbosity`, `notebook.cellFailureDiagnostics`, `notebook.cellFocusIndicator`, `notebook.cellToolbarLocation`, `notebook.cellToolbarVisibility`, `notebook.codeActionsOnSave`, `notebook.compactView`, `notebook.confirmDeleteRunningCell`, `notebook.consolidatedOutputButton`, `notebook.consolidatedRunButton`, `notebook.defaultFormatter`, `notebook.diff.enablePreview`, `notebook.diff.experimental.toggleInline`, `notebook.diff.ignoreMetadata`, `notebook.diff.ignoreOutputs`, `notebook.diff.overviewRuler`, `notebook.displayOrder`, `notebook.dragAndDropEnabled`, `notebook.editorOptionsCustomizations`, `notebook.experimental.generate`, `notebook.experimental.remoteSave`, `notebook.find.filters`, `notebook.formatOnCellExecution`, `notebook.formatOnSave.enabled`, `notebook.globalToolbar`, `notebook.globalToolbarShowLabel`, `notebook.gotoSymbols.showAllSymbols`, `notebook.inlineValues`, `notebook.insertFinalNewline`, `notebook.insertToolbarLocation`, `notebook.lineNumbers`, `notebook.markdown.lineHeight`, `notebook.markup.fontFamily`, `notebook.markup.fontSize`, `notebook.multiCursor.enabled`, `notebook.navigation.allowNavigateToSurroundingCells`, `notebook.outline.showCodeCellSymbols`, `notebook.outline.showCodeCells`, `notebook.outline.showMarkdownHeadersOnly`, `notebook.output.fontFamily`, `notebook.output.fontSize`, `notebook.output.lineHeight`, `notebook.output.linkifyFilePaths`, `notebook.output.minimalErrorRendering`, `notebook.output.scrolling`, `notebook.output.textLineLimit`, `notebook.output.wordWrap`, `notebook.scrolling.revealNextCellOnExecute`, `notebook.showCellStatusBar`, `notebook.showFoldingControls`, `notebook.stickyScroll.enabled`, `notebook.stickyScroll.mode`, `notebook.undoRedoPerCell`, `notebook.variablesView`, `npm.autoDetect`, `npm.enableRunFromFolder`, `npm.exclude`, `npm.fetchOnlinePackageInfo`, `npm.packageManager`, `npm.runSilent`, `npm.scriptExplorerAction`, `npm.scriptExplorerExclude`, `npm.scriptHover`, `npm.scriptRunner`, `onboarding.developerMode`, `onboarding.developerModeVariations`, `onboarding.enabled`, `output.smartScroll.enabled`, `php.suggest.basic`, `php.validate.enable`, `php.validate.executablePath`, `php.validate.run`, `remote.WSL.debug`, `remote.WSL.downloadInBackground`, `remote.WSL.experimental.scriptLessStartup`, `remote.WSL.fileWatcher.polling`, `remote.WSL.fileWatcher.pollingInterval`, `remote.WSL.serverProbeRetryTimeout`, `remote.WSL.useShellEnvironment`, `remote.WSL1.connectionMethod`, `remote.WSL2.connectionMethod`, `remote.autoForwardPorts`, `remote.autoForwardPortsFallback`, `remote.autoForwardPortsSource`, `remote.defaultExtensionsIfInstalledLocally`, `remote.downloadExtensionsLocally`, `remote.extensionKind`, `remote.forwardOnOpen`, `remote.localPortHost`, `remote.otherPortsAttributes`, `remote.portsAttributes`, `remote.restoreForwardedPorts`, `remote.tunnels.access.hostNameOverride`, `remote.tunnels.access.preventSleep`, `scm.alwaysShowActions`, `scm.alwaysShowRepositories`, `scm.autoReveal`, `scm.compactFolders`, `scm.countBadge`, `scm.defaultViewMode`, `scm.defaultViewSortKey`, `scm.diffDecorations`, `scm.diffDecorationsGutterAction`, `scm.diffDecorationsGutterPattern`, `scm.diffDecorationsGutterVisibility`, `scm.diffDecorationsGutterWidth`, `scm.diffDecorationsIgnoreTrimWhitespace`, `scm.graph.badges`, `scm.graph.pageOnScroll`, `scm.graph.pageSize`, `scm.graph.showIncomingChanges`, `scm.graph.showOutgoingChanges`, `scm.inputFontFamily`, `scm.inputFontSize`, `scm.inputMaxLineCount`, `scm.inputMinLineCount`, `scm.providerCountBadge`, `scm.repositories.explorer`, `scm.repositories.selectionMode`, `scm.repositories.sortOrder`, `scm.repositories.visible`, `scm.showActionButton`, `scm.showInputActionButton`, `scm.workingSets.default`, `scm.workingSets.enabled`, `scss.completion.completePropertyWithSemicolon`, `scss.completion.triggerPropertyValueCompletion`, `scss.format.braceStyle`, `scss.format.enable`, `scss.format.maxPreserveNewLines`, `scss.format.newlineBetweenRules`, `scss.format.newlineBetweenSelectors`, `scss.format.preserveNewLines`, `scss.format.spaceAroundSelectorSeparator`, `scss.hover.documentation`, `scss.hover.references`, `scss.lint.argumentsInColorFunction`, `scss.lint.boxModel`, `scss.lint.compatibleVendorPrefixes`, `scss.lint.duplicateProperties`, `scss.lint.emptyRules`, `scss.lint.float`, `scss.lint.fontFaceProperties`, `scss.lint.hexColorLength`, `scss.lint.idSelector`, `scss.lint.ieHack`, `scss.lint.importStatement`, `scss.lint.important`, `scss.lint.propertyIgnoredDueToDisplay`, `scss.lint.universalSelector`, `scss.lint.unknownAtRules`, `scss.lint.unknownProperties`, `scss.lint.unknownVendorSpecificProperties`, `scss.lint.validProperties`, `scss.lint.vendorPrefix`, `scss.lint.zeroUnits`, `scss.validate`, `security.allowedUNCHosts`, `security.promptForLocalFileProtocolHandling`, `security.promptForRemoteFileProtocolHandling`, `security.restrictUNCAccess`, `security.workspace.trust.banner`, `security.workspace.trust.emptyWindow`, `security.workspace.trust.enabled`, `security.workspace.trust.startupPrompt`, `security.workspace.trust.untrustedFiles`, `sessions.chat.experimental.agentsPickerInAttachContextMenu`, `sessions.chatTimeline.display`, `settingsSync.ignoredExtensions`, `settingsSync.ignoredSettings`, `settingsSync.keybindingsPerPlatform`, `simpleBrowser.focusLockIndicator.enabled`, `task.allowAutomaticTasks`, `task.autoDetect`, `task.notifyWindowOnTaskCompletion`, `task.problemMatchers.neverPrompt`, `task.quickOpen.detail`, `task.quickOpen.history`, `task.quickOpen.showAll`, `task.quickOpen.skip`, `task.reconnection`, `task.saveBeforeRun`, `task.slowProviderWarning`, `task.verboseLogging`, `telemetry.editStats.details.enabled`, `telemetry.editStats.enabled`, `telemetry.editStats.showDecorations`, `telemetry.editStats.showStatusBar`, `telemetry.enableCrashReporter`, `telemetry.enableTelemetry`, `telemetry.feedback.enabled`, `telemetry.performance.inputLatencySamplingProbability`, `telemetry.telemetryLevel`, `terminal.explorerKind`, `terminal.external.linuxExec`, `terminal.external.osxExec`, `terminal.external.windowsExec`, `terminal.integrated.accessibleViewFocusOnCommandExecution`, `terminal.integrated.accessibleViewPreserveCursorPosition`, `terminal.integrated.agentHostProfile.linux`, `terminal.integrated.agentHostProfile.osx`, `terminal.integrated.agentHostProfile.windows`, `terminal.integrated.allowChords`, `terminal.integrated.allowInUntrustedWorkspace`, `terminal.integrated.allowMnemonics`, `terminal.integrated.allowedLinkSchemes`, `terminal.integrated.altClickMovesCursor`, `terminal.integrated.autoReplies`, `terminal.integrated.automationProfile.linux`, `terminal.integrated.automationProfile.osx`, `terminal.integrated.automationProfile.windows`, `terminal.integrated.bellDuration`, `terminal.integrated.commandsToSkipShell`, `terminal.integrated.confirmOnExit`, `terminal.integrated.confirmOnKill`, `terminal.integrated.copyOnSelection`, `terminal.integrated.cursorBlinking`, `terminal.integrated.cursorStyle`, `terminal.integrated.cursorStyleInactive`, `terminal.integrated.cursorWidth`, `terminal.integrated.customGlyphs`, `terminal.integrated.cwd`, `terminal.integrated.defaultLocation`, `terminal.integrated.defaultProfile.linux`, `terminal.integrated.defaultProfile.osx`, `terminal.integrated.defaultProfile.windows`, `terminal.integrated.detectLocale`, `terminal.integrated.developer.devMode`, `terminal.integrated.developer.ptyHost.latency`, `terminal.integrated.developer.ptyHost.startupDelay`, `terminal.integrated.drawBoldTextInBrightColors`, `terminal.integrated.enableBell`, `terminal.integrated.enableFileLinks`, `terminal.integrated.enableImages`, `terminal.integrated.enableKittyKeyboardProtocol`, `terminal.integrated.enableMultiLinePasteWarning`, `terminal.integrated.enableNotifications`, `terminal.integrated.enablePersistentSessions`, `terminal.integrated.enableVisualBell`, `terminal.integrated.enableWin32InputMode`, `terminal.integrated.env.linux`, `terminal.integrated.env.osx`, `terminal.integrated.env.windows`, `terminal.integrated.environmentChangesRelaunch`, `terminal.integrated.fastScrollSensitivity`, `terminal.integrated.focusAfterRun`, `terminal.integrated.fontFamily`, `terminal.integrated.fontLigatures.enabled`, `terminal.integrated.fontLigatures.fallbackLigatures`, `terminal.integrated.fontLigatures.featureSettings`, `terminal.integrated.fontSize`, `terminal.integrated.fontWeight`, `terminal.integrated.fontWeightBold`, `terminal.integrated.gpuAcceleration`, `terminal.integrated.hideOnLastClosed`, `terminal.integrated.hideOnStartup`, `terminal.integrated.ignoreBracketedPasteMode`, `terminal.integrated.ignoreProcessNames`, `terminal.integrated.inheritEnv`, `terminal.integrated.initialHint`, `terminal.integrated.letterSpacing`, `terminal.integrated.lineHeight`, `terminal.integrated.localEchoEnabled`, `terminal.integrated.localEchoExcludePrograms`, `terminal.integrated.localEchoLatencyThreshold`, `terminal.integrated.localEchoStyle`, `terminal.integrated.macOptionClickForcesSelection`, `terminal.integrated.macOptionIsMeta`, `terminal.integrated.middleClickBehavior`, `terminal.integrated.minimumContrastRatio`, `terminal.integrated.mouseWheelScrollSensitivity`, `terminal.integrated.mouseWheelZoom`, `terminal.integrated.persistentSessionReviveProcess`, `terminal.integrated.persistentSessionScrollback`, `terminal.integrated.profiles.linux`, `terminal.integrated.profiles.osx`, `terminal.integrated.profiles.windows`, `terminal.integrated.rescaleOverlappingGlyphs`, `terminal.integrated.resizeDimensionsOverlay.enabled`, `terminal.integrated.rightClickBehavior`, `terminal.integrated.scrollback`, `terminal.integrated.sendKeybindingsToShell`, `terminal.integrated.shellIntegration.decorationsEnabled`, `terminal.integrated.shellIntegration.enabled`, `terminal.integrated.shellIntegration.environmentReporting`, `terminal.integrated.shellIntegration.history`, `terminal.integrated.shellIntegration.quickFixEnabled`, `terminal.integrated.shellIntegration.showCommandGuide`, `terminal.integrated.shellIntegration.timeout`, `terminal.integrated.showExitAlert`, `terminal.integrated.showLinkHover`, `terminal.integrated.smoothScrolling`, `terminal.integrated.splitCwd`, `terminal.integrated.stickyScroll.enabled`, `terminal.integrated.stickyScroll.ignoredCommands`, `terminal.integrated.stickyScroll.maxLineCount`, `terminal.integrated.suggest.cdPath`, `terminal.integrated.suggest.enabled`, `terminal.integrated.suggest.inlineSuggestion`, `terminal.integrated.suggest.insertTrailingSpace`, `terminal.integrated.suggest.providers`, `terminal.integrated.suggest.quickSuggestions`, `terminal.integrated.suggest.runOnEnter`, `terminal.integrated.suggest.selectionMode`, `terminal.integrated.suggest.showStatusBar`, `terminal.integrated.suggest.suggestOnTriggerCharacters`, `terminal.integrated.suggest.upArrowNavigatesHistory`, `terminal.integrated.suggest.windowsExecutableExtensions`, `terminal.integrated.tabStopWidth`, `terminal.integrated.tabs.allowAgentCliTitle`, `terminal.integrated.tabs.defaultColor`, `terminal.integrated.tabs.defaultIcon`, `terminal.integrated.tabs.description`, `terminal.integrated.tabs.enableAnimation`, `terminal.integrated.tabs.enabled`, `terminal.integrated.tabs.focusMode`, `terminal.integrated.tabs.hideCondition`, `terminal.integrated.tabs.location`, `terminal.integrated.tabs.separator`, `terminal.integrated.tabs.showActions`, `terminal.integrated.tabs.showActiveTerminal`, `terminal.integrated.tabs.title`, `terminal.integrated.textBlinking`, `terminal.integrated.unicodeVersion`, `terminal.integrated.useWslProfiles`, `terminal.integrated.windowsReparentConsoleWindow`, `terminal.integrated.windowsUseConptyDll`, `terminal.integrated.wordSeparators`, `terminal.sourceControlRepositoriesKind`, `testing.alwaysRevealTestOnStateChange`, `testing.automaticallyOpenPeekView`, `testing.automaticallyOpenPeekViewDuringAutoRun`, `testing.automaticallyOpenTestResults`, `testing.countBadge`, `testing.coverageBarThresholds`, `testing.coverageMinimapEnabled`, `testing.coverageToolbarEnabled`, `testing.defaultGutterClickAction`, `testing.displayedCoveragePercent`, `testing.followRunningTest`, `testing.gutterEnabled`, `testing.resultsView.layout`, `testing.saveBeforeTest`, `testing.showAllMessages`, `testing.showCoverageInExplorer`, `timeline.pageOnScroll`, `timeline.pageSize`, `update.channel`, `update.enableWindowsBackgroundUpdates`, `update.mode`, `update.showOutdatedBuildReminder`, `update.showPostInstallInfo`, `update.showReleaseNotes`, `update.titleBar`

</details>

<details><summary>实时语言服务：118 项（editor 118）</summary>

`editor.acceptSuggestionOnCommitCharacter`, `editor.acceptSuggestionOnEnter`, `editor.aiStats.enabled`, `editor.codeActionWidget.includeNearbyQuickFixes`, `editor.codeActionWidget.showHeaders`, `editor.codeActions.triggerOnFocusChange`, `editor.codeActionsOnSave`, `editor.codeLens`, `editor.codeLensFontFamily`, `editor.codeLensFontSize`, `editor.colorDecorators`, `editor.colorDecoratorsActivatedOn`, `editor.colorDecoratorsLimit`, `editor.defaultColorDecorators`, `editor.dropIntoEditor.enabled`, `editor.dropIntoEditor.preferences`, `editor.dropIntoEditor.showDropSelector`, `editor.editContext`, `editor.experimental.asyncTokenization`, `editor.experimental.asyncTokenizationLogging`, `editor.experimental.asyncTokenizationVerification`, `editor.experimental.preferTreeSitter.css`, `editor.experimental.preferTreeSitter.ini`, `editor.experimental.preferTreeSitter.regex`, `editor.experimental.preferTreeSitter.typescript`, `editor.experimental.treeSitterTelemetry`, `editor.experimentalGpuAcceleration`, `editor.experimentalWhitespaceRendering`, `editor.foldingImportsByDefault`, `editor.formatOnType`, `editor.gotoLocation.alternativeDeclarationCommand`, `editor.gotoLocation.alternativeDefinitionCommand`, `editor.gotoLocation.alternativeImplementationCommand`, `editor.gotoLocation.alternativeReferenceCommand`, `editor.gotoLocation.alternativeTypeDefinitionCommand`, `editor.inlayHints.enabled`, `editor.inlayHints.fontFamily`, `editor.inlayHints.fontSize`, `editor.inlayHints.maximumLength`, `editor.inlayHints.padding`, `editor.inlineCompletionsAccessibilityVerbose`, `editor.inlineSuggest.edits.allowCodeShifting`, `editor.inlineSuggest.edits.longDistanceHintContextLineCount`, `editor.inlineSuggest.edits.renderSideBySide`, `editor.inlineSuggest.edits.showCollapsed`, `editor.inlineSuggest.edits.showLongDistanceHint`, `editor.inlineSuggest.enabled`, `editor.inlineSuggest.experimental.emptyResponseInformation`, `editor.inlineSuggest.experimental.showOnSuggestConflict`, `editor.inlineSuggest.experimental.suppressInlineSuggestions`, `editor.inlineSuggest.fontFamily`, `editor.inlineSuggest.minShowDelay`, `editor.inlineSuggest.showToolbar`, `editor.inlineSuggest.suppressInSnippetMode`, `editor.inlineSuggest.suppressSuggestions`, `editor.inlineSuggest.syntaxHighlightingEnabled`, `editor.inlineSuggest.triggerCommandOnProviderChange`, `editor.lightbulb.enabled`, `editor.linkedEditing`, `editor.occurrencesHighlightDelay`, `editor.parameterHints.cycle`, `editor.parameterHints.enabled`, `editor.pasteAs.enabled`, `editor.pasteAs.preferences`, `editor.pasteAs.showPasteSelector`, `editor.quickSuggestions`, `editor.quickSuggestionsDelay`, `editor.screenReaderAnnounceInlineSuggestion`, `editor.showDeprecated`, `editor.showUnused`, `editor.snippetSuggestions`, `editor.suggest.filterGraceful`, `editor.suggest.filteredTypes`, `editor.suggest.insertMode`, `editor.suggest.localityBonus`, `editor.suggest.matchOnWordStartOnly`, `editor.suggest.preview`, `editor.suggest.selectionMode`, `editor.suggest.shareSuggestSelections`, `editor.suggest.showClasses`, `editor.suggest.showColors`, `editor.suggest.showConstants`, `editor.suggest.showConstructors`, `editor.suggest.showCustomcolors`, `editor.suggest.showDeprecated`, `editor.suggest.showEnumMembers`, `editor.suggest.showEnums`, `editor.suggest.showEvents`, `editor.suggest.showFields`, `editor.suggest.showFiles`, `editor.suggest.showFolders`, `editor.suggest.showFunctions`, `editor.suggest.showIcons`, `editor.suggest.showInlineDetails`, `editor.suggest.showInterfaces`, `editor.suggest.showIssues`, `editor.suggest.showKeywords`, `editor.suggest.showMethods`, `editor.suggest.showModules`, `editor.suggest.showOperators`, `editor.suggest.showProperties`, `editor.suggest.showReferences`, `editor.suggest.showSnippets`, `editor.suggest.showStatusBar`, `editor.suggest.showStructs`, `editor.suggest.showTypeParameters`, `editor.suggest.showUnits`, `editor.suggest.showUsers`, `editor.suggest.showValues`, `editor.suggest.showVariables`, `editor.suggest.showWords`, `editor.suggest.snippetsPreventQuickSuggestions`, `editor.suggestFontSize`, `editor.suggestLineHeight`, `editor.suggestOnTriggerCharacters`, `editor.suggestSelection`, `editor.tabCompletion`, `editor.wordBasedSuggestions`

</details>

<details><summary>工作台：48 项（workbench 48）</summary>

`workbench.accounts.showAvatar`, `workbench.activityBar.iconClickBehavior`, `workbench.browser.autoReloadOnFileChange`, `workbench.browser.dataStorage`, `workbench.browser.enableChatTools`, `workbench.browser.enableRemoteProxy`, `workbench.browser.experimentalUserTools.enabled`, `workbench.browser.maxHistoryEntries`, `workbench.browser.newTabPlacement`, `workbench.browser.openLocalhostLinks`, `workbench.browser.pageZoom`, `workbench.browser.searchEngine`, `workbench.browser.sendElementsToChat.attachImages`, `workbench.browser.showInTitleBar`, `workbench.cloudChanges.autoResume`, `workbench.cloudChanges.continueOn`, `workbench.commandPalette.experimental.askChatLocation`, `workbench.commandPalette.experimental.enableNaturalLanguageSearch`, `workbench.commandPalette.experimental.suggestCommands`, `workbench.editor.autoLockGroups`, `workbench.editor.empty.hint`, `workbench.editor.historyBasedLanguageDetection`, `workbench.editor.languageDetection`, `workbench.editor.languageDetectionHints`, `workbench.editor.preferHistoryBasedLanguageDetection`, `workbench.enableExperiments`, `workbench.experimental.cloudChanges.autoStore`, `workbench.experimental.cloudChanges.partialMatches.enabled`, `workbench.experimental.modernUI`, `workbench.experimental.modernUIEditorTabStyle`, `workbench.experimental.modernUIUppercaseViewHeaders`, `workbench.experimental.share.enabled`, `workbench.externalBrowser`, `workbench.externalUriOpeners`, `workbench.layoutControl.type`, `workbench.navigationControl.enabled`, `workbench.panel.opensMaximized`, `workbench.productIconTheme`, `workbench.reduceMotion`, `workbench.remoteIndicator.showExtensionRecommendations`, `workbench.sash.hoverDelay`, `workbench.secondarySideBar.defaultVisibility`, `workbench.secondarySideBar.forceMaximized`, `workbench.secondarySideBar.showLabels`, `workbench.tips.enabled`, `workbench.trustedDomains.promptInTrustedWorkspace`, `workbench.view.showQuietly`, `workbench.welcomePage.walkthroughs.openOnInstall`

</details>

<details><summary>搜索：3 项（search 3）</summary>

`search.useGlobalIgnoreFiles`, `search.useIgnoreFiles`, `search.useParentIgnoreFiles`

</details>

<details><summary>文件：3 项（files 3）</summary>

`files.participants.timeout`, `files.refactoring.autoSave`, `files.simpleDialog.enable`

</details>

<details><summary>编辑器：1 项（editor 1）</summary>

`editor.minimap.renderCharacters`

</details>

<details><summary>资源管理器：3 项（explorer 3）</summary>

`explorer.decorations.badges`, `explorer.decorations.colors`, `explorer.excludeGitIgnore`

</details>

<details><summary>零散排除：16 项（workbench 12, editor 4）</summary>

`editor.defaultFoldingRangeProvider`, `editor.rename.enablePreview`, `editor.renameOnType`, `editor.snippets.codeActions.enabled`, `workbench.commandPalette.showAskInChat`, `workbench.editor.markdownDefaultEditorInAgentsWindow`, `workbench.editor.useModal`, `workbench.editorLargeFileConfirmation`, `workbench.modernUIFrostedGlass`, `workbench.modernUIFrostedGlassOpacity`, `workbench.reduceTransparency`, `workbench.settings.applyToAllProfiles`, `workbench.settings.enableNaturalLanguageSearch`, `workbench.settings.showAISearchToggle`, `workbench.shadows`, `workbench.welcomePage.experimentalOnboarding`

</details>
