"""Issue 拆分骨架的唯一数据源（阶段 0.4）。

每项：(键, 里程碑, 难度, 领域, 标题, [前置依赖的键])。编号 Rxx 按本列表顺序自动分配。
原则：接口先行（A1、A2 串行完成后再分头并行）；只服务一个界面的数据层与界面合并为纵向条目；
每个里程碑最后有一个【集成验收】。
"""

ISSUES = [
    # ───────────── M1：接口先行（单线程，合并后才分头并行） ─────────────
    ("contracts", "M1", 4, "infra", "【接口先行 A1】架构与接口契约：ARCHITECTURE.md，共享接口写成可编译的类型与空实现（文本源、打开文档服务、命令注册与上下文键、设置读取、主题颜色、反馈、列表组件、界面字符串表）", []),
    ("skeleton", "M1", 3, "ui", "【接口先行 A2】行走骨架：用最简实现打通“打开文件夹 → 文件树 → 点击文件 → 带高亮显示”的完整链路", ["contracts"]),
    # ───────────── M1：基础设施（可与 A1 并行） ─────────────
    ("ci", "M1", 2, "infra", "工程规范与 CI（AGENTS.md 与完成标准、CI、核心层禁止依赖 gpui 的检查、测试项目环境变量）", []),
    ("perf", "M1", 2, "infra", "性能基准与预算守护（测试数据生成、基准框架、预算文件）", ["ci"]),
    ("gpui", "M1", 4, "ui", "内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随", []),
    ("webview", "M1", 4, "ui", "WebView2 嵌入验证：遮挡、焦点路由、首开耗时与内存、主题与滚动同步（PLAN 0.2a）", []),
    # ───────────── M1：核心层（在 A1 的接口后面实现） ─────────────
    ("buffer", "M1", 2, "core", "文本缓冲：编码识别（自动识别与候选编码语义）、BOM、换行符保持（含混合换行符）", ["contracts", "skeleton"]),
    ("largefile", "M1", 3, "core", "文件大小分级与超大文件：内存映射、分块行索引、追加内容的增量索引", ["buffer"]),
    ("syntax", "M1", 3, "syntax", "tree-sitter 集成：C#/JSON/YAML/XML/Markdown 高亮查询（捕获名即 TextMate scope）与增量解析", ["contracts", "skeleton"]),
    ("theme", "M1", 3, "core", "主题引擎：加载 VS Code 主题 JSON、TextMate scope 匹配（移植 vscode-textmate）、颜色默认值表", ["contracts", "skeleton"]),
    ("golden", "M1", 2, "syntax", "着色黄金测试：用 VS Code colorize 测试数据逐字符比对颜色", ["syntax", "theme"]),
    ("langcfg", "M1", 2, "syntax", "语言配置：加载 language-configuration.json，JavaScript 正则兼容", ["contracts"]),
    ("settings", "M1", 3, "core", "设置层：与 VS Code 共用 settings.json（JSONC、语言级覆盖、热加载、保留格式写入、默认值差异规则）", ["contracts"]),
    ("keys", "M1", 3, "core", "键位与命令层：共用 keybindings.json、键表达式与 when 表达式、命令注册表（VS Code 命令 ID）", ["contracts", "skeleton"]),
    ("workspace", "M1", 2, "workspace", "工作区：扫描、排除规则（不读 .gitignore）、文件监听、只读规则", ["contracts", "skeleton"]),
    # ───────────── M1：界面（在 A2 的骨架上替换为正式实现） ─────────────
    ("shell", "M1", 3, "ui", "工作台：标题栏与命令中心、菜单、活动栏、侧边栏、面板、状态栏容器、布局持久化、启动预算，以及反馈分级（临时消息、提示条、通知、模态框、勿扰）", ["skeleton", "webview"]),
    ("groups", "M1", 3, "ui", "编辑器组与标签：网格分屏、预览标签、拖动、固定、“打开文档”服务", ["skeleton"]),
    ("lists", "M1", 3, "ui", "通用列表、树与快速输入组件（键盘导航、虚拟化、过滤）", ["skeleton", "webview"]),
    ("codeview", "M1", 4, "ui", "代码视图 Element：虚拟化、f64 滚动、整形缓存与预取、行号、只读选区、命中测试、滚动条", ["skeleton", "buffer"]),
    ("codeview-hl", "M1", 3, "ui", "代码视图：高亮与主题、括号匹配、文本同名高亮、折叠（语法、标记、缩进）、缩进参考线、空白字符、只读状态界面", ["codeview", "syntax", "theme", "langcfg", "workspace"]),
    ("findwidget", "M1", 2, "ui", "当前文件查找条（正则、大小写、全词、在选区内查找；替换留到 M3）", ["codeview"]),
    ("largefile-ui", "M1", 2, "ui", "超大文件与超长行的界面（只读提示条、行索引进度、转到行等待、流式查找、截断标记）", ["largefile", "codeview", "findwidget"]),
    ("explorer", "M1", 2, "ui", "资源管理器（文件树、紧凑文件夹、定位当前文件、过滤）", ["workspace", "lists", "groups"]),
    ("quickopen", "M1", 2, "ui", "快速打开与命令面板：移植 VS Code 模糊匹配算法及测试用例，支持 :行号、@符号、>命令、命令中心", ["workspace", "lists", "keys", "groups", "outline"]),
    ("search", "M1", 3, "workspace", "全局搜索：搜索引擎（search.* 语义、流式结果、取消、GBK）与搜索视图", ["workspace", "buffer", "lists", "groups"]),
    ("outline", "M1", 2, "ui", "大纲：符号数据、大纲视图、面包屑、转到文件中的符号", ["syntax", "langcfg", "lists", "groups"]),
    ("statusbar", "M1", 2, "ui", "状态栏条目（行列、缩进、编码切换与重新打开、换行符、语言模式、只读标记）", ["shell", "buffer", "langcfg", "largefile"]),
    ("welcome", "M1", 1, "ui", "欢迎页、最近打开、启动参数打开，以及首选项入口（选择主题、打开共用 settings.json 并显示提示条、齿轮菜单）", ["groups", "settings", "keys", "shell", "theme", "lists"]),
    ("markdown", "M1", 3, "ui", "Markdown 预览（WebView2：完整渲染、默认预览、切换、滚动同步、阻止远程资源、实例回收）", ["webview", "groups", "theme", "settings"]),
    ("dpi", "M1", 3, "ui", "高 DPI 与多显示器验证与修复", ["codeview", "shell", "quickopen", "markdown"]),
    ("integ-m1", "M1", 3, "infra", "【集成验收】M1：在两个 Unity 项目上按场景清单走完端到端流程，修复接缝问题，核对性能预算",
     ["perf", "gpui", "largefile", "golden", "settings", "shell", "codeview-hl", "largefile-ui", "explorer", "quickopen",
      "search", "outline", "statusbar", "welcome", "markdown", "dpi"]),
    # ───────────── M2 C# 导航 ─────────────
    ("idx-format", "M2", 3, "lang-cs", "C# 索引格式 v2（紧凑编码、二进制局部键、UTF-8 列、排序路径表、元数据段）", []),
    ("idx-perf", "M2", 4, "indexer", "索引进程性能优化（大型项目 < 90 s、峰值 < 3 GB）", ["idx-format"]),
    ("idx-meta", "M2", 3, "indexer", "外部符号元数据视图的生成", ["idx-perf"]),
    ("idx-incr", "M2", 4, "indexer", "增量索引：常驻模式、单文件重绑、覆盖层", ["idx-perf", "idx-meta"]),
    ("idx-life", "M2", 3, "lang-cs", "索引生命周期与状态：缓存目录、后台预读、原子替换、进程管理，以及状态栏五种状态、详情、日志、失败通知", ["idx-format", "workspace", "shell"]),
    ("l1", "M2", 3, "lang-cs", "L1 快速声明索引（tree-sitter）", ["syntax", "workspace"]),
    ("router", "M2", 3, "lang-cs", "查询路由与未保存修改的位置映射", ["buffer", "idx-life", "l1"]),
    ("navtests", "M2", 2, "lang-cs", "C# 导航准确率测试集（200 例）与回归运行器", []),
    ("nav-ui", "M2", 3, "ui", "C# 导航交互：转到定义、转到实现、Ctrl+点击、悬停、导航历史、语义级同名高亮、近似结果提示", ["router", "codeview-hl", "groups"]),
    ("peek", "M2", 3, "ui", "速览窗口：速览定义、引用、实现（D3 默认方式）", ["router", "codeview", "lists"]),
    ("meta-ui", "M2", 2, "ui", "元数据视图界面", ["idx-meta", "nav-ui", "peek"]),
    ("idx-pack", "M2", 2, "release", "索引组件打包与版本握手", ["idx-incr", "idx-life"]),
    ("integ-m2", "M2", 3, "infra", "【集成验收】M2：用导航测试集与两个 Unity 项目验收跳转、引用、速览、元数据视图、增量更新，修复接缝问题",
     ["navtests", "nav-ui", "peek", "meta-ui", "idx-pack"]),
    # ───────────── M3 轻编辑 ─────────────
    ("edit-core", "M3", 3, "core", "编辑核心模型（编辑事务、撤销重做、光标模型、单词边界规则移植）", ["buffer", "langcfg"]),
    ("edit-ui", "M3", 4, "ui", "代码视图的编辑交互与输入法", ["gpui", "codeview", "edit-core"]),
    ("replace", "M3", 2, "ui", "当前文件替换（全部替换可一次撤销）", ["findwidget", "edit-core"]),
    ("save", "M3", 3, "core", "保存与外部修改（编码保持、无法编码字符提示、原子写入、占用重试、冲突处理、自动重载）", ["edit-core", "workspace", "shell", "syntax"]),
    ("hotexit", "M3", 2, "core", "热退出与崩溃恢复（D8：从 M4 提前，避免 M3 交付后意外退出丢失修改）", ["save", "shell"]),
    ("integ-m3", "M3", 2, "infra", "【集成验收】M3：输入法场景、编码往返零差异、外部修改与保存冲突、编辑后高亮与索引的联动、热退出与崩溃恢复", ["edit-ui", "replace", "save", "hotexit", "integ-m2"]),
    # ───────────── M4 体验补全（P1） ─────────────
    ("multicursor", "M4", 2, "ui", "多光标与行操作", ["edit-ui", "save", "findwidget"]),
    ("autoedit", "M4", 3, "ui", "注释切换、自动缩进、自动闭合、包围、回车规则", ["edit-ui", "langcfg"]),
    ("smartselect", "M4", 2, "ui", "智能扩选、括号跳转与选择", ["edit-ui", "syntax", "codeview-hl"]),
    ("sticky", "M4", 2, "ui", "粘性滚动", ["codeview-hl", "outline"]),
    ("minimap", "M4", 3, "ui", "缩略图与概览标尺", ["codeview-hl"]),
    ("brackets", "M4", 2, "ui", "括号对着色与活动缩进参考线高亮", ["codeview-hl"]),
    ("wrap", "M4", 3, "ui", "软换行", ["codeview"]),
    ("semantic", "M4", 2, "lang-cs", "C# 语义高亮与不生效预处理代码置灰（D9）", ["nav-ui", "theme"]),
    ("nav-extra", "M4", 2, "ui", "转到类型定义、工作区符号、侧边引用视图", ["router", "quickopen", "peek"]),
    ("structure", "M4", 3, "docview", "JSON/YAML/XML：结构树、语法校验、问题面板与转到下一个问题、格式化与压缩、复制路径", ["outline", "syntax", "shell", "edit-core"]),
    ("replace-all", "M4", 2, "ui", "全局替换", ["search", "edit-core", "save"]),
    ("fileops", "M4", 2, "ui", "资源管理器文件操作（新建、重命名、删除到回收站、复制路径）", ["explorer"]),
    ("image", "M4", 1, "ui", "图片预览", ["groups"]),
    ("langs-extra", "M4", 3, "syntax", "其他语言高亮：HLSL、Lua、Protobuf，以及自写 ShaderLab 语法", ["syntax", "golden"]),
    ("basket", "M4", 2, "ui", "上下文篮子", ["codeview", "outline", "keys"]),
    ("tail", "M4", 1, "ui", "跟随文件末尾、格式化后查看", ["largefile-ui", "structure"]),
    
    ("i18n", "M4", 2, "ui", "英文界面翻译与界面语言切换（字符串表基础设施已在 A1 中完成）", ["shell"]),
    ("ext-themes", "M4", 2, "ui", "只读加载 VS Code 扩展主题、Seti 文件图标主题", ["theme", "explorer"]),
    ("zen", "M4", 1, "ui", "Zen 模式、全屏、居中布局", ["shell"]),
    ("settings-ui", "M4", 3, "ui", "图形化设置界面", ["settings", "lists"]),
    ("links", "M4", 1, "ui", "链接检测与打开", ["codeview"]),
    ("langdetect", "M4", 1, "syntax", "语言检测（启发式）", ["syntax"]),
    ("nonet", "M4", 3, "ui", "精简 GPUI 的网络依赖", ["gpui"]),
    ("integ-m4", "M4", 3, "infra", "【集成验收】M4：FEATURES 中全部 P1 功能的端到端验收与性能回归检查",
     ["multicursor", "autoedit", "smartselect", "sticky", "minimap", "brackets", "wrap", "semantic", "nav-extra", "structure",
      "replace-all", "fileops", "image", "langs-extra", "basket", "tail", "i18n", "ext-themes", "zen",
      "settings-ui", "links", "langdetect", "nonet", "integ-m1", "integ-m2", "integ-m3"]),
    # 注：v0.1 首发时 R77（integ-v01，M5）实际先于本条完成；骨架不允许依赖更晚的里程碑，所以这里不列它，见正文的范围调整说明。
    # ───────────── M5 发布 ─────────────
    ("release", "M5", 2, "release", "Windows 发布流水线、安装包、体积守护、第三方许可声明", ["shell", "idx-pack"]),
    # ───────────── 2026-11-13 追加：v0.1 首发（见 docs/MILESTONE_v0.1.md） ─────────────
    ("updater", "M5", 3, "release", "自动更新：GitHub Releases 更新清单、ed25519 校验、后台下载、重启时静默安装、索引组件更新（唯一的网络出口）",
     ["release", "hotexit", "nonet", "settings", "shell"]),
    ("integ-v01", "M5", 3, "infra", "【集成验收】v0.1 发布候选：干净机器安装、自动更新升级、M1–M3 与提前条目的回归、性能预算全部 enforced",
     ["integ-m1", "integ-m2", "integ-m3", "multicursor", "autoedit", "sticky", "wrap", "nav-extra", "nonet",
      "release", "updater"]),
]

# v0.1 首发包含的条目（GitHub 里程碑「v0.1 首发」）。已关闭的条目也列在这里，方便核对范围；
# 跟进 bug 与性能 issue 不在本骨架中，直接在 GitHub 里程碑里维护（见 docs/MILESTONE_v0.1.md §3.4）。
V01 = {k for k, m, *_ in ISSUES if m in ("M1", "M2", "M3")} | {
    "multicursor", "autoedit", "sticky", "wrap", "nav-extra", "nonet", "release", "updater", "integ-v01",
}
