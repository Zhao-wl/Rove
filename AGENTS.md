# AGENTS.md

> 所有会话开工前读这一份就够了：这里只写**规则**，背景与取舍放在 `docs/` 里。
> 本文件由 F 链（工程与测试，#3）维护。其他会话**只允许追加**（尤其是第 8 节的用法），不要重排、不要改写别人的条目。

## 1. 项目定位与不做清单

Rove 是用 Rust + GPUI 0.2.2 开发的高性能代码阅读编辑器，Windows 优先。

- 价值排序：**高效阅读** > 可靠的局部编辑 > 辅助（把选中代码打包交给外部 AI）。
- 明确不做：内置 AI（对话/补全/Agent）、插件系统、终端、编译/构建/调试、版本控制操作、Remote/SSH/协同编辑、用网页内核构建界面（唯一例外：Markdown 预览用系统 WebView2，按需创建）。
- 详见 [docs/PROJECT_CHARTER.md](docs/PROJECT_CHARTER.md)（定位、不做清单、性能指标 §6）与 [docs/FEATURES.md](docs/FEATURES.md)（逐项功能判定与优先级）。

**两条贯穿始终的硬约束**

1. **运行时不发起任何网络请求**，唯一例外是自动更新模块（[PROJECT_CHARTER](docs/PROJECT_CHARTER.md) §2.1，2026-11-13 修订）：联网代码只能出现在更新模块所在的 crate 里。
2. **不读取 `.gitignore`**，排除规则走 `files.exclude` 等设置项（[FEATURES](docs/FEATURES.md) §6）。

## 2. crate 分层与依赖方向

**核心层不得依赖 `gpui`。** 只有 `rove-ui`（UI 组件）与 `rove-app`（入口与组装）可以依赖它。CI 会逐 crate 检查 `cargo tree -p <crate> -i gpui --all-features --target all` 必须为空（`--all-features` 才能看到 optional 依赖，`--target all` 才能看到平台条件依赖；`dev-dependencies` 也算）。

- 允许依赖 `gpui` 的 crate 列在 [.github/ui-crates.txt](.github/ui-crates.txt)；
- 判定采用**默认拒绝**：`cargo metadata` 里除 `ui-crates.txt` 之外的所有 workspace 成员都按核心层检查；[.github/core-crates.txt](.github/core-crates.txt) 是给人看的名册，漏登记只产生 `::warning::`，不会让新 crate 逃过检查；
- crate 划分、职责与依赖方向以 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 为准（由 #78 提供，可能晚于本文件合并）。**本文件不复述架构**：架构问题一律去那份文档，不要在这里加内容。

## 3. 认领、分支与 PR

1. 读 issue 正文 + 本文件 + [docs/issues/README.md](docs/issues/README.md) §0；确认**前置依赖全部已关闭**，否则停下来问维护者。
2. 把 issue 指派给自己。
3. 从最新 `main` 建分支：`issue/<编号>-<短名>`，例如 `issue/13-settings`。
4. PR 标题以 `[#<编号>]` 开头，描述里写 `Closes #<编号>`，并**逐条对照验收标准**打勾。
5. **不要直接推 `main`。会话只提 PR，由维护者合并。**

## 4. 提交规范

Conventional Commits：`feat:` `fix:` `perf:` `docs:` `test:` `chore:`（重构用 `refactor:`），范围写 crate 名：

```
feat(rove-lang-cs): 支持 GetComponent<T> 的定义跳转
fix(rove-ui): 修复高 DPI 下的行号错位
docs: 补充索引格式的元数据段说明
```

一个提交做一件事；不要在一提交里混多个 issue 的改动。

## 5. 完成标准（Definition of Done）

### 5.1 必做项

- [ ] `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全部通过（与 CI 同参，见 §9）。
- [ ] 替换掉自己负责的 `SKELETON(<键名>)` 标记，并同步更新 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) 中的对照表（该表由 #78 提供）。
- [ ] 涉及性能的改动附基准数据，做法见 §8.1（`cargo xtask bench` / `perf/budgets.toml` / `perf/README.md`，由 #4 提供）。不允许只写"感觉更快了"。
- [ ] 移植第三方代码或资产时，在 `THIRD_PARTY_NOTICES.md` 注明来源与许可（该文件由 #80 建立）。
- [ ] 自己的入口点有集成测试（见 5.2 第 3 条），且 PR 里写清怎么跑。
- [ ] 不引入网络请求（更新模块除外），不读取 `.gitignore`（§1 的两条硬约束）。
- [ ] PR 描述里逐条引用 issue 的验收标准并说明如何验证。

### 5.2 三条硬规则（逐字照抄自 issue #3，评审时逐条对照）

1. 新增的命令（VS Code 命令 ID）、设置项、颜色键、界面字符串、视图与状态栏条目，一律通过 #1（【接口先行 A1】架构与接口契约） 的登记机制注册，**不允许写死**；
2. **不得在自己的 PR 里顺手修改 #1（【接口先行 A1】架构与接口契约） 的契约**；确需修改时单独提一个小 PR，说明原因与影响面，合并后其他会话 rebase；
3. 每个 issue 必须为自己的入口点补一个集成测试，并保证 #2（【接口先行 A2】行走骨架） 的端到端测试继续通过。

## 6. 并行开发的合并规则

多个会话同时在不同 worktree 里开发，按下面的规则减少返工：

- **一个 issue 一个分支、一个 PR**；PR 只改自己负责的 crate 或模块。
- **提 PR 之前先 `git fetch && git rebase origin/main`**，本地 fmt / clippy / test 通过后再提。
- **后合并的一方负责解决冲突**：先合并的 PR 不动手，后合并的一方 rebase 到新 `main` 上解决。
- **`Cargo.lock` 冲突不要手工解决**：`git checkout origin/main -- Cargo.lock`，再 `cargo build` 重新生成，然后把结果和新代码一起提交。
- **共享热点文件只允许追加、不重排**：根目录 `Cargo.toml`（`members` 与 `[workspace.dependencies]`）、各 crate 的 `lib.rs`（模块声明）、`rove-app` 的组装代码、`docs/ARCHITECTURE.md`、#1 产出的登记表（命令 / 设置项 / 颜色键 / 界面字符串 / 视图与状态栏条目）、`.github/core-crates.txt` 与 `.github/ui-crates.txt`、本文件。冲突时保留双方条目、按原有顺序追加。
- **不要在自己的 PR 里修改 #1 的契约**（见 5.2 第 2 条）；契约改动单独提 PR，合并后其他会话 rebase。

## 7. 拆分原则（[docs/issues/README.md](docs/issues/README.md) §0，逐字保留）

1. **接口先行**：R01 定义全部跨 issue 共享的接口与登记机制，R02 用最简实现把它们串成可运行的链路。两者合并后再分头并行；其余 issue 都是“在骨架上把某个最简实现替换成正式实现”。
2. **纵向切分**：只服务一个界面的数据层与界面放在同一个 issue（如搜索引擎 + 搜索视图）；被多方共用的层（文本缓冲、工作区、设置、键位）才单独成 issue。
3. **集成验收**：每个里程碑最后由一个会话负责端到端验收，专门发现并修复接缝问题。
4. **完成标准**（写入 `AGENTS.md`）：通过 R01 的登记机制加入命令、设置项、颜色键、界面字符串和视图，不得写死；不得在自己的 PR 里修改契约，需要时单独提 PR；删除自己负责的 `SKELETON(键名)` 标记；保持 R02 的端到端测试通过，并为自己的入口补集成测试。
5. **文件归属**：每个 issue 只改自己负责的 crate 或模块。共享热点文件（根目录 `Cargo.toml`、各 crate 的 `lib.rs`、`rove-app` 的组装代码）只允许追加并保持有序，以减少合并冲突。
6. **功能覆盖**：`FEATURES.md` 的每个 P0/P1 都在 [COVERAGE.md](docs/issues/COVERAGE.md) 中有负责 issue（由 `scripts/issues/coverage.py` 生成并校验）。

## 8. 测试

- 单元测试与集成测试跟着 crate 走：`cargo test --workspace` 必须全绿。
- **需要真实 Unity 项目的测试**通过环境变量指定项目根目录：
  - `ROVE_TEST_UNITY_MEDIUM`：中型项目；`ROVE_TEST_UNITY_LARGE`：大型项目。
  - **具体路径不写进仓库**（不写进代码、不写进测试数据、不写进文档示例）。
  - 变量未设置或路径不存在时**跳过而不是失败**，并在日志里打印「已跳过」与原因；CI 上不设置这两个变量。
  - 统一辅助函数在 `rove-test-support`（`crates/rove-test-support`）：各 crate 加 `[dev-dependencies] rove-test-support = { workspace = true }`，测试里用 `rove_test_support::require_unity_project!(UnityProject::Medium)` 取路径，未设置会直接跳过当前测试。
  - 「已跳过」的原因打到 stderr；cargo 默认只在失败时显示，想看到就加 `-- --nocapture` 或设 `RUST_TEST_NOCAPTURE=1`（CI 的 test 任务就是这么跑的，日志里能直接搜到「已跳过」）。
- 每个 issue 为自己的入口点补一个集成测试（5.2 第 3 条）；UI 相关入口用 #2 的端到端测试框架。

### 8.1 涉及性能的改动必须附基准数据（#4）

只写"感觉更快了"不算完成。做法：

1. `cargo xtask gen-testdata`（默认写到 `target/perf-testdata`，已 gitignore；**CI 上不要生成 `big1g.txt`**）
2. 改之前先存一份基线：`cargo xtask bench --save target/bench-before.json`
3. 改完再跑并比较：`cargo xtask bench --baseline target/bench-before.json --check-budgets`
   - 退化超过 15% 会输出 `::warning::`（只报警，不失败：托管机器波动比 15% 大）
   - `--check-budgets` 按 `perf/budgets.toml` 判定，超预算以非零码退出
4. 涉及 `rove.exe` 的改动附 `cargo xtask check-size` 的输出（相对 M0 6.2 MB 基线的变化）；
   需要冷启动数据时跑 `cargo xtask startup`（启动 release 版 `rove.exe`，读首帧标记；
   `cargo xtask bench` 也会顺带测；「可交互」的埋点留给 #16）
5. 有 C# 索引文件时可以加 `--cs-index <idx>` 一起测导航查询（没有索引就整组跳过）

规则：

- 预算与采集口径见 `perf/budgets.toml` 与 `perf/README.md`；**不允许为了让 CI 绿改大预算数值**，
  要改走 `PROJECT_CHARTER` 的修订流程并留下记录。
- 基准只允许调用 #1 定义的公开接口，不要为了测量绕开接口或在接口上开后门。
- 新增基准：在 `xtask/benches/` 加文件、在 `xtask/Cargo.toml` 加 `[[bench]]`、在
  `xtask/src/bench.rs` 的 `TARGETS` 登记；要被预算判定的，在 `budgets.toml` 里把 `bench`
  指向 `<组>/<用例>`。
- CI：`perf.yml` 在 `main` 的 push 上保存基线 artifact，手动触发时与最近一次 `main` 比较；
  `ci.yml` 的「体积守护」任务每个 PR 都报告体积。

<!-- #39 追加：nav_check 用法 -->

### 8.2 C# 导航准确率测试集（#39，`nav_check`）

200 个人工整理的导航用例在 `tests/nav-cases/*.toml`（格式与分类见该目录的 `README.md`），由
`rove-lang-cs` 的 example `nav_check` 运行。**只在本地跑，不进 CI**（CI 上没有测试项目）。

- 需要 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`（同上，路径不写进仓库）；未设置时跳过并说明原因，退出码 0。
- 索引默认读 `target/nav-check/<medium|large>.idx`，也可用 `--index-medium` / `--index-large` 或环境变量
  `ROVE_NAV_INDEX_MEDIUM` / `ROVE_NAV_INDEX_LARGE` 指定；加 `--build` 先用 `tools/rove-indexer-cs` 重建（需先
  `dotnet build tools/rove-indexer-cs -c Release`；大型项目约 2–3 分钟、峰值数 GB 内存）。
- 输出分类准确率（通过 / 失败 / 过期 / 不支持）与失败详情，写出 `target/nav-check/nav-check-<来源>.json`（机器可读）和 `.md`。
- `--source l2`（默认，完整语义索引）/ `--source l1`（L1 快速声明索引，#37；`type_ref` 分类即其验收口径）。
- 前后对比：`--baseline <上次的 json>` 列出状态变化，加 `--fail-on-regression` 时有“通过 → 失败”就以 1 退出。
- 格式或查询接口变了，**只改** `crates/rove-lang-cs/examples/nav_check/adapter.rs`。

```powershell
cargo run --release -p rove-lang-cs --example nav_check -- --build            # 首次：建索引并跑全部
cargo run --release -p rove-lang-cs --example nav_check -- --project medium   # 只跑中型
cargo run --release -p rove-lang-cs --example nav_check -- --baseline target/nav-check/before.json --fail-on-regression
cargo run --release -p rove-lang-cs --example nav_check -- --lint             # 只检查用例格式与锚点，不查询
```

**用例过期了怎么重新整理**（测试项目是真实项目，代码会变）：

1. 运行器先按行列找；行文本与 `anchor` 对不上时在上下 40 行内按锚点重新定位（`--window` 可调），定位成功就用新位置。
2. 定位失败的用例标为“过期”：计入统计但不算失败。`cargo run -p rove-lang-cs --example nav_check -- --lint` 列出全部过期与已重定位的用例；`cargo test -p rove-lang-cs --test nav_check` 在设置了环境变量时过期超过 10% 会失败。
3. 逐条处理过期用例：打开报告里的文件，找到原来的代码（被移动、改名或删除）。还在 → 更新 `line` / `col` / `anchor`，并重新核对 `expect` 的期望位置与引用数；不在了 → 在同一分类里另选一个“按名称解析一定会错”的点替换，保持每类至少 5 个、每个项目 100 个。
4. 期望值必须读源码独立得出，`source` 写清怎么确认的；**不要从被测索引里抄**。改完后跑一遍并更新 `tests/nav-cases/baseline-l2.md`。

## 9. 工具链与本地检查

- **Rust 工具链固定 MSVC**：仓库里的 `rust-toolchain.toml` 指定 `stable-x86_64-pc-windows-msvc`。**gnu toolchain 编不过**（见 `M0_UI_SPIKE_REPORT` §2.2 坑 1），不要切换。
- **构建 gpui 需要 Windows SDK**：`rove-ui` / `rove-app` 依赖 gpui，而 gpui 的着色器编译要调用 Windows SDK 的 `fxc.exe`，**它必须在 `PATH` 上**，否则 clean build 会失败（#80/#81 在普通 shell 上踩过；CI 接入 gpui 后同样要处理，见 `M0_UI_SPIKE_REPORT` §2.2）。
- **.NET 9 SDK**：`tools/rove-indexer-cs` 是 C# 索引进程，改动后用 `dotnet build tools/rove-indexer-cs -c Release` 验证。
- **PowerShell**：CI 用 GitHub 托管的 `pwsh` 7；仓库里的检查脚本同时兼容 Windows PowerShell 5.1（脚本内部会把控制台输出编码临时切成 UTF-8，所以在中文代码页 CP936 下也不会因为 cargo 的 UTF-8 输出而报错）。本地命令示例统一写成 `pwsh`，在只装了 5.1 的机器上把 `pwsh` 换成 `powershell` 即可。
- 本地等价于 CI 的命令：

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask check-deps
pwsh -NoProfile -File .github/scripts/check-core-crates.ps1
pwsh -NoProfile -File .github/scripts/check-eol.ps1
dotnet build tools/rove-indexer-cs -c Release
```

## 10. CI 会检查什么

`.github/workflows/ci.yml` 在 `main` 的 push 和所有 PR 上运行（`windows-latest`）：

| 检查 | 失败意味着 |
|---|---|
| `cargo fmt --all --check` | 本地没跑 `cargo fmt` |
| `cargo clippy --workspace --all-targets -- -D warnings` | 有警告；`-D warnings` 下警告即错误 |
| `cargo test --workspace` | 测试失败；需要 Unity 项目的测试必须自行跳过 |
| 核心层禁 `gpui`（`.github/scripts/check-core-crates.ps1`） | 核心层 crate 引入了 `gpui`（新 crate 漏登记只警告，不失败，但也会被检查） |
| 依赖图禁 HTTP / TLS 栈（`cargo xtask check-deps`，#74） | 依赖图里出现了 `reqwest` / `hyper` / `rustls` / `ring` / `tokio` 等；确实去不掉的项写进 `xtask/src/deps.rs` 的 `ALLOWED` 白名单并注明理由 |
| 行尾检查（`.github/scripts/check-eol.ps1`） | 提交的文件行尾与 `.gitattributes` 不一致（默认 LF，`.ps1` 为 CRLF） |
| `dotnet build tools/rove-indexer-cs -c Release` | 索引进程编不过 |
| 体积守护（`size` 任务：`cargo xtask check-size`） | `rove.exe` 超过 `perf/budgets.toml` 的 `[size].budget_bytes`；每个 PR 都会报告体积与相对 6.2 MB 基线的变化 |

另有 `.github/workflows/perf.yml`：`main` 的 push 上跑基准并保存基线 artifact；手动触发时与最近一次
`main` 比较，退化超过 15% 输出 `::warning::`（**只报警、不失败**，托管机器波动大）。

CI 上不做绝对性能判定（托管机器波动大）；相对比较与预算由 #4 负责。

## 11. 相关文档

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)：crate 划分、接口归属、依赖方向（架构问题的唯一依据；由 #78 提供）
- [docs/PROJECT_CHARTER.md](docs/PROJECT_CHARTER.md)：定位、不做清单、性能指标
- [docs/FEATURES.md](docs/FEATURES.md)：功能判定与优先级
- [docs/issues/README.md](docs/issues/README.md)：issue 拆分骨架、批次与依赖
- [docs/M0_LANG_SPIKE_REPORT.md](docs/M0_LANG_SPIKE_REPORT.md)、[docs/M0_UI_SPIKE_REPORT.md](docs/M0_UI_SPIKE_REPORT.md)：M0 技术验证结论与坑

<!-- #35 追加：incr_check 用法 -->

### 8.3 C# 增量索引的一致性与延迟（#35，`incr_check`）

`rove-lang-cs` 的 example `incr_check` 在真实 Unity 项目的**副本**上检验增量索引（不改原项目）。**只在本地跑，不进 CI**；需要
`ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`（未设置时跳过）与 `dotnet build tools/rove-indexer-cs -c Release`。

- 一致性：`--edits N`（默认 1000）次随机编辑（插入行、删除语句、改签名、新增 / 删除 / 重命名文件），每 `--checkpoint K` 次
  （默认 100）对副本全量索引一次，逐文件比较“基础索引 + 覆盖层”与全量结果；不一致时自动二分重放，打印第一次出错的编辑。
  **固定随机种子**：输出第一行打印 `seed`，用 `--seed <值>` 复现同一串编辑。
- 延迟与内存：`--latency N` 做 N 次普通保存，给出单文件更新（索引进程重绑 / 往返 / 查询可见）的 p50、p99，冷启动耗时，
  常驻进程预热后与空闲时的内存，以及全量索引的峰值内存。**注明机器负载**：本机同时开着 Unity 等程序时数字只能作参考。
- 副本与报告在 `target/incr-check/`（`report-<项目>.json`，副本目录里有逐次编辑的 `edits.log`）；排查时可用 `--compare <全量索引> <基础索引> <覆盖层日志>` 对留下的文件重新比较。大型项目一次检查点约等于一次全量索引（约 1–2 分钟、峰值数 GB 内存）。

```powershell
cargo run --release -p rove-lang-cs --example incr_check -- --project large --latency 50 --edits 0
cargo run --release -p rove-lang-cs --example incr_check -- --project large --edits 1000 --checkpoint 100 --seed 42
```

<!-- #43 追加：索引组件打包与发现 -->

### 8.4 C# 索引组件的打包、发现与验证（#43）

- 打包：`powershell -NoProfile -File scripts/components/package-indexer.ps1`（trim 版，约 32 MB / zip 14 MB；`-NoTrim` 出支持 `--generators` 的未 trim 包）。产物 `target/components/<版本>/` 与 zip，约定见 [docs/CS_INDEX_FORMAT.md](docs/CS_INDEX_FORMAT.md)「组件包与安装约定」（发布流水线 #76 照此执行）。
- 发现：开发时用环境变量 `ROVE_INDEXER_CS` 指向 `rove-indexer-cs.exe` / `.dll`（`tools/.../bin` 的开发路径已移除）；正式规则是 `%LOCALAPPDATA%\Rove\components\indexer-cs\<版本>\`，测试里用 `ROVE_COMPONENTS_DIR` 或 `component::Roots` 替换位置。
- 对打包产物跑 `incremental` 测试：`ROVE_INDEXER_CS=<exe> cargo test -p rove-lang-cs --test incremental -j 1`。
- 干净 Windows（Sandbox）验证步骤与 `scripts/components/smoke-indexer.ps1` 见同一份文档，发版时重跑。

<!-- #29 追加：markdown_probe 用法 -->

### 8.5 Markdown 预览的真实 WebView2 验收（#29，`markdown_probe`）

GPUI 测试窗口没有 HWND，入口测试用假后端；真实 WebView2 的首开耗时、内存、渲染结果、滚动同步、主题不重建、空闲回收与
网络连接由 `rove-ui` 的 example `markdown_probe` 验收。**只在本地跑，不进 CI**（会弹出真实窗口，约 45 秒）。

- 报告写到 `target/markdown-probe/report.json`，标准输出每个阶段一行 `MARKDOWN_PROBE {...}`；`ROVE_MARKDOWN_TRACE=1` 另打印
  controller 创建、页面就绪、首次渲染的耗时与渲染统计。
- `ROVE_WEBVIEW_USER_DATA=<目录>` 指定 WebView2 用户数据目录（默认 `%LOCALAPPDATA%\Rove\WebView2`）；第一次用新目录是冷启动，
  数字明显偏大，报告里要分开写。`ROVE_WEBVIEW_BROWSER_FOLDER=<不存在的目录>` 可模拟“没有 WebView2 运行时”。
- 网络：全程每 500 ms 用 `netstat -ano` 采样 WebView2 进程树的连接，报告的 `network_connections_of_webview_processes` 应为空。
  这不是 OS 级抓包；需要更强的证据时另用 `pktmon`（要管理员权限）。**注明机器负载**。

```powershell
$env:ROVE_MARKDOWN_TRACE=1; cargo run -p rove-ui --example markdown_probe
```

<!-- #11 追加：着色黄金测试 -->

### 8.6 着色黄金测试（#11，`golden_colors`）

用 VS Code `vscode-colorize-tests` 的数据逐字符比对颜色：fixture → #9 的高亮 → #10 的主题 → 与
`colorize-results` 的期望颜色比较。**比颜色不比 scope**。数据在 `tests/golden/`（原样复制，见那里的
`README.md`），阈值在 `tests/golden/thresholds.toml`，口径与差异原因见
[docs/GOLDEN_COLORS.md](docs/GOLDEN_COLORS.md)。

- 判定进 CI：`cargo test -p rove-syntax --test golden_colors`（顺带把报告写到 `out/golden/`，已 gitignore）。
- 本地排查：`cargo run -p rove-syntax --example golden_colors`，可加 `--language` / `--fixture` /
  `--theme` 过滤，`--chars` 逐字符打印期望颜色与实际颜色。
- **改高亮查询前后都跑一遍**：`out/golden/report.md` 里有按语言与主题的一致率、差异成因分类和逐条差异。
- 加一种语言只需要在 `thresholds.toml` 的 `[languages]` 下加一行（省略 `threshold` 表示只统计）。
  **低于 0.98 的阈值必须带 `reason`**，否则测试失败；为了让测试通过而调低阈值是不允许的。

<!-- #76 追加：发布与体积 -->

### 8.7 发布、安装包与体积守护（#76）

发布的全部做法在 [docs/RELEASE.md](docs/RELEASE.md)（安装器选型理由、`latest.json` 的格式与签名、
干净机器验收清单、人工事项）。日常开发只需要记住三件事：

- **体积**：`cargo xtask check-size` 现在同时报告 `rove.exe` 与**发布包**，按
  `perf/budgets.toml` 的 `[size].budget_bytes`（30 MB）判定发布包，超限 CI 失败。
  发布包的内容与嵌入资产分项登记在 `packaging/package-contents.toml`：**往 `crates/*/assets`
  或 `data` 里加资源时顺手在那里加一行**，否则体积报告看不出是谁长的。
- **依赖许可**：加依赖后跑 `cargo xtask licenses --check`。许可不在
  `packaging/licenses/allowed.toml` 里，或 `packaging/licenses/rust-dependencies.md` 没重新生成
  （`cargo xtask licenses --write`），CI 都会失败。移植代码或随包资产仍然要写
  `THIRD_PARTY_NOTICES.md`（§5.1）。
- **版本号只有一处**：根 `Cargo.toml` 的 `[workspace.package] version`。版本资源、安装包、
  `latest.json` 与「关于」对话框都从它来，**不要在别处再写一遍**。

```powershell
cargo xtask release            # 本地出便携版 / 安装包 / 签名的 latest.json（不打 tag、不上传）
cargo xtask check-size         # rove.exe 与发布包的体积 + 嵌入资产分项
cargo xtask licenses --check   # 依赖许可
cargo xtask icon --check       # 应用图标与生成器一致
```

**签名私钥绝不进仓库**：它在 `%LOCALAPPDATA%` 下的 `Rove-release\update-signing.key`，
公钥 `packaging/release/update-signing.pub` 提交。不要把私钥内容打印到任何输出或 PR 里。

<!-- #149 追加：唯一的网络出口 -->

### 8.8 自动更新是唯一的网络出口（#149）

§1 的第一条硬约束「运行时不发起任何网络请求」只有一个例外，就是 `crates/rove-update`。
完整做法在 [docs/UPDATE.md](docs/UPDATE.md)（清单格式、全部请求、失败分级、演练办法）。
日常开发只要记住四条：

- **联网代码只能写在 `rove-update` 里**，而且只在它的 `http` 模块里。
  `cargo xtask check-deps` 会扫每个 workspace 成员的 `Cargo.toml`：除 `rove-update` 外
  任何 crate 打开 `Win32_Networking_*` 之类的 `windows` feature，检查直接失败。
  需要联网的新功能先问维护者，**不要自己加第二个出口**。
- **更新地址与验签公钥在发布构建里是常量**。演练用的覆盖
  （`ROVE_UPDATE_TEST_BASE_URL`、`Updater::set_test_public_key`）整段裹在
  `#[cfg(debug_assertions)]` 里，release 的 `rove.exe` 里不存在。
  **演练一律用本机 HTTP 服务器**（见 `crates/rove-update/tests/rehearsal.rs`），
  不要为了试更新去 GitHub 上建 Release。
- **清单格式改动要两边一起改**：生成在 `xtask/src/release.rs`，解析在
  `rove-update/src/manifest.rs`，文档在 `docs/UPDATE.md` §2 与 `docs/RELEASE.md` §4。
  加字段是安全的（解析器忽略不认识的字段），改字段含义不是。
- **更新不弹模态框、不加状态栏常驻条目**：只用状态栏临时消息与带按钮的通知
  （`UX_SPEC` §6.1 的第 1 级与第 4 级）。

```powershell
cargo test -p rove-update -j 1                      # 版本 / 清单 / 验签 / 五种「不更新」/ 四个 mode
cargo test -p rove-update --test rehearsal -j 1     # 本机 HTTP 服务器上的 v0.1.0 → v0.1.1 演练
cargo run -p xtask -- check-deps                    # 网络出口与 HTTP/TLS 栈
```
<!-- #30 追加：高 DPI 与多显示器 -->

### 8.9 高 DPI 与多显示器（#30）

- 改动绘制几何（细线、插入符、浮层位置、WebView2 bounds）时，先读 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) §53：
  GPUI 的几何量一律是**逻辑像素**，要「1 物理像素」的细线用 `rove_ui::dpi::{hairline, vline, hline, snap}`，不要写死 `px(1.)`。
- 回归测试：`cargo test -p rove-ui --test code_view_dpi`（100% / 125% / 150% / 200% 参数化）、
  `cargo test -p rove-ui dpi::`、`quick_input_overlay`、`preview_scale`。测试窗口的缩放系数用
  `VisualTestContext::simulate_scale_factor_change(scale)`（vendor 补丁，见 `vendor/gpui/ROVE_PATCHES.md` 的 R30 条目）。
- 截图与**真正的双显示器混合 DPI** 不能用模拟代替：清单与判据在 [docs/DPI_CHECKLIST.md](docs/DPI_CHECKLIST.md)，
  发版前与改动渲染路径后重跑。


<!-- #44 追加：M2 集成验收 -->

### 8.10 M2 集成验收的接缝测试（#44）

M2 的接缝（精确度标记的传递链、索引原子替换时开着的三个视图、组件缺失的降级、界面与查询层是否一致）
固化在 `crates/rove-ui/src/skeleton/workbench/m2_acceptance_tests.rs`，分工与结论见
[docs/M2_ACCEPTANCE_REPORT.md](docs/M2_ACCEPTANCE_REPORT.md)。

- 合成那几个测试照常进 CI：`cargo test -p rove-ui m2_ -j 1`。
- 其中 `m2_real_project_ui_matches_the_query_layer` 还需要 `ROVE_TEST_UNITY_MEDIUM` **和**该项目的
  L2 索引文件（默认 `target/nav-check/medium.idx`，可用 `ROVE_NAV_INDEX_MEDIUM` 指定）。缺任何一项
  都跳过并把原因打到 stderr，不算失败。索引用
  `cargo run --release -p rove-lang-cs --example nav_check -- --build` 生成。
- 导航准确率的两份基线：精确层 `tests/nav-cases/baseline-l2.md`、近似层
  `tests/nav-cases/baseline-l1.md`（近似层的低准确率是**设计使然**，读法写在那份文件里）。
  用 `--baseline tests/nav-cases/baseline-l2.json --fail-on-regression` 可以做前后对比。

<!-- #51 追加：M3 集成验收 -->

### 8.11 M3 集成验收的编码往返与接缝测试（#51）

M3 的两条硬线（编码往返零差异、编辑 → 保存 → 索引重绑的链路）固化成两组测试，逐条结论、
性能实测与**人工事项清单**见 [docs/M3_ACCEPTANCE_REPORT.md](docs/M3_ACCEPTANCE_REPORT.md)。

- **编码往返（核心层）**：`cargo test -p rove-buffer --test m3_roundtrip -j 1 -- --nocapture`。
  它**先扫真实 Unity 项目**（跳过 `Library` / `Temp` / `obj` / `Logs`，每个项目最多 6,000 个文本文件），
  按编码 / 换行 / 末尾换行 / 单行大小分类，再对每类的真实样本做字节级往返；项目里没有的类别
  （实测两个项目的非 UTF-8 文件为 **0**）用真实文件的内容 + 构造的编码补齐。
  **一个字节都不往测试项目里写**（全部在临时目录的副本上做）。`--nocapture` 打印的那张表就是报告 §3。
- **界面接缝**：`cargo test -p rove-ui m3_ -j 1 -- --nocapture`。合成那几条照常进 CI；
  `m3_real_project_file_edits_and_saves_byte_identically` 需要 `ROVE_TEST_UNITY_MEDIUM`，缺失时跳过。
- **改保存路径或 `BufferStates` 登记时务必重跑** `m3_edit_then_save_updates_syntax_outline_breadcrumb_and_precision`：
  它钉住的是「保存 → 索引重绑 → 查询由近似变精确」。注意**两个「磁盘内容哈希」不是同一个算法**：
  `SaveOutcome::Saved { hash }` 是 #49 自写去重用的 FNV-1a（`rove_buffer::save::content_hash`），
  `BufferStates` 比的是 C# 索引文件表的 xxHash64（`rove_lang_cs::router::hash_bytes`）。
  混用过一次，后果是保存后整份文件的导航结果永远停在「近似」（报告 §5.1）。
- **输入法矩阵只能实机做**：`spikes/ime_test.ps1` 会抢前台、注入按键、改输入法状态，
  判据要看截图。#51 本次**一条都没有跑**，全部留在报告的人工清单里——不要把它当成已验收。
