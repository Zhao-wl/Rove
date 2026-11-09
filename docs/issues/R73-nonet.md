# R73 精简 GPUI 的网络依赖

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 3 困难 | ui | 第 2 批 | R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

`M0_UI_SPIKE_REPORT.md` §4.7（构建与依赖）把这件事写成了 GPUI 的一条明确缺陷：**`gpui` 强制依赖一个 HTTP 客户端，不是可选 feature**。

```
ring  ← rustls ← hyper-rustls ← zed-reqwest(0.12.15-zed) ← gpui_http_client ← gpui
tokio ← h2     ← hyper        ← hyper-rustls ← zed-reqwest ← gpui_http_client ← gpui
```

`gpui/Cargo.toml` 里 `[dependencies.http_client] package = "gpui_http_client"` 没有 `optional = true`，关不掉。对一个「不发起任何网络请求」的阅读器（`PROJECT_CHARTER.md` §2 非目标）来说，链接进 reqwest + hyper + rustls + tokio + ring 是多余的**审计面与供应链面**——报告也写明了：最终二进制仍只有 6.2 MB（链接器剔除了未用代码），**所以这不是体积问题**。报告给出的缓解方案就是本 issue：「fork 后把 `http_client` 改成 optional，或在 CI 加网络系统调用断言」。

R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随） 已经建立了 `vendor/gpui` 与 `ROVE_PATCHES.md`，并明确写着本 issue **必须等它合并后再开工**，否则会在同一目录上冲突。

必读：`docs/M0_UI_SPIKE_REPORT.md` §4.7（上面的依赖链、411 个包、469 s clean build）、§3 指标对比总表（`rove.exe` 6.2 MB）、§6.1（GPUI 的风险表，含「强制链接 HTTP/TLS 栈」一行及缓解方案）、§8（未验证项）；`PROJECT_CHARTER.md` §2（非目标：无网络）、§6（发布包 < 30 MB）；R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随） 的 `vendor/gpui/ROVE_PATCHES.md` 约定。

## 目标

### 1. 让 HTTP 客户端从依赖树里消失

两条路线，**在 PR 中二选一并说明理由**（优先选补丁更小、升级 GPUI 时更容易重新应用的那条）：

- **A. 改成 optional feature**：给 `vendor/gpui/Cargo.toml` 的 `http_client` 加上 `optional = true`，新增 feature（例如 `http-client`，默认关闭）；gpui 内部用到 `HttpClient` 的地方（`cx.http_client()`、远程资源加载）在 feature 关闭时编译为一个**总是返回「未启用网络」错误的空实现**。
- **B. 替换 `gpui_http_client`**：在 workspace 的 `[patch.crates-io]` 中把 `gpui_http_client` 指向一个本地的 stub crate，保持同样的公开 API（trait 与类型签名）但不依赖 reqwest / hyper / rustls / tokio / ring，所有请求直接返回错误。gpui 源码零改动。

无论哪条路线，**语义都是「调用即失败」而不是「静默成功」**：万一某处代码真的发起请求，必须立刻在日志里看得见，而不是悄悄返回空结果。

### 2. 确认没有功能依赖它

- 逐一确认 Rove 用到的 gpui 能力里没有走 HTTP 的：资源 / 图片加载只读本地文件（与 R63（图片预览） 对齐，它的解码栈也不得引入网络依赖）、字体只用本机字体与内置回退（与 R68（只读加载 VS Code 扩展主题、Seti 文件图标主题） 的图标字体对齐）。
- Markdown 预览走系统 WebView2，不经过 gpui 的 HTTP 客户端（远程资源的拦截由 R29（Markdown 预览） 的 `rove.markdown.allowRemoteResources` 负责），本 issue 只需确认两者互不影响。
- 发现确实有功能依赖时，**先报出来再改方案**，不要为了让依赖树好看而砍掉 Rove 要用的能力。

### 3. 测量前后对比

在同一台机器、同一套缓存条件下测量，并在 PR 与 `ROVE_PATCHES.md` 中给出对照表（基线来自 M0 报告）：

| 指标 | 基线（Route A / gpui 0.2.2） | 改动后 |
|---|---|---|
| `cargo tree` 去重后的依赖包数 | 411 | |
| clean build（release，16 核，依赖已在本地 registry 缓存） | 469.0 s | |
| 只改 `main.rs` 的 release 增量构建 | 64–83 s | |
| `rove.exe` 体积 | 6.2 MB | |

体积预计**几乎不变**（链接器本来就剔除了未用代码），这不是失败；本 issue 的收益是审计面与编译时间。

### 4. 防回归

- 新增一条依赖检查（`cargo xtask check-deps` 或等效脚本）：`cargo tree` 中出现 `reqwest` / `zed-reqwest` / `hyper` / `rustls` / `ring` 即失败，**纳入 CI**（与 R03（工程规范与 CI） 协调，加入它的检查清单）。某一项确实去不掉时，在检查脚本里写成带理由的显式白名单，而不是直接放行。
- `tokio` 如果被其他依赖（非 gpui 路径）带进来，同样在白名单里写明来源与理由。

### 5. 记录

在 `vendor/gpui/ROVE_PATCHES.md` 中按 R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随） 定下的格式追加一条：改了哪个文件、为什么改、原始代码是什么、升级 GPUI 时如何重新应用，以及上面的前后对照表。

## 不包含

- `update_ime_position` 的修复与 `vendor/` 目录的建立（R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随），本 issue 的前置）。
- 其余依赖的精简（图片解码、SVG、GIF 等栈，M0 报告同样点名但与网络无关）：如确有必要另提 issue，本 issue 只动 HTTP/TLS 这一条链。
- 运行时网络访问的整体验收（用网络监视器走完一次完整会话）：属于 R74（【集成验收】M4）；本 issue 只负责编译期依赖与 CI 检查。
- Markdown 预览中远程资源的拦截（R29（Markdown 预览））。
- 任何 R01（【接口先行 A1】架构与接口契约） 接口改动；gpui 类型仍然不得出现在核心层。

## 验收标准

- [ ] `cargo tree` 中不再出现 `reqwest` / `zed-reqwest` / `hyper` / `hyper-rustls` / `rustls` / `ring`；去不掉的项逐条说明原因
- [ ] 依赖包数、clean build 时间、增量构建时间、`rove.exe` 体积的前后对照表已给出（基线 411 个包 / 469.0 s / 64–83 s / 6.2 MB）
- [ ] 选择的路线（A 或 B）及理由写在 PR 与 `ROVE_PATCHES.md` 中，补丁改动范围尽可能小
- [ ] 真的调用到 HTTP 客户端时**返回明确错误并记录日志**，不是静默成功（有单元测试）
- [ ] CI 中的依赖检查生效：人为加回 `reqwest` 依赖后 CI 失败
- [ ] 界面功能无回退：窗口、输入法、图片 / 字体加载、Markdown 预览（WebView2）全部正常；R02（【接口先行 A2】行走骨架） 的端到端测试通过
- [ ] `cargo build --workspace`、`cargo test --workspace`、`cargo clippy --workspace -- -D warnings` 全部通过
- [ ] `vendor/gpui/ROVE_PATCHES.md` 中新增的这一条包含重新应用补丁的步骤
- [ ] `THIRD_PARTY_NOTICES.md` 中 gpui 的 Apache-2.0 声明仍然正确（如果新增了 stub crate，说明它是 Rove 自己的代码）

## 风险与提示

- **这是补丁，不是重构**。GPUI 在 crates.io 上只有 7 个 0.x 版本、API 不稳定（`M0_UI_SPIKE_REPORT.md` §6.1），补丁越小，下次升级越容易。不要顺手整理 gpui 的其他依赖或模块。
- 路线 B（patch `gpui_http_client`）的好处是 gpui 源码零改动，升级 gpui 时不需要重新打补丁；代价是必须跟住 `gpui_http_client` 的 API，它升级时 stub 也要跟着改。路线 A 反之。**权衡写进 PR**，不要只写结论。
- 优先把改动提给上游（zed-industries/zed）：`optional = true` 这种改动上游接受的可能性不低，合并后我们的补丁就能删掉。注意开发机访问 GitHub 受限（`M0_UI_SPIKE_REPORT.md` §2.2 坑 2）。
- 不要把「依赖树干净」当成「运行时不联网」的证明。两者都要有：本 issue 管编译期，R74（【集成验收】M4） 管运行期的实测。
- 体积没有明显变化是**预期结果**。如果 PR 里写成「体积下降了 X MB」，多半是测量口径出了问题（例如对比了 debug 与 release）。
