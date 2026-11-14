# 自动更新（issue #149）

> 这是 Rove **唯一允许发起网络请求的地方**（[PROJECT_CHARTER](PROJECT_CHARTER.md) §2.1，2026-11-13 修订）。
> 发布侧（怎么出包、怎么签名、怎么上传）在 [RELEASE.md](RELEASE.md)；这里写更新端：
> 请求了什么、怎么校验、什么时候做、失败了怎么办。

## 1. 一次更新会发生什么请求

**全部请求都在这张表里**。没有请求体、没有自定义请求头、没有 cookie、没有查询参数、
没有设备标识，也不带任何项目信息。

| 方法 | 地址 | 什么时候 | 请求头 |
|---|---|---|---|
| GET | `https://github.com/Zhao-wl/Rove/releases/latest/download/latest.json` | 每次检查 | 只有 `User-Agent: Rove/<版本>` |
| GET | `https://github.com/Zhao-wl/Rove/releases/latest/download/latest.json.sig` | 每次检查 | 同上 |
| GET | 清单里的 `installer.url`（GitHub 会 302 到 `objects.githubusercontent.com`） | 确认要更新之后 | 同上 |
| GET | 清单里的 `components[].url` | 只有**已经装了**索引组件且清单里有更新时 | 同上 |

- 地址**写死**在 `crates/rove-update/src/endpoint.rs`，设置与环境变量都改不了它。
  唯一的例外是演练用的覆盖，见本文 §7。
- 传输用 Windows 自带的 **WinHTTP**（`windows` crate 的 `Win32_Networking_WinHttp`）：
  HTTPS 走系统证书库与系统代理设置，不引入 reqwest / hyper / rustls / ring / tokio。
  重定向跟随，但**绝不从 https 掉到 http**。
- **每一个请求在日志里都有一行**：`%LOCALAPPDATA%\Rove\logs\update.log`，内容是
  时间、方法、地址、状态码与字节数。日志只在本机，**不上传**。
- `rove.update.mode = "none"` 时连传输层都不构造（`Updater::transport()` 为 `None`），
  所以不可能发出任何请求。

## 2. 更新清单 `latest.json`

格式由本 issue 定义，由 #76 的 `cargo xtask release` 生成并签名。当前 `schema = 1`：

```json
{
  "schema": 1,
  "product": "rove",
  "channel": "stable",
  "version": "0.1.1",
  "released": "2026-11-14",
  "commit": "<40 位提交号>",
  "notes_url": "https://github.com/Zhao-wl/Rove/releases/tag/v0.1.1",
  "installer": {
    "file": "Rove-0.1.1-win-x64-setup.exe",
    "url":  "https://github.com/.../Rove-0.1.1-win-x64-setup.exe",
    "size": 12345678,
    "sha256": "<64 位十六进制>",
    "silent_args": ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL", "/SP-"]
  },
  "portable": { "file": "...", "url": "...", "size": 0, "sha256": "..." },
  "components": [
    { "id": "indexer-cs", "version": "1.2.0.0", "file": "...", "url": "...", "size": 0, "sha256": "..." }
  ]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `schema` | 是 | 不是 `1` 一律当作「没有更新」（老版本看不懂新清单时不会乱来） |
| `product` | 是 | 必须是 `rove` |
| `version` | 是 | 2–4 段十进制，可带 `-预发布`；解析不了 = 没有更新 |
| `channel` / `released` / `commit` / `notes_url` | 否 | 只用于显示与日志 |
| `installer` | 否 | 为 `null`（#76 在没装 Inno Setup 时会这样）时只提示新版本，不下载 |
| `installer.silent_args` | 否 | 静默安装参数**来自清单**，更新端不写死 |
| `portable` | 否 | 便携版产物；更新端不用它（便携版不自动更新） |
| `components[]` | 否 | 可选组件；当前只有 `indexer-cs` |
| `min_version` | 否 | 低于它的版本提示「必须更新」。#76 当前不产出这个字段，解析器先支持着 |

**不认识的字段一律忽略**，所以以后加字段不会让已经发布的 Rove 以为清单坏了。

### 签名

- 分离式签名：`latest.json.sig` 的内容是一行 `ed25519 <base64 的 64 字节签名>`，
  签的是 `latest.json` 的**字节**。
- 验签时读同样的两份原始字节，**绝不重新序列化 JSON**（字段顺序或空白的差异会让验签失败）。
- 公钥是仓库里的 [`packaging/release/update-signing.pub`](../packaging/release/update-signing.pub)，
  用 `include_str!` **编进 `rove.exe`**——仓库里的公钥和二进制里的公钥永远是同一把。
  私钥在仓库外（`%LOCALAPPDATA%\Rove-release\update-signing.key`），与更新端无关。
- `cargo xtask release-keygen` 打印的公钥指纹和
  `rove_update::signature::fingerprint()` 算出来的是同一个值，可以人工对一下。

### 五种「不更新」

下面五种情况结果都是**当作没有更新**，只写日志，旧版本一个字节都不受影响
（`crates/rove-update` 里每一种都有单元测试）：

| 情况 | 在哪判定 |
|---|---|
| 签名验不过（含签名文件格式不对） | `signature::verify` |
| 版本号解析不了 | `version::Version::parse` |
| 版本号不比当前新（含相同、更旧、预发布） | `Version::is_newer_than` |
| 下载内容的 SHA-256 不符 | `download::download_verified` |
| 下载内容的体积不符（含服务器多发） | 同上 |

后两种会把下载的文件删掉并发一条通知；前三种静默。

## 3. `rove.update.mode`

设置键登记在 [`docs/compat/SETTINGS.md`](compat/SETTINGS.md) §4。
键名带 `rove.` 前缀：settings.json 和 VS Code 共用，VS Code 自己有一个 `update.mode`。

| 取值 | 启动时 | 之后 | 发现新版本 |
|---|---|---|---|
| `none` | 不检查 | 不检查 | —（一个连接都不建） |
| `manual` | 不检查 | 不检查 | 命令触发时才检查，先问用户再下载 |
| `start` | 首帧后约 2 秒检查一次 | 不再检查 | 先问用户再下载 |
| `default`（**默认**） | 首帧后 20 秒检查一次 | 每 6 小时一次 | 后台下载，下载好提示「重启以更新到 x.y.z」 |

**第一次检查一定排在首帧之后**（`skeleton/workbench.rs` 的 `first_frame` 分支里才调
`start_update_checks`），所以冷启动预算完全不受影响。检查与下载都在后台执行器上跑。

## 4. 下载、提示与安装

```text
%LOCALAPPDATA%\Rove\updates\
    pending.json                     待安装版本的记录（同时只保留一个）
    Rove-0.1.1-win-x64-setup.exe     校验通过的安装包
    <下载中>.part                    校验通过后才改名成最终文件
```

- 下载先落 `.part`，`size` 与 `sha256` **都对上**才改名；任何一步失败都把临时文件删掉。
- 下载前把下载目录清空：**同时只保留一个待安装版本**。
- 下载好之后弹一条通知（第 4 级，`UX_SPEC` §6.1）：「已下载 Rove x.y.z，重启以更新」，
  按钮是「重启」与「稍后」。**不弹模态框**，**状态栏不加常驻条目**。
- 点「稍后」就什么都不做：正常退出时**不会**自动安装；下次启动如果这个版本还比当前新，
  再提示一次。
- 点「重启」：走 #50 的热退出把会话与未保存的修改存好 → 关窗口 → 拉起
  `cmd /D /C "<安装包> <silent_args> & start "" "<rove.exe>""` → 安装器装完把 Rove 拉起来 →
  #50 恢复会话。安装器是用户级的，**全程不弹 UAC**。
- 启动时如果 `pending.json` 里的版本已经不比当前版本新，说明**装上了**：记录与安装包一起清掉。

命令（命令面板与齿轮菜单，全部经 #1 的登记机制）：

| 命令 ID | 标题 | 什么时候可用 |
|---|---|---|
| `update.checkForUpdates` | 检查更新 | 一直 |
| `update.restartToUpdate` | 重启以更新 Rove | 有下载好的待安装版本时 |

「关于」对话框（#76）多一行**更新**：已是最新（带上次检查时间）/ 正在下载 /
已下载待重启安装 / 已关闭 / 上次检查失败（时间与原因）。

### 便携版

便携版（`rove.exe` 不在 `%LOCALAPPDATA%\Programs\Rove` 下）**不自动更新**：
只在检查时提示「发现新版本 x.y.z，请到发布页下载」，绝不在用户自己摆放的目录里替换文件。

## 5. C# 索引组件（#43）

- **只有已经装了组件**才检查组件更新；没装的不替用户下载。
- 组件包（zip）下载并校验后解到 `%LOCALAPPDATA%\Rove\components\indexer-cs\<新版本>.staging\`，
  确认里面有 `component.json` 之后整体改名成 `<新版本>\`——**校验通过才切换**。
  版本握手仍然是 #43 的那一套（`Roves` 的发现规则会自己选版本号最高的兼容候选）。
- 旧版本**在下次启动**清理（`component::cleanup_old`）；文件被占用就删不掉，留到再下次。
- zip 解压是本仓库自己写的只读实现（`crates/rove-update/src/zip.rs`）：只认「存储」与
  「deflate」，条目名出现 `..`、绝对路径、盘符或反斜杠时**整包作废**，解压总量有上限。

### 产品里在哪两处接上

核心层只提供 `plan` / `install` / `cleanup_old`，**位置与版本规则一概由界面侧按 #43 的
`rove_lang_cs::lifecycle::component` 填**（所以 `rove-update` 不依赖 `rove-lang-cs`）。
翻译在 `crates/rove-ui/src/update/component.rs`，两个接入点都在
`crates/rove-ui/src/skeleton/workbench/update_host.rs` 里，都跑在后台执行器上：

| 时机 | 做什么 |
|---|---|
| 主程序安装包下载**成功之后** | `component::follow_update`：装了组件才下载并切换，没装什么都不做 |
| 下次启动的**首帧之后** | `component::cleanup`：删掉不再使用的旧版本目录（冷启动预算不受影响） |

清理要保留的那一版这样定：优先取 `pending.json` 里的 `component_version`（**只在那次更新
已经装上之后**才算数），否则取 #43 的发现规则当前选中的那一版；两样都定不下来就**一个目录
都不删**。`rove.exe` 旁边的 `components`（随包放置）与 `ROVE_INDEXER_CS`（开发覆盖）不归
更新管，`rove.update.mode = "none"` 时也不碰组件。

组件这一步失败**不打扰用户**，只写进 `logs\update.log`：主程序的更新才是主线，组件下次
检查时会再试。产品路径的验收在 `crates/rove-ui/src/skeleton/workbench/update_tests.rs`
（已安装时升级 / 未安装时不下载 / 下次启动清理 / `none` 时不动组件）。

## 6. 失败与降级

| 情况 | 行为 |
|---|---|
| 网络不通、超时、DNS / 代理失败 | 静默，只写日志，下次按计划再试 |
| GitHub 返回 404（还没发布）/ 403（限流） | 同上。**连续失败也不弹通知** |
| 清单验签失败 / 版本号非法 / 版本号不更新 | 静默，当作没有更新 |
| SHA-256 或体积不符 | 删除下载 + 发通知，旧版本继续可用 |
| 磁盘空间不足、下载目录写不了 | 放弃 + 发通知 |
| 安装器调用失败、退出码非 0、重启后版本号没变 | 发通知 + 清掉待安装记录，旧版本继续可用 |
| 拿不到 `%LOCALAPPDATA%` | 更新整体不可用（不另找地方写文件） |

## 7. 怎么演练（不要在 GitHub 上建 Release）

自动化演练在 `crates/rove-update/tests/rehearsal.rs`：一个只监听 `127.0.0.1` 的临时
HTTP 服务器 + 测试里现生成的 ed25519 密钥对，走的是**真实的 WinHTTP 传输**。
覆盖「检查 → 下载 → 校验 → 提示 → 重启后清理」、组件跟随升级与旧版本清理、
安装包被动过手脚、清单换了钥匙签、`none` 零连接五条。

```powershell
cargo test -p rove-update --test rehearsal -j 1
```

两道只在**调试构建**里存在的闸让这件事成为可能，发布的 `rove.exe` 里它们都不存在：

| 覆盖 | 怎么生效 | 发布构建里 |
|---|---|---|
| 地址 `ROVE_UPDATE_TEST_BASE_URL` | 读环境变量的那段代码裹在 `#[cfg(debug_assertions)]` 里 | 那段代码不参与编译，地址是常量 |
| 验签公钥 `Updater::set_test_public_key` | 方法与存放它的字段都裹在 `#[cfg(debug_assertions)]` 里 | 方法与字段都不存在 |

所以 release 的 `rove.exe` **没有任何运行期开关**能换掉更新地址或验签公钥。

## 8. 体积与冷启动

更新模块进 `rove.exe` 的代价（`cargo xtask check-size`，同一台机器、同一组构建参数、
**开发机 LTO 关着**，所以绝对值比 CI 大，只看相对变化）：

| | `rove.exe` | 发布包 |
|---|---|---|
| 加本模块之前（`b86a727`） | 29.44 MiB（30,867,968 B） | 29.47 MiB |
| 加本模块之后 | 29.77 MiB（31,213,056 B） | 29.79 MiB |
| 变化 | **+337 KiB（+1.1%）** | +337 KiB |

这 337 KiB 是 ed25519-dalek + curve25519-dalek + sha2 + WinHTTP 绑定 + 本 crate 自己的代码。
**注意**：本机为了省内存把 `CARGO_PROFILE_RELEASE_LTO` 设成了 `off`，两边都是这么测的；
CI 用仓库里的 `lto = "thin"` + `codegen-units = 1`，绝对值会小一截，预算判定以 CI 为准。

冷启动（`cargo xtask startup`，首帧，n = 5，**本机同时开着 Unity 等程序，负载较高**，
只作前后对比）：

| | median | min | max |
|---|---|---|---|
| 之前 | 305.4 ms | 278.1 ms | 336.3 ms |
| 之后 | 299.3 ms | 285.6 ms | 462.8 ms |

中位数没有变化（差值在噪声里）。这是预期的：启动时只多了「读一个设置 + 看一眼
`pending.json`」，真正的检查排在首帧之后 20 秒（`default`）或 2 秒（`start`）。

## 9. 人工事项（本 issue 做不了的）

- [ ] **干净虚拟机上的真实演练**：装 `v0.1.0` → 用测试仓库或预发布 Release 发布 `v0.1.1` →
      自动检查、下载、提示 → 点「重启以更新」→ 重启后版本是 `v0.1.1`，
      **未保存的修改与会话原样恢复**（#50）。两台快照机（Win10 / Win11），测完回滚。
      与 [RELEASE.md](RELEASE.md) §7 的清单一起做。
- [ ] **`none` 的零连接实测**：`rove.update.mode = "none"` 跑满一次完整会话，
      用 `netstat -ano` 或资源监视器确认 `rove.exe` 进程树零出站连接。
      口径：每 500 ms 采样一次 `netstat -ano`，过滤 `rove.exe` 的 PID 与子进程，
      记录全程的连接条数（应为 0）。这属于 #150 的全程监视范围。
- [ ] **限流与断网**：拔网线、用防火墙拦住 `github.com`、以及 GitHub 403 限流时，
      确认只写日志、不打扰用户。
- [ ] 空闲机器上复测冷启动数字（本机同时开着 Unity 等程序，负载较高）。
