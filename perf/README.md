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

# 4) 体积报告
cargo build --release -p rove-app
cargo xtask check-size                    # 报告相对 M0 基线（6.2 MB）的变化

# 5) 冷启动（首帧 + 可交互）与空窗口常驻内存
cargo xtask startup                       # 默认启动 release 版 rove.exe 5 次，
                                          # 用 target/perf-project 这个临时小项目，
                                          # 再用不带路径参数的空窗口采 3 次内存
cargo xtask startup --save target/startup.json
cargo xtask startup --no-memory           # 只要启动耗时

# 6) 一次会话的网络监视（#150；rove.update.mode = none 下必须零出站连接）
cargo xtask netwatch --mode none --seconds 60 --project <Unity 项目> --require-silent

# 7) 发布门槛里机器能判的两条（预算没有 pending、验收报告逐行有结论）
cargo xtask check-release-gate
```

`cargo xtask bench` 也会顺带做启动测量（`startup/first_frame`）；没有 release 构建时跳过，
或加 `--no-startup` 显式关掉。

**「可交互」的埋点在 #150 补上了**：`rove-ui` 的 `skeleton::workbench::startup_probe`
在首帧之后注入一个无副作用的按键（`right`）并等包含这次响应的那一帧画完，打印
`INTERACTIVE_SINCE_PROCESS_MS`。埋点**默认关闭**，`cargo xtask startup` 起子进程时才设
`ROVE_STARTUP_PROBE=1`，普通使用的 `rove.exe` 不受影响。按键经 `Window::dispatch_keystroke`
注入，**不含 Windows 键盘消息的排队时间**（口径同 #47 的 `typing/key_to_frame`）。
预算 `cold_start_interactive` 因此改成 `enforced`，`bench = startup/interactive`；
首帧（`startup/first_frame`）仍然只记录、不设门槛。

**空窗口常驻内存**（`empty_window_memory`）也由 `cargo xtask startup` 采集：启动**不带任何路径参数**
的 `rove.exe`（空窗口 + 欢迎页，临时用户目录里 `rove.update.mode = none`），首帧之后稳定 2 s
（`--startup-settle` 可调）再按**进程树**取三个数，取多次的中位数（按工作集排序）：

| 结果 id | 计数器 | 判定 |
|---|---|---|
| `startup/empty_window_working_set` | `Win32_Process.WorkingSetSize` | **判定口径**：`empty_window_memory` < 60 MB |
| `startup/empty_window_private_working_set` | `Win32_PerfRawData_PerfProc_Process.WorkingSetPrivate` | 只记录（`empty_window_private_working_set`） |
| `startup/empty_window_private` | `Win32_Process.PrivatePageCount` | 只记录（`empty_window_private_commit`） |

三个数都要写进报告。性能计数器类拿不到时（被禁用、权限不足）私有工作集这一项不写进结果，
按「未测量」报告，**不要当成 0**。

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
3. **内存区分工作集与私有提交**（口径 2026-11-16 由 #219 写细，数值未改）
   - 三个数各是什么：
     - **工作集**（`WorkingSetSize`）：当前**驻留在物理内存**里的页，含共享页（exe / DLL / 映射文件）；
     - **私有工作集**（`WorkingSetPrivate`）：工作集里私有的那部分，任务管理器的「内存(专用工作集)」列；
     - **私有提交**（`PrivatePageCount`，commit charge）：已向系统**预订**的私有地址空间，
       **不要求驻留物理内存**——提交了却从未被触碰、或者已经被换出的页都算在里面。
   - 打开 1 GB 文件时，工作集包含 mmap 的**文件页**（可回收），不等于真实占用；
   - **「常驻内存」一类的目标判定工作集**（`PROJECT_CHARTER` §6 的「空窗口常驻内存 < 60 MB」）：
     这条目标是按 M0 原型的 47.7 MB **工作集**定的，换成私有提交判定等于换了基线却不换数值，
     而且私有提交不是「常驻」量。要给私有提交单独定目标值，走 `PROJECT_CHARTER` 的修订流程另开 issue；
   - 报告里**三个数都要写**，并注明采样时刻（空闲多久之后）与当时的机器负载。
     私有提交 − 私有工作集就是「已提交但没有驻留」的那部分（图形栈与分配器的预留主要在这里）：
     空窗口上这个差额很大是正常的，不说明有泄漏，也不该拿它和 60 MB 比。

另外：

- **机器差异必须写清**。参考设备是 8 核 / 16 GB / SSD / 集显；开发机是 16 核 + 独显 + D 盘机械硬盘
  （`M0_LANG_SPIKE_REPORT` 里 3.4 s 的冷读就是这么来的）。同一份数字在两台机器上的含义不同。
- **共享缓存未命中的数字最重要**。`small.txt` 是乐观值，`uniq.txt`（每行唯一）才是真实的整形成本。
  M0 结论：可见行全部重新整形时，GPUI 与自研栈都守不住 120 fps（帧间隔 p99 ≈ 16.8 ms），
  所以 `uniq.txt` 的「全唯一行滚动」是一条正式基准，不允许因为「不好看」删掉。

<!-- #215 追加 -->

### 按键到上屏（#215 修订，`typing` 组）

`PROJECT_CHARTER` §6 的「按键到上屏 < 8 ms」在 2026-11-16 由 #215 重新定义了**口径**（走章程修订流程，
**数值没有改动**，也没有动 `budgets.toml` 里的任何数字）。原因是：按老口径这条指标在 120 Hz 上
**结构上不可能达到**，而 Rove 自己的开销离 8 ms 还差一个数量级。

**这条路径在 Windows 上长什么样**（源码位置都在 `vendor/gpui/src/platform/windows/`）：

1. `platform.rs::begin_vsync_thread` 起一条 `VSyncProvider` 线程，`DwmFlush()` 等到垂直同步，
   然后对每个窗口 `RedrawWindow(RDW_INVALIDATE)`；
2. UI 线程收到 `WM_PAINT` → `events.rs::draw_window` → `window.rs` 的 `request_frame`；
3. `request_frame` 先跑 `on_next_frame` 回调，**再**绘制（`window.rs` 的 `draw` + `present`）；
4. `directx_renderer.rs::present` 用的是 `Present(0, …)`——**不等垂直同步**，交给 DWM 合成。

两个结论直接回答了 #215 的问题：

- **不需要「在 prepaint 之前再采样一次输入」的钩子**。键盘消息（`WM_KEYDOWN` / `WM_CHAR`）与
  前台任务（`dispatcher.rs` 用 `PostMessageW` 投递）在 Win32 消息队列里都**优先于 `WM_PAINT`**
  （`WM_PAINT` 是合成出来的最低优先级消息，队列里还有别的消息时 `GetMessage` 不会返回它）。
  也就是说：一帧开始绘制之前，这一帧之前到达的按键**已经**被吸收了。能省的半帧并不存在。
- **省不掉的是「空等」**：按键在一帧里的哪个相位到达是随机的，到达之后必须等到下一次垂直同步
  才会有人绘制。这段空等的均值是半个刷新间隔，p99 接近一个刷新间隔（120 Hz 下 8.33 ms）。

所以一次按键的墙钟时间分成三段，基准（`xtask/benches/typing.rs`）分别记：

| criterion id | 含义 | 谁决定 |
|---|---|---|
| `typing/key_vsync_wait` | 按键到达 → 下一次垂直同步的**空等** | 显示器刷新率 + 按键到达相位 |
| `typing/key_handling` | 按键处理（事务 + 视图增量更新 + 事件） | Rove |
| `typing/frame_cpu` | 这一帧里代码视图的绘制 CPU（prepaint → paint） | Rove |
| **`typing/key_active`** | **判定口径**：处理 + 整帧绘制（帧回调开始 → 代码视图 paint 结束），不含空等 | Rove |
| `typing/key_to_paint` | 墙钟：按键进入 → 这一帧 paint 结束（= 上面三段之和） | 两者都有 |
| `typing/key_to_frame` | **#47 的旧口径**，终点取「这一帧之后的下一次帧回调」，只为和历史数据对比 | 两者都有 |

- 预算 `keystroke_to_paint`（8 ms、p99、`enforced`）判的是 `typing/key_active`；
- 墙钟记在 `keystroke_to_paint_wall`（`recorded`，不设门槛）；
- 另有一条**不靠计时器**的硬要求：按键必须被**紧接着的那一帧**吸收（不多等一帧）。基准打印
  「按键被第几帧吸收」，判定靠 `FrameStats::text_version`（这一帧画的是哪个版本的文本）；
  与机器无关的那部分由 `cargo test -p rove-ui --test code_view_latency` 钉住。
- 进程内测不到的部分要在报告里写清：注入走 `Window::dispatch_keystroke`，**不含** Windows 键盘消息
  的排队时间与输入法；`Present(0)` 之后还有 DWM 合成与扫描输出（**至少再一个刷新间隔**）。
  真·「光子到眼睛」的数字只能用外部测量，记在人工事项里。

**输入采样相位**：#47 的基准在帧回调**里**注入按键，等于把相位固定在 0（刚过垂直同步、这一帧
还没开始绘制）——真实按键不可能总落在这里，而且这个相位会让「到下一次帧回调」的旧口径正好等于
一个完整的刷新间隔。现在默认把注入点**均匀铺开**在一个刷新间隔里（黄金比例低差异序列，上限取
间隔的 80%，见 `xtask/src/typing_plan.rs`，单元测试跟 `cargo test -p xtask` 跑）：帧回调之后、
这一帧绘制完成之后由前台任务自旋到目标相位再注入，所以等待不会推迟本帧的绘制。

```powershell
cargo bench -p xtask --bench typing                              # 默认：相位均匀铺开
$env:ROVE_TYPING_PHASE = "frame-start"; cargo bench -p xtask --bench typing   # 复现 #47 的固定相位
```

开发机（120 Hz，**机器负载较高**：同时开着 Unity 与另一个构建槽）上两种相位各 400 个样本：

| 口径 | #47 相位（帧回调里） | #215 相位（均匀铺开） |
|---|---:|---:|
| 旧口径 `key_to_frame` p99 | 10.403 ms（mean 8.326） | 16.718 ms（mean 13.335） |
| 墙钟 `key_to_paint` p99 | 2.083 ms（mean 1.072） | 9.593 ms（mean 5.990） |
| **判定 `key_active` p99** | **2.083 ms**（mean 1.072） | **2.289 ms**（mean 1.375） |
| 空等 p99 | 0（相位恒为 0） | 7.859 ms（mean 4.616） |
| 不多等一帧 | 400/400 | 400/400 |

旧相位 + 旧口径跑出来的 8.326 ms / p99 10.403 ms，与改动前同一台机器上的 8.327 ms / p99 9.727 ms
一致——说明改写没有改变旧数字测的是什么，只是把终点从「下一次帧回调」挪回了「这一帧 paint 结束」，
并把相位铺开。绝对值只作相对比较用，空闲机器上的复测是人工事项。

## CI 里的行为

- **不做绝对判定**。托管机器的波动比 15% 还大；绝对指标在本地参考设备上测量并记入报告。
- `perf.yml` 在 `main` 的 push 上跑一遍基准并把结果上传成 artifact（`perf-baseline`）；
  PR 打上 `perf` 标签（之后每次 push 也会重跑）时下载最近一次 `main` 的 artifact 作为基线，
  逐项比较，**退化超过 15% 报警**（`::warning::`，不失败）。还没有 `main` 基线时，
  第一次运行会顺手建立基线，之后只与本分支上一次 perf 运行比较（绝不拿别的分支当基线）。
- 判定口径：导航类预算（`goto_definition` / `find_references`）用 **p99**（criterion 的
  `sample.json` 逐次迭代耗时），与 CHARTER / M0 一致；其他用 `mean`。要改口径在
  `budgets.toml` 里加 `aggregate = "p99"`。
- 相对比较会跳过低于 100 ns 的用例（`--compare-floor-ns` 可调）：托管 runner 的噪声比这种
  量级的信号大得多，当前这些用例又都是占位实现（`EmptyText` / `EmptyWorkspace`）。
- CI **不生成 `big1g.txt`**（1 GiB），只跑 `small.txt` / `uniq.txt` 级别的用例。
- `ci.yml` 的「体积守护」任务构建 release 版 `rove.exe` 并运行 `cargo xtask check-size`
  （结果同时以 `::notice::` 出现在 PR 的检查注解里），然后运行 `cargo xtask startup` 采集首帧。
  `[size].baseline_bytes` 用来报告主程序相对 M0 基线的变化。

## 预算怎么加 / 怎么改

- 新增指标：在 `budgets.toml` 加一条 `[[budget]]`，写清 `owner`（负责达标的 issue）。
  可以被 `cargo xtask bench` 测到的，把 `bench` 写成 criterion 的用例 id（`<组>/<用例>`），
  `aggregate` 选 `mean`（默认）或 `p99`；
- 暂时达不到：`status = "pending"` + `reason`，**不要**为了让 CI 绿把数值调大；
  但 **v0.1 起发布时不允许还有 `pending` 条目**（`docs/MILESTONE_v0.1.md` §5），
  `cargo test -p xtask` 与 `cargo xtask check-release-gate` 都会挡住；
- 单位与判定路径必须自洽：时间类条目（`ns` / `us` / `ms` / `s`）的 `bench` 指向 criterion 的用例 id；
  **内存一类的非时间指标**（目前只有 MB）的 `bench` 指向结果文件 `metrics` 表里的 id
  （`cargo xtask startup` 写进去的那些），单位就是条目自己的 `unit`；
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
