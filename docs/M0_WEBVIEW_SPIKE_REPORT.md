# Rove M0 WebView2 嵌入验证报告（进行中）

> Issue #6。日期：2026-11-09。当前版本仅记录原型设计与验证边界；没有运行数据的项目不得视为通过。

## 1. 结论与工作台影响

当前 **NO-GO（尚未完成技术验收，不等于否决 D5）**。不得据此宣布 #16 的弹出层方案已获得实测确认。

首选验证路径：GPUI 拥有顶层 HWND，WebView2 使用 windowed hosting。原生弹出层与预览重叠时，临时隐藏整个 WebView2，GPUI 显示占位，弹出层关闭后恢复。此方案不需要把所有弹出层改成顶层窗口，但会暂时遮掉完整预览。必须通过重叠截图、焦点恢复和生命周期实测才能推荐集成。

## 2. 原型与环境

原型：`spikes/webview-gpui/`，独立 Cargo.lock，不进入产品 crates。GPUI 固定 0.2.2，启用 windows-manifest；直接使用 webview2-com 0.38.2（与 GPUI 同用 windows 0.61），避免额外窗口框架，直接访问 GPUI 提供的 raw-window-handle。

构建：MSVC。实际运行时版本、显示器缩放、构建耗时和运行结果将在实测后填写。

## 3. 实测矩阵

| 项目 | 当前证据 | 状态 |
|---|---|---|
| 原生层与 WebView2 重叠 | 原型有无遮挡规避/隐藏规避两个阶段 | 待运行截图 |
| 焦点、Ctrl+P / Ctrl+Shift+P / Ctrl+Shift+V / Esc / Tab | COM 加速键回调与路由日志已实现 | 待真实按键测试 |
| 首开时间与内存 | measure.ps1，每类三次，按宿主及后代进程统计 | 待测量 |
| 可信桥接、主题、滚动 | 本地桥接与版本化 JSON 协议 | 待测量 |
| 远程资源拦截 | 请求拦截返回 403 并计数 | 待运行 |
| 文档脚本禁用 | SetIsScriptEnabled(false)，页面含执行探针 | 待运行 |
| 无运行时 | 可指定不存在的 browser folder，测试创建失败回调 | 待故障注入；不等于真实无安装环境 |
| 创建→销毁→重建 | auto 模式包含两个实例周期 | 待运行与内存回落观察 |
| DPI / 混合缩放 | 每帧从客户区物理尺寸设置 bounds | 待人工验证 |
| 骨架占位 | 创建前就存在 GPUI 占位内容 | 待可见时序验证 |

## 4. 推荐集成设计草案（未通过验收）

- HWND 所有权：GPUI 持有宿主；窗口内预览管理器持有 controller，回调通过弱引用访问状态，销毁调用 Close。
- 延迟创建：只有预览打开事件创建环境和 controller，回调全异步；先显示骨架。窗口关闭或超时释放时递增 generation，丢弃迟到回调，防止重新挂回已关闭窗口。
- 每窗口一个实例。切换文档复用实例；所有预览关闭后按 300 秒默认值回收。原型用短时周期验证机制，不声称测过完整五分钟。
- 布局提交使用客户区物理坐标；不要把 GPUI 逻辑像素直接交给 controller。
- 弹出层管理器需要登记占用矩形和可见周期，判断是否与预览相交。重叠时先隐藏 webview，再显示 GPUI 弹出层；结束后恢复并按照原焦点目标恢复焦点。通知与悬停也必须纳入，不能只处理命令面板。
- 不在本 issue 修改契约。如果 #16 的登记接口缺少占用区域/可见周期，应按契约流程另提 PR。

## 5. 按键归属（原型意图，待实测）

| 按键/入口 | 原型处理者 | 产品集成要求 |
|---|---|---|
| Ctrl+P | 宿主 quickOpen | 标记 Handled，再异步进入 Rove 命令分发 |
| Ctrl+Shift+P | 宿主 commandPalette | 同上 |
| Ctrl+Shift+V | 宿主 togglePreview | 交给已登记命令 |
| Esc | 宿主关闭弹出层 | 按上下文处理，不吞掉正常页面选择逻辑 |
| Tab / Shift+Tab | 原型返回宿主 | 产品需要正确的边界 Tab 顺序，不能粗暴拦截所有页面 Tab |
| Ctrl+F | 原型标记 findDeferred | 本次不据此声称已实现预览内查找 |
| Ctrl+R | 禁止默认刷新 | Rove 刷新命令另行管理 |
| 右键菜单 / DevTools | 禁用默认入口 | 需要时使用已登记的 Rove 菜单 |

## 6. 安全与通信

用户已明确：禁止文档内嵌及外来脚本，允许 Rove 打包的可信桥接/渲染脚本。原型禁用文档 JavaScript，用 AddScriptToExecuteOnDocumentCreated 注入可信脚本；页面自身的 script 探针用于检测是否被执行。真实 Markdown 仍须在进入 WebView2 前白名单净化，不能因禁用脚本而放弃过滤 HTML。

本地内容映射为 `https://rove.local/`（DENY_CORS）；非本地域请求返回 403 并计数，导航与新窗口分别拦截。此处“本地域”只代表虚拟映射，不需要 DNS 或网络服务。后续仍需操作系统级网络证据，不能把日志没有请求等同于绝无后台联网。

消息草案：`{version:1,type,seq,...payload}`。宿主→页面：ping、theme、scroll；页面→宿主：ready、pong、themeApplied、scrollApplied、scroll。产品还需要 documentId/generation、来源校验、大小上限、严格 schema 和 echo 抑制。滚动消息通过 requestAnimationFrame 合并；宿主 ping 最快 16ms 一次，最终频率应以实际往返分位数决定。

## 7. 性能口径

measure.ps1 分别运行全新 profile 与复用 profile 各三次。全新 profile 不等于清空 OS 页缓存后的冷启动。报告必须分别列出进程启动→首帧、开始创建→controller、开始创建→页面 ready。

空闲内存需同时报告宿主工作集、宿主+WebView2 进程树工作集及私有提交。共享页求和可能重复统计，不能把宿主单进程 <60MB 当成开启预览后整应用 <60MB。原型持续发送探针并刷新帧，测量值不能冒充停止活动后的空闲值。

## 8. 备选与未验证清单

若隐藏策略 UX 不可接受，备选为独立弹出 HWND（会影响所有弹出层定位、激活、DPI、可访问性），或 visual hosting（需要接入合成视觉树与输入转发，超出普通子窗口嵌入）。原生 pulldown-cmark 方案需重新决策 D5，并评估内嵌 HTML、Mermaid、数学公式能力损失。

当前表内所有待测项仍未验证；不据文档或源码推断标记为实测通过。

## 9. 官方依据

- [Windowed vs. Visual hosting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting)
- [ICoreWebView2：可信脚本注入与消息 API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2)
