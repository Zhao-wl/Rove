# C# 导航准确率基线（#39 建立，#44 复测，#212 补齐运算符与索引器后重测）

> 生成：2026-11-16，`cargo run --release -p rove-lang-cs --example nav_check`，查询来源 `l2`（完整语义索引）。
> 索引用本分支的 `tools/rove-indexer-cs`（RVCSIDX **v2**，索引进程 **1.2.0.0**）重建。
> 机器可读的同一份结果：[`baseline-l2.json`](baseline-l2.json)，下次运行可用 `--baseline tests/nav-cases/baseline-l2.json` 对比。
> 近似层（L0/L1）的同一组用例见 [`baseline-l1.md`](baseline-l1.md)。

## 结论

200 例：通过 **194**、失败 6、过期 0，准确率 **97.0%**。**类型引用（#37 验收口径）20/20。**
比 #44 当天的 87.0%（174 通过）多 20 例：`operator` 与 `indexer` 两类由 0/10、0/10 变成 **5/5、5/5**
（两个项目各 5 例），因为 #212 给用户定义的运算符与索引器补上了使用处与声明处的出现位置。
与 #44 的基线逐例对比里只有「失败 → 通过」，没有任何「通过 → 失败」。

剩下的 6 例全部是 `#if` 不生效的一侧，两个项目对称：

| 失败原因 | 用例数 | 负责 issue | 说明 |
|---|---:|---|---|
| `#if` 不生效一侧没有语义信息 | 6（conditional 的 inactive 一侧） | 设计如此 | 不生效代码不参与编译，L2 不可能给出精确结果；这一侧由 L0/L1 的近似结果兜底，#38 的路由会把它标成「近似」（`INACTIVE_REASON`）。生效一侧 6/6 通过 |

不生效一侧用例的期望值是「读源码时认为应该去的地方」，是给近似层的目标；精确层（L2）在这一侧拿不到结果属于正常。

**大型项目的文件数从 #44 当天的 21,908 变成 21,677**：测试项目是真实项目，期间有人删改过代码。
200 个用例没有一例因此过期（运行器按锚点重定位，`过期 0`）。

## 索引前后对比（#212）

同一台机器、同一天、`before` 与 `after` 交替各跑两轮全量索引（`before` = `origin/main` 的索引进程，
`after` = 本分支）。**本机负载较高**（同时开着 Unity 等程序），耗时只看相对变化；
空闲机器上的复测记为人工事项。

| 项目 | 指标 | before | after | 变化 |
|---|---|---:|---:|---|
| 大型（21,677 个 .cs） | 索引体积 | 197,691,640 B | **199,539,304 B** | **+1,847,664 B（+0.93%）** = 190.3 MiB，目标 < 200 MB 仍然满足 |
| | 出现位置 | 6,049,054 | **6,142,673** | +93,619（+1.55%），其中运算符 / 索引器使用处 90,112 条 |
| | 全量耗时 | 88.5 / 91.3 s | 95.1 / 87.6 s | 在噪声内（目标 < 90 s，余量很小，见下） |
| | 峰值工作集 | 2,906 / 2,848 MB | 2,858 / 2,952 MB | 在噪声内 |
| 中型（7,718 个 .cs） | 索引体积 | 76,580,040 B | **77,668,928 B** | +1,088,888 B（+1.42%） |
| | 出现位置 | 1,925,603 | **1,961,795** | +36,192（+1.88%），其中使用处 32,938 条 |
| | 全量耗时 | 47.2 / 40.7 s | 45.4 / 43.7 s | 在噪声内 |
| | 峰值工作集 | 1,555 / 1,550 MB | 1,629 / 1,578 MB | 在噪声内 |

体积说明：只记**用户定义**的运算符与索引器（内建运算符与数组下标不记），所以增量集中在
真正能跳转的位置上，大型项目 +0.93%。按 MiB 口径（< 209,715,200 B）与十进制口径
（< 200,000,000 B）都仍在 200 MB 以内，但十进制口径的余量只剩 **0.23%**（461,000 B 左右），
**下一次往出现位置里加东西之前必须先看这一条**。耗时在 90 s 的门槛附近波动，与 #44 的结论一致：
本机负载较高时就会贴线，空闲机器上 #33 实测 72–75 s。

查询来源：`l2`

- medium：完整语义索引（格式 v2，索引进程 1.2.0.0，7718 个文件）
- large：完整语义索引（格式 v2，索引进程 1.2.0.0，21677 个文件）

## 分类准确率

### 项目 medium

| 分类 | 通过 | 失败 | 过期 | 不支持 | 准确率 |
|---|---:|---:|---:|---:|---:|
| var 推断 (`var`) | 6 | 0 | 0 | 0 | 100.0% |
| 链式调用 (`chain`) | 6 | 0 | 0 | 0 | 100.0% |
| 方法重载 (`overload`) | 6 | 0 | 0 | 0 | 100.0% |
| 扩展方法 (`extension`) | 7 | 0 | 0 | 0 | 100.0% |
| 泛型 (`generic`) | 7 | 0 | 0 | 0 | 100.0% |
| 继承与重写 (`inheritance`) | 6 | 0 | 0 | 0 | 100.0% |
| 接口实现 (`interface`) | 6 | 0 | 0 | 0 | 100.0% |
| partial (`partial`) | 6 | 0 | 0 | 0 | 100.0% |
| Lambda / LINQ (`lambda`) | 6 | 0 | 0 | 0 | 100.0% |
| 条件编译 (`conditional`) | 3 | 3 | 0 | 0 | 50.0% |
| 外部符号 (`external`) | 6 | 0 | 0 | 0 | 100.0% |
| 局部变量与参数 (`local`) | 6 | 0 | 0 | 0 | 100.0% |
| 事件 / 委托 (`event`) | 6 | 0 | 0 | 0 | 100.0% |
| 运算符重载 (`operator`) | 5 | 0 | 0 | 0 | 100.0% |
| 索引器 (`indexer`) | 5 | 0 | 0 | 0 | 100.0% |
| 类型引用（#37 验收） (`type_ref`) | 10 | 0 | 0 | 0 | 100.0% |
| **合计** | 97 | 3 | 0 | 0 | 97.0% |

### 项目 large

| 分类 | 通过 | 失败 | 过期 | 不支持 | 准确率 |
|---|---:|---:|---:|---:|---:|
| var 推断 (`var`) | 6 | 0 | 0 | 0 | 100.0% |
| 链式调用 (`chain`) | 6 | 0 | 0 | 0 | 100.0% |
| 方法重载 (`overload`) | 6 | 0 | 0 | 0 | 100.0% |
| 扩展方法 (`extension`) | 7 | 0 | 0 | 0 | 100.0% |
| 泛型 (`generic`) | 7 | 0 | 0 | 0 | 100.0% |
| 继承与重写 (`inheritance`) | 6 | 0 | 0 | 0 | 100.0% |
| 接口实现 (`interface`) | 6 | 0 | 0 | 0 | 100.0% |
| partial (`partial`) | 6 | 0 | 0 | 0 | 100.0% |
| Lambda / LINQ (`lambda`) | 6 | 0 | 0 | 0 | 100.0% |
| 条件编译 (`conditional`) | 3 | 3 | 0 | 0 | 50.0% |
| 外部符号 (`external`) | 6 | 0 | 0 | 0 | 100.0% |
| 局部变量与参数 (`local`) | 6 | 0 | 0 | 0 | 100.0% |
| 事件 / 委托 (`event`) | 6 | 0 | 0 | 0 | 100.0% |
| 运算符重载 (`operator`) | 5 | 0 | 0 | 0 | 100.0% |
| 索引器 (`indexer`) | 5 | 0 | 0 | 0 | 100.0% |
| 类型引用（#37 验收） (`type_ref`) | 10 | 0 | 0 | 0 | 100.0% |
| **合计** | 97 | 3 | 0 | 0 | 97.0% |

**全部**：200 例，通过 194、失败 6、过期 0、不支持 0，准确率 97.0%。
**类型引用（#37 验收口径）**：20 例，通过 20、失败 0，准确率 100.0%。

## 失败用例

- **m-conditional-04**（medium · 条件编译 · 失败）`Assets/Football/ThirdParty/UIParticle/Runtime/UIParticleRTDebugLogger.cs:2553:70`
  - 定义：期望 [Assets/Football/ThirdParty/UIParticle/Runtime/UIParticleRTDebugLogger.cs:24]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：期望 2 处，实际 0 处
- **m-conditional-05**（medium · 条件编译 · 失败）`Assets/Football/ThirdParty/UIParticle/Runtime/UIParticleRTDebugLogger.cs:2929:77`
  - 定义：期望 [Assets/Football/ThirdParty/UIParticle/Runtime/UIParticleRTDebugLogger.cs:2717]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：期望 2 处，实际 0 处
- **m-conditional-06**（medium · 条件编译 · 失败）`Assets/Football/ThirdParty/UIParticle/Runtime/Internal/ProjectSettings/PreloadedProjectSettings.cs:177:37`
  - 定义：期望 [Assets/Football/ThirdParty/UIParticle/Runtime/Internal/ProjectSettings/PreloadedProjectSettings.cs:131]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/ThirdParty/UIParticle/Runtime/Internal/ProjectSettings/PreloadedProjectSettings.cs:177, Assets/Football/ThirdParty/UIParticle/Runtime/Internal/ProjectSettings/PreloadedProjectSettings.cs:197]（实际共 0 处）
- **l-conditional-02**（large · 条件编译 · 失败）`Assets/X1/ScriptCore/Common/UI/Component/FontEffect/CustomGradient.cs:101:5`
  - 定义：期望 [Assets/X1/ScriptCore/Common/UI/Component/FontEffect/CustomGradient.cs:33]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptCore/Common/UI/Component/FontEffect/CustomGradient.cs:97]（实际共 0 处）
- **l-conditional-04**（large · 条件编译 · 失败）`Assets/TFWCore/Script/UI/Component/TFWCanvasScaler.cs:25:13`
  - 定义：期望 [Assets/TFWCore/Script/UI/Component/TFWCanvasScaler.cs:29]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/UI/Component/TFWCanvasScaler.cs:21]（实际共 0 处）
- **l-conditional-06**（large · 条件编译 · 失败）`Assets/TFWCore/Script/UI/Component/TFWInputField.cs:141:17`
  - 定义：期望 [Assets/TFWCore/Script/UI/Component/TFWInputField.cs:25]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/UI/Component/TFWInputField.cs:136]（实际共 0 处）
