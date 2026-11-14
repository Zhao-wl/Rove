# C# 导航准确率基线（#39 建立，#44 复测）

> 生成：2026-11-14，`cargo run --release -p rove-lang-cs --example nav_check -- --build`，查询来源 `l2`（完整语义索引）。
> 索引用当天 `main` 的 `tools/rove-indexer-cs`（RVCSIDX **v2**，索引进程 **1.2.0.0**）重建。
> 机器可读的同一份结果：[`baseline-l2.json`](baseline-l2.json)，下次运行可用 `--baseline tests/nav-cases/baseline-l2.json` 对比。
> 近似层（L0/L1）的同一组用例见 [`baseline-l1.md`](baseline-l1.md)。

## 结论

200 例：通过 **174**、失败 26、过期 0，准确率 **87.0%**。**类型引用（#37 验收口径）20/20。**
比 #39 当天的 81.0%（162 通过）多 12 例：`external` 一类由 0/12 变成 **12/12**，因为 #34 的元数据视图已经合并，
外部符号有了落点。其余失败仍然集中在三个已知缺口，两个项目完全对称：

| 失败原因 | 用例数 | 负责 issue | 说明 |
|---|---:|---|---|
| 运算符使用处没有出现位置 | 10（operator 全部） | 见 `docs/M2_ACCEPTANCE_REPORT.md` §6 列出的后续 issue | 索引进程只收集 `SimpleNameSyntax` 与声明标识符，`a + b` 的运算符记号不产生出现位置；`operator` 声明本身也不在 `DeclIdentifier` 里 |
| 索引器使用处没有出现位置 | 10（indexer 全部） | 同上 | 同上：`x[i]` 的元素访问不产生出现位置 |
| `#if` 不生效一侧没有语义信息 | 6（conditional 的 inactive 一侧） | 设计如此 | 不生效代码不参与编译，L2 不可能给出精确结果；这一侧由 L0/L1 的近似结果兜底，#38 的路由会把它标成「近似」（`INACTIVE_REASON`）。生效一侧 6/6 通过 |

不生效一侧用例的期望值是「读源码时认为应该去的地方」，是给近似层的目标；精确层（L2）在这一侧拿不到结果属于正常。

查询来源：`l2`

- medium：完整语义索引（格式 v2，索引进程 1.2.0.0，7718 个文件）
- large：完整语义索引（格式 v2，索引进程 1.2.0.0，21908 个文件）

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
| 运算符重载 (`operator`) | 0 | 5 | 0 | 0 | 0.0% |
| 索引器 (`indexer`) | 0 | 5 | 0 | 0 | 0.0% |
| 类型引用（#37 验收） (`type_ref`) | 10 | 0 | 0 | 0 | 100.0% |
| **合计** | 87 | 13 | 0 | 0 | 87.0% |

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
| 运算符重载 (`operator`) | 0 | 5 | 0 | 0 | 0.0% |
| 索引器 (`indexer`) | 0 | 5 | 0 | 0 | 0.0% |
| 类型引用（#37 验收） (`type_ref`) | 10 | 0 | 0 | 0 | 100.0% |
| **合计** | 87 | 13 | 0 | 0 | 87.0% |

**全部**：200 例，通过 174、失败 26、过期 0、不支持 0，准确率 87.0%。
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
- **m-operator-01**（medium · 运算符重载 · 失败）`Assets/Football/Runtime/Core/MatchComposition/Skills/MatchDribbleCommit.cs:26:31`
  - 定义：期望 [Assets/Football/Runtime/Core/Mathematics/V3.cs:21]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/MatchComposition/Skills/MatchDribbleCommit.cs:26]（实际共 0 处）
- **m-operator-02**（medium · 运算符重载 · 失败）`Assets/Football/Runtime/Core/MatchComposition/Skills/MatchDribbleCommit.cs:35:37`
  - 定义：期望 [Assets/Football/Runtime/Core/Mathematics/V3.cs:20]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/MatchComposition/Skills/MatchDribbleCommit.cs:35]（实际共 0 处）
- **m-operator-03**（medium · 运算符重载 · 失败）`Assets/Football/Runtime/Core/FootballActions/Calculations/BallControlCalculator.cs:53:40`
  - 定义：期望 [Assets/Football/Runtime/Core/Mathematics/V3.cs:22]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/FootballActions/Calculations/BallControlCalculator.cs:53]（实际共 0 处）
- **m-operator-04**（medium · 运算符重载 · 失败）`Assets/Football/Tests/Editor/MatchPresentation/MatchRootMappingTests.cs:22:37`
  - 定义：期望 [Assets/Football/Runtime/Core/Mathematics/Q4.cs:45]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Tests/Editor/MatchPresentation/MatchRootMappingTests.cs:22]（实际共 0 处）
- **m-operator-05**（medium · 运算符重载 · 失败）`Assets/Football/Runtime/Core/Activation.cs:58:44`
  - 定义：期望 [Assets/Football/Runtime/Core/MatchConfiguration/ScenarioId.cs:21]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/Activation.cs:58]（实际共 0 处）
- **m-indexer-01**（medium · 索引器 · 失败）`Assets/Football/Runtime/Core/TeamAI/Formation/AdaptedFormation.cs:173:34`
  - 定义：期望 [Assets/Football/Runtime/Core/TeamAI/Tactics/TeamTactics.cs:12]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/TeamAI/Formation/AdaptedFormation.cs:173]（实际共 0 处）
- **m-indexer-02**（medium · 索引器 · 失败）`Assets/Football/Runtime/Core/TeamAI/Tactics/TeamTactics.cs:18:61`
  - 定义：期望 [Assets/Football/Runtime/Core/TeamAI/Tactics/FormationTactics.cs:27]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/TeamAI/Tactics/TeamTactics.cs:18]（实际共 0 处）
- **m-indexer-03**（medium · 索引器 · 失败）`Assets/Football/Tests/Editor/MatchAnalytics/MatchMetricsCollectorTests.cs:54:39`
  - 定义：期望 [Assets/Football/Runtime/Core/MatchAnalytics/MatchMetricsSummary.cs:20]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Tests/Editor/MatchAnalytics/MatchMetricsCollectorTests.cs:54]（实际共 0 处）
- **m-indexer-04**（medium · 索引器 · 失败）`Assets/Football/Runtime/Core/ActionSelection/SelectionTrace.cs:205:38`
  - 定义：期望 [Assets/Football/Runtime/Core/ActionSelection/SelectionTrace.cs:161]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/ActionSelection/SelectionTrace.cs:205]（实际共 0 处）
- **m-indexer-05**（medium · 索引器 · 失败）`Assets/Football/Runtime/Core/MatchConfiguration/MatchConfigResolver.cs:57:26`
  - 定义：期望 [Assets/Football/Runtime/Core/MatchConfiguration/Provenance/MatchConfigSources.cs:30]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Football/Runtime/Core/MatchConfiguration/MatchConfigResolver.cs:57]（实际共 0 处）
- **l-conditional-02**（large · 条件编译 · 失败）`Assets/X1/ScriptCore/Common/UI/Component/FontEffect/CustomGradient.cs:101:5`
  - 定义：期望 [Assets/X1/ScriptCore/Common/UI/Component/FontEffect/CustomGradient.cs:33]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptCore/Common/UI/Component/FontEffect/CustomGradient.cs:97]（实际共 0 处）
- **l-conditional-04**（large · 条件编译 · 失败）`Assets/TFWCore/Script/UI/Component/TFWCanvasScaler.cs:25:13`
  - 定义：期望 [Assets/TFWCore/Script/UI/Component/TFWCanvasScaler.cs:29]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/UI/Component/TFWCanvasScaler.cs:21]（实际共 0 处）
- **l-conditional-06**（large · 条件编译 · 失败）`Assets/TFWCore/Script/UI/Component/TFWInputField.cs:141:17`
  - 定义：期望 [Assets/TFWCore/Script/UI/Component/TFWInputField.cs:25]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/UI/Component/TFWInputField.cs:136]（实际共 0 处）
- **l-operator-01**（large · 运算符重载 · 失败）`Assets/X1/ScriptGame/Buff/BuffHolder.cs:72:52`
  - 定义：期望 [Assets/X1/ScriptGame/Prop/PropValue.cs:44]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptGame/Buff/BuffHolder.cs:72]（实际共 0 处）
- **l-operator-02**（large · 运算符重载 · 失败）`Assets/X1/ScriptGame/Buff/BuffHolder.cs:112:52`
  - 定义：期望 [Assets/X1/ScriptGame/Prop/PropValue.cs:50]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptGame/Buff/BuffHolder.cs:112]（实际共 0 处）
- **l-operator-03**（large · 运算符重载 · 失败）`Assets/X1/ScriptGame/Prop/PropMgr.Get.cs:163:24`
  - 定义：期望 [Assets/X1/ScriptGame/Prop/PropValue.cs:44]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptGame/Prop/PropMgr.Get.cs:163]（实际共 0 处）
- **l-operator-04**（large · 运算符重载 · 失败）`Assets/X1/ScriptGame/City/CityEntity/Worker/LWorker.cs:342:29`
  - 定义：期望 [Assets/X1/ScriptGame/City/CityEntity/Worker/LWorker.cs:523]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptGame/City/CityEntity/Worker/LWorker.cs:342]（实际共 0 处）
- **l-operator-05**（large · 运算符重载 · 失败）`Assets/Launch/Script/Common/Version.cs:50:24`
  - 定义：期望 [Assets/Launch/Script/Common/Version.cs:43]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/Launch/Script/Common/Version.cs:50]（实际共 0 处）
- **l-indexer-01**（large · 索引器 · 失败）`Assets/TFWCore/Script/Editor/Map/LayerGrid/ContentProviderGridLayerEditor.Export.cs:71:40`
  - 定义：期望 [Assets/TFWCore/Script/Map/Layer/GridMatrix.cs:14]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/Editor/Map/LayerGrid/ContentProviderGridLayerEditor.Export.cs:71]（实际共 0 处）
- **l-indexer-02**（large · 索引器 · 失败）`Assets/TFWCore/Script/Editor/Map/LayerGrid/GridFlagLabelUtility.cs:23:35`
  - 定义：期望 [Assets/TFWCore/Script/Editor/Map/LayerGrid/GridMatrix.cs:11]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/Editor/Map/LayerGrid/GridFlagLabelUtility.cs:23]（实际共 0 处）
- **l-indexer-03**（large · 索引器 · 失败）`Assets/TFWCore/Script/Map/Layer/GridLayer.cs:36:23`
  - 定义：期望 [Assets/TFWCore/Script/Map/Layer/GridMatrix.cs:14]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/TFWCore/Script/Map/Layer/GridLayer.cs:36]（实际共 0 处）
- **l-indexer-04**（large · 索引器 · 失败）`Assets/X1/ScriptCore/Common/Timer/TimerMgr.cs:349:31`
  - 定义：期望 [Assets/X1/ScriptCore/Common/Collections/ExtendArray.cs:36]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptCore/Common/Timer/TimerMgr.cs:349]（实际共 0 处）
- **l-indexer-05**（large · 索引器 · 失败）`Assets/X1/ScriptCore/Common/Collections/SwapExtendArray.cs:95:30`
  - 定义：期望 [Assets/X1/ScriptCore/Common/Collections/ExtendArray.cs:36]，实际 []；绑定到 <该位置没有出现位置记录>
  - 引用：缺少 [Assets/X1/ScriptCore/Common/Collections/SwapExtendArray.cs:95]（实际共 0 处）
