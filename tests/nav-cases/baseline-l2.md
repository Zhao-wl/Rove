# C# 导航准确率基线（#39）

> 生成：2026-11-11，`cargo run -p rove-lang-cs --example nav_check`，查询来源 `l2`（完整语义索引）。
> 索引用当前 `main` 的 `tools/rove-indexer-cs`（RVCSIDX **v2**，索引进程 1.0.2.0）当天重建。issue 正文写的是“v1 索引”，
> 写 issue 时 #32 尚未合并；现在 `main` 上的读取端只认 v2，所以基线以 v2 为准。
> 机器可读的同一份结果：[`baseline-l2.json`](baseline-l2.json)，下次运行可用 `--baseline tests/nav-cases/baseline-l2.json` 对比。

## 结论

200 例：通过 162、失败 38、过期 0，准确率 81.0%。**类型引用（#37 验收口径）20/20。**
失败全部集中在 4 个已知缺口，每个缺口都有对应的后续 issue；其余 12 类在两个项目上均为 100%。

| 失败原因 | 用例数 | 负责 issue | 说明 |
|---|---:|---|---|
| 外部符号没有元数据视图 | 12（external 全部） | #34 | 索引**已绑定到正确的文档注释 ID**（详情里的“绑定到 …”与期望逐字一致），只是 v2 的元数据段为空，跳转没有落点 |
| 运算符使用处没有出现位置 | 10（operator 全部） | 未指派（索引进程的收集范围） | 索引进程只收集 `SimpleNameSyntax` 与声明标识符，`a + b` 的运算符记号不产生出现位置；`operator` 声明本身也不在 `DeclIdentifier` 里 |
| 索引器使用处没有出现位置 | 10（indexer 全部） | 未指派（同上） | 同上：`x[i]` 的元素访问不产生出现位置 |
| `#if` 不生效一侧没有语义信息 | 6（conditional 的 inactive 一侧） | #37 / #38 | 不生效代码不参与编译，L2 不可能给出精确结果；这一侧应由 L0/L1 的近似结果兜底，并标记为“近似”。生效一侧 6/6 通过 |

不生效一侧用例的期望值是“读源码时认为应该去的地方”（定义 + 源码中所有文本上的使用处），是给近似层的目标；
精确层（L2）在这一侧拿不到任何结果属于正常。#38 接入精确度标记后，这 6 例应改为断言 `precision = "approximate"`。

查询来源：`l2`

- medium：完整语义索引（格式 v2，索引进程 1.0.2.0，7718 个文件）
- large：完整语义索引（格式 v2，索引进程 1.0.2.0，21221 个文件）

## 分类准确率（运行器原始输出）

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
| 外部符号 (`external`) | 0 | 6 | 0 | 0 | 0.0% |
| 局部变量与参数 (`local`) | 6 | 0 | 0 | 0 | 100.0% |
| 事件 / 委托 (`event`) | 6 | 0 | 0 | 0 | 100.0% |
| 运算符重载 (`operator`) | 0 | 5 | 0 | 0 | 0.0% |
| 索引器 (`indexer`) | 0 | 5 | 0 | 0 | 0.0% |
| 类型引用（#37 验收） (`type_ref`) | 10 | 0 | 0 | 0 | 100.0% |
| **合计** | 81 | 19 | 0 | 0 | 81.0% |

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
| 外部符号 (`external`) | 0 | 6 | 0 | 0 | 0.0% |
| 局部变量与参数 (`local`) | 6 | 0 | 0 | 0 | 100.0% |
| 事件 / 委托 (`event`) | 6 | 0 | 0 | 0 | 100.0% |
| 运算符重载 (`operator`) | 0 | 5 | 0 | 0 | 0.0% |
| 索引器 (`indexer`) | 0 | 5 | 0 | 0 | 0.0% |
| 类型引用（#37 验收） (`type_ref`) | 10 | 0 | 0 | 0 | 100.0% |
| **合计** | 81 | 19 | 0 | 0 | 81.0% |

**全部**：200 例，通过 162、失败 38、过期 0、不支持 0，准确率 81.0%。
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
- **m-external-01**（medium · 外部符号 · 失败）`Assets/Football/Runtime/Presentation/ProceduralPitch/GrfPitchTextureView.cs:22:22`
  - 定义：期望外部符号 M:UnityEngine.GameObject.GetComponent``1，实际没有落点（无元数据视图）；绑定到 T GameObject.GetComponent<T>() [UnityEngine.CoreModule|M:UnityEngine.GameObject.GetComponent``1]
- **m-external-02**（medium · 外部符号 · 失败）`Assets/Football/Runtime/Presentation/ProceduralPitch/GrfPitchTextureView.cs:7:47`
  - 定义：期望外部符号 T:UnityEngine.MonoBehaviour，实际没有落点（无元数据视图）；绑定到 MonoBehaviour [UnityEngine.CoreModule|T:UnityEngine.MonoBehaviour]
- **m-external-03**（medium · 外部符号 · 失败）`Assets/Football/Runtime/Presentation/Rendering/GrfDeferredLighting.cs:63:47`
  - 定义：期望外部符号 P:UnityEngine.Transform.position，实际没有落点（无元数据视图）；绑定到 Vector3 Transform.position [UnityEngine.CoreModule|P:UnityEngine.Transform.position]
- **m-external-04**（medium · 外部符号 · 失败）`Assets/Football/Editor/AnimationDiagnostics/LogicalPoseWindow.cs:45:19`
  - 定义：期望外部符号 M:UnityEngine.Debug.Log(System.Object)，实际没有落点（无元数据视图）；绑定到 void Debug.Log(object message) [UnityEngine.CoreModule|M:UnityEngine.Debug.Log(System.Object)]
- **m-external-05**（medium · 外部符号 · 失败）`Assets/Football/Editor/VisualMotion/VisualMotionPublication.cs:150:32`
  - 定义：期望外部符号 M:UnityEngine.Mathf.RoundToInt(System.Single)，实际没有落点（无元数据视图）；绑定到 int Mathf.RoundToInt(float f) [UnityEngine.CoreModule|M:UnityEngine.Mathf.RoundToInt(System.Single)]
- **m-external-06**（medium · 外部符号 · 失败）`Assets/Football/Runtime/Core/Mathematics/V3.cs:42:44`
  - 定义：期望外部符号 M:System.Math.Sqrt(System.Double)，实际没有落点（无元数据视图）；绑定到 double Math.Sqrt(double d) [netstandard|M:System.Math.Sqrt(System.Double)]
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
- **l-external-01**（large · 外部符号 · 失败）`Assets/TFWCore/Script/UI/UICommonBluredBackground.cs:126:29`
  - 定义：期望外部符号 M:UnityEngine.Component.GetComponent``1，实际没有落点（无元数据视图）；绑定到 T Component.GetComponent<T>() [UnityEngine.CoreModule|M:UnityEngine.Component.GetComponent``1]
- **l-external-02**（large · 外部符号 · 失败）`Assets/Launch/Flow/LaunchUpgrader.cs:98:23`
  - 定义：期望外部符号 T:System.Version，实际没有落点（无元数据视图）；绑定到 Version [netstandard|T:System.Version]
- **l-external-03**（large · 外部符号 · 失败）`Assets/X1/ScriptGame/Battle/UI/UIBattleHeroSkill.cs:313:36`
  - 定义：期望外部符号 M:UnityEngine.GameObject.SetActive(System.Boolean)，实际没有落点（无元数据视图）；绑定到 void GameObject.SetActive(bool value) [UnityEngine.CoreModule|M:UnityEngine.GameObject.SetActive(System.Boolean)]
- **l-external-04**（large · 外部符号 · 失败）`Assets/TFWCore/Script/Utils/UnityGameObjectWrap.cs:181:22`
  - 定义：期望外部符号 P:UnityEngine.Transform.position，实际没有落点（无元数据视图）；绑定到 Vector3 Transform.position [UnityEngine.CoreModule|P:UnityEngine.Transform.position]
- **l-external-05**（large · 外部符号 · 失败）`Assets/TFWCore/Script/Common/QualitySetting.cs:16:35`
  - 定义：期望外部符号 T:UnityEngine.MonoBehaviour，实际没有落点（无元数据视图）；绑定到 MonoBehaviour [UnityEngine.CoreModule|T:UnityEngine.MonoBehaviour]
- **l-external-06**（large · 外部符号 · 失败）`Assets/X1/ScriptCore/Common/Camera/Mode/CameraModeFollow.cs:125:45`
  - 定义：期望外部符号 P:UnityEngine.Time.deltaTime，实际没有落点（无元数据视图）；绑定到 float Time.deltaTime [UnityEngine.CoreModule|P:UnityEngine.Time.deltaTime]
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
