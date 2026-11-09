# Rove M0 UI 渲染方案验证报告

> 版本：v1.0 · 日期：2026-11-09
> 对应里程碑：M0 技术验证 ①「UI 方案对比」
> 结论对应立项文档 [PROJECT_CHARTER.md](PROJECT_CHARTER.md) 第 9 节待定事项「M0 结论：UI 方案」

---

## 1. 结论摘要

**建议采用 Route A（GPUI），但必须附带 5 项约束条件**（详见第 7 节）。

核心理由：在两个功能等价的原型上实测，GPUI 的**冷启动（中位 396 ms vs 684 ms）**、**空闲内存（47.7 MB vs 144 MB）**、**Windows 文本渲染正确性（DirectWrite 原生）** 三项都明显优于自研栈；更关键的是它自带 flexbox 布局、焦点管理、键位绑定、列表虚拟化、滚动条等一整套控件层，而 M1「阅读器」需要文件树 / 多标签 / 分屏 / 快速打开 / 搜索面板 / 大纲 / 面包屑 —— 这些在 Route B 里全部要从裸四边形开始手写。

Route A 的明显劣势是**每帧 CPU 开销是 Route B 的 2.2–5.5 倍**（缓存未命中 / 命中），以及 411 个编译期依赖、469 s 的 clean build、强制链接的 HTTP/TLS 栈。

两项必须单独决策的事实：

- **两条路线都达不到立项文档的 150 ms 冷启动目标**（396 ms / 684 ms）。
- **当可见行全部需要重新整形时，两条路线都守不住 120 fps**（帧间隔 p99 均约 16.8 ms，即每 100 帧丢 1 帧）。这是架构问题而非选型问题：整形必须离开绘制关键路径。

---

## 2. 验证方法与环境

### 2.1 硬件与显示

| 项 | 值 |
|---|---|
| CPU | 16 逻辑核 |
| 内存 | 48 GB |
| GPU | NVIDIA GeForce GTX 1060 6GB（主显示器，**120 Hz**）+ Intel UHD 750 |
| 显示缩放 | 100%（scale factor = 1.0，本机无法验证高 DPI，见 4.6） |
| OS | Windows 11 Pro 10.0.22631 |

> 注意：主显示器 120 Hz，所以「平滑滚动」的理想帧间隔是 **8.33 ms**，不是 16.7 ms。下文所有 `frame interval` 数据都应对照 8.33 ms 读。

### 2.2 工具链（含一个必须记录的坑）

| 项 | 值 |
|---|---|
| rustc / cargo | 1.97.1 (8bab26f4f 2026-07-14) |
| 实际使用的 toolchain | `stable-x86_64-pc-windows-msvc`（**不是本机默认的 gnu**，见坑 1） |
| 链接器 | MSVC 14.50.35717（VS 2026，`vcvars64.bat`） |
| 构建 profile | `release` + `lto = "thin"` + `codegen-units = 1`（与主仓库 root `Cargo.toml` 一致） |

**坑 1（必读）**：本机 rustup 默认 toolchain 是 `stable-x86_64-pc-windows-gnu`，**两条路线都无法用 gnu toolchain 构建** —— `windows-sys` / `windows-result` 需要 `dlltool.exe`，`ring` 需要 `gcc.exe`，都不存在：

```
error: error calling dlltool 'dlltool.exe': program not found
error: could not compile `windows-result` (lib)
warning: ring@0.17.14: ToolNotFound: failed to find tool "gcc.exe"
```

两个原型里都放了 `rust-toolchain.toml` 固定到 msvc。**主仓库 `crates/` 后续也应当固定 msvc toolchain**，否则新同事 clone 下来直接构建失败。

**坑 2**：本机访问 `index.crates.io` 时 schannel 报 `CRYPT_E_REVOCATION_OFFLINE (0x80092013)`，cargo 完全无法拉取索引。原因是 Windows 证书吊销缓存里没有 GlobalSign 的 CRL，而 CryptoAPI 在握手内拉取失败。用 `certutil -verify -urlfetch <cert>` 预热一次吊销缓存后恢复正常（**没有**关闭任何证书校验）。GitHub 在本机始终不可达，所以 **Route A 的「pinned git rev of zed-industries/zed」备选方案无法验证**，只验证了 crates.io 上的 `gpui` 包。

### 2.3 两个原型

位置：`spikes/ui-gpui/`、`spikes/ui-wgpu/`（通过 root `Cargo.toml` 的 `exclude = ["spikes"]` 排除在 workspace 之外，各自独立 `Cargo.lock`）。

两者**功能等价**，都实现：

1. 命令行传入文件路径，只读查看器：等宽字体、行号槽、**仅布局/绘制可见行**（虚拟化）、鼠标滚轮滚动、点击定位光标；
2. `memmap2` 映射 + 行偏移索引：打开时同步索引头部 1 MiB（保证首屏立即可用），其余在**后台线程**按 32 MiB 分块扫描（`memchr`），状态栏实时显示索引进度百分比；
3. 底部一行可编辑缓冲区，专门用来测中文 IME；
4. 混排文本（CJK + ASCII + emoji）字体回退、DPI 感知；
5. 内建测量：`--bench-scroll N --bench-step N` 连续滚动 N 帧后打印帧间隔与每帧 CPU 的 avg/p99/max 然后退出；首帧时打印 epoch 时间戳。

| | Route A (`ui-gpui`) | Route B (`ui-wgpu`) |
|---|---|---|
| `doc.rs`（mmap + 行索引，两边完全相同） | 145 行 | 145 行 |
| `gfx.rs`（wgpu 渲染器 + 字形图集 + WGSL） | — | 508 行 |
| `main.rs`（视图 + 事件 + IME + 测量） | 861 行 | 658 行 |
| **合计** | **1006 行** | **1311 行** |

> 代码量差距被低估了：Route A 的 861 行里大部分是我们自己的查看器逻辑和 IME 样板（后者直接改编自 gpui 自带的 `examples/input.rs`）；Route B 多出来的 508 行只是「把字形画到屏幕上」这一件事，**完全没有任何控件层**。

### 2.4 依赖版本与许可证

**Route A**

| crate | 版本 | 许可证 |
|---|---|---|
| `gpui` | **0.2.2**（crates.io，`default-features = false, features = ["windows-manifest"]`） | **Apache-2.0**（单一许可，非 MIT 双授权） |
| `memmap2` | 0.9 | MIT OR Apache-2.0 |
| `memchr` | 2 | MIT OR Unlicense |

`gpui` 仓库是 `github.com/zed-industries/zed`，主页 `gpui.rs`。crates.io 上**总共只有 7 个版本**（`0.1.0`、`0.1.0-test`、`0.2.0-test.4`、`0.2.0`、`0.2.1-test3`、`0.2.1`、`0.2.2`），即独立发包是很新的事。

Windows 后端走 **DirectWrite + DirectX**（`src/platform/windows/direct_write.rs`、`directx_renderer.rs`、`directx_atlas.rs`），不走 blade/Vulkan。`windows-manifest` feature 会嵌入声明 `dpiAwareness = PerMonitorV2` 的清单，**必须显式打开**（我们关掉了 default features，否则会连带引入 `font-kit` / `wayland` / `x11`）。

**Route B**

| crate | 版本 | 许可证 |
|---|---|---|
| `winit` | **0.30.13**（最新稳定线；0.31.0-beta.4 为预发布，未采用） | Apache-2.0 |
| `wgpu` | **30.0.1**（`default-features = false, features = ["dx12","wgsl"]`） | MIT OR Apache-2.0 |
| `parley` | **0.12.0**（布局/整形，含 `fontique` 0.12 字体库与回退） | Apache-2.0 OR MIT |
| `swash` | **0.2.10**（字形光栅化） | Apache-2.0 OR MIT |
| `pollster` / `bytemuck` / `memmap2` / `memchr` | 0.4 / 1 / 0.9 / 2 | 宽松 |

许可证层面两条路线都可用于闭源商业产品（Apache-2.0 不具传染性）。**注意：`gpui` 是 Apache-2.0 单授权，不是 MIT/Apache 双授权**，若将来有 GPL 兼容性顾虑需单独评估（Apache-2.0 与 GPLv2 不兼容，与 GPLv3 兼容）。

### 2.5 测试数据与测量口径

测试文件由 `spikes/gen_testdata.py` 生成到 `spikes/testdata/`（已 gitignore，**未提交**）：

| 文件 | 大小 | 行数 | 内容 |
|---|---|---|---|
| `small.txt` | 46 KB | 402 | 7 种模式循环：纯 ASCII、中文、中英混排 + emoji（含 ZWJ 家庭、肤色修饰）、日韩、tab 缩进、300 字符长行 |
| `big1g.txt` | **1,073,822,050 B（1.00 GiB）** | **14,290,000** | 类 C# 代码，每 50 行插入一行中文；文本每 2000 行循环一次 |
| `uniq.txt` | 33 MB | 400,000 | **每行文本都唯一**，用于测量真实（缓存未命中）的整形开销 |

测量口径（两个原型完全一致，脚本 `spikes/measure.ps1`）：

- **冷启动到首帧**：launcher 在 `Start-Process` 前记录 UTC epoch ms，进程在「首帧已提交」的回调里打印自己的 epoch ms，两者相减。**包含进程创建 + Rust 运行时初始化 + 框架初始化 + 首帧绘制与呈现**。
- **帧间隔（frame interval）**：连续滚动时相邻两帧「已呈现」回调之间的墙钟间隔。受 vsync 限制，理想值 8.33 ms。
- **每帧 CPU**：一帧中「布局 + 整形 + 生成绘制指令」的耗时，**不含 swapchain acquire / present 的 vsync 阻塞**。
  - Route A：自定义 `Element` 的 `prepaint` 开始到 `paint` 结束。
  - Route B：`redraw()` 开始到调用 `Gfx::render()` 之前。
  - 口径说明：Route B 第一版把 `get_current_texture()` 的 vsync 阻塞算进去了，导致显示 7.2 ms/帧，修正后是 149 µs/帧 —— 这类误读很容易发生，记录在此备忘。
- **内存**：不带 bench 参数启动，空闲 8–14 s 后采样 `WorkingSet64` 与 `PrivateMemorySize64`。
- **行索引完成时间**：从 `Doc::open` 进入到后台线程扫完全文件。

**关于缓存的重要说明**：GPUI 的 `TextSystem` 内部有按 (文本, 字体, 字号) 键控的 `LineLayoutCache`（`src/text_system/line_layout.rs`）。为了可比，Route B 也加了一个等价的「整形行 → 字形四边形」缓存。因此 `small.txt`（7 种模式循环）上的每帧 CPU 是**缓存几乎全命中**的乐观值；`uniq.txt` 上的才是**缓存全未命中**的真实整形成本。两个数都给出。

---

## 3. 指标对比总表

| 指标 | 目标（立项文档 §6） | **Route A：GPUI 0.2.2** | **Route B：winit+wgpu+parley** | 优胜 |
|---|---|---|---|---|
| **冷启动 → 首帧**（small.txt，n=5） | < 150 ms（到可交互） | **356 / 396 / 535 ms**（min/中位/max） | 559 / 684 / 756 ms | **A** |
| 冷启动 → 首帧（1 GB 文件，n=2–3） | — | 433 / 444 / 1108 ms | 603 / 686 / 1038 ms | **A** |
| **空闲内存（46 KB 文件）** | 空窗口 < 60 MB | **工作集 47.7 MB** / 私有提交 98.1 MB | 工作集 144.2 MB / 私有提交 454 MB | **A** |
| 空闲内存（1 GB 文件，索引完成后） | — | 工作集 1175 MB / 私有提交 **205 MB** | 工作集 1276 MB / 私有提交 567 MB | **A** |
| `Doc::open`（mmap + 首屏 1 MiB 索引） | — | 0.18–1.02 ms | 0.15–1.02 ms | 持平（同一份代码） |
| **1 GB 首屏可见**（进程启动算起） | < 1 s | **444 ms**（中位） | 686 ms（中位） | **A** |
| 1 GB 全量行索引完成（页缓存热） | — | 583–598 ms | 826–857 ms | 持平（同代码，差异为调度噪声） |
| 1 GB 全量行索引完成（冷，首次读盘） | — | ~3125 ms | ~3349 ms | 持平（受磁盘带宽限制，约 330 MB/s） |
| **滚动帧间隔 avg**（small，300 帧 ×3 行） | 稳定 120 fps（8.33 ms） | **8.34 ms** | **8.29 ms** | 持平 |
| 滚动帧间隔 p99（small） | — | 10.34–13.67 ms | 10.80–16.39 ms | 持平 |
| 滚动帧间隔 max（small） | — | 11.62–21.61 ms | 11.08–68.73 ms（5 次中 1 次离群） | 略 A |
| 滚动帧间隔 avg（1 GB，400 帧 ×37 行） | — | 8.57 ms | 8.32 ms | 持平 |
| 滚动帧间隔 p99 / max（1 GB） | — | 16.36 / 35.07 ms | 10.73 / 13.75 ms | **B** |
| 滚动帧间隔 avg / p99 / max（1 GB 跨全文件，800 帧 ×20000 行） | — | 8.35 / 11.84 / 17.77 ms | 8.38 / 10.95 / 18.02 ms | 持平 |
| **每帧 CPU avg**（small，缓存命中） | 按键到上屏 < 8 ms | 823 µs | **149 µs** | **B（5.5×）** |
| 每帧 CPU p99（small） | — | 2066 µs | **589 µs** | **B** |
| **每帧 CPU avg**（1 GB，step 37） | — | 4026 µs | **711 µs** | **B（5.7×）** |
| 每帧 CPU avg（1 GB 跨全文件，step 20000） | — | 1318 µs | **535 µs** | **B（2.5×）** |
| **每帧 CPU avg**（`uniq.txt`，**缓存全未命中**，400 帧 ×37 行） | — | 4279 µs | **1962 µs** | **B（2.2×）** |
| 每帧 CPU p99（`uniq.txt`，缓存全未命中） | — | 7804 µs | **4243 µs** | **B** |
| 滚动帧间隔 avg / p99 / max（`uniq.txt`，缓存全未命中） | 8.33 ms | 8.66 / **16.83** / 24.67 ms | 8.47 / **16.86** / 24.34 ms | 持平（**两者都掉帧**） |
| **release 二进制体积** | 发布包 < 30 MB | **6328 KB（6.2 MB）** | 7226 KB（7.1 MB） | **A** |
| **clean build 时间**（依赖已在本地 registry 缓存，16 核） | — | **469.0 s（7 分 49 秒）** | **127.5 s（2 分 08 秒）** | **B（3.7×）** |
| `cargo tree \| wc -l` | — | **990 行** | **311 行** | **B（3.2×）** |
| 依赖包数（`cargo tree` 去重后） | — | **411 个** | **123 个** | **B（3.3×）** |
| `Cargo.lock` 包数（含非 Windows 目标） | — | 588 | 280 | **B** |
| 进程 DPI 感知级别（实测 `GetProcessDpiAwareness`） | PerMonitorV2 | PER_MONITOR_AWARE（V2） | PER_MONITOR_AWARE（V2） | 持平 |
| 中文 IME（Microsoft 拼音，实机驱动验证） | 组合态 + 候选框跟随 | **通过** | **通过** | 持平（细节见 §5） |
| CJK / emoji 字体回退 | 必须 | **通过**（DirectWrite 系统回退 + COLR 彩色 emoji） | **通过**（fontique 回退 + swash COLR/CBDT） | 持平 |
| Tab 字符渲染 | — | **正确**（当空白处理） | **缺陷**：渲染成 `.notdef` 豆腐块 | **A** |
| 多字体混排时基线一致 | — | **一致** | **缺陷**：含 CJK 的行整体下移（回退字体 ascent 不同） | **A** |

---

## 4. 分项分析

### 4.1 冷启动

两条路线都**没有达到 150 ms 目标**，且差距不小：

- Route A 中位 **396 ms**，其中 `Doc::open` 只占 0.2 ms，Rust 运行时初始化可忽略 —— 几乎全部是 GPUI 自身的初始化：DirectWrite 工厂与字体集合、DirectX 设备与 swapchain、taffy 布局树、键位映射表。
- Route B 中位 **684 ms**（5 次 559–756 ms），开销在 wgpu 的 DX12 instance/adapter/device 创建（wgpu 本身就以初始化慢著称）加上 `FontContext::new()` 枚举系统字体。

| | Route A | Route B | 目标 |
|---|---|---|---|
| 冷启动 → 首帧（中位，n=5） | 396 ms | 684 ms | 150 ms |
| 其中 `Doc::open` | 0.25 ms | 0.23 ms | — |

这是本次验证**最需要决策的一项**。可选项：

1. **放宽目标**到 ~300 ms（Route A 现状已接近，优化到 250 ms 可期）；
2. 用启动画面/预热掩盖（违背「轻量」气质，不建议）；
3. 深挖 Route A 的 396 ms 分布（本次未做火焰图），确认字体集合枚举能否延迟到首次整形之后。

> 立项文档写的是「冷启动到**可交互**」，而本报告测的是「到**首帧已呈现**」。首帧呈现后窗口即可响应输入，口径基本一致。

### 4.2 内存

Route A 的 47.7 MB 工作集**达到了 < 60 MB 目标**；Route B 的 144 MB **超标 2.4 倍**，私有提交 454 MB 更是异常。

Route B 的内存去向**本次未做归因**（诚实记录为未验证）。可疑项：2048×2048 RGBA 字形图集（16 MB 显存 + 可能的系统侧副本）、wgpu/DX12 分配器的预留、`fontique` 加载的系统字体库（Microsoft YaHei 约 20 MB）、NVIDIA 驱动的提交量。**在接受 Route B 之前必须先解释清楚这 100 MB**，否则「轻量」的卖点直接不成立。

**1 GB 文件的内存要分开读**：

- 工作集 ~1.2 GB 是 **mmap 的文件页**被后台索引线程全部触达导致的。这些页是文件支持（file-backed）、可回收的，系统内存压力下会被丢弃，不等于真实占用。
- 真正属于应用的是私有提交：Route A **205 MB**、Route B **567 MB**。其中约 **114 MB 是行索引本身**（14,290,000 行 × `u64` 偏移 = 114 MB）。
- **设计结论**：`Vec<u64>` 行索引对 GB 级文件太重。Rove 的 `rove-buffer` 应改为「分块索引」：只存每 N 行（如每 256 行）的 u64 锚点 + 块内 u16/u32 相对偏移，或对 < 4 GB 文件直接用 u32。这与 UI 方案无关，但本次验证把它量化出来了。

### 4.3 大文件打开与行索引

「mmap + 首屏同步索引 1 MiB + 后台分块索引余下部分」这个策略**完全成立**：`Doc::open` 恒定在 1 ms 量级，与文件大小无关；首屏在首帧就能画出来。1 GB 文件「打开到首屏」的 444 ms / 686 ms 里，**文件本身只贡献 1 ms**，其余全是框架启动 —— 所以「打开 1 GB < 1 s」这一目标两条路线都满足，而且是靠架构而不是靠框架。

全量索引耗时：页缓存热时 0.58–0.86 s（memchr + Vec 扩容受限），首次读盘 ~3.1–3.3 s（磁盘带宽约 330 MB/s）。索引未完成期间滚动范围会持续增长，状态栏显示百分比 —— 两个原型都验证了这个交互是可以接受的。

### 4.4 滚动流畅度

**两条路线都稳定跑在 120 Hz 上限（avg 8.3 ms），没有掉帧感**。差别在余量：

- `small.txt`（缓存命中）：A 每帧 CPU 823 µs，B 149 µs。
- `big1g.txt` step 37（部分缓存未命中）：A 4026 µs，B 711 µs。
- `big1g.txt` step 20000（跨越整个 1 GB，每帧都要触达新的 mmap 页）：A 1318 µs，B 535 µs，p99 分别 2533 / 1788 µs，max 17.8 / 18.0 ms —— **缺页中断没有造成可感知的卡顿**。

Route A 的 p99 / max 在 1 GB step-37 场景下明显更差（16.4 / 35.1 ms vs 10.7 / 13.8 ms）。

**真实（缓存全未命中）场景是最重要的一组数**。`uniq.txt` 每一行文本都唯一，两边的整形缓存都完全不命中：

| `uniq.txt`，400 帧 × 37 行 | Route A | Route B |
|---|---|---|
| 每帧 CPU avg | 4279 µs | **1962 µs** |
| 每帧 CPU p99 | 7804 µs | **4243 µs** |
| 帧间隔 avg | 8.66 ms | 8.47 ms |
| 帧间隔 **p99** | **16.83 ms** | **16.86 ms** |
| 帧间隔 max | 24.67 ms | 24.34 ms |

p99 ≈ 16.8 ms 意味着**两条路线都会漏掉 vsync**（8.33 → 16.67 ms），约每 100 帧丢 1 帧。也就是说：**当可见的 40 行全部需要重新整形时，两条路线都守不住稳定 120 fps。** 这不是框架选型问题，而是「不能在绘制关键路径上同步整形」的架构结论 —— 必须把整形结果按行缓存，并在滚动方向上预取（见第 7 节约束 3）。

**CPU 差距要打折看**：
1. 我们的 GPUI 原型对每一行都单独调 `shape_line`（行号槽一次、正文一次），并且每帧把 `ShapedLine` 回写进 view —— 这是原型级用法，不是 GPUI 的能力上限。Zed 本身能在大文件上跑高帧率，说明框架层面没有硬上限。
2. 参考设备是「8 核 / 16 GB / 集显」。本机 16 核 + 独显。若 A 的 4.3 ms 在参考设备上翻倍到 8.6 ms，就超出 120 fps 的 8.33 ms 预算 —— **而 M1 之后还要叠加语法高亮的 per-frame 成本**。这是 Route B 唯一真正有分量的优势（缓存未命中时领先 2.2×，命中时领先 5.5×）。

### 4.5 文本渲染质量与字体回退

截图见 `spikes/shots/`（已 gitignore，未提交）：`viewer-gpui.png` / `viewer-wgpu.png`（整窗）、`text-quality-gpui.png` / `text-quality-wgpu.png`（3 倍放大对比）。

**两者都正确完成了**：Consolas 等宽主字体；中文、日文、韩文经系统回退正常显示；emoji（🚀🔥✅😀🇨🇳）**彩色**显示；行号槽对齐；点击定位光标。

**质量差异（3 倍放大目视对比）**：

| | Route A（DirectWrite + swash 无关） | Route B（parley 布局 + swash 光栅化） |
|---|---|---|
| 抗锯齿 | 灰度 AA，字重略粗、对比度更高（DirectWrite 带 gamma / 对比度增强，crate 内有 `alpha_correction.hlsl`） | 灰度 AA，字形略细 |
| 次像素定位 | DirectWrite 内部处理 | **未实现**：字形只在整数相位光栅化一次（`Render::offset(0,0)`），没有做次像素相位缓存。等宽场景影响小，比例字体会劣化 |
| ClearType（次像素 AA） | 未启用（走 alpha 纹理路径） | 未启用 |
| 彩色 emoji | COLR（`TranslateColorGlyphRun`，源码注释 `todo: support formats other than COLR`） | COLR + CBDT（swash `Source::ColorOutline` / `ColorBitmap`） |
| **Tab 字符** | 正确（视为空白） | **缺陷**：渲染成 `.notdef` 豆腐块，需要自己做 tab 展开 |
| **多字体行的基线** | 一致 | **缺陷**：含 CJK 的行整体下移，因为 `GlyphRun::baseline()` 取自该行实际用到的字体度量，回退字体 ascent 更大。代码编辑器必须用主字体度量钉死基线 |

两个缺陷都不难修（tab 展开 + 固定基线各几十行），但它们说明一件事：**DirectWrite 免费替你处理的边界情况，自研栈要一个个踩**。对一个「代码阅读器」而言，这类细节的长尾很长（连字、组合字符、RTL、变体选择符、零宽字符、制表位、全角半角对齐……）。

### 4.6 DPI

- Route A：`windows-manifest` feature 嵌入的清单声明 `<dpiAwareness>PerMonitorV2</dpiAwareness>`。
- Route B：winit 在 `platform_impl/windows/dpi.rs::become_dpi_aware()` 里主动调用 `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`，并对窗口调用 `EnableNonClientDpiScaling`。
- **实测**：对两个运行中的进程调用 `GetProcessDpiAwareness`，均返回 `PROCESS_PER_MONITOR_DPI_AWARE`（HRESULT 0）。
- 两个原型都把 `scale_factor` 接进了布局（Route B 的所有尺寸都乘 `scale`，并在 `ScaleFactorChanged` 时清空整形缓存；Route A 由框架统一处理）。

**未验证**：本机显示缩放为 100%，**没有在 125% / 150% / 多显示器不同缩放比下实际验证过**。立项文档 §7 把「高 DPI 与多显示器不同缩放比」列为 Windows 专项风险，这一项仍然悬空，建议在 M1 用一台 150% 缩放的机器补测。

### 4.7 构建与依赖

- 依赖规模差距是 3.2–3.3 倍（990 vs 311 树行；411 vs 123 个包）。Route A 这 411 个包里包含了相当多与代码阅读器无关的东西：图片解码、SVG、GIF，以及**一整套 HTTP/TLS 栈**。
- **`gpui` 强制依赖一个 HTTP 客户端**（不是可选 feature）：

  ```
  ring ← rustls ← hyper-rustls ← zed-reqwest(0.12.15-zed) ← gpui_http_client ← gpui
  tokio ← h2 ← hyper ← hyper-rustls ← zed-reqwest ← gpui_http_client ← gpui
  ```

  `gpui/Cargo.toml` 里 `[dependencies.http_client] package = "gpui_http_client"` 没有 `optional = true`，无法关掉。对一个「不发起任何网络请求」（立项文档 §4.4）的阅读器来说，链接进 reqwest + hyper + rustls + tokio + ring 是多余的审计面与供应链面。注意 `zed-reqwest` 还是 Zed 自己发布的 reqwest 分支 —— 又一处与 Zed 的耦合。最终二进制仍只有 6.2 MB（链接器剔除了未用代码），所以这是**审计与供应链问题，不是体积问题**。
- `gpui` 在 Windows 上即便关掉 default features，依然会编译 macOS/Linux 相关的若干 crate（core-graphics / cocoa / calloop 等出现在 fetch 列表里，只是最终不参与链接）。
- **clean build：469 s vs 127.5 s（3.7×）**，16 核并行、依赖已在本地 registry 缓存（不含下载时间）。对日常开发影响有限（只改自己的 `main.rs` 时的 release 增量构建：**Route A 64–83 s、Route B 49.7 s**，差距主要来自 `lto = "thin"` + `codegen-units = 1` 下的最终链接），但对 CI 和「换分支 / 改依赖」场景是实打实的成本。
- 二进制体积两者都远小于 30 MB 目标，不构成区分点。
- 编译期还出现一条未来不兼容警告：`proc-macro-error2 v2.0.1`（Route A 依赖链内）。

---

## 5. 中文 IME 验证（Microsoft 拼音）

### 5.1 测试方法（实机驱动，非纸面分析）

本机只装了一种输入法：`zh-Hans-CN` + Microsoft 拼音（TIP GUID `{FA550B04-5AD7-411F-A5AC-CA038EC515D7}`），HKL `0x08040804`。

用 `spikes/ime_test.ps1` 驱动：

1. 启动原型，用 `AttachThreadInput` + `SetForegroundWindow` 把窗口提到前台；
2. **安全阀**：注入任何按键前都校验 `GetForegroundWindow() == 原型窗口句柄`，不相等就直接中止并打印 `ABORTING - no keystrokes sent`（第一次尝试时前台抢夺失败，脚本确实中止了，避免把按键打进别的程序）；
3. 点击底部输入条获取焦点；
4. 用 `keybd_event` + `MapVirtualKey` 提供的**真实扫描码**（只给 VK 不给扫描码时 `ToUnicode` 失败，按键根本到不了应用，这个坑卡了两轮）注入 `Ctrl+Space` 把微软拼音从英文模式切到中文模式 —— 本机注册表 `HKCU\Software\Microsoft\InputMethod\Settings\CHS` 的 `English Switch Key = 2`、`Default Mode = 1`，默认是英文态，单敲 Shift 和单敲 Ctrl 都切不动，只有 Ctrl+Space 有效；
5. 注入 `n i h a o`，等 1.2 s，全屏截图；
6. 注入空格提交首选项，再截图；
7. **再次注入同一个切换键还原用户原本的输入法模式**（切换键是自反的，敲两次回到原状态）。

### 5.2 Route A（GPUI）结果：通过

应用标准输出：

```
IME bounds_for_range 0..0 -> Bounds { origin: Point { x: 8px, y: 775px }, size: Size { 0px × 20px } }
IME preedit: "n"       marked=Some(0..1)  sel=1..1
IME preedit: "ni"      marked=Some(0..2)  sel=2..2
IME preedit: "ni'h"    marked=Some(0..4)  sel=4..4
IME preedit: "ni'ha"   marked=Some(0..5)  sel=5..5
IME preedit: "ni'hao"  marked=Some(0..6)  sel=6..6
IME commit: "浣犲ソ"  buffer="浣犲ソ"      <- 终端用 GBK 代码页解读 UTF-8 的乱码，实际内容是「你好」
```

截图 `spikes/shots/ime-gpui.png` 证实：

- **组合态（preedit）内嵌渲染**在我们自己的输入条里，带下划线，插入符在组合串末尾；
- **候选框正好出现在插入符正下方**，候选项「1 你好 / 2 你号 / 3 拟好 / 4 倪浩 / …」正常；
- **提交插入了正确的「你好」**（控制台里显示为 `浣犲ソ` 只是 UTF-8 被 GBK 代码页解读的终端乱码，缓冲区内容正确）。

### 5.3 Route B（winit + wgpu）结果：通过

应用标准输出：

```
IME enabled
IME preedit: "n"      cursor=Some((1, 1))
IME preedit: "ni"     cursor=Some((2, 2))
IME preedit: "ni'h"   cursor=Some((4, 4))
IME preedit: "ni'ha"  cursor=Some((5, 5))
IME preedit: "ni'hao" cursor=Some((6, 6))
IME preedit: ""       cursor=None
IME commit: "浣犲ソ"                      <- 同上，实际内容是「你好」
IME disabled
```

截图 `spikes/shots/ime-wgpu.png` 证实组合态内嵌 + 下划线 + 候选框跟随插入符 + 提交正确。

（`buffer` 开头多出来的一个空格是我们注入 `Ctrl+Space` 时空格先于模式切换到达应用造成的测试副作用，不是缺陷。）

### 5.4 API 对比与一个真实缺陷

| | Route A（GPUI） | Route B（winit） |
|---|---|---|
| 底层 API | **IMM32**（`WM_IME_STARTCOMPOSITION` / `WM_IME_COMPOSITION` / `ImmGetCompositionString`），**不走 TSF** | IMM32（`platform_impl/windows/ime.rs`），同样不走 TSF |
| 应用侧接口 | `EntityInputHandler` trait：`replace_and_mark_text_in_range`（组合态）、`replace_text_in_range`（提交）、`marked_text_range`、`bounds_for_range`、`selected_text_range`、`character_index_for_point`、`text_for_range`、`unmark_text` | `WindowEvent::Ime(Ime::{Enabled, Preedit(String, Option<(usize,usize)>), Commit(String), Disabled})` + `Window::set_ime_allowed` / `set_ime_cursor_area` / `set_ime_purpose` |
| 组合态光标位置 | 从 `GCS_CURSORPOS` 取，传给 `replace_and_mark_text_in_range` 的 `new_selected_range` | `Preedit` 的 `Option<(usize,usize)>`，winit 还解析了 `GCS_COMPATTR`（可区分已转换/未转换段） |
| 候选框定位 | 框架在 **`WM_IME_STARTCOMPOSITION` 时调用一次** `ImmSetCompositionWindow(CFS_POINT)` + `ImmSetCandidateWindow(CFS_CANDIDATEPOS)`，位置来自 `bounds_for_range` | 应用**任意时刻**可调 `set_ime_cursor_area`，内部同样设置 `COMPOSITIONFORM` + `CANDIDATEFORM`。我们在每帧结束时按插入符位置调用一次 |
| 偏移量单位 | **UTF-16**（需要自己写 UTF-8 ↔ UTF-16 转换，gpui 的 `examples/input.rs` 里有现成样板） | **UTF-8 字节**（更顺手） |

**缺陷（Route A，需要记录在风险里）**：`gpui-0.2.2/src/platform/windows/window.rs:860` 把 `update_ime_position` 实现成了空函数，注释写着：

```rust
fn update_ime_position(&self, _bounds: Bounds<Pixels>) {
    // There is no such thing on Windows.
}
```

这个注释是错的 —— Windows 完全需要在组合过程中重新定位候选框。全仓库搜索 `handle_ime_position` 只有 `WM_IME_STARTCOMPOSITION` 一个调用点，也就是说 **GPUI 在 Windows 上只在组合开始时定位一次候选框，组合串变长后候选框不会跟着插入符移动**。短串（如本次的 `ni'hao`）看不出问题，长串整句输入时会偏。修法很直接（在 `WM_IME_COMPOSITION` 里也调一次 `handle_ime_position`），但需要我们自己 fork 或给上游提 PR。

Route B 不存在这个问题，因为定位权在应用侧。

---

## 6. 风险

### 6.1 Route A（GPUI）

| 风险 | 严重度 | 说明 / 缓解 |
|---|---|---|
| 框架掌握主循环、窗口、渲染器三件套 | **高** | 换渲染后端、改 vsync 策略、接入自定义合成都要动框架。缓解：核心层（buffer / syntax / workspace / lang）保持 UI 无关，UI 层可整体替换 |
| 版本极新、API 不稳定（crates.io 共 7 个 0.x 版本） | **高** | 固定到 0.2.2 并锁 `Cargo.lock`；必要时 vendor 进仓库（Apache-2.0 允许） |
| API 面向 Zed 自身需求演进，与 Rove 诉求未必一致 | 中 | 同上；准备好 fork |
| Windows 后端成熟度低于 macOS（Zed 是 macOS 优先） | 中 | 本次已实测渲染、滚动、IME、DPI 均可用；但 IME 候选框重定位就是一个现成的 Windows 侧 bug |
| 411 个编译期依赖，含大量无关栈（图片/SVG/GIF） | 中 | 仅编译期负担；运行时体积 6.2 MB 可接受 |
| **强制链接 HTTP/TLS 栈**（`gpui_http_client` → `zed-reqwest` → hyper/rustls/tokio/ring，无法关闭） | 中 | 与「不发起任何网络请求」的定位冲突，是审计与供应链面而非体积问题。缓解：fork 后把 `http_client` 改成 optional，或在 CI 加网络系统调用断言 |
| `Pixels` 是 `f32`，大滚动偏移精度不足 | 中 | 1429 万行 × 20 px ≈ 2.86 亿 px，f32 在该量级的分辨率约 32 px → 用 `uniform_list` 这类按像素偏移的内建控件会抖动。**必须自己用 f64 行号做滚动模型**（本原型已这样做） |
| Apache-2.0 单授权（非双授权） | 低 | 闭源商用无碍；与 GPLv2 不兼容 |
| GitHub 在本机不可达，无法回退到 git rev | 低 | crates.io 版本已验证可用 |

### 6.2 Route B（自研栈）

| 风险 | 严重度 | 说明 / 缓解 |
|---|---|---|
| **零控件层** | **极高** | M1 需要文件树、多标签、分屏、快速打开、全局搜索面板、大纲、面包屑、滚动条、右键菜单、焦点与 Tab 序。全部要在裸四边形上手写。本原型 1311 行只换来「只读显示文本 + 一个单行输入框」 |
| 空闲内存 144 MB（目标 < 60 MB）且去向不明 | **高** | 接受该方案前必须先归因并压到 60 MB 以内，否则「轻量」不成立 |
| 冷启动 614 ms（目标 150 ms） | **高** | 比 Route A 还慢 55%；wgpu 初始化慢是已知问题 |
| 依赖全部 pre-1.0，API churn 剧烈 | **高** | 本次亲历：wgpu 29→30 改坏 13 处调用（`Instance::new` 改传值、`InstanceDescriptor` 去掉 `Default`、`PipelineLayoutDescriptor.push_constant_ranges` → `immediate_size`、`buffers: &[Option<VertexBufferLayout>]`、`multiview` → `multiview_mask`、`get_current_texture()` 返回枚举而非 `Result`、`frame.present()` → `queue.present(frame)`、`SurfaceConfiguration` 新增 `color_space`、`RequestAdapterOptions` 新增 `apply_limit_buckets`）；parley 0.12 把 `FontStack` 改名成 `FontFamily`；winit 0.31 仍在 beta。这条路线等于长期维护一条升级跑步机 |
| 文本细节长尾 | 中 | 本次就暴露了 tab 豆腐块、回退字体基线漂移、无次像素定位；后面还有连字、组合字符、RTL、变体选择符、制表位、全半角对齐 |
| 字形图集没有淘汰策略 | 中 | 本原型用 shelf 分配器填满即停（`ATLAS_FULL`）。生产需要 LRU + 重打包 |
| macOS 移植 | 低 | winit / wgpu（Metal）/ parley（fontique CoreText 后端）/ swash 都跨平台；但 macOS IME、菜单、全屏、触控板惯性滚动仍要自己做 |

### 6.3 两条路线共同的风险

| 风险 | 说明 |
|---|---|
| **冷启动均未达标** | 396 ms / 614 ms vs 目标 150 ms。需要单独决策：优化、还是放宽指标 |
| 行索引内存 | 1 GB 文件的 `Vec<u64>` 行索引占 114 MB。与 UI 无关，但必须在 `rove-buffer` 里改成分块索引 |
| 高 DPI 未实测 | 只验证了进程 DPI 感知级别为 PerMonitorV2，没在 125%/150%/混合缩放下跑过 |
| 必须用 msvc toolchain | 本机默认 gnu toolchain 两条路线都编不过 |

---

## 7. 建议

### 采用 Route A（GPUI 0.2.2），附 5 项约束条件

**为什么是 A**

1. **进度风险压倒性**。M0 之后最大的不确定性不是「能不能画出字」，而是「什么时候能有一个能用的阅读器」。GPUI 自带 taffy flexbox 布局、焦点与 Tab 序、键位绑定与 action 分发、`uniform_list` 虚拟列表、滚动条、拖放、图片/SVG。Route B 这些全要手写 —— 本次 1311 行只换来只读文本显示，而 M1 的 UI 面远不止于此。
2. **实测数据站在 A 这边**，而且方向与直觉相反：冷启动快 1.7×（中位 396 vs 684 ms）、空闲内存少 3×（47.7 vs 144 MB，且 A 已达标 B 超标 2.4×）、二进制更小、1 GB 首屏更快。
3. **Windows 文本正确性是免费的**。DirectWrite 处理了 tab、基线、次像素定位、gamma、系统字体回退、COLR emoji。Route B 在第一天就暴露了 tab 豆腐块和基线漂移两个缺陷 —— 这类长尾对「代码阅读器」是持续成本。
4. **macOS 移植路径更可信**。Zed 是 macOS 优先，GPUI 的 Metal 后端是被最充分验证的那条路；而 Route B 的 macOS IME / 菜单 / 惯性滚动要我们从头做。
5. **许可证无障碍**：Apache-2.0，可闭源商用，可 fork。

**为什么不是 B（尽管它每帧 CPU 低 2.5–5 倍）**

每帧 CPU 的差距确实是 B 唯一有分量的优势，但：A 的 823 µs（缓存命中）/ 4026 µs（1 GB）是**我们原型级用法**的数字（逐行 `shape_line`、每帧回写 `ShapedLine`），不是 GPUI 的上限；Zed 本身能在大文件上跑高帧率。而 B 的零控件层、144 MB 内存、pre-1.0 API churn 是**结构性**的，不会因为我们写得更好而消失。

### 必须附带的 5 项约束

1. **固定版本并准备 fork**：锁 `gpui = 0.2.2` + `Cargo.lock`；在 `tools/` 或 `vendor/` 预留 vendor 路径。至少要 fork 修掉 Windows 侧 `update_ime_position` 空实现（§5.4），让候选框在整句输入时跟随插入符。
2. **核心层对 UI 零依赖**：`rove-buffer` / `rove-syntax` / `rove-workspace` / `rove-lang-cs` / `rove-docview` 不得出现 `gpui` 依赖，只有 `rove-ui` / `rove-app` 可以。这样 UI 方案可整体替换，本次两个原型共用同一份 `doc.rs` 已经证明这是可行的。
3. **代码视图自己写 Element + 自己管整形缓存**：
   - 滚动模型用 `f64` 行号偏移，绝不能用 `f32` 像素偏移，所以不能直接用 `uniform_list`（§6.1 精度问题）。
   - **不能在绘制关键路径上同步整形**：实测缓存全未命中时两条路线都会丢帧（§4.4）。需要按「行内容 → 整形结果」缓存 + 沿滚动方向预取（在后台线程或上一帧的空闲时间里预整形），目标是把每帧 CPU 压到 1 ms 以内。
   - 把这一项作为 M1 的性能基准测试用例（CI 里跑 `uniq.txt` 式的全唯一行滚动）。
4. **冷启动单独立项**：用 ETW / 火焰图拆解那 396 ms，确认 DirectWrite 字体集合枚举与 DirectX 设备创建能否延迟或并行化。若 2 周内压不到 250 ms，就把立项文档 §6 的 150 ms 改成一个有依据的值。
5. **保留 Route B 原型作为逃生通道**：`spikes/ui-wgpu` 不要删。如果出现「GPUI 升级打断我们」或「每帧 CPU 在参考设备上吃不下语法高亮」这两种情况，它是现成的对照组和起点。

### 触发重新评估的条件

- 参考设备（8 核 / 16 GB / 集显）上，GPUI 方案在 1 GB 文件 + 语法高亮下每帧 CPU 超过 5 ms；
- GPUI 在 6 个月内发生一次我们无法在 1 周内跟上的破坏性变更；
- GPUI Windows 后端出现我们修不了的渲染或输入缺陷。

---

## 8. 未验证事项（诚实清单）

以下内容**本次没有测量或验证**，不要当成已验证：

1. **高 DPI 实际表现**：只用 `GetProcessDpiAwareness` 确认了两个进程都是 PerMonitorV2，**没有**在 125% / 150% / 多显示器不同缩放下实际跑过，也没验证跨显示器拖动时的重排。
2. **Route B 的 144 MB 内存去向**：未做归因（图集 / wgpu 分配器 / fontique 字体库 / 驱动，占比未知）。
3. **Route A 的 396 ms 冷启动构成**：未做 profiling。
4. **GPUI 的性能上限**：我们测的是原型级用法，不是框架能力上限；也没有尝试 `uniform_list` 等内建虚拟列表的性能（因为 f32 精度问题主动绕开了）。
5. **GPUI 的 pinned git rev 方案**：GitHub 在本机不可达，只验证了 crates.io 的 `gpui 0.2.2`。
6. **IME 的深水区**：只验证了「组合 → 候选框 → 提交」主路径。**未验证**：整句长串输入时候选框是否跟随（源码分析判断 GPUI 会偏，见 §5.4，但未实测）、候选框在屏幕边缘的翻转、组合中途用鼠标点击其他位置、组合中途切换输入法、全角/半角符号、Shift 临时英文、第三方输入法（搜狗 / 微信 / QQ 拼音，本机未安装）、日文假名转换与韩文组字。
7. **编码处理**：GBK / UTF-16 / BOM 的识别与保持，本次原型一律按 UTF-8 处理（非 UTF-8 走 lossy），**完全未验证**。属于 `rove-buffer` 范畴，但 UI 层对双向代理对（surrogate pair）、组合字符的光标移动需要配套验证。
8. **macOS**：两条路线的 macOS 行为**一行代码都没跑过**，相关结论均为源码/文档层面的推断。
9. **编辑能力**：两个原型都只有「只读查看 + 一个单行输入框」，没有 rope、撤销重做、多光标、大文件编辑。
10. **长行性能**：两个原型都把单行截断到 2048 字节（`MAX_LINE_BYTES`）以避免病态行拖垮布局。真实的超长行（minified JS / 单行 JSON）行为未验证。

---

## 附录 A：如何复现

```powershell
# 0) 本机必须用 msvc toolchain（两个原型目录下已有 rust-toolchain.toml）
#    并在 Developer 环境里构建：
#    "D:\Tools\Visual Studio\2026\VC\Auxiliary\Build\vcvars64.bat"

# 1) 生成测试数据（写入 spikes/testdata/，已 gitignore）
cd spikes
python -I gen_testdata.py

# 2) 构建
cd ui-gpui && cargo build --release && cd ..
cd ui-wgpu && cargo build --release && cd ..

# 3) 交互运行
.\ui-gpui\target\release\ui-gpui.exe .\testdata\small.txt
.\ui-wgpu\target\release\ui-wgpu.exe .\testdata\big1g.txt

# 4) 跑指标（启动时间 / 帧时间 / 内存 / 体积）
powershell -File .\measure.ps1 -Exe .\ui-gpui\target\release\ui-gpui.exe `
    -File .\testdata\small.txt -Label gpui-small -Runs 5 -Frames 300 -Step 3

# 5) 截图（窗口置顶后抓屏）
powershell -File .\capture.ps1 -Exe .\ui-wgpu\target\release\ui-wgpu.exe `
    -File .\testdata\small.txt -Png .\shots\viewer-wgpu.png

# 6) 驱动微软拼音做 IME 验证（注入按键前会校验前台窗口，失败即中止）
powershell -File .\ime_test.ps1 -Exe .\ui-gpui\target\release\ui-gpui.exe `
    -File .\testdata\small.txt -Tag gpui -ForceChinese -SwitchKey ctrlspace

# 7) clean build 计时
powershell -File .\cleanbuild.ps1 -Dir (Resolve-Path .\ui-wgpu)
```

## 附录 B：原型内建的测量输出

两个原型都在标准输出打印同一组键值，便于脚本解析：

```
MAIN_EPOCH_MS <u128>             main() 入口的 UTC epoch ms
DOC_OPEN_MS <f64>                Doc::open 耗时（mmap + 首屏 1 MiB 索引）
FIRSTFRAME_SINCE_MAIN_MS <f64>   main() 入口到首帧已呈现
FIRSTFRAME_EPOCH_MS <u128>       首帧已呈现时刻的 UTC epoch ms
SCALE_FACTOR <f32>               窗口 DPI 缩放
FULL_INDEX_MS <u64> LINES <n>    全量行索引完成耗时与总行数
SCROLL_FRAMES <n>
SCROLL_INTERVAL_MS avg= p99= max=    连续滚动的帧间隔
SCROLL_CPU_US avg= p99= max=         每帧 CPU（不含 vsync 阻塞）
ADAPTER <string>                 仅 Route B：实际选中的 GPU 适配器与后端
QUADS_FIRST_FRAME <n>            仅 Route B：首帧四边形数量
IME preedit: ... / IME commit: ...   IME 事件轨迹
```
