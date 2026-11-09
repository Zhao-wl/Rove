## 背景

Markdown 预览是 Rove 里**唯一**使用网页内核的地方（D5，立项文档 §2 的唯一例外），也是唯一一处“默认打开方式与 VS Code 不同”的文档类型（D6）。`PROJECT_CHARTER.md` §4.3 里「原生渲染预览（`pulldown-cmark`）、暂不支持 Mermaid」的写法已被 D5 取代，早期草稿中的 `rove-docview` Markdown 渲染模型同样作废，**以 `UX_SPEC.md` §4.3 为准**。

本 issue 必须**按 [[webview]] 的验证报告（`docs/M0_WEBVIEW_SPIKE_REPORT.md`）推荐的集成方案实现**：遮挡、焦点路由、首开耗时与内存、主题与滚动同步的通信方式都以那份报告的结论为准，不在这里另选方案。

必读：`UX_SPEC.md` §4.3（完整重写过）、§6.1、§6.2、§6.4（没有 WebView2 运行时），`FEATURES.md` §9、§12（D5、D6、D7），`docs/compat/SYNTAX_AND_LANGUAGES.md` §5（代码块用 highlight.js + VS Code 的 `highlight.css`）、`SETTINGS.md`（`markdown.*`、`rove.markdown.*`、`workbench.editorAssociations`）、`KEYBINDINGS.md`、`THEME_COLORS.md`（`markdownAlert.*`）。

## 目标

### 1. 预览编辑器类型与默认打开方式（D6）

- 注册预览编辑器类型，标识沿用 VS Code 的 `vscode.markdown.preview.editor`。
- 默认关联 `{"*.md": "vscode.markdown.preview.editor"}`，**只在用户没有设置 `workbench.editorAssociations` 时生效**，通过 [[settings]] 的“Rove 默认值差异”机制登记，不写入共用设置文件（FEATURES §13 规则 5）。
- 打开来源决定模式（UX_SPEC §4.3）：文件树与 [[quickopen]] → 预览；**搜索结果、跳转、引用 → 源码并定位到匹配处**。来源语义由 [[groups]] 的打开文档服务传递。

### 2. 切换与并排

- 编辑器标题栏按钮「显示源码 / 显示预览」（通过 [[contracts]] 的标题栏按钮登记点注册）。
- 命令与默认键全部沿用 VS Code 1.145：`markdown.togglePreview`（`ctrl+shift+v`，`when` 条件照抄 `KEYBINDINGS.md`）、`markdown.showPreviewToSide`（`ctrl+k v`，左右并排的源码 + 预览）、`markdown.showPreview`、`markdown.showSource`、`markdown.preview.refresh`。
- 预览中双击切换到源码并定位到对应行，遵循 `markdown.preview.doubleClickToSwitchToEditor`。

### 3. 渲染

- 样式直接使用 VS Code 的 `markdown.css` 与 `highlight.css`（MIT，登记到 `THIRD_PARTY_NOTICES.md`），跟随当前颜色主题：主题变更时把变量推给 WebView2，不重建实例。
- 代码块高亮用 highlight.js（SYNTAX_AND_LANGUAGES §5）；与编辑器里的 tree-sitter 是两套实现，颜色可能有细微差别，这与 VS Code 一致。
- Mermaid 与 KaTeX **离线打包**，不联网；`markdown.math.enabled`、`markdown.math.macros` 生效。
- 遵循 `markdown.preview.*`（`breaks`、`linkify`、`typographer`、`frontMatter`、`fontFamily`、`fontSize`、`lineHeight`、`markEditorSelection`、`openMarkdownLinks`）与 `markdown.styles`。
- 提示块颜色用登记过的 `markdownAlert.*` 颜色键。
- 本地图片正常显示；链接：相对路径的 `.md` 在预览中打开（`markdown.preview.openMarkdownLinks`、`markdown.links.openLocation`），外部链接交给系统浏览器，打开前确认（FEATURES §3）。

### 4. 安全

- 内嵌 HTML 经过白名单过滤后渲染，**脚本一律禁止执行**。
- 远程资源默认不加载：预览顶部显示第 3 级提示条「已阻止 N 个远程资源」，可为当前文档放行；全局开关 `rove.markdown.allowRemoteResources`（默认 false）。
- WebView2 不允许导航到任何外部地址；渲染所需的全部资源都来自打包内容与本地文件。

### 5. 实例生命周期与性能

- **一个窗口共用一个 WebView2 实例**，第一次打开预览时才创建，不影响冷启动预算（< 400 ms）。
- 所有预览关闭后，经过 `rove.markdown.webviewIdleTimeout`（默认 300 秒）销毁实例释放内存。
- 实例就绪前显示骨架占位（标题与正文的灰条，UX_SPEC §4.3、§6.2），不留白。
- 本机没有 WebView2 运行时时：`.md` 改为以源码打开，弹**一次**通知，说明需要安装运行时并给出微软官方下载页链接；**绝不自动下载**（UX_SPEC §6.4）。

### 6. 滚动同步与大纲

- 编辑器与预览双向滚动同步（`markdown.preview.scrollEditorWithPreview`、`scrollPreviewWithEditor`），需要源码行 ↔ 渲染块的双向映射。
- 预览中的大纲由 [[outline]] 的 Markdown 标题符号提供；点击标题在预览中滚动到对应位置。
- 选择与复制文本由 WebView2 原生提供；锁定预览（`markdown.preview.toggleLock`、`markdown.showLockedPreviewToSide`）如果在 [[webview]] 推荐的方案下是顺手的，一并实现。

### 7. 登记与替换

编辑器类型、命令、设置项、颜色键、界面字符串都通过 [[contracts]] 的登记机制加入。替换 [[skeleton]] 中的 `SKELETON(markdown)` 标记（如有），删除标记并更新 `docs/ARCHITECTURE.md` 的对照表。

## 不包含

- 所见即所得 Markdown 编辑器与全部 `markdown.editor.*` 命令（D7，明确不做）。
- 预览中的 `Ctrl+F` 查找：`KEYBINDINGS.md` 已把 `editor.action.webvieweditor.*` 判为 P1（UX_SPEC §4.3）。若 [[webview]] 的方案下用 WebView2 自带的查找能力即可实现，就一并做；否则在 PR 中说明，留给后续 issue。
- Markdown **源码**中的链接跳转（由 [[links]] 负责）；链接路径补全（Rove 不做补全）；Markdown 源码的编辑能力（M3）。
- 把预览中的选区加入上下文篮子：[[basket]]。
- 修改 [[contracts]] 的接口或 [[webview]] 定下的嵌入方案：需要改时走契约变更流程，单独提小 PR。

## 验收标准

- [ ] 双击 `.md` 默认打开预览；用户在共用 `settings.json` 里设置了 `workbench.editorAssociations` 时以用户设置为准
- [ ] 从搜索结果打开 `.md` 时打开的是源码并定位到匹配行
- [ ] `Ctrl+Shift+V` 切换、`Ctrl+K V` 并排、标题栏按钮三条路径都可用，`when` 条件与 `KEYBINDINGS.md` 一致
- [ ] 渲染测试样例覆盖：GFM 表格与任务列表、内嵌 HTML（含被过滤掉的 `<script>`）、Mermaid 图、KaTeX 公式、提示块、本地图片、代码块高亮
- [ ] 断网状态下 Mermaid 与 KaTeX 正常渲染；含远程图片的文档显示「已阻止 N 个远程资源」提示条，放行后才加载
- [ ] 第一次打开预览之前进程中没有 WebView2 实例（冷启动预算不受影响，数据接入 [[perf]]）；所有预览关闭后超时销毁实例，内存回落
- [ ] 就绪前显示骨架占位；首开耗时与内存符合 [[webview]] 报告中的结论（附实测）
- [ ] 模拟缺少 WebView2 运行时时，`.md` 以源码打开并只弹一次通知，带官方下载链接且不触发任何下载
- [ ] 双向滚动同步在长文档（≥ 2000 行）上误差在一屏以内；双击预览能定位到正确的源码行
- [ ] 主题切换时预览样式跟随，且不重建 WebView2 实例
- [ ] 新增的命令、设置项、颜色键、界面字符串都经由登记机制，无硬编码
- [ ] [[skeleton]] 的端到端测试仍然通过；另加覆盖本 issue 的集成测试（打开 `.md` → 断言进入预览模式 → 切换到源码）
- [ ] 无运行时网络访问（CI 中以断网或网络拦截方式验证）

## 风险与提示

- `markdown.preview.doubleClickToSwitchToEditor`：VS Code 的默认值是 `false`，Rove 的默认值是 `true`（已记录在 `SETTINGS.md` §3 的默认值差异表中，只在用户未设置时生效）。按设置项语义实现。
- 遮挡与焦点路由是这条路线上最大的风险（PLAN 0.2a）。命令面板、菜单、悬停卡片与预览区域重叠时的表现，必须按 [[webview]] 报告的结论实现并在验收里逐个点一遍。
- 预览只渲染，不执行。任何“为了支持某个特性而打开脚本”的改动都要先改 UX_SPEC。
