# AGENTS.md

> 所有会话开工前读这一份就够了：这里只写**规则**，背景与取舍放在 `docs/` 里。
> 本文件由 F 链（工程与测试，#3）维护。其他会话**只允许追加**（尤其是第 8 节的用法），不要重排、不要改写别人的条目。

## 1. 项目定位与不做清单

Rove 是用 Rust + GPUI 0.2.2 开发的高性能代码阅读编辑器，Windows 优先。

- 价值排序：**高效阅读** > 可靠的局部编辑 > 辅助（把选中代码打包交给外部 AI）。
- 明确不做：内置 AI（对话/补全/Agent）、插件系统、终端、编译/构建/调试、版本控制操作、Remote/SSH/协同编辑、用网页内核构建界面（唯一例外：Markdown 预览用系统 WebView2，按需创建）。
- 详见 [docs/PROJECT_CHARTER.md](docs/PROJECT_CHARTER.md)（定位、不做清单、性能指标 §6）与 [docs/FEATURES.md](docs/FEATURES.md)（逐项功能判定与优先级）。

**两条贯穿始终的硬约束**

1. **运行时不发起任何网络请求**（[PROJECT_CHARTER](docs/PROJECT_CHARTER.md) §4.4）。
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
- [ ] 涉及性能的改动附基准数据，做法见 #4（性能基准与预算守护）：`cargo xtask bench` 与 `perf/README.md` 由 #4 提供，合并后把跑法补充到本文件 §8。不允许只写"感觉更快了"。
- [ ] 移植第三方代码或资产时，在 `THIRD_PARTY_NOTICES.md` 注明来源与许可（该文件由 #80 建立）。
- [ ] 自己的入口点有集成测试（见 5.2 第 3 条），且 PR 里写清怎么跑。
- [ ] 不引入网络请求，不读取 `.gitignore`（§1 的两条硬约束）。
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

<!-- #39 追加：nav_check 用法 -->
<!-- #4 追加：基准与预算的跑法 -->

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
| 行尾检查（`.github/scripts/check-eol.ps1`） | 提交的文件行尾与 `.gitattributes` 不一致（默认 LF，`.ps1` 为 CRLF） |
| `dotnet build tools/rove-indexer-cs -c Release` | 索引进程编不过 |

CI 上不做绝对性能判定（托管机器波动大）；相对比较与预算由 #4 负责。

## 11. 相关文档

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)：crate 划分、接口归属、依赖方向（架构问题的唯一依据；由 #78 提供）
- [docs/PROJECT_CHARTER.md](docs/PROJECT_CHARTER.md)：定位、不做清单、性能指标
- [docs/FEATURES.md](docs/FEATURES.md)：功能判定与优先级
- [docs/issues/README.md](docs/issues/README.md)：issue 拆分骨架、批次与依赖
- [docs/M0_LANG_SPIKE_REPORT.md](docs/M0_LANG_SPIKE_REPORT.md)、[docs/M0_UI_SPIKE_REPORT.md](docs/M0_UI_SPIKE_REPORT.md)：M0 技术验证结论与坑
