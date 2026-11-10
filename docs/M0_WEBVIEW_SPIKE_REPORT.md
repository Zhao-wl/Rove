# Rove M0 WebView2 嵌入验证报告

> Issue #6 · 实测：2026-11-09；预算口径确认：2026-11-10 · 部分验收完成，PR 保持 Draft。
> 原型与原始证据：`spikes/webview-gpui/`。工作树中只保留两批证据（`evidence/2026-11-10-retain-close/`、`evidence/2026-11-10-review3/`）；其余历史批次已删除，固定链接见§16。

## 1. 结论摘要

**技术路线判定：GO（Hwnd＋相交隐藏）；整体issue验收未齐，PR仍保持Draft。** 最新状态以§13为准：真实手测已确认Hwnd模式可见、相交隐藏有效、125%与双屏通过，Tab双向路由通过；Baseline正文不可见，应排除。Transparent的加载/隐藏/关闭背景修复已获100%与125%真实截图通过。快速200ms重开在Hwnd125%及两模式100%获3/3手测通过；Transparent125%有一次未收到o的输入失败，同间隔延长按键持续时间后3/3通过，本轮补原生焦点恢复。之后（§15）定位到 x 回调内同步 Close 阻塞 UI 线程 29–459 ms，关闭改为“隐藏保留＋空闲超时后帧内回收”，本机100%自动注入 Transparent 30/30、Hwnd 15/15 立即重新显示；125% 物理短按复验暂缓，不作为本 PR 的阻塞项。

对工作台：Hwnd＋相交隐藏已有原型级手测支持，可据此设计；尚未覆盖四类产品控件和产品骨架整体验收，不能将候选方案当作最终集成通过。以下历史批次保留用于追踪，不覆盖最新手测与修复结论。

**预算口径已由用户于 2026-11-10 确认：60 MB 只约束未开启预览的基础阅读器，WebView2 预览单独列预算。** 11-09旧构建开启预览后的历史进程树工作集实测为 **324–341 MiB**，私有提交 **240–250 MiB**；这些是观测值，不是已批准的预览预算上限，不再作为违反基础阅读器 60 MB 预算的理由。预览的具体预算数值尚未设定。当前 NO-GO 来自验收未齐，不再包含“60 MB 适用范围待确认”这一阻塞。

| 状态 | 预算与记录口径 | 当前证据边界 |
|---|---|---|
| 未开启预览的基础阅读器 | 保留 60 MB 约束 | 本原型持续刷新，不能代替产品静止空闲验收 |
| WebView2 预览开启 | 单列预算；分别记录宿主、WebView2 子进程及合计的工作集/私有提交 | 现有 324–341 MiB 是活动原型进程树合计，含共享页重复计数可能性；具体上限未设定 |
| 预览全部关闭且完成回收 | 单独验证资源释放和基础阅读器内存恢复 | 49–52 MiB 来自 11-09 旧构建的 Close 回落观测，**已不适用于§15 的生命周期**（见下注），不是产品静止空闲预算已通过 |

> **关于「回收后 49–52 MiB」的适用范围。** 该数字出自§4.5 的 11-09 旧构建：当时每次打开都新建 environment，`Close()` 会连带销毁浏览器环境与全部子进程。当前实现（§13 起）一个窗口持有一个 environment、Close 只销毁 controller，§15 又把用户关闭改成「隐藏保留」——`x` 之后 controller 仍然存活，只调用了 `SetIsVisible(false)`，没有 `TrySuspend()`，**保留期内的内存仍是预览开启的量级**（§10 当前构建实测峰值工作集 327–347 MiB、私有提交 245–262 MiB）。因此不能用 49–52 MiB 描述「关闭预览后」的内存，也不能用它论证 300 秒超时前的占用。超时回收后的稳定值与 300 秒全释放均未在本 spike 测过，见§6.1。

人工验收入口：[C 链手测清单](../spikes/webview-gpui/MANUAL_ACCEPTANCE.md)，包含 #5/#6 启动命令、逐步操作和反馈模板。

对 #16 / #18 / #24：优先待验方案是“弹出层与预览相交时隐藏 WebView2，GPUI 显示占位，关闭后恢复”。它不强制所有弹出层改成独立顶层 HWND，但要求统一登记弹出区域、可见周期和焦点恢复目标。**在视觉测试通过前，仅为建议，不能当作已验证结论。**

## 2. 方法与环境

- GPUI `=0.2.2`，`default-features=false`，`windows-manifest` 显式开启。直接使用 `webview2-com =0.38.2`，与 GPUI 共用 windows 0.61；相比引入额外窗口框架，可直接控制现有 HWND、COM 事件与 controller 生命周期。
- Rust/MSVC 1.97.1，release + thin LTO + 单 codegen unit。WebView2 运行时文件版本实测为 **157.0.4341.42**。
- `spikes/webview-gpui` 独立 manifest/lock，在 workspace 外；不改 `crates/` 契约，也不依赖 #5 的 vendor 补丁。
- `measure.ps1`：全新 profile 三次、复用 profile 三次。**不是重启/清空 OS 页缓存意义的冷启动**。每次约 16 秒，1 秒后按需创建、5 秒重叠、8 秒隐藏规避、11 秒销毁、13 秒重建。
- 每约 0.7–0.9 秒枚举本原型的后代进程并求和工作集/私有提交。工作集可能重复统计共享页；包括原型启动产生的辅助进程，不把其他应用的 WebView2 混入。
- 通信探针每 ≥16ms 发送一条，滚动事件以 requestAnimationFrame 合并。该场景持续刷新，**不是静止空闲测量**。
- `first_render` 是进程内 main→首次 Render 回调，不是外部进程启动→显示器首帧。controller 与 ready 时间从开始创建计时，彼此不能混用。

## 3. 历史六次实测数据（旧构建，非当前源码验收）

这些日志来自开发过程中的早期二进制，未记录 commit/构建身份；缺少当前源码必写的 `localImageLoaded`/`kind` 字段，不能作为当前源码的验收证据，也不能用当前 `verify_run.py` 当作同一 schema 检查。原始文件保留不补字段。`final-communication.jsonl` 有 kind、缺 localImageLoaded；旧 acceptance.jsonl 虽有两者，仍缺构建身份。下表仅保留历史观察，前一版本化复跑见第8节；最新修复构建与手测结论见第14节。

每行取该进程第一次创建；同进程第二次创建另见回收项。内存取 4–10 秒样本中位数。

| Profile | 首次 Render ms | Controller ms | 页面 ready ms | 进程树工作集 MiB | 私有提交 MiB | 最多进程数 |
|---|---:|---:|---:|---:|---:|---:|
| 全新 1 | 318.85 | 280.46 | 2442.44 | 340.57 | 245.96 | 8 |
| 全新 2 | 266.15 | 281.94 | 2418.06 | 340.79 | 250.03 | 8 |
| 全新 3 | 354.36 | 314.32 | 2486.95 | 341.18 | 248.73 | 8 |
| 复用 1 | 354.76 | 366.32 | 2553.04 | 326.15 | 240.38 | 8 |
| 复用 2 | 304.43 | 252.64 | 2386.72 | 326.38 | 242.30 | 8 |
| 复用 3 | 297.27 | 282.66 | 2444.86 | 323.92 | 241.45 | 8 |

这六次首次 Render 均 <400ms，且 WebView2 到 1 秒才创建，证明原型采用了按需创建路径；**不能据此保证产品冷启动 <400ms**。其他烟雾运行出现过 748ms、2548ms 的首次 Render，宿主存在明显调度波动；评审者另报告一次646ms首帧，不能隐藏在六次低值之外（未附原始日志，不混入本仓库样本）。最终应由 #4 用产品基准测量。

ready 约 2.4–2.6 秒，骨架占位是必需项。controller→ready 之间约两秒的来源尚未归因，不应把 controller 时间当作可交互时间。

## 4. 分项结果

### 4.1 遮挡与弹出层

最新固定提交三模式手测见§13：Baseline不透明合成遮住网页，Hwnd正文可见但网页覆盖原始GPUI矩形，相交隐藏/恢复有效；Transparent正文可见且GPUI矩形在上，但原版加载/隐藏透底。此结果确认叠放依赖渲染模式，不存在通用的单向覆盖规则。

优先候选是Hwnd＋相交隐藏；产品仍需统一弹出层区域、焦点交接和恢复管理。Transparent不透明背景已通过新版复验，可作为替代候选；其125%一次短按未到达宿主仍需焦点修复后复验。旧黑屏/PrintWindow截取不作为证据，当前采用评审可见桌面截图与固定提交日志。

### 4.2 焦点与按键

最新人工反馈：KEY-1～7在两档缩放通过，包含Tab/Shift+Tab双向真实路由；KEY-8/9在Hwnd与Transparent通过。Baseline受正文不可见限制，不宣称完整通过。下表为原型路由，产品命令与完整Tab顺序仍需集成验收。

| 按键/入口 | 原型处理者 | 集成要求 |
|---|---|---|
| Ctrl+P | quickOpen，标记 Handled | 异步交给 Rove 命令分发 |
| Ctrl+Shift+P | commandPalette | 同上 |
| Ctrl+Shift+V | togglePreview | 登记命令负责切换 |
| Esc | 返回宿主、关闭弹出层 | 依上下文处理 |
| Tab / Shift+Tab | 可信桥接keydown返回宿主；宿主MoveFocus进入网页 | 产品应实现边界 Tab 顺序；原型粗粒度退出不能直接照搬 |
| Ctrl+F | findDeferred | 没有实现查找 UI，不声称原生查找可用 |
| Ctrl+R | 禁止默认刷新 | 产品刷新走独立命令 |
| 右键/DevTools | 禁用默认入口 | 按需接入 Rove 菜单 |

### 4.3 通信、主题和滚动

历史六次试验各收到 261–271 条未标注 kind 的往返回复，不能称为 ping-only，也不能与当前混合 pong/scrollApplied/themeApplied 总量混淆；各次 p50 为 **0.776–0.944ms**，p99 为 **1.733–2.342ms**。

另一次包含真实样式变更和滚动的运行（`final-communication.jsonl`）：

| 回复 | 样本数 | p50 ms | p99 ms |
|---|---:|---:|---:|
| pong | 176 | 0.703 | 1.706 |
| scrollApplied | 86 | 0.950 | 2.157 |
| themeApplied | 2 | 1.037 | 样本不足，不报告分位数 |

同时收到 55 条页面 scroll 事件；themeApplied 样本仅用于功能及单次延迟观察，不作稳定性结论。主题变更没有创建新 controller。滚动的布局误差与长文档源码行映射属于 #29，未在本 spike 实现。

这些历史低延迟样本不能保证每次回复都在一帧内。评审者在当前版本复跑报告 405 次中 8 次超过16ms（约1.98%），p99约36ms、最大约114ms；该批原始日志未附在仓库，作为评审观测单列，不混入本次重跑统计。滚动同步必须允许跨多帧延迟：非阻塞投递、同帧合并、只保留最新目标、丢弃旧seq/旧documentId应答、禁止无界排队，页面回送加来源标记避免回声循环。60Hz只是发包探针速率，不是延迟保证或频率上限；120Hz和满负载尚未验证。

协议草案：`{version:1,type,seq,...payload}`。宿主→页面：ping、theme、scroll；页面→宿主：ready、pong、themeApplied、scrollApplied、scroll。原型检查来源、版本和 64KiB 大小，超时清理未应答探针；产品还需 documentId/generation、字段 schema、过期文档丢弃和映射版本。

### 4.4 文档脚本与本地资源

用户明确的语义是：**禁止文档及外来脚本，允许 Rove 打包的可信桥接/渲染脚本**。

实测差异：`IsScriptEnabled(false)` 时，AddScriptToExecuteOnDocumentCreated 的直接语句与 ExecuteScript 可执行，但 DOMContentLoaded / 消息事件桥接没有形成持续 pong；日志在 `global-script-disabled.jsonl`。不能只凭 API 调用成功宣布方案可用。

最终原型使用页面 CSP `script-src 'none'` 拒绝文档脚本，宿主通过 ExecuteScript 注入打包桥接；早期注入仅作探针。六次运行、每次两次创建，文档 script 探针全部保持未执行，可信桥接持续应答。生产仍必须在拼接 HTML 前白名单净化；CSP 不能替代净化，不能允许文档覆盖 CSP 或注入可信脚本。

资源目录映射到 `https://rove.local/`，DENY_CORS；非本地域资源请求返回 403 并计数。六次运行均各拦截两次远程图片请求（两个实例各一次）。最终 `acceptance.jsonl` 的两个实例均确认映射的本地图片 `naturalWidth > 0`，且当时版本的校验器曾检查通过；该文件缺少构建身份，当前 `verify_run.py` 会拒绝为 LEGACY/UNVERSIONED，不能作为当前源码验收。顶层外部导航和新窗口也设置了拦截。**尚无 OS 级网络跟踪，不能声称 WebView2 后台绝无网络访问**；代码只证明页面资源拦截路径。

### 4.5 缺失运行时与回收

把 browser folder 指向不存在目录，创建返回 `0x80070002`，原型保持宿主界面并记录错误，没有下载逻辑。该故障注入没有卸载系统运行时，**不等于在真正未安装机器上验收**。产品应退回源码、一次通知与官方下载链接，不自动下载。

以下为11-09旧构建历史数据，不代表当前构建：六次均完成创建→Close→再创建。Close 约 0.75–1.65 秒后，进程树降至 2 个，工作集回落至 **49.3–52.1 MiB**，私有提交 **85.7–88.1 MiB**。一次在 11.75 秒仍有 6 个进程，到 12.65 秒降至 2；因此 Close 不是瞬间释放全部子进程。

重建 controller 223–245ms，页面 ready 2345–2383ms。原型用短时周期验证回收机制，未等待完整 300 秒，未测长时间多轮资源增长。

**这段 49.3–52.1 MiB 不适用于当前生命周期**：当时 `Close()` 连带销毁 environment，而现在 environment 随窗口存活、`x` 只隐藏保留 controller（§13、§15）。详见§1 的注记与§6.1。

## 5. 集成设计（有条件建议）

1. GPUI 拥有顶层 HWND；每窗口的预览管理器拥有一个 controller。原型不修改 GPUI 或共享契约。
2. 第一次打开预览时异步创建，先显示骨架；不要嵌套消息泵阻塞 UI。关闭或取消时递增 generation，迟到回调立即 Close，不挂回失效窗口。
3. 按布局提交客户区物理 bounds；GPUI 逻辑坐标乘当前 scale，跨屏后更新。当前原型从 GetClientRect 获取物理尺寸。**100%/125% 与混合 DPI 双屏往返拖动已有真实手测证据（见§13、§14）**；但原型从未显式同步 `ICoreWebView2Controller3::RasterizationScale`（只依赖 WebView2 自身跟随父窗口 DPI），**150% 也没有专门测过**，这两项留给 #29（见 5.1）。
4. 先按§4.1实测渲染模式与叠放关系，再决定是否相交隐藏；不得继续假设WebView2必定覆盖GPUI。焦点与Tab边界由宿主负责，视觉通过前不能解除#16依赖。
5. 文档切换复用实例；用户关闭预览只隐藏（SetIsVisible(false)）并保留 controller，`webviewIdleTimeout` 到期后才 Close；超时前重新打开直接复用，不重建。`ICoreWebView2Controller::Close()` 是同步调用，本机实测 29–459 ms（中位 48 ms），**不得在按键/输入回调里调用**，只在空闲超时或窗口关闭时从帧回调/定时器执行。新文档消息必须携带 generation/documentId。见§15。
6. 接口缺口走单独契约 PR，本次没有擅自修改 #1 的接口。

### 5.1 #29 必须做、本 spike 未验证的事项

本 spike 的原型只证明「这条路线能跑通」，下面这些**在原型里是缺口或只有临时做法**，必须由 #29（Markdown 预览正式实现）落实并补证据。行号对应本分支当前的 `spikes/webview-gpui/src/main.rs`。

**（1）运行时联网：AGENTS.md 硬约束目前零证据。**
创建环境时 `CreateCoreWebView2EnvironmentWithOptions` 的 options 参数直接传 `None`（`spikes/webview-gpui/src/main.rs:232-241`），没有通过 `ICoreWebView2EnvironmentOptions` 的 `AdditionalBrowserArguments` 关掉 WebView2 自带的联网行为；`configure()` 里设置的只有脚本、DevTools、右键菜单、状态栏和浏览器快捷键（`src/main.rs:307-315`），**没有碰 `ICoreWebView2Settings8::IsReputationCheckingRequired`**，SmartScreen 信誉检查、组件/实验配置更新、崩溃上报等后台请求全部保持默认开启。原型里唯一的网络防线是 `WebResourceRequested`（`src/main.rs:348-351` 注册过滤器与处理器），**它只拦得住页面发起的资源请求，拦不住浏览器内核自身的后台连接**。
AGENTS.md §1 的硬约束「运行时不发起任何网络请求」因此**目前零证据**（§4.4 也已声明「尚无 OS 级网络跟踪」）。#29 必须：显式关闭 SmartScreen 与自动更新/实验配置拉取，并做一次 OS 级抓包（Fiddler/Wireshark/`pktmon`）在「打开预览 → 空闲 → 回收」全程确认无外发连接，把抓包结果作为验收证据。

**（2）空闲回收依赖渲染帧，产品按需重绘时超时永不到期。**
空闲超时判定写在 `render()` 里（`src/main.rs:576-589`：读 `dismissed_at`，`auto_flow::dispose_due()` 判定后调 `close_to_host()`），而原型之所以能持续判定，只是因为它每帧无条件 `window.request_animation_frame()` 加 `on_next_frame` 自唤醒（`src/main.rs:671-674`）——这是基准探针行为，不是产品行为。产品是按需重绘：预览隐藏、用户不操作时根本不出帧，**`webviewIdleTimeout` 永远不会到期，controller 会被无限期保留**。#29 必须把回收改成定时器驱动（Win32 `SetTimer` / GPUI 的定时任务），到期后再投递到 UI 线程执行 `Close()`，不得依赖帧回调。

**（3）隐藏保留没有挂起，保留期内存仍是预览开启量级。**
`dismiss()`（`src/main.rs:145-157`）只记录 `dismissed_at` 并把 Win32 焦点还给宿主，真正的隐藏是下一帧的 `SetIsVisible(false)`（`src/main.rs:633-639`）。**全程没有调用 `ICoreWebView2_3::TrySuspend()` / `Resume()`**，WebView2 的渲染进程仍在运行，所以保留期（默认 300 秒）内的内存仍是预览开启的量级——§1 引用的 324–341 MiB、§10 当前构建实测峰值 327–347 MiB 都属于这个量级；§1 里「回收后 49–52 MiB」来自 11-09 旧构建（当时 Close 连带销毁 environment），**已不适用于§15 的生命周期**，§1 已就地加注。#29 必须在隐藏时 `TrySuspend()`、重新显示时 `Resume()`，并分别测量「挂起保留」「超时回收后」两个稳态的内存，再定预览预算。

**（4）其余已知缺口（本 spike 只登记，不修）。**

| 缺口 | 位置 | #29 要做什么 |
|---|---|---|
| `close()` 在持有 `state.borrow_mut()` 期间调用 COM `remove_*` 与 `Close()` | `src/main.rs:91-116` | COM 回调可能在同一线程重入并再次 `borrow_mut()`，触发 `RefCell` panic。产品要先复制出 controller/webview/token 并释放借用，再调 COM（§9 对布局路径已这么改，关闭路径没改） |
| 导航/下载/权限拦截不全 | `src/main.rs:324-347`（`add_NavigationStarting` + `add_NewWindowRequested`） | 只拦顶层导航和新窗口，**没有 `add_FrameNavigationStarting`（子框架/iframe）、`add_DownloadStarting`（下载）、`add_PermissionRequested`（地理位置/剪贴板/通知等权限）**，这三类都要显式拒绝 |
| 没有显式 COM 初始化 | 全文无 `CoInitializeEx` | 原型隐含依赖「GPUI 的窗口线程已经是 STA」。产品不得依赖这个未写明的前提，要显式初始化并断言线程套间 |
| 没有设置 DPI 相关属性 | `configure()`，`src/main.rs:300-492` | 未设置 `ICoreWebView2Controller3::RasterizationScale` 与 `ShouldDetectMonitorScaleChanges`，跨屏/改缩放后的清晰度与命中区域依赖默认行为；150% 与跨屏改缩放要专测 |
| 重建时无条件显示 | `configure()` 末尾 `controller.SetIsVisible(true)`（`src/main.rs:492`） | 不看当前是否处于「弹出层相交隐藏」或「已 dismiss」状态就置为可见，异步创建完成的那一帧可能闪一下。产品应按当前可见性状态决定初值 |
| 测量设施混进时序 | `WH_GETMESSAGE` 线程钩子（`src/main.rs:717-724`，进程存活期间从不卸载）与每帧 JSON 日志 | 钩子和逐帧 stdout 写入都会改变 UI 线程时序，**本报告里的按键/Close 耗时都含这层干扰**；#29 的正式测量不要带这两样，或者做带/不带的对照 |

## 6. 替代方案与未完成项

若隐藏整块预览不可接受，可评估独立弹出 HWND：需要解决所有弹出层的激活、坐标、DPI、可访问性和销毁；或者 visual hosting：需要接入合成视觉树与输入转发。两者本次均未实现，不能据此宣称更好。

60 MB 不约束开启预览后的整个进程树，不能再据此要求放弃 WebView2。若后续确定的独立预览预算或视觉/焦点验收仍无法满足，再评估 pulldown-cmark 原生渲染；这需要重新决策 FEATURES D5，重新估算内嵌 HTML、Mermaid、数学公式能力与实现成本，不能无声删减功能。

暂缓（不阻塞本 PR）：关闭保留方案在Transparent125%物理短按下的人工复验（本机只有100%，自动注入见§15）。当前未完成：四类真实产品控件及完整Tab顺序；150%专门验证；真正无运行时环境；OS网络跟踪；当前静止空闲预算、外部冷启动首帧、300秒全释放与长期循环；独立预览预算数值。100%/125%重叠截图、两模式按键和混合DPI跨屏已有真实证据，见§14。

### 6.1 遗留到 #6 / #29 的事项（明确不在本 PR 完成）

issue #6 的验收标准里有两项**本 PR 不做、转入后续**，在这里逐条写明，避免当成已通过：

1. **冷/热启动与内存口径、静止空闲预算——未完成。** 本报告所有「首次 Render」都是进程内 `main`→首个 Render 回调，不是外部进程启动→显示器首帧（§2）；所有内存都来自持续 `request_animation_frame` 的探针场景（`benchmark_mode: continuous_frame_probe_not_idle`），**不是静止空闲测量**（§7、§10）。未开启预览的基础阅读器 60 MB 静止空闲预算、WebView2 预览的独立预算数值，都**尚未设定也未测量**。真正的外部首帧与预算判定由 #4 的产品基准（`cargo xtask bench` / `startup` / `perf/budgets.toml`）负责，预览侧口径由 #29 补。
2. **真机无 WebView2 运行时环境、300 秒全释放——未完成。** §4.5 只做了「browser folder 指向不存在目录」的故障注入（返回 `0x80070002`），**没有卸载系统运行时，不等于在真正未安装 WebView2 的机器上验收**；需要干净测试机，不要卸载日常系统的运行时。300 秒默认空闲超时的全链路释放也没有实测：§4.5 的 300 秒等待从未执行，§15 只用 `ROVE_WEBVIEW_IDLE_MS=1000` 验证了回收路径走得通，而且按 5.1 第（2）条，当前的帧驱动超时在产品的按需重绘下根本不会到期。两项都转入 #29，并需要 5.1 第（3）条的 `TrySuspend` 到位后重测内存。

此外，5.1 列出的全部缺口（运行时联网取证、定时器驱动回收、挂起/恢复、`close()` 重入、子框架/下载/权限拦截、COM 初始化、`RasterizationScale`、重建闪帧、测量设施干扰）同样不在本 PR 范围内，逐条交给 #29。

## 7. 构建环境、选型与许可证

2026-11-10 本机查询：Windows 11 Pro 10.0.22631；Intel Core i7-11700 @2.50GHz，8核16线程；物理内存51,247,124,480字节（约47.73GiB）；NVIDIA GTX1060 6GB（驱动27.21.14.6137）与Intel UHD750（32.0.101.7088），另有GameViewer虚拟显示驱动。虚拟显示存在不代表本次获得有效画面，DPI/截图验收仍待补齐。

没有实作或测量 wry 子webview方案。本次直接选择 webview2-com 是为了在现有GPUI HWND上精确控制COM事件、资源拦截和生命周期，不能宣称“wry试过但失败”或存在实测性能优劣。

构建必须使用 MSVC + Windows SDK，`where.exe fxc` 应成功；本机为 `D:\Windows Kits\10\bin\10.0.26100.0\x64\fxc.exe`。新的构建身份生成还需要Git在PATH中。CI应初始化VS开发者环境并检查这些前提；CI文件由F链负责，本PR未修改。

直接依赖版本与许可记录以本原型Cargo.lock及上游包为准，见下方依赖表。WebView2运行时是系统独立组件，不能把Rust绑定的MIT许可当作运行时许可。完整传递依赖许可审计仍属发布流程。

| 直接依赖 | 锁定版本 | 上游许可 |
|---|---|---|
| gpui | 0.2.2 | Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| serde_json | 1.0.154 | MIT OR Apache-2.0 |
| webview2-com | 0.38.2 | MIT |
| windows | 0.61.3 | MIT OR Apache-2.0 |

`img-src https:` 是有意的拦截测试夹具：让远程图片请求到达WebResourceRequested，验证403与计数，当前阻止远程图片依赖该拦截层，并非CSP本身禁止。产品应在净化/资源重写后收窄CSP，结合文档级放行机制；不直接照搬这个夹具策略。

SetBounds/SetIsVisible已改为值变化时调用，事件token按类型保存并在Close前取消订阅，异步创建和脚本参数由完成回调保活。持续请求帧和父窗口位置通知仍是探针行为；不能把这些结果称为静止空闲测量。

### 构建验证与官方依据

workspace 与原型 fmt、严格 clippy（`-D warnings`）通过；workspace test 与原型 release test 通过，但当前是零测试用例，不能替代运行证据。未执行 #2 E2E，因为 #2 尚未提供。

- [Windowed vs. Visual hosting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting)
- [ICoreWebView2：注入与消息 API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2)
- [ICoreWebView2Settings：脚本设置语义](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings)


## 8. 2026-11-10 评审复跑（50c93cb 历史构建自动验收子集）

构建源码 commit `50c93cbbdf0f70484cd5e22a25f9af83447de013`，源码 Git-blob 指纹 `90a03d84e611e495e61e145ee12431d10a87fb22`，构建时间 `2026-11-10T01:59:51+00:00`，dirty=true（构建输入已提交，spike 目录内 README、验收脚本和证据仍有未提交更新；dirty 只检查 spike 目录，不包含根 docs 报告）。可执行文件 SHA-256：`06951e78fcb5f8ccc83cf330d44860ef23d669fa07d8d52df19a3de419e2f65c`。日志及清单在 [`evidence/2026-11-10/`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10)（已从工作树删除，固定链接，下同；见§16）；上一轮保存在其 `earlier-eb3c41f/` 子目录。verify_run.py按当前构建输入重新计算源码指纹，拒绝不匹配证据；文档更新不影响该指纹。

`repeat-auto.ps1 -Runs 3` 使用同一二进制、每次全新profile；不是OS冷缓存试验，也没有重测六次内存表。三次均通过脚本/资源阻止、本地图片、主题、双向滚动、创建与重建的运行校验。

| 运行 | 首次Render ms | 总回复 | p99 ms | 最大 ms | >16ms 次数/比例 | key_route |
|---|---:|---:|---:|---:|---:|---:|
| auto-1 | 368.47 | 443 | 2.636 | 3.756 | 0 / 0.00% | 0 |
| auto-2 | 408.80 | 430 | 2.707 | 3.911 | 0 / 0.00% | 0 |
| auto-3 | 379.48 | 431 | 4.075 | 12.085 | 0 / 0.00% | 0 |

总计1304次回复、>16ms为0次；这是本批观测，不表示评审报告的8/405次慢回复已被消除，更不能归因为本次减少SetBounds调用（没有受控A/B）。逐类型统计在summary.json，包含pong、scrollApplied和themeApplied，不把混合回复称为ping-only。主题每轮仅4样本，不据此推导稳定尾延迟。

**三次首Render中1次超过400ms，不能宣布启动达标。** 该计时仍不是外部首帧；保留评审646ms及历史748/2548ms观测作为风险信息，不只展示低值。

本历史批次三次key_route全部为0：无人值守运行不提供物理按键证据；当时仍未获得有效重叠截图；后续已获得手测证据，见§14，PR保持Draft。

构建：原型fmt、严格clippy、release build/test通过（Rust目标0用例）；Python统计回归3/3；运行验收3/3。这三种证据分开报告。#5的4项UTF-16测试属于另一个PR，不混入此处。

## 9. 第二轮评审的数据合并与接入限制

各批分别列出，不能只选择最快的一批，也不把未提供原始日志的评审观测合入自有统计：

| 来源 | 首 Render ms | 超过400ms | 回复超过16ms |
|---|---|---|---|
| 作者 earlier-eb3c41f | 515 / 288 / 417 | 2/3 | 0/1366 |
| 作者 50c93cb 历史构建（§8） | 368 / 409 / 379 | 1/3 | 0/1304 |
| 第二轮评审，同HEAD两批 | 342 / 688 / 423 / 417 / 406 / 427 | 5/6 | 14/2586（0.54%） |

评审单轮最高慢回复比例3.37%，p99 59.7ms、max103.8ms。评审原始日志未随评论提供，以上按评论记录，不声称独立复核。首开超预算及长尾风险仍存在；通信协议可用于设计，首开/内存数据不宜直接用作产品预算依据。

第三轮修复前未重新测内存（最新补测见§10）：§4.5及摘要的内存仅是11-09历史记录。真正冷启动首帧、当前构建内存、视觉遮挡和物理按键均待测。

构建支持有Git检出的离线环境（依赖需预先缓存），但不支持缺Git或不含.git的源码tarball；这是证据原型的明确前提，不是产品分发方案。dirty仅覆盖spike目录。修改任一构建输入后必须重建、重新录制验收证据；旧数据仍可用summarize.py作历史统计，不能拿来通过当前校验。

第三轮修复已将布局参数读取与COM调用分离，调用期间不持有State借用。此前RefCell可变借用跨越COM调用的同步重入panic风险已修复。产品集成必须先复制调用参数、释放借用，再调用COM，成功后按controller/generation核对并更新缓存；本轮仅登记原型限制，不将其认定为产品可复用实现。

## 10. 第三轮修复与当前构建内存补测

基于main `5f6f780`，构建身份：`{"schema": 2, "commit": "b00ce0a5034ed294f921f0f9420b71858dd7603b", "source_git_blob": "3e9f59c12723e4642c7ecf715cae51abd83f9010", "dirty": "false", "built_unix_seconds": "1794280355", "benchmark_mode": "continuous_frame_probe_not_idle"}`。exe SHA-256随run.json保存，原始日志、内存采样和逐轮校验输出在 `spikes/webview-gpui/evidence/2026-11-10-review3/`（本批保留在工作树中）。

| 运行 | 首Render ms | 峰值工作集 MiB | 峰值私有提交 MiB | >16ms/回复 | 运行校验 |
|---|---|---:|---:|---:|---|
| fresh-profile-1 | [426.76279999999997] | 346.8 | 262.3 | 4/393 | PASS |
| fresh-profile-2 | [369.3961] | 331.3 | 244.8 | 4/81 | FAIL |
| fresh-profile-3 | [599.7845000000001] | 339.9 | 245.2 | 0/29 | FAIL |
| reused-profile-1 | [343.274] | 327.3 | 254.0 | 0/418 | PASS |
| reused-profile-2 | [348.48639999999995] | 333.3 | 253.9 | 0/448 | PASS |
| reused-profile-3 | [366.0081] | 329.8 | 256.4 | 0/432 | PASS |

严格校验4/6通过；失败轮次未完成两次ready，不能标记生命周期通过。采样工具增加负载的可能性存在，但未做受控A/B，不能归因；失败原始日志全部保留。峰值为全运行采样最大值，忽略进程退出后的空值，不是静止空闲预算或Close后稳定值。此为全新/复用profile测试，不是真正OS冷启动外部首帧。key_route仍为0，视觉/焦点阻断不变。

COM布局调用期间不再持有State借用；验收断言已改为明确错误提示。两分支workspace fmt/clippy及43项测试通过；IME另4项通过。WebView原型fmt/clippy/release build/test通过（Rust目标0用例），Python3项通过。Git checkout仍为证据构建前提，无Git/tarball支持未实现；离线缓存构建为预期、未实测。

## 11. 重建未ready的根因与修复

§10失败批次的自动脚本按进程启动绝对时间推进：第13秒创建第二个实例，第16秒退出，留给重建的时间最多约3秒；不检查ready，并且长帧可能直接跳过阶段。fresh-profile-2首次ready已达6254ms，第二次controller创建耗时1839ms，随后在ready前被关闭；fresh-profile-3第二次连controller完成日志都没来得及出现。这证明原测试流程能截断仍在进行的初始化，不是“已完成但没记录”。不能由此断言初始化本身已优化或慢启动已消失。

修复改为逐阶段状态机，每次创建等待ready后才开始观察计时，重建ready后至少观察3秒才退出；每次ready等待上限20秒，超时记录error并失败。外层90秒只作失控进程保护，不是性能预算。阶段日志与严格校验要求1–8顺序完整、两次ready后推进及最后观察期完整。新增3个纯逻辑回归覆盖慢ready、长帧不跳步、重建观察时间。

修复后以同一内存采样脚本复跑3次全新profile、3次复用profile，严格运行校验 **6/6通过**。身份：`{"schema": 2, "commit": "746373c91835e12543b73c60a6b18108664a7749", "source_git_blob": "25c8656b6f9a53bea0376b59e43b314ed7c23a45", "dirty": "false", "built_unix_seconds": "1794280960", "benchmark_mode": "continuous_frame_probe_not_idle"}`。日志、exe哈希、内存采样及校验输出见 [`evidence/2026-11-10-ready-fix/`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-ready-fix)（已删除，固定链接）。

| 运行 | 第一次ready ms | 重建ready ms | 重建ready后观察 ms | 校验 |
|---|---:|---:|---:|---|
| fresh-profile-1 | 2523.46 | 2437.61 | 3008.52 | PASS |
| fresh-profile-2 | 2536.60 | 2399.92 | 3000.47 | PASS |
| fresh-profile-3 | 2478.71 | 2516.79 | 3003.37 | PASS |
| reused-profile-1 | 2442.35 | 2463.94 | 3000.62 | PASS |
| reused-profile-2 | 2484.91 | 2522.78 | 3000.46 | PASS |
| reused-profile-3 | 2595.43 | 2411.46 | 3007.49 | PASS |

两次ready、完整阶段序列、双向通信、本地图片、脚本/资源拦截及两次close均通过。原失败批次仍保留，不以新结果覆盖。3项阶段回归与3项Python统计测试通过；workspace fmt/clippy/test通过（43项，2个环境相关doc-test忽略）；原型fmt/严格clippy/release build/test通过。未宣称性能变快，也不解除视觉/按键及真正冷启动的待验项。

## 12. 人工反馈后的可见性与Tab修复

源码确认GPUI DirectComposition目标topmost=true，但实际画面仍需复测。新增launch-review.ps1三模式入口及run_start模式日志，移除ready后无条件绘制的右侧占位。Tab/Shift+Tab改由可信桥接监听真实keydown并preventDefault，经已有来源/version检查返回宿主；宿主Tab用MoveFocus NEXT/PREVIOUS进入网页。合成keydown不作为物理验收证据，原型仍为整页退出策略，产品需实现焦点边界顺序。

新增手测必须覆盖：三模式下O打开能看到网页；U原始弹出层与H隐藏对照，各截图；网页与宿主双向Tab/Shift+Tab；100%/125%、跨屏、主题、三轮销毁重建。旧KEY通过项记录保留，但模式/代码改变后的结果不能自动继承。WV-1～8、KEY-5/9仍待真实视觉与按键复验。

焦点方向依据：[ICoreWebView2Controller::MoveFocus](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2controller)，NEXT进入首元素、PREVIOUS进入末元素。

本轮自动回归：同一新构建分别在Baseline/Hwnd/Transparent模式运行，3/3严格校验通过。证据在 [`evidence/2026-11-10-composition/`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-composition)（已删除，固定链接），各有模式字段、构建指纹、exe哈希及汇总。三模式均未操作物理按键，不能算KEY-5/9或可见性通过。桥接逻辑测试覆盖Tab/Shift+Tab以及非Tab/不可信事件拒绝；3项Rust阶段回归、3项Python统计回归通过；workspace fmt/clippy/test和原型严格clippy/release build/test通过。

合入#2后再次rebase至main `d645fac`，workspace fmt/严格clippy与56项测试通过（2个环境相关doc-test忽略）。rebase换行规范化后严格指纹检查拒绝先前三模式证据，已重新构建并重录3/3通过，当时证据位于 [`evidence/2026-11-10-composition-final/`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-composition-final)（已删除，固定链接）。旧composition目录仅为历史记录。

## 13. 前轮三模式真实手测与快速重开修复（最新状态见§14）

[固定提交手测证据](https://github.com/Zhao-wl/Rove/tree/3e46ab2aadd555ee4d3cef39e09c8dcec6a5a06e/pr-81)对应构建a9955d6，exe SHA-256 2e31be1e630e05223aa086b0c10c34ae0a308038a71c91a1a6514bad906e7e2b。Windows 11 23H2、100%/125%、双1920×1080。以下是评审真实手测，不是本代理自动运行所得：

- Baseline正文不可见，判定不适合作为集成模式，保留为负对照。
- Hwnd正文可见；原始弹出矩形被网页覆盖，相交隐藏/恢复有效；尺寸、125%、混合DPI双屏、主题通过。快速x→o约200ms出现一次无第二controller；等待约半秒的复测3/3通过，不抹除首轮失败。
- Transparent正文和完整GPUI弹出矩形可见，但加载/隐藏透出后方窗口；跨屏未测。
- KEY-1～7两档通过，KEY-8/9在Hwnd/Transparent通过，Baseline存在视觉限制。Tab路由修复已获得物理按键证据。

因此建议工作台优先以Hwnd+相交隐藏作为**已获手测支持的候选**；四类产品控件、完整Tab顺序、300秒回收和产品骨架整体验收仍未测，不能宣称产品集成完成。Transparent保留为待修复复验的候选，不以其原始叠放优点掩盖透底。

本轮修复：Transparent只在ready且可见时留透明区域，其余阶段根背景不透明，防止加载/关闭/隐藏透底。纯逻辑覆盖4种组合，画面仍须重新截图。

快速重开原始日志包含第二次environment，没有controller；这排除了“o完全没有被处理”的猜测，但不足以唯一证明底层WebView2挂起原因。原实现每次重新创建环境，本轮改为一个窗口持有一个environment，Close仅销毁controller，重建复用environment；记录open_requested、controller_requested/completed、environment_reused及host_key以区分请求丢失、异步失败和回调缺失。auto关闭→重开间隔改为200ms，每次仍等待ready并保留观察期，不靠额外睡眠掩盖问题。

环境现随窗口存活，Close控制器不等于释放全部浏览器环境，因此历史Close内存回落值不能用于这一版本；产品的300秒全释放需额外生命周期设计。主线接口未改。

本轮构建身份：`{"schema": 2, "commit": "001565549cf21c96def176e50aac7f9c040590b6", "source_git_blob": "e8b3a9e61b9b96838411479d089e2ff8ce6fa42b", "dirty": "false", "built_unix_seconds": "1794295050", "benchmark_mode": "continuous_frame_probe_not_idle", "disable_direct_composition": false, "transparent_preview": true}`。Hwnd/Transparent各3轮快速重开，严格校验6/6通过；原始事件、exe哈希与统计在 [`evidence/2026-11-10-rapid-reopen/`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-rapid-reopen)（已删除，固定链接）。

| 运行 | close→open阶段间隔 ms | 重建ready ms | 校验 |
|---|---:|---:|---|
| Hwnd/auto-1 | 200.93 | 2527.61 | PASS |
| Hwnd/auto-2 | 201.33 | 2513.32 | PASS |
| Hwnd/auto-3 | 201.05 | 2660.75 | PASS |
| Transparent/auto-1 | 210.11 | 2658.47 | PASS |
| Transparent/auto-2 | 209.69 | 2670.67 | PASS |
| Transparent/auto-3 | 200.46 | 2612.67 | PASS |

每轮1次environment创建、1次复用、2次controller请求/完成、2次ready/close均通过；自动测试覆盖200ms调度与消息，不替代125%下调整尺寸后快速按键的完整原始手测。5项Rust阶段/背景回归、3项Python统计及桥接逻辑测试通过；workspace fmt/clippy/test通过（56项，2个环境相关doc-test忽略）。Transparent透底修复仍待新版截图，不把背景布尔测试当作视觉验收。

## 14. 最新复验：区分重建与输入未到达

[固定复验证据](https://github.com/Zhao-wl/Rove/tree/441136a0763b3866eb000ca2411ce126f9561337/retest-2026-11-10/pr-81)，构建0015655，exe ef9a9b8159817e914a90db4452b16ef60d8cca35d1cf6158852293874e0abda1。

Transparent四态不透底在100%/125%通过；两模式混合DPI、主题与KEY-1～9通过。Hwnd125%三次200ms重开通过，两模式100%各3/3通过。Transparent125%首轮约200.25ms间隔没有host_key=o；重测约200.15–200.18ms、按键保持28/32ms时3/3通过。因此这次失败不是已经进入environment/controller的重建失败，不能与旧失败混为一谈；短按注入、原生焦点等因素尚未被唯一归因。

检查发现x关闭后原代码仅恢复GPUI FocusHandle，未显式恢复Win32焦点。本轮在同一路径补SetFocus(host HWND)，立即用GetFocus读取实际目标并记录host_focus_restored；host_key也携带native_focus与host HWND。自动关闭重建复用同一函数并校验焦点确实归宿主。无延迟抢焦点、无补发o、无改变按键持续时间，不把防御性修复声称为唯一根因已证实。

复验重点：Transparent125%尺寸/浅色主题后，沿用首轮短按持续时间与200ms间隔，检查closed后的host_focus_restored.ok、后续host_key=o及重建日志。若o仍未出现，保留Windows输入发送时刻与持续时间，不能将该次标记为WebView controller失败。此前通过的手测结果保留；产品弹出控件、完整Tab遍历、300秒全释放、真实冷启动/预算/无运行时环境仍未验。

本轮构建：`{"schema": 2, "commit": "2f427642a2b4dc9686344eb34893ea0c5a37bf2f", "source_git_blob": "8d398451b1b76833cdc6592866c4c221917af179", "dirty": "true", "built_unix_seconds": "1794298329", "benchmark_mode": "continuous_frame_probe_not_idle", "disable_direct_composition": false, "transparent_preview": true}`；原生焦点断言与完整生命周期运行 **6/6通过**，证据在 [`evidence/2026-11-10-native-focus/`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-native-focus)（已删除，固定链接），各轮包含exe SHA-256。

| 运行 | close→open间隔 ms | 重建ready ms | GetFocus=host | 校验 |
|---|---:|---:|---|---|
| Hwnd/auto-1 | 207.48 | 2532.77 | true | PASS |
| Hwnd/auto-2 | 200.78 | 2539.44 | true | PASS |
| Hwnd/auto-3 | 208.16 | 2551.18 | true | PASS |
| Transparent/auto-1 | 200.97 | 2547.57 | true | PASS |
| Transparent/auto-2 | 201.50 | 2543.55 | true | PASS |
| Transparent/auto-3 | 200.02 | 2517.56 | true | PASS |

main e3a93c9下workspace fmt/严格clippy/56项测试通过（2项环境相关doc-test忽略）；原型5项测试、Python3项统计回归及桥接逻辑测试通过。这里的自动调用不模拟物理短按，不宣称原始Transparent125%个例已经完全排除；下一轮按清单保留原始按键持续时间复验。

## 15. 关闭改为“隐藏保留＋延迟回收”（f7e3cf6 之后）

**丢键诊断。** 原型新增线程级 `WH_GETMESSAGE` 钩子，记录 UI 线程实际取出的每条 `WM_KEYDOWN/UP`（窗口、焦点、前台、是否处于宿主按键回调内），所有日志带进程内时间戳，`closed` 记录 `Close()` 耗时。用 `shortpress.ps1` 在本机（Windows 11，双 1920×1080，**仅 100%**）以 SendInput 注入 `x`→200 ms→`o`，按住 1.6–8 ms，每次注入前校验前台窗口：旧逻辑 Transparent 共 38 次（含 7 次先调整尺寸、切浅色），`o` 全部被 UI 线程取到并进入宿主，未复现丢键；嵌套取键 0 次，前台不一致 0 次。125% 下的个例仍未复现，**根因未唯一确定**。

**确定的问题。** 同一批数据显示 `x` 回调内同步 `Controller.Close()` 耗时 28.7–459.0 ms（30 次，中位 47.6 ms），最长一次超过 200 ms 重开间隔，期间 UI 线程完全阻塞，`o` 只能排队。无论 125% 个例是否由此引起，在输入回调里同步销毁 WebView 都不应进入产品。

**修复。** `x` 只隐藏并保留 controller（`dismissed`），立刻把 Win32 焦点还给宿主；`o` 在空闲窗口内直接重新显示（`reshown`），不重建；`ROVE_WEBVIEW_IDLE_MS`（默认 300000，对应 `rove.markdown.webviewIdleTimeout`）到期或按 `d` 时，才在下一次渲染帧里 `Close()`。自动生命周期探针仍走真实 Close→重建。

**验证**（证据：`spikes/webview-gpui/evidence/2026-11-10-retain-close/`，本批保留在工作树中，事件日志已去掉 roundtrip/local_resource 噪声）：

| 场景 | 结果 |
|---|---|
| 旧逻辑 Transparent，短按 x→200 ms→o（t2） | 30/30 进入宿主；Close 28.7–459.0 ms |
| 新逻辑 Transparent，先调整尺寸＋浅色（t4） | 30/30 `reshown`，无 Close、无重建 |
| 新逻辑 Hwnd，先调整尺寸＋浅色（t6） | 15/15 `reshown` |
| 空闲 1 s 后回收再打开（t5，`ROVE_WEBVIEW_IDLE_MS=1000`，间隔 1.5 s） | 8/8 `dispose`→`closed`→重建→`ready`；帧内 Close 17–32 ms |
| `repeat-auto.ps1 -Runs 3` | 3/3 lifecycle 校验通过 |
| 截图 | 关闭态不透底并提示保留；重新显示 300 ms 后正文可见 |

原型 7 项 Rust 测试（新增复用/延迟回收判定 2 项）、`clippy -D warnings` 通过。自动注入不等于物理按键；125% 物理短按复验**暂缓**，不作为本 PR 的阻塞项，需要时按 MANUAL_ACCEPTANCE“关闭保留与延迟回收”执行。

## 16. 证据批次的保留与历史位置

本 PR 合入前精简了 `spikes/webview-gpui/evidence/`：**工作树中只保留两批**，其余 7 批已用 `git rm -r` 删除。删除只影响工作树，不影响历史——所有被删批次都完整存在于提交 `ab7d088` 的树里，下表是固定链接（permalink），点开即可看到当时的原始日志、`run.json`、exe SHA-256 与校验输出。

保留在工作树中的两批：

| 批次 | 对应章节 | 为什么保留 |
|---|---|---|
| `evidence/2026-11-10-retain-close/` | §15 | 本报告的**最新结论**（隐藏保留＋延迟回收、Close 28.7–459.0 ms 诊断、30/30 与 15/15 `reshown`、1 s 空闲回收 8/8）全部依赖这批 |
| `evidence/2026-11-10-review3/` | §10 | 当前构建口径下**唯一**的内存采样（峰值工作集 327–347 MiB），§1 的内存注记与 5.1 第（3）条引用它 |

已删除、改为固定链接的批次：

| 批次（固定链接） | 对应章节 | 内容 |
|---|---|---|
| [`2026-11-09`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-09) | §3、§4.3、§4.4、§4.5 | 11-09 旧构建六次实测、`final-communication.jsonl`、`global-script-disabled.jsonl`、`acceptance.jsonl`、`missing-runtime.jsonl` |
| [`2026-11-10`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10) | §8 | 50c93cb 历史构建的评审复跑（含 `earlier-eb3c41f/` 子目录） |
| [`2026-11-10-ready-fix`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-ready-fix) | §11 | 重建未 ready 修复后的 6/6 复跑 |
| [`2026-11-10-composition`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-composition) | §12 | 三模式（Baseline/Hwnd/Transparent）首批自动回归 |
| [`2026-11-10-composition-final`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-composition-final) | §12 | rebase 到 d645fac 后重录的三模式回归 |
| [`2026-11-10-rapid-reopen`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-rapid-reopen) | §13 | 200ms 快速重开、environment 复用 |
| [`2026-11-10-native-focus`](https://github.com/Zhao-wl/Rove/tree/ab7d0888660308ddbb0350891ae4c7d77ea54566/spikes/webview-gpui/evidence/2026-11-10-native-focus) | §14 | 原生焦点恢复断言与完整生命周期 6/6 |

说明：
- 这些历史批次的构建指纹与当前源码不一致，`verify_run.py` 本来就会以 `SOURCE MISMATCH` 或 `LEGACY/UNVERSIONED` 拒绝它们（§8、§9），只能用 `summarize.py` 作历史统计；删除它们不会让任何现行校验失去输入。
- `verify_run.py`、`summarize.py`、`evidence_stats.py`、`repeat-auto.ps1` 都按命令行参数接收日志路径，**没有硬编码任何批次目录**，因此本次删除不需要改脚本。
- 后续若要重新引用某批历史数据，用上表的固定链接，不要再往工作树里复制回来。
