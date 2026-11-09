## 背景

Rove 由多个会话在多个工作区（git worktree）里并行推进。没有统一约定时，各会话会各自发明分支命名、提交规范、测试方式和"完成"的含义，合并时不断返工。本 issue 定义所有会话共用的工程规范，并用 CI 把其中能自动检查的部分固化下来。

本 issue 不依赖 [[contracts]]，可以与它并行；但 `AGENTS.md` 中与架构相关的内容（crate 划分、依赖方向、接口归属）以 [[contracts]] 产出的 `docs/ARCHITECTURE.md` 为准，这里只做链接，不复述。

## 目标

### 1. `AGENTS.md`（`CLAUDE.md` 只写一行引用它）

内容：

- 项目定位与"不做清单"摘要，链接 `docs/PROJECT_CHARTER.md`、`docs/FEATURES.md`；
- crate 分层与依赖方向，**核心层不得依赖 `gpui`**（只有 `rove-ui` / `rove-app` 可以），细节链接 `docs/ARCHITECTURE.md`；
- 认领流程：指派给自己 → 分支 `issue/<编号>-<短名>`（如 `issue/13-settings`）→ PR 标题带 `Closes #<编号>`；
- 提交规范：Conventional Commits（`feat:`、`fix:`、`perf:`、`docs:`、`test:`、`chore:`），范围用 crate 名；
- **完成标准（Definition of Done）**，其中必须明确写出三条规则：
  1. 新增的命令（VS Code 命令 ID）、设置项、颜色键、界面字符串、视图与状态栏条目，一律通过 [[contracts]] 的登记机制注册，**不允许写死**；
  2. **不得在自己的 PR 里顺手修改 [[contracts]] 的契约**；确需修改时单独提一个小 PR，说明原因与影响面，合并后其他会话 rebase；
  3. 每个 issue 必须为自己的入口点补一个集成测试，并保证 [[skeleton]] 的端到端测试继续通过；
  - 另加：`cargo fmt`、`clippy` 零警告、测试通过；涉及性能的改动附基准数据（见 [[perf]]）；替换掉自己负责的 `SKELETON(<键>)` 标记并同步更新 `docs/ARCHITECTURE.md` 中的对照表；移植第三方代码或资产时在 `THIRD_PARTY_NOTICES.md` 注明来源与许可；
- 两条贯穿始终的硬约束：**运行时不发起任何网络请求**（`PROJECT_CHARTER` §4.4），**不读取 `.gitignore`**（`FEATURES` §6）；
- 共享热点文件（根 `Cargo.toml`、`docs/ARCHITECTURE.md`、登记表所在文件）的修改规则：只追加、不重排，冲突时以后合并者负责 rebase；
- 本地 Unity 测试项目通过环境变量 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 指定，**具体路径不写进仓库**；
- 工具链：MSVC（仓库中已有 `rust-toolchain.toml`，固定 `stable-x86_64-pc-windows-msvc`；gnu toolchain 编不过，见 `M0_UI_SPIKE_REPORT` §2.2 坑 1）；.NET 9 SDK（`tools/rove-indexer-cs`）。

### 2. CI（GitHub Actions，`windows-latest`）

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `dotnet build tools/rove-indexer-cs -c Release`
- Cargo registry / target 与 NuGet 缓存（GPUI 的 clean build 实测 469 s，缓存命中率直接决定 CI 时长）
- 行尾检查：`.gitattributes` 已声明 `* text=auto eol=lf`（`.ps1` 为 CRLF），CI 校验不出现回退

### 3. 核心层禁止依赖 gpui 的检查

一个独立任务：对每个核心层 crate 跑 `cargo tree -p <crate> -i gpui`，输出必须为空。crate 名单从一个清单文件（例如 `.github/core-crates.txt`）读取，[[contracts]] 确定最终划分后只需改这个清单。

### 4. 测试项目与环境变量

需要 Unity 测试项目的测试，在环境变量未设置时**跳过而不是失败**（CI 上不设置这两个变量），并在日志中打印"已跳过"。提供一个统一的辅助函数供各 issue 使用。

### 5. 模板

PR 模板（勾选完成标准的三条规则）、issue 模板（任务 / 缺陷）。

## 不包含

- 性能基准与预算守护（[[perf]]）、发布流水线与体积守护（[[release]]）。
- 架构决策本身（[[contracts]]）。

## 验收标准

- [ ] `AGENTS.md` 覆盖上面列出的全部条目，完成标准中的三条规则逐字可查
- [ ] CI 在 `main` 和 PR 上运行，当前 `main` 能全绿
- [ ] 故意给 `rove-buffer` 加一条 `gpui` 依赖时，CI 失败（验证后撤回，在 PR 中附截图）
- [ ] 故意引入一个 clippy 警告时，CI 失败
- [ ] 未设置 `ROVE_TEST_UNITY_MEDIUM` 时，相关测试显示为跳过，`cargo test --workspace` 仍然通过
- [ ] `dotnet build tools/rove-indexer-cs -c Release` 在 CI 上通过
- [ ] PR 模板与 issue 模板可用

## 风险与提示

- **性能数字与机器强相关**：开发机的 D 盘是机械硬盘（`M0_LANG_SPIKE_REPORT` 中索引冷读 3.4 s 就是这么来的），GitHub 托管机器的波动更大。所以 CI 里**不要**做绝对的性能判定，绝对指标的采集与相对比较交给 [[perf]]。
- `AGENTS.md` 是所有会话每次开工都会读的文件，**要短、要能执行**。背景解释放到 `docs/`，这里只留规则。
