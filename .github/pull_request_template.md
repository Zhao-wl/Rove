# 描述

<!-- 这个 PR 解决什么，对应 issue 的哪一部分。 -->

Closes #

## 改动

<!-- 按 crate / 模块列出改了什么；共享热点文件（根 Cargo.toml、各 crate 的 lib.rs、rove-app 组装代码）是否只做了追加。 -->

## 完成标准（AGENTS.md §5）

- [ ] `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 本地通过（CI 会再跑一遍）
- [ ] 替换掉自己负责的 `SKELETON(<键名>)` 标记，并同步更新 `docs/ARCHITECTURE.md` 的对照表
- [ ] 为自己的入口点补了集成测试（写明怎么跑）
- [ ] 涉及性能的改动附了基准数据（`cargo xtask bench`，见 #4 与 `perf/README.md`）
- [ ] 移植第三方代码或资产时，在 `THIRD_PARTY_NOTICES.md` 注明了来源与许可
- [ ] 没有引入网络请求，没有读取 `.gitignore`

## 三条硬规则（AGENTS.md §5.2，评审时会逐条对照）

- [ ] 新增的命令（VS Code 命令 ID）、设置项、颜色键、界面字符串、视图与状态栏条目，一律通过 #1 的登记机制注册，**没有写死**
- [ ] **没有在本 PR 里修改 #1 的契约**（需要修改时单独提 PR，说明原因与影响面）
- [ ] 保证 #2 的端到端测试继续通过

## 验收标准逐条对照

<!-- 从 issue 正文复制验收标准，逐条写清「怎么验证 / 结果」。 -->

- [ ] ……

## 验证方式

<!-- 贴命令与关键输出：测试名、基准数据、截图等。需要 Unity 测试项目的测试，写清是否设置 ROVE_TEST_UNITY_* 后运行。 -->

```powershell
cargo test --workspace
```

## 并行开发

- [ ] 已 `git fetch && git rebase origin/main`
- [ ] 只改了自己负责的部分；热点文件（`Cargo.toml`、`lib.rs`、组装代码、`.github/core-crates.txt`）只有追加
- [ ] `Cargo.lock` 若有冲突，按 AGENTS.md §6 取 main 的版本再 `cargo build` 重新生成（不手工解决）
- [ ] 新增 crate 已在 `.github/core-crates.txt` 或 `.github/ui-crates.txt` 追加一行
