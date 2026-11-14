# 发布流程（issue #76）

> 本文是发布的操作手册与约定。规则在 `AGENTS.md`，体积预算在 `perf/budgets.toml`，
> 组件包的约定在 [CS_INDEX_FORMAT](CS_INDEX_FORMAT.md)「组件包与安装约定」。

Rove 的发布有两条等价的路径，产出同一套产物：

| 路径 | 入口 | 什么时候用 |
|---|---|---|
| GitHub Actions | [`.github/workflows/release.yml`](../.github/workflows/release.yml)，打 `v*` tag 触发 | 正常情况 |
| 本地 | `cargo xtask release` + 生成出来的 `upload.ps1` | **Actions 额度耗尽时**（当前就是这种情况） |

两条路径调用的是同一段代码（`xtask/src/release.rs`），唯一的差别是签名私钥的来源：
Actions 读 secret `ROVE_UPDATE_SIGNING_KEY`，本地读 `%LOCALAPPDATA%\Rove-release\update-signing.key`。

## 1. 产物

一次发布产出下面这些东西（都在 `target/release-dist/<版本>/`）：

| 文件 | 说明 |
|---|---|
| `Rove-<版本>-win-x64.zip` | 便携版：解压即用，不写注册表 |
| `Rove-<版本>-win-x64-setup.exe` | 安装包：用户级安装，见下一节 |
| `SHA256SUMS.txt` | 上面全部产物的 SHA-256 |
| `latest.json` + `latest.json.sig` | #149 的更新清单与 ed25519 分离签名 |
| `release-notes.md` | Release 说明草稿（SmartScreen、WebView2、卸载说明都写好了） |
| `upload.ps1` | 人工执行的上传脚本（打 tag → 建**草稿** Release → 上传） |
| `rove-indexer-cs-<版本>-win-x64.zip` | 可选的 C# 索引组件包，由 `scripts/components/package-indexer.ps1` 单独打，`--component` 传进来 |

发布包的内容清单在 [`packaging/package-contents.toml`](../packaging/package-contents.toml)：
`rove.exe` + `LICENSE.txt` + `THIRD_PARTY_NOTICES.md` + `README.md`。主题、语言配置、codicons、
Markdown 预览资源等**全部嵌在 `rove.exe` 里**，所以包里没有资源目录；体积报告按来源分项列出它们。

## 2. 安装器选型：Inno Setup（2026-11-13 确认）

| 维度 | Inno Setup 6 | WiX 4/5 |
|---|---|---|
| 用户级安装（不需要管理员） | `PrivilegesRequired=lowest` 一行搞定 | 可以做 per-user MSI，但要处理 `ALLUSERS`/`MSIINSTALLPERUSER`，更绕 |
| 升级与卸载 | AppId 相同即原地升级；`uninsdeletekey` 自动收回注册表 | 组件规则（GUID、KeyPath）严格，写错容易留残留或升级失败 |
| 文件关联与右键菜单 | `[Registry]` 几行，带卸载标记 | 需要 `util:` 扩展或自定义 CA，成本更高 |
| CI 无人值守构建 | `choco install innosetup` + `ISCC.exe script.iss`，几秒 | 需要 WiX Toolset + MSBuild，构建链更长 |
| 静默安装参数 | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`（#149 直接用） | `msiexec /qn`，但 per-user MSI 的静默升级坑更多 |
| 产物体积 | 单个 exe，LZMA2，比 MSI 小 | MSI 通常更大 |

结论：**Inno Setup**。VS Code 的 User Installer 也是 Inno Setup，用户习惯一致；MSI 的唯一优势
（组策略部署）正好在本 issue 的「不包含」里。脚本是 [`packaging/windows/rove.iss`](../packaging/windows/rove.iss)。

安装器的约定（逐条对应验收标准，`cargo test -p xtask` 里有对脚本文本的检查）：

- 装到 `%LOCALAPPDATA%\Programs\Rove`，`PrivilegesRequired=lowest`，**只写 `HKCU`，不写 `HKLM`**；
- 可选项**默认全不勾**：桌面快捷方式、文件关联、文件右键菜单、目录右键菜单、C# 索引组件。
  其中「安装 C# 索引组件」是条件编译出来的：只有 `cargo xtask release --component <zip>`
  把组件包带进来（转成 ISCC 的 `/DComponentZip=<绝对路径>`）时才会出现，否则整段不生成；
- 文件关联走 `HKCU\Software\Classes\Rove.SourceFile` + 各扩展名的 `OpenWithProgids`，
  **不写扩展名的默认值**，所以只出现在「打开方式」里，不抢默认程序。
  清单：`.cs .json .jsonc .yaml .yml .xml .md .log .txt .shader .hlsl .lua .asmdef`；
- 右键菜单「用 Rove 打开」：文件用 `"%1"`，目录与目录空白处用 `"%V"`（路径作为启动参数 → 以它为工作区打开）。
  **不往 PATH 里加东西**（UX_SPEC §1.6：只接受启动参数，不提供 shell 命令行工具）；
- 卸载清除程序目录、快捷方式与全部注册表项；用户数据（`%LOCALAPPDATA%\Rove`）**默认保留**，
  卸载进度页上有一个默认不勾的「同时删除用户数据」；
- 静默升级：`/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /SP-`，`CloseApplications=yes`
  让安装器等 `rove.exe` 退出，`RestartApplications=no`（重启与启动参数由 #149 的更新端决定）。
  这组参数同时写进 `latest.json` 的 `installer.silent_args`，更新端照着调用即可；
- **安装器不联网**：不捆绑、不下载 WebView2 运行时，也不下载索引组件（组件来自随包的本地 zip）。

## 3. 本地发布（Actions 额度耗尽时用这条）

```powershell
# 0. 一次性：生成签名密钥对（私钥在仓库外，公钥提交进仓库）
cargo xtask release-keygen

# 1. 可选：打索引组件包（需要 .NET 9 SDK）
powershell -NoProfile -File scripts\components\package-indexer.ps1

# 2. 构建 + 打包 + 签名（不打 tag、不上传）
cargo xtask release --component target\components\rove-indexer-cs-<版本>-win-x64.zip

# 3. 核对体积与产物，然后人工上传（脚本会先让你输入 yes 确认）
cargo xtask check-size
powershell -NoProfile -File target\release-dist\<版本>\upload.ps1
```

要点：

- 版本号的**唯一来源**是根 `Cargo.toml` 的 `[workspace.package] version`。改版本只改那一处，
  版本资源、安装包、`latest.json`、「关于」对话框都跟着变；`cargo xtask check-version --tag v0.1.0`
  校验 tag 与它一致（CI 在打 tag 时跑同一条命令）。
- 没装 Inno Setup 时 `cargo xtask release` 会跳过安装包并给出提示，其余产物照常产出；
  加 `--require-installer` 则视为失败。ISCC.exe 的位置可以用 `ROVE_ISCC` 指定。
- 产物可复现：`Cargo.lock` 已锁定，工具链固定在 `rust-toolchain.toml`；
  Actions 会把 `rustc -vV` / `cargo -vV` 存进 `toolchain.txt` 一起上传，本地发布时请一并记录。
- **本命令不打 tag、不创建 Release、不上传任何东西**；上传是人工的一步。

## 4. 更新清单 `latest.json` 与签名

格式由 #149 定义（字段逐项说明、更新端怎么校验、怎么演练都在 [UPDATE](UPDATE.md)），
本 issue 负责生成与签名。当前字段（`schema = 1`）：

```json
{
  "schema": 1,
  "product": "rove",
  "channel": "stable",
  "version": "0.1.0",
  "released": "2026-11-14",
  "commit": "<40 位提交号>",
  "notes_url": "https://github.com/Zhao-wl/Rove/releases/tag/v0.1.0",
  "installer": { "file": "...", "url": "...", "size": 0, "sha256": "...", "silent_args": ["/VERYSILENT", "..."] },
  "portable": { "file": "...", "url": "...", "size": 0, "sha256": "..." },
  "components": [ { "id": "indexer-cs", "version": "1.2.0.0", "file": "...", "url": "...", "size": 0, "sha256": "..." } ]
}
```

`installer` 在没有产出安装包时是 `null`（例如本机没装 Inno Setup 就跑了 `cargo xtask release`），
`components` 没有可选组件时是空数组；**正式发布的清单里两者都应该有内容**。

签名是**分离式**的：对 `latest.json` 的**字节**签名，写出 `latest.json.sig`，内容是一行
`ed25519 <base64 的 64 字节签名>`。验签时读同样的两个文件，**不要重新序列化 JSON**
（字段顺序或空白的差异会让验签失败）。公钥在
[`packaging/release/update-signing.pub`](../packaging/release/update-signing.pub)（同样是
`ed25519 <base64>`，带注释行），#149 把它编进 `rove.exe`。

- 私钥在**仓库外**：`%LOCALAPPDATA%\Rove-release\update-signing.key`（`ROVE_RELEASE_KEY` 可改位置）。
  xtask 从不打印私钥内容，错误信息里也只有路径。
- 校验：`cargo xtask release-verify --file <latest.json>`（发布命令自己也会先验一遍再收工）。
- 换密钥对意味着已发布的清单全部失效：`release-keygen` 发现私钥已存在时会拒绝覆盖，确需重来加 `--force`。

## 5. 体积守护

`cargo xtask check-size` 报告两个数并按**发布包**判定：

- `rove.exe` 的体积与相对 M0 基线（6.2 MB）的变化；
- 发布包（`package-contents.toml` 里的全部文件）的体积，占 `perf/budgets.toml` 的
  `[size].budget_bytes`（30 MB，CHARTER §6）的百分比，**超限 CI 失败**；
- 嵌入 `rove.exe` 的资产分项（Markdown 预览资源、codicons、主题、语言配置、高亮查询、
  默认键位表、设置目录），这样体积长了能看出是谁长的。

C# 索引组件是独立的可选包，**不计入** 30 MB：用 `--component <zip>` 让它只被报告。

## 6. 依赖许可

`cargo xtask licenses --check`（CI 的「依赖许可汇总」任务）：

- 许可信息来自 `cargo metadata`，不需要联网，也不需要额外装工具；
- 允许的许可写在 [`packaging/licenses/allowed.toml`](../packaging/licenses/allowed.toml)，
  按 SPDX 语义判定（`A OR B` 取其一，`A AND B` 两边都要）。所以 `Apache-2.0 OR GPL-2.0-only`
  可以用，纯 copyleft 的依赖会失败；
- 汇总表 [`packaging/licenses/rust-dependencies.md`](../packaging/licenses/rust-dependencies.md)
  由 `cargo xtask licenses --write` 生成并提交；**加了依赖忘了重新生成也会失败**。

移植代码与随包资产的逐项声明仍然在 [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md)，
它随安装包与便携版一起分发，「关于」对话框里有一个按钮直接打开安装目录里的那一份。

## 7. 干净机器上的验收清单

**这是发布最有价值的一步**：开发机上一切正常不说明任何问题。两台快照虚拟机
（干净的 Windows 10 x64 与 Windows 11 x64，不预装 VS Code、.NET、WebView2），每次测完回滚。

| # | 步骤 | 期望 |
|---|---|---|
| 1 | 双击安装包 | SmartScreen 提示「未知发布者」→「更多信息」→「仍要运行」可放行（未签名，D11） |
| 2 | 走完向导（可选项全不勾） | 不弹 UAC，装到 `%LOCALAPPDATA%\Programs\Rove`，开始菜单有快捷方式 |
| 3 | 启动 Rove，打开一个 Unity 项目 | 能读代码、能搜索；没有 .NET 时 C# 导航降级为声明索引并在状态栏说明 |
| 4 | 打开一个 `.md` | 有 WebView2 → 预览；没有 → 以源码打开并提示一次（**不自动下载**） |
| 5 | 「关于」对话框 | 版本 / 提交号 / 构建日期 / gpui / 索引组件状态齐全，「复制」可用 |
| 6 | 重新安装，勾上文件关联与两个右键菜单 | `.cs` 出现在「打开方式」里但默认程序不变；文件与目录右键都有「用 Rove 打开」，目录项以该目录为工作区打开 |
| 7 | 装 `v0.1.0` 后装 `v0.1.1` | 原地升级，不留旧版本目录，设置与工作区状态还在 |
| 8 | 静默升级 `setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART` | 无界面完成；运行中的 `rove.exe` 会被要求退出后再装 |
| 9 | 卸载（不勾「同时删除用户数据」） | 程序目录、快捷方式、`HKCU\Software\Classes` 下的 `Rove.*` 与 `shell\Rove` 全部消失；`%LOCALAPPDATA%\Rove` 还在 |
| 10 | 再装一次并卸载，勾上「同时删除用户数据」 | `%LOCALAPPDATA%\Rove` 也被删除 |
| 11 | 用户名含中文 / 空格、OneDrive 重定向的账户 | 以上全部重跑一遍 |

检查注册表残留：

```powershell
reg query HKCU\Software\Classes /f Rove /s
reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall /f Rove /s
```

## 8. 人工事项（本 issue 不做、也做不了的）

- [ ] 把私钥备份进密码管理器，并存入 GitHub Actions 的 secret `ROVE_UPDATE_SIGNING_KEY`
      （内容就是 `update-signing.key` 的那一行 `ed25519 <base64>`）。
- [ ] 本机装 Inno Setup 6（<https://jrsoftware.org/isdl.php>）后重跑 `cargo xtask release`，
      产出安装包并在干净虚拟机上走完第 7 节的清单。
- [ ] Authenticode 代码签名证书的采购（D11：首版不签名，SmartScreen 会提示未知发布者）。
- [ ] Release 说明里附一张 SmartScreen 提示的截图（`release-notes.md` 已写好文字说明）。
- [ ] 第 7 节第 7 条（`v0.1.0` → `v0.1.1`）跑完后，接着验自动更新的那一轮：
      见 [UPDATE](UPDATE.md) §8（装 v0.1.0 → 发布 v0.1.1 → 自动检查、下载、提示 →
      重启后版本变了且会话原样恢复）。
