# 性能基准与预算（issue #4）

这个目录放**预算**与**采集口径**，不放数据、不放基线。

| 文件 | 内容 |
|---|---|
| `budgets.toml` | `PROJECT_CHARTER` §6 的机器可读版本：每条指标的数值、单位、负责 issue、状态 |
| `README.md` | 采集口径、怎么跑、怎么加基准 |
| `../target/perf-testdata/` | `cargo xtask gen-testdata` 的生成物（1 GiB 数据不进仓库，也不进 CI） |

## 怎么跑

```powershell
# 1) 生成测试数据（默认 target/perf-testdata，已 gitignore；CI 上不要生成 big1g.txt）
cargo xtask gen-testdata                    # 三个文件全生成（big1g 约 1 GiB）
cargo xtask gen-testdata --only small       # 只生成 small.txt
cargo xtask gen-testdata --only uniq        # 只生成 uniq.txt（CI 用这个）

# 2) 跑基准（criterion，统一入口）
cargo xtask bench                                    # 所有可用的基准组
cargo xtask bench --quick                            # 快速模式（CI 用）
cargo xtask bench --cs-index out/medium.idx          # 额外跑索引查询（没有索引就整组跳过）
cargo xtask bench --save target/bench.json           # 保存机器可读结果
cargo xtask bench --baseline target/bench-main.json  # 与基线比较，退化 > 15% 报警
cargo xtask bench --check-budgets --from target/bench.json  # 复核已保存的结果（不重新跑）

# 3) 校验预算（超预算以非零码退出，并打印超出项与幅度）
cargo xtask bench --check-budgets

# 4) 体积守护
cargo build --release -p rove-app
cargo xtask check-size                    # 报告相对 M0 基线（6.2 MB）的变化
```

`cargo xtask` 由 `.cargo/config.toml` 里的 alias 提供，等价于 `cargo run -p xtask --`。

## 采集口径

口径不一致的数字没有意义。以下三条是 M0 已经踩过坑、必须照做的：

1. **冷启动记录两个时间点**
   - **首帧已呈现**：与 M0 口径一致（用来和历史数据对比）；
   - **可交互**：首帧之后自动注入一个按键，记录它被处理并重新绘制完成的时刻。
   - **预算按「可交互」考核**；「首帧」单独记录，不设门槛。M0 实测的 396 ms 只到首帧，
     按新口径可能略超 400 ms 的预算，以实测为准（见 `budgets.toml` 的 `cold_start_*`）。
2. **每帧 CPU 不含 swapchain acquire / present 的 vsync 阻塞**
   - M0 记录过一次因为把 `get_current_texture()` 的等待算进去，把 149 µs 读成 7.2 ms 的教训
     （`M0_UI_SPIKE_REPORT` §2.5）。
   - 帧间隔（frame interval）是另一个指标：相邻两帧「已呈现」的墙钟间隔，受 vsync 限制，
     理想值 8.33 ms。
3. **内存区分工作集与私有提交**
   - 打开 1 GB 文件时，工作集包含 mmap 的**文件页**（可回收），不等于真实占用；
   - 报告里两个数都要写，并注明采样时刻（空闲多久之后）。

另外：

- **机器差异必须写清**。参考设备是 8 核 / 16 GB / SSD / 集显；开发机是 16 核 + 独显 + D 盘机械硬盘
  （`M0_LANG_SPIKE_REPORT` 里 3.4 s 的冷读就是这么来的）。同一份数字在两台机器上的含义不同。
- **共享缓存未命中的数字最重要**。`small.txt` 是乐观值，`uniq.txt`（每行唯一）才是真实的整形成本。
  M0 结论：可见行全部重新整形时，GPUI 与自研栈都守不住 120 fps（帧间隔 p99 ≈ 16.8 ms），
  所以 `uniq.txt` 的「全唯一行滚动」是一条正式基准，不允许因为「不好看」删掉。

## CI 里的行为

- **不做绝对判定**。托管机器的波动比 15% 还大；绝对指标在本地参考设备上测量并记入报告。
- `perf.yml` 在 `main` 的 push 上跑一遍基准并把结果上传成 artifact（`perf-baseline`）；
  手动触发（`workflow_dispatch`）时下载最近一次 `main` 的 artifact 作为基线，逐项比较，
  **退化超过 15% 报警**（`::warning::`，不失败）。
- CI **不生成 `big1g.txt`**（1 GiB），只跑 `small.txt` / `uniq.txt` 级别的用例。
- `ci.yml` 的「体积守护」任务构建 release 版 `rove.exe` 并运行 `cargo xtask check-size`，
  PR 里能看到体积与相对 6.2 MB 基线的变化；超过 `budgets.toml` 的 `[size].budget_bytes` 时失败。

## 预算怎么加 / 怎么改

- 新增指标：在 `budgets.toml` 加一条 `[[budget]]`，写清 `owner`（负责达标的 issue）与 `source`。
  可以被 `cargo xtask bench` 测到的，把 `bench` 写成 criterion 的用例 id（`<组>/<用例>`）；
- 暂时达不到：`status = "pending"` + `reason`，**不要**为了让 CI 绿把数值调大；
- 需要改数值：走 `PROJECT_CHARTER` 的修订流程并留下记录（像冷启动从 150 ms 改到 400 ms 那样），
  然后同步 `budgets.toml`。

## 怎么加一组基准

1. 在 `xtask/benches/` 下新建 `<name>.rs`，用 criterion 写，**只调用 #1 定义的公开接口**；
2. 在 `xtask/Cargo.toml` 加 `[[bench]] name = "<name>" harness = false`；
3. 在 `xtask/src/bench.rs` 的 `TARGETS` 里登记，并写清前置条件（例如需要索引文件）；
4. 需要判定的指标在 `budgets.toml` 里把 `bench` 指向 `<组>/<用例>`，并把 `status` 从
   `pending` 改成 `enforced`。

## 与 M0 数据的关系

`cargo xtask gen-testdata` 的产物与 `spikes/gen_testdata.py` 一致（`small.txt` 46,492 B / 401 行、
`uniq.txt` 33,057,614 B / 400,000 行、`big1g.txt` 1,073,822,050 B / 14,290,000 行，误差 < 1%），
因此新数据可以直接与 `M0_UI_SPIKE_REPORT` / `M0_LANG_SPIKE_REPORT` 里的数字对比。
`gen-testdata` 会自己校验这个误差并打印对照表。
