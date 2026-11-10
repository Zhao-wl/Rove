# Rove M0 WebView2 嵌入验证报告

> Issue #6 · 实测：2026-11-09；预算口径确认：2026-11-10 · 部分验收完成，PR 保持 Draft。
> 原型与原始证据：`spikes/webview-gpui/`、`evidence/2026-11-09/`。

## 1. 结论摘要

**当前集成判定：NO-GO（验收未齐，不等于否决 D5）。**

前一版本化复跑见第8节；最新修复构建见第10节。前一批：作者最终批次3次首Render有1次超过400ms；评审同HEAD复跑6次有5次超过400ms，物理按键与视觉仍未验收。

已实测：GPUI HWND 中异步创建 WebView2；虚拟域名页面加载；远程图片拦截；文档脚本禁止而可信桥接可用；主题/滚动消息往返；销毁后内存回落及重新创建；不存在的运行时目录可检测失败。

尚不能解除 #16 的依赖：**缺少有效的重叠截图、真实键盘焦点路由与高 DPI 实测**。当前宿主无法把原型可靠置前，屏幕裁剪得到黑屏；PrintWindow 又没有包含 WebView2 子 HWND，均已排除，不能冒充视觉通过。

**预算口径已由用户于 2026-11-10 确认：60 MB 只约束未开启预览的基础阅读器，WebView2 预览单独列预算。** 11-09旧构建开启预览后的历史进程树工作集实测为 **324–341 MiB**，私有提交 **240–250 MiB**；这些是观测值，不是已批准的预览预算上限，不再作为违反基础阅读器 60 MB 预算的理由。预览的具体预算数值尚未设定。当前 NO-GO 来自验收未齐，不再包含“60 MB 适用范围待确认”这一阻塞。

| 状态 | 预算与记录口径 | 当前证据边界 |
|---|---|---|
| 未开启预览的基础阅读器 | 保留 60 MB 约束 | 本原型持续刷新，不能代替产品静止空闲验收 |
| WebView2 预览开启 | 单列预算；分别记录宿主、WebView2 子进程及合计的工作集/私有提交 | 现有 324–341 MiB 是活动原型进程树合计，含共享页重复计数可能性；具体上限未设定 |
| 预览全部关闭且完成回收 | 单独验证资源释放和基础阅读器内存恢复 | 原型回落至 49–52 MiB 仅为当前运行证据，不是产品静止空闲预算已通过 |

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

这些日志来自开发过程中的早期二进制，未记录 commit/构建身份；缺少当前源码必写的 `localImageLoaded`/`kind` 字段，不能作为当前源码的验收证据，也不能用当前 `verify_run.py` 当作同一 schema 检查。原始文件保留不补字段。`final-communication.jsonl` 有 kind、缺 localImageLoaded；旧 acceptance.jsonl 虽有两者，仍缺构建身份。下表仅保留历史观察，前一版本化复跑见第8节；最新修复构建见第10节。前一批。

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

原型右半区为 windowed WebView2，GPUI 弹出矩形跨越左右区。无遮挡规避阶段和隐藏规避阶段均运行，SetIsVisible 调用路径正常。**没有有效视觉证据，因此遮挡规避未通过验收。**

PrintWindow 截图只显示 GPUI 内容，缺失子 HWND，不能用来证明原生层压在 WebView2 上方。屏幕裁剪黑屏也已排除，未纳入 evidence。

推荐的最小验证方案仍是隐藏整个预览：工作台保留预览占位；弹出层先登记相交区域，再隐藏 webview，然后呈现原生层；结束后恢复。隐藏期间不能把输入继续送到已隐藏的 WebView2。通知、菜单、悬停与命令面板全部要走同一管理器。相交判断、焦点恢复需后续集成测试，原型只用一个共用矩形代替四类控件。

### 4.2 焦点与按键

下表是已实现的路由意图，**尚未进行真实按键验收**，不代表产品命令已经接通。

| 按键/入口 | 原型处理者 | 集成要求 |
|---|---|---|
| Ctrl+P | quickOpen，标记 Handled | 异步交给 Rove 命令分发 |
| Ctrl+Shift+P | commandPalette | 同上 |
| Ctrl+Shift+V | togglePreview | 登记命令负责切换 |
| Esc | 返回宿主、关闭弹出层 | 依上下文处理 |
| Tab / Shift+Tab | 返回宿主 | 产品应实现边界 Tab 顺序；原型粗粒度退出不能直接照搬 |
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

## 5. 集成设计（有条件建议）

1. GPUI 拥有顶层 HWND；每窗口的预览管理器拥有一个 controller。原型不修改 GPUI 或共享契约。
2. 第一次打开预览时异步创建，先显示骨架；不要嵌套消息泵阻塞 UI。关闭或取消时递增 generation，迟到回调立即 Close，不挂回失效窗口。
3. 按布局提交客户区物理 bounds；GPUI 逻辑坐标乘当前 scale，跨屏后更新。当前原型从 GetClientRect 获取物理尺寸，但**高 DPI/混合显示器仍未实测**。
4. 所有弹出层统一提供区域与可见周期，按相交决定隐藏/恢复；焦点策略与 Tab 边界由宿主明确负责。视觉通过前不能解除 #16 的依赖。
5. 文档切换复用实例；全部预览关闭后按设置延时回收，窗口关闭立即回收。新文档消息必须携带 generation/documentId。
6. 接口缺口走单独契约 PR，本次没有擅自修改 #1 的接口。

## 6. 替代方案与未完成项

若隐藏整块预览不可接受，可评估独立弹出 HWND：需要解决所有弹出层的激活、坐标、DPI、可访问性和销毁；或者 visual hosting：需要接入合成视觉树与输入转发。两者本次均未实现，不能据此宣称更好。

60 MB 不约束开启预览后的整个进程树，不能再据此要求放弃 WebView2。若后续确定的独立预览预算或视觉/焦点验收仍无法满足，再评估 pulldown-cmark 原生渲染；这需要重新决策 FEATURES D5，重新估算内嵌 HTML、Mermaid、数学公式能力与实现成本，不能无声删减功能。

未完成：有效重叠截图/录屏；四类真实弹出控件逐项验收；键盘与鼠标焦点实测；125%/150% 和跨屏；真正无运行时环境；OS 网络拦截；静止空闲预算；外部首帧时序；300 秒回收与长期循环；独立预览预算具体数值的制定（适用范围已确认）。

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

构建源码 commit `50c93cbbdf0f70484cd5e22a25f9af83447de013`，源码 Git-blob 指纹 `90a03d84e611e495e61e145ee12431d10a87fb22`，构建时间 `2026-11-10T01:59:51+00:00`，dirty=true（构建输入已提交，spike 目录内 README、验收脚本和证据仍有未提交更新；dirty 只检查 spike 目录，不包含根 docs 报告）。可执行文件 SHA-256：`06951e78fcb5f8ccc83cf330d44860ef23d669fa07d8d52df19a3de419e2f65c`。日志及清单在 `spikes/webview-gpui/evidence/2026-11-10/`；上一轮保存在其 `earlier-eb3c41f/` 子目录。verify_run.py按当前构建输入重新计算源码指纹，拒绝不匹配证据；文档更新不影响该指纹。

`repeat-auto.ps1 -Runs 3` 使用同一二进制、每次全新profile；不是OS冷缓存试验，也没有重测六次内存表。三次均通过脚本/资源阻止、本地图片、主题、双向滚动、创建与重建的运行校验。

| 运行 | 首次Render ms | 总回复 | p99 ms | 最大 ms | >16ms 次数/比例 | key_route |
|---|---:|---:|---:|---:|---:|---:|
| auto-1 | 368.47 | 443 | 2.636 | 3.756 | 0 / 0.00% | 0 |
| auto-2 | 408.80 | 430 | 2.707 | 3.911 | 0 / 0.00% | 0 |
| auto-3 | 379.48 | 431 | 4.075 | 12.085 | 0 / 0.00% | 0 |

总计1304次回复、>16ms为0次；这是本批观测，不表示评审报告的8/405次慢回复已被消除，更不能归因为本次减少SetBounds调用（没有受控A/B）。逐类型统计在summary.json，包含pong、scrollApplied和themeApplied，不把混合回复称为ping-only。主题每轮仅4样本，不据此推导稳定尾延迟。

**三次首Render中1次超过400ms，不能宣布启动达标。** 该计时仍不是外部首帧；保留评审646ms及历史748/2548ms观测作为风险信息，不只展示低值。

三次key_route全部为0：无人值守运行不提供物理按键证据；本次仍未获得有效重叠截图，遮挡/焦点阻塞未解除，PR保持Draft。

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

基于main `5f6f780`，构建身份：`{"schema": 2, "commit": "b00ce0a5034ed294f921f0f9420b71858dd7603b", "source_git_blob": "3e9f59c12723e4642c7ecf715cae51abd83f9010", "dirty": "false", "built_unix_seconds": "1794280355", "benchmark_mode": "continuous_frame_probe_not_idle"}`。exe SHA-256随run.json保存，原始日志、内存采样和逐轮校验输出在 `spikes/webview-gpui/evidence/2026-11-10-review3/`。

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

修复后以同一内存采样脚本复跑3次全新profile、3次复用profile，严格运行校验 **6/6通过**。身份：`{"schema": 2, "commit": "746373c91835e12543b73c60a6b18108664a7749", "source_git_blob": "25c8656b6f9a53bea0376b59e43b314ed7c23a45", "dirty": "false", "built_unix_seconds": "1794280960", "benchmark_mode": "continuous_frame_probe_not_idle"}`。日志、exe哈希、内存采样及校验输出见 `evidence/2026-11-10-ready-fix/`。

| 运行 | 第一次ready ms | 重建ready ms | 重建ready后观察 ms | 校验 |
|---|---:|---:|---:|---|
| fresh-profile-1 | 2523.46 | 2437.61 | 3008.52 | PASS |
| fresh-profile-2 | 2536.60 | 2399.92 | 3000.47 | PASS |
| fresh-profile-3 | 2478.71 | 2516.79 | 3003.37 | PASS |
| reused-profile-1 | 2442.35 | 2463.94 | 3000.62 | PASS |
| reused-profile-2 | 2484.91 | 2522.78 | 3000.46 | PASS |
| reused-profile-3 | 2595.43 | 2411.46 | 3007.49 | PASS |

两次ready、完整阶段序列、双向通信、本地图片、脚本/资源拦截及两次close均通过。原失败批次仍保留，不以新结果覆盖。3项阶段回归与3项Python统计测试通过；workspace fmt/clippy/test通过（43项，2个环境相关doc-test忽略）；原型fmt/严格clippy/release build/test通过。未宣称性能变快，也不解除视觉/按键及真正冷启动的待验项。
