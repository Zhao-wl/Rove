# Rove M0 WebView2 嵌入验证报告

> Issue #6 · 2026-11-09 · 部分验收完成，PR 保持 Draft。
> 原型与原始证据：`spikes/webview-gpui/`、`evidence/2026-11-09/`。

## 1. 结论摘要

**当前集成判定：NO-GO（验收未齐，不等于否决 D5）。**

已实测：GPUI HWND 中异步创建 WebView2；虚拟域名页面加载；远程图片拦截；文档脚本禁止而可信桥接可用；主题/滚动消息往返；销毁后内存回落及重新创建；不存在的运行时目录可检测失败。

尚不能解除 #16 的依赖：**缺少有效的重叠截图、真实键盘焦点路由与高 DPI 实测**。当前宿主无法把原型可靠置前，屏幕裁剪得到黑屏；PrintWindow 又没有包含 WebView2 子 HWND，均已排除，不能冒充视觉通过。

内存是明确的产品约束：开启预览后的进程树工作集为 **324–341 MiB**，私有提交 **240–250 MiB**。若 60 MB 是开启预览后整个应用的硬预算，本路线 no-go；若只约束未开启预览的基础阅读器，应单列预览预算。此口径已提交用户确认，未自行放宽。

对 #16 / #18 / #24：优先待验方案是“弹出层与预览相交时隐藏 WebView2，GPUI 显示占位，关闭后恢复”。它不强制所有弹出层改成独立顶层 HWND，但要求统一登记弹出区域、可见周期和焦点恢复目标。**在视觉测试通过前，仅为建议，不能当作已验证结论。**

## 2. 方法与环境

- GPUI `=0.2.2`，`default-features=false`，`windows-manifest` 显式开启。直接使用 `webview2-com =0.38.2`，与 GPUI 共用 windows 0.61；相比引入额外窗口框架，可直接控制现有 HWND、COM 事件与 controller 生命周期。
- Rust/MSVC 1.97.1，release + thin LTO + 单 codegen unit。WebView2 运行时文件版本实测为 **157.0.4341.42**。
- `spikes/webview-gpui` 独立 manifest/lock，在 workspace 外；不改 `crates/` 契约，也不依赖 #5 的 vendor 补丁。
- `measure.ps1`：全新 profile 三次、复用 profile 三次。**不是重启/清空 OS 页缓存意义的冷启动**。每次约 16 秒，1 秒后按需创建、5 秒重叠、8 秒隐藏规避、11 秒销毁、13 秒重建。
- 每约 0.7–0.9 秒枚举本原型的后代进程并求和工作集/私有提交。工作集可能重复统计共享页；包括原型启动产生的辅助进程，不把其他应用的 WebView2 混入。
- 通信探针每 ≥16ms 发送一条，滚动事件以 requestAnimationFrame 合并。该场景持续刷新，**不是静止空闲测量**。
- `first_render` 是进程内 main→首次 Render 回调，不是外部进程启动→显示器首帧。controller 与 ready 时间从开始创建计时，彼此不能混用。

## 3. 六次实测数据

每行取该进程第一次创建；同进程第二次创建另见回收项。内存取 4–10 秒样本中位数。

| Profile | 首次 Render ms | Controller ms | 页面 ready ms | 进程树工作集 MiB | 私有提交 MiB | 最多进程数 |
|---|---:|---:|---:|---:|---:|---:|
| 全新 1 | 318.85 | 280.46 | 2442.44 | 340.57 | 245.96 | 8 |
| 全新 2 | 266.15 | 281.94 | 2418.06 | 340.79 | 250.03 | 8 |
| 全新 3 | 354.36 | 314.32 | 2486.95 | 341.18 | 248.73 | 8 |
| 复用 1 | 354.76 | 366.32 | 2553.04 | 326.15 | 240.38 | 8 |
| 复用 2 | 304.43 | 252.64 | 2386.72 | 326.38 | 242.30 | 8 |
| 复用 3 | 297.27 | 282.66 | 2444.86 | 323.92 | 241.45 | 8 |

这六次首次 Render 均 <400ms，且 WebView2 到 1 秒才创建，证明原型采用了按需创建路径；**不能据此保证产品冷启动 <400ms**。其他烟雾运行出现过 748ms、2548ms 的首次 Render，宿主存在明显调度波动；最终应由 #4 用产品基准测量。

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

六次性能试验各收到 261–271 个 ping 回复；各次 p50 为 **0.776–0.944ms**，p99 为 **1.733–2.342ms**。

另一次包含真实样式变更和滚动的运行（`final-communication.jsonl`）：

| 回复 | 样本数 | p50 ms | p99 ms |
|---|---:|---:|---:|
| pong | 176 | 0.703 | 1.706 |
| scrollApplied | 86 | 0.950 | 2.157 |
| themeApplied | 2 | 1.037 | 样本不足，不报告分位数 |

同时收到 55 条页面 scroll 事件；themeApplied 样本仅用于功能及单次延迟观察，不作稳定性结论。主题变更没有创建新 controller。滚动的布局误差与长文档源码行映射属于 #29，未在本 spike 实现。

在此负载下约 60Hz 的小 JSON 通信可行；**不是证明频率上限**，也没有验证 120Hz 或满负载。推荐同帧合并、只保留最新滚动目标、不累计队列。页面回送应带 source/seq，避免双向回声循环。

协议草案：`{version:1,type,seq,...payload}`。宿主→页面：ping、theme、scroll；页面→宿主：ready、pong、themeApplied、scrollApplied、scroll。原型检查来源、版本和 64KiB 大小，超时清理未应答探针；产品还需 documentId/generation、字段 schema、过期文档丢弃和映射版本。

### 4.4 文档脚本与本地资源

用户明确的语义是：**禁止文档及外来脚本，允许 Rove 打包的可信桥接/渲染脚本**。

实测差异：`IsScriptEnabled(false)` 时，AddScriptToExecuteOnDocumentCreated 的直接语句与 ExecuteScript 可执行，但 DOMContentLoaded / 消息事件桥接没有形成持续 pong；日志在 `global-script-disabled.jsonl`。不能只凭 API 调用成功宣布方案可用。

最终原型使用页面 CSP `script-src 'none'` 拒绝文档脚本，宿主通过 ExecuteScript 注入打包桥接；早期注入仅作探针。六次运行、每次两次创建，文档 script 探针全部保持未执行，可信桥接持续应答。生产仍必须在拼接 HTML 前白名单净化；CSP 不能替代净化，不能允许文档覆盖 CSP 或注入可信脚本。

资源目录映射到 `https://rove.local/`，DENY_CORS；非本地域资源请求返回 403 并计数。六次运行均各拦截两次远程图片请求（两个实例各一次）。最终 `acceptance.jsonl` 的两个实例均确认映射的本地图片 `naturalWidth > 0`，且 `verify_run.py` 对生命周期、脚本/资源阻止、图片、主题与双向滚动探针检查通过。顶层外部导航和新窗口也设置了拦截。**尚无 OS 级网络跟踪，不能声称 WebView2 后台绝无网络访问**；代码只证明页面资源拦截路径。

### 4.5 缺失运行时与回收

把 browser folder 指向不存在目录，创建返回 `0x80070002`，原型保持宿主界面并记录错误，没有下载逻辑。该故障注入没有卸载系统运行时，**不等于在真正未安装机器上验收**。产品应退回源码、一次通知与官方下载链接，不自动下载。

六次均完成创建→Close→再创建。Close 约 0.75–1.65 秒后，进程树降至 2 个，工作集回落至 **49.3–52.1 MiB**，私有提交 **85.7–88.1 MiB**。一次在 11.75 秒仍有 6 个进程，到 12.65 秒降至 2；因此 Close 不是瞬间释放全部子进程。

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

若整个应用始终 <60MB 是硬约束，WebView2 实测不满足；回到 pulldown-cmark 原生渲染需要重新决策 FEATURES D5，重新估算内嵌 HTML、Mermaid、数学公式能力与实现成本，不能无声删减功能。

未完成：有效重叠截图/录屏；四类真实弹出控件逐项验收；键盘与鼠标焦点实测；125%/150% 和跨屏；真正无运行时环境；OS 网络拦截；静止空闲预算；外部首帧时序；300 秒回收与长期循环；用户对预览内存预算的决定。

## 7. 构建验证与官方依据

workspace 与原型 fmt、严格 clippy（`-D warnings`）通过；workspace test 与原型 release test 通过，但当前是零测试用例，不能替代运行证据。未执行 #2 E2E，因为 #2 尚未提供。

- [Windowed vs. Visual hosting](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/windowed-vs-visual-hosting)
- [ICoreWebView2：注入与消息 API](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2)
- [ICoreWebView2Settings：脚本设置语义](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings)
