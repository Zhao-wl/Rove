# R75 Windows 发布流水线、安装包、体积守护、第三方许可声明

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M5 发布 | 2 中等 | release | 第 6 批 | R16（工作台）、R43（索引组件打包与版本握手） | R76（自动更新）、R77（【集成验收】v0.1 发布候选） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

`PLAN.md` 的 M5 只有一句完成标准：**在一台干净的 Windows 上能安装并使用**。这是 Rove 第一次离开开发机，所以本 issue 要一次把发布的全部环节补齐：流水线、安装包、体积守护、图标与版本资源、可选的 C# 索引组件、第三方许可声明，以及干净机器上的安装与卸载验证。

硬指标来自 `PROJECT_CHARTER.md` §6：**发布包（不含 C# 索引组件）< 30 MB**。M0 实测 `rove.exe` 为 6.2 MB（`M0_UI_SPIKE_REPORT.md` §3），所以预算是宽裕的——但只有在每个 PR 都盯着它的前提下才会一直宽裕。

R43（索引组件打包与版本握手） 已经在 M2 定下了 C# 索引组件的安装位置与发现规则（`%LOCALAPPDATA%\Rove\components\indexer-cs\`），并写明「具体形式与 R75（Windows 发布流水线、安装包、体积守护、第三方许可声明） 协商」「**Rove 运行期不得主动联网下载**，组件的获取由用户发起」。本 issue 必须与它对齐，不能另起一套。

必读：`PROJECT_CHARTER.md` §6（体积与性能指标）、§2.1（网络原则：自动更新是唯一的网络出口，2026-11-13 修订）、§3（Windows 10/11 x64 首发）；`PLAN.md`（M5 一行）；`FEATURES.md` §7「自动更新」一行与 D11、§11（可复用资产与许可清单）；`UX_SPEC.md` §1.6（资源管理器右键菜单「用 Rove 打开」是安装时的可选项，P1）；R43（索引组件打包与版本握手） 的安装位置与握手约定。

## 目标

### 1. 发布流水线（打 tag 触发）

- GitHub Actions：打 `v*` tag 时构建 release 版（`lto`、`codegen-units=1`，与 M0 的测量口径一致），产出**便携版 zip** 与**安装包**两种形态，附 SHA256 校验值，发布为草稿 Release 等人工确认。
- 版本号**单一来源**：`Cargo.toml` 的 version → 版本资源 → 安装包 → 「关于」对话框，由构建脚本注入，不在三处各写一遍；CI 校验 tag 与 `Cargo.toml` 一致。
- 构建产物可复现：记录 Rust 工具链版本并锁定 `Cargo.lock`。

### 2. 安装包：WiX 或 Inno Setup 二选一

**在 PR 中给出选型理由**（评估维度：是否需要管理员权限、能否做用户级安装、升级与卸载的可靠性、文件关联与右键菜单的实现成本、CI 上无人值守构建的难度、产物体积）。默认按 VS Code 的做法提供**用户级安装**（装到 `%LOCALAPPDATA%\Programs\Rove`，不需要管理员权限，不写 `HKLM`）。

安装器中的可选项（默认不勾选，照搬 VS Code 的安装器习惯）：

- **文件关联**：`.cs`、`.json`、`.yaml`、`.xml`、`.md`、`.log` 等（具体清单在 PR 中列出），走 `HKCU\Software\Classes` 的 `Rove.<ext>` ProgID + `OpenWithProgids`，**不抢默认程序**。
- **资源管理器右键菜单「用 Rove 打开」**（`UX_SPEC.md` §1.6，P1）：对文件与**目录**都提供；目录项把路径作为启动参数传给 `rove.exe`（`UX_SPEC.md` §1.6：只接受启动参数，不提供 shell 命令行工具，**因此不往 PATH 里加东西**）。
- 创建开始菜单与桌面快捷方式。

### 3. 体积守护

- 每个 PR 的体积检查已由 R04（性能基准与预算守护） 从 M1 起建立（预算值在 `perf/budgets.toml`）。本 issue 把检查对象从 `rove.exe` 扩展到**完整的发布包**（二进制 + 全部随包资产），发布包超过 **30 MB** 时 CI 失败。
- 体积报告要能看出是谁长的（至少给出 release 二进制 + 随包资产（主题、语言配置、Seti 图标与字体、grammars、Mermaid / KaTeX / highlight.js、codicons）各自的体积）。

### 4. 图标与版本资源

应用图标（多尺寸 `.ico`，含 256×256）、`winres` 版本信息（`FileDescription`、`ProductName`、`CompanyName`、`FileVersion`、`LegalCopyright`）、任务栏与 Alt+Tab 图标。实现 `workbench.action.showAboutDialog`（`KEYBINDINGS.md` §7 附录「P0 · 窗口与设置」中的命令，至今无人认领）：显示版本号、提交号、构建日期、gpui 版本、C# 索引组件是否安装及其版本，并提供「复制」按钮——排查用户问题时这是第一手信息。

### 5. 可选的 C# 索引组件包

- 独立于主安装包发布（`PROJECT_CHARTER.md` §6：组件 60–90 MB，可选），安装到 R43（索引组件打包与版本握手） 约定的 `%LOCALAPPDATA%\Rove\components\indexer-cs\`，目录布局与版本握手规则沿用 R43（索引组件打包与版本握手），**两边在开工前再对齐一次**。
- 呈现形式：安装器中的一个可勾选项（勾选时从随发布附带的本地包安装），以及一个可单独下载安装的独立包。**Rove 不会替用户首次下载组件**；已经安装的组件由 R76（自动更新） 跟随主程序检查更新。未安装时的降级行为已由 R36（索引生命周期与状态） / R43（索引组件打包与版本握手） 实现，本 issue 不改。
- 组件包的体积**不计入** 30 MB 预算，但要单独报告。

### 6. `THIRD_PARTY_NOTICES.md`

把散落在各 issue 中的声明汇总成一份完整文件（`FEATURES.md` §11、`PLAN.md`「贯穿始终的工程要求」），至少覆盖：

| 来源 | 许可 | 用途 |
|---|---|---|
| VS Code（microsoft/vscode） | MIT | 默认主题、语言配置、默认键位表、设置项 schema 与说明文字、`markdown.css` / `highlight.css`、colorize 测试数据、`vscode-textmate` 的主题匹配、模糊匹配 / 单词边界 / 缩进规则等算法 |
| codicons | **CC-BY-4.0** | 界面图标——**必须署名**，不能只写一行许可名 |
| theme-seti + seti-ui 字体 | MIT | 文件图标主题（R68（只读加载 VS Code 扩展主题、Seti 文件图标主题）） |
| tree-sitter 与各语言 grammar | 各自许可（逐个列出） | 语法高亮（C#/JSON/YAML/XML/Markdown/HLSL/Lua/Protobuf；自写的 ShaderLab 语法是 Rove 自己的代码） |
| Mermaid、KaTeX、highlight.js | 各自许可 | Markdown 预览（R29（Markdown 预览），离线打包） |
| gpui | **Apache-2.0**（单一许可，不是 MIT/Apache 双授权） | 界面框架（R05（内置 GPUI 0.2.2 并修复 Windows 输入法候选框跟随）） |
| 其余 Rust 依赖 | 自动汇总 | 用 `cargo-about` / `cargo-deny` 这类工具生成并纳入 CI，防止新增依赖漏登记 |

文件随安装包与便携版一起分发，并能从「关于」对话框打开。

### 7. 干净机器上的安装与卸载验证

- 在**干净的 Windows 10 与 Windows 11 x64**（虚拟机即可，不预装 VS Code、不预装 .NET、不预装 WebView2 运行时）上各走一遍：安装 → 启动 → 打开一个 Unity 项目 → 读代码 → 卸载。
- 没有 WebView2 运行时时按 R29（Markdown 预览） 已有的降级提示处理；**安装器不捆绑、不联网下载 WebView2**，在 PR 与 README 中写明这一限制。
- 卸载干净：程序目录、快捷方式、注册表项（文件关联与右键菜单在卸载时一并移除）全部清除；用户数据（`%LOCALAPPDATA%\Rove`，含索引缓存、设置备份、工作区状态）**默认保留**，卸载器提供一个「同时删除用户数据」的勾选项。
- 升级安装（装 `v0.1.0` 后装 `v0.1.1`）不残留旧版本、不丢用户数据。手动运行新安装包与 R76（自动更新） 的静默升级走同一条路径。

### 8. 为自动更新准备的产物（2026-11-13 追加）

- 安装器支持无人值守升级（Inno Setup 的 `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART` 一类参数），升级时先等 `rove.exe` 退出，装完可按参数重新启动并带上原来的启动参数；具体参数与 R76（自动更新） 对齐。
- 发布流水线为每个 Release 生成 `latest.json` 更新清单（版本号、发布说明链接、安装包与索引组件包的 URL / SHA256 / 体积），用 ed25519 私钥签名后作为 Release 资产上传；私钥只存在 GitHub Actions 的 secret 里，公钥由 R76（自动更新） 编进 `rove.exe`。清单格式由 R76（自动更新） 定义，本 issue 只负责生成与签名。
- **不做 Authenticode 代码签名**（D11：当前只有维护者本人使用），README 写明首次运行时 SmartScreen 的提示与放行方法。
- 安装器选型定为 **Inno Setup**（2026-11-13 确认：用户级安装、静默升级、CI 无人值守构建都简单），PR 中仍需写明理由。

## 不包含

- 自动更新的客户端（检查、下载、校验、重启安装）：R76（自动更新）。本 issue 只提供安装器的静默升级能力与签名的更新清单（第 8 节）。
- C# 索引组件的构建、裁剪与版本握手逻辑（R43（索引组件打包与版本握手））；本 issue 只负责把它打成可选包并与安装位置对齐。
- macOS / Linux 的打包（`PROJECT_CHARTER.md` §3：Windows 首发）。
- 性能基准与其他预算的守护、`rove.exe` 的逐 PR 体积检查（R04（性能基准与预算守护））；CI 的基础配置（R03（工程规范与 CI））。本 issue 只把体积检查扩展到完整发布包。
- 应用商店上架、企业部署（MSI 组策略、静默安装参数以外的内容）。
- 代码签名证书的采购：首版**不签名**，SmartScreen 会对下载的安装包提示「未知发布者」，在 README 与 Release 说明中写清楚。这是行政事项，不在本 issue 内解决，但要如实记录。

## 验收标准

- [ ] 打 tag 后流水线自动产出便携版 zip 与安装包，附 SHA256；tag 与 `Cargo.toml` 版本不一致时 CI 失败
- [ ] 发布包（不含 C# 索引组件）**< 30 MB**，并给出二进制与各类随包资产的体积明细
- [ ] 每个 PR 都能看到 `rove.exe` 与发布包的体积变化；人为把一个大文件塞进资产目录后 CI 因超限失败
- [ ] 安装包选型（WiX / Inno Setup）及理由写在 PR 中
- [ ] **用户级安装不需要管理员权限**，不写 `HKLM`；安装后开始菜单与桌面快捷方式可用
- [ ] 文件关联为可选项且默认不勾选；勾选后 `.cs` 等类型出现在「打开方式」中且**不抢默认程序**
- [ ] 右键菜单「用 Rove 打开」为可选项；对文件与目录都生效，目录项能把路径作为启动参数打开工作区
- [ ] `rove.exe` 有应用图标与完整的版本信息资源（在文件属性中可见）
- [ ] `workbench.action.showAboutDialog` 显示版本、提交号、构建日期、gpui 版本、索引组件状态，并可一键复制
- [ ] C# 索引组件包能安装到 `%LOCALAPPDATA%\Rove\components\indexer-cs\`，被 R36（索引生命周期与状态） 正确发现并通过版本握手；未安装时降级行为不变
- [ ] 安装器本身不联网；`rove.update.mode = "none"` 时运行期零网络连接（全程监视由 R77（【集成验收】v0.1 发布候选） 负责）
- [ ] 每个 Release 都有 ed25519 签名的 `latest.json`，签名能用 R76（自动更新） 的公钥验证；私钥只在 Actions secret 中
- [ ] `THIRD_PARTY_NOTICES.md` 覆盖上表全部条目，codicons 的 CC-BY-4.0 **署名完整**；依赖许可汇总已纳入 CI，新增一个未登记许可的依赖会失败
- [ ] 在干净的 Windows 10 与 Windows 11 上各完成一次「安装 → 打开 Unity 项目读代码 → 卸载」，逐条记录结果（含没有 WebView2 运行时时的表现）
- [ ] 卸载后程序目录、快捷方式、注册表项清除；用户数据默认保留，勾选后删除
- [ ] 升级安装（`v0.1.0` → `v0.1.1`）不残留旧版本、不丢用户数据与设置
- [ ] `cargo build --workspace`、`cargo test --workspace`、`cargo clippy --workspace -- -D warnings` 全部通过

## 风险与提示

- **「干净的 Windows」是这个 issue 的全部价值所在**。开发机上一切正常不说明任何问题：缺 WebView2、缺 VC++ 运行库、系统盘只有 C、用户名含中文或空格、路径里有 OneDrive 重定向——这些都只在干净机器上才暴露。准备两台虚拟机快照，每次测完回滚。
- 文件关联与右键菜单是**唯一会写注册表**的地方，也是卸载最容易残留的地方。写之前先想清楚卸载怎么收回；宁可少关联几种扩展名，也不要留下孤儿项。
- 30 MB 的预算现在看很宽裕（二进制 6.2 MB），但随包资产会悄悄长大：Seti 字体、grammars、Mermaid / KaTeX / highlight.js、主题 JSON。体积报告一定要**分项**，否则超限那天不知道该砍谁。
- codicons 是 CC-BY-4.0，与其他 MIT 资产不同，**要求署名**。许可声明不能只列一行名字，要按 CC-BY 的要求给出来源与作者。
- 不签名意味着用户第一次运行会看到 SmartScreen 警告。这会显著影响「分发给团队使用」的体验，建议在 Release 说明里给出截图和一句话解释，并把签名作为后续的行政待办报上去。
- 与 R43（索引组件打包与版本握手） 的接缝（安装位置、可选组件的呈现方式）在 M2 就已经约定过。**开工第一件事是回去核对那份约定**，不要做完才发现路径或目录布局对不上。
