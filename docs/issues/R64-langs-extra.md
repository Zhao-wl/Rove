# R64 其他语言高亮：HLSL、Lua、Protobuf，以及自写 ShaderLab 语法

| 里程碑 | 难度 | 领域 | 并行批次 | 前置依赖 | 被依赖 |
|---|---|---|---|---|---|
| M4 体验补全 | 3 困难 | syntax | 第 5 批 | R09（tree-sitter 集成）、R11（着色黄金测试） | R74（【集成验收】M4） |

> 认领前请阅读 `AGENTS.md`、`docs/ARCHITECTURE.md`（R01 完成后）与 `docs/issues/README.md`。前置依赖全部合并后才能开工。

## 背景

D2（`FEATURES.md` §12）规定 Unity 项目里常见的其他语言**只做高亮和折叠**，不做跳转和引用。`SYNTAX_AND_LANGUAGES.md` §2 已经把四种语言的语法来源选好了，其中三种有现成的 MIT 语法可用，**ShaderLab 必须自己写**：现有的两个 `tree-sitter-shaderlab` 都是 0–1 星的个人项目且没有许可证，无法使用（§3）。

本 issue 等于把 R09（tree-sitter 集成） 的那套做法（捕获名直接写成完整的 TextMate scope）再套用四次，外加一个自写语法。它依赖 R11（着色黄金测试） 把「≥ 90%」这个阈值真正测出来——R11（着色黄金测试） 已经把框架做成「加一种语言只需加一行配置」，本 issue 直接复用。

必读：`docs/compat/SYNTAX_AND_LANGUAGES.md` §1.1、§1.2（阈值表）、§2（语言方案表）、§3（ShaderLab 整节）、§6（语言配置的 8 份中的 `hlsl`、`shaderlab`、`lua`），`FEATURES.md` §9（其他语言一行）、§11、§12（D2），`UX_SPEC.md` §3.1，`docs/compat/SETTINGS.md`（`files.associations`、`editor.maxTokenizationLineLength`）。

## 目标

### 1. 引入三种现成语法

按 `SYNTAX_AND_LANGUAGES.md` §2 的仓库与许可（全部 MIT，在 `THIRD_PARTY_NOTICES.md` 注明来源与版本），通过 Cargo 依赖或 vendor 的方式**编译进 Rove，运行时不下载**：

- HLSL：`tree-sitter-grammars/tree-sitter-hlsl`
- Lua：`tree-sitter-grammars/tree-sitter-lua`
- Protobuf：`coder3101/tree-sitter-proto`

### 2. 自写 ShaderLab 语法

放在 `grammars/tree-sitter-shaderlab/`，以 **MIT 许可**发布（`SYNTAX_AND_LANGUAGES.md` §3）。

- 外层结构：`Shader "name" { Properties { … } SubShader { Tags { … } Pass { … } } Fallback … CustomEditor … }`，加上渲染状态命令（`Cull`、`ZWrite`、`Blend`、`ZTest`、`Stencil` 等）与属性声明（`_Name ("Label", 2D) = "white" {}` 这类形式）。
- **代码块注入**：`CGPROGRAM … ENDCG`、`HLSLPROGRAM … ENDHLSL`、`CGINCLUDE … ENDCG`、`HLSLINCLUDE … ENDHLSL` 中的内容通过 tree-sitter 的语言注入交给 HLSL 语法处理，让 `.shader` 里的着色器代码与 `.hlsl` 文件获得同样的高亮。
- 语法本身约几百行，工作量中等；要能被 R09（tree-sitter 集成） 的增量解析正常驱动（接收 `InputEdit` 后只重解析变化部分）。
- 语法仓库内自带 corpus 测试（tree-sitter 标准的 `test/corpus/`），与 Rust 侧的黄金测试分开。

### 3. `highlights.scm`

每种语言一份，**捕获名使用对应 VS Code 内置语法为同类 token 产生的 scope**（`extensions/hlsl/syntaxes`、`extensions/shaderlab/syntaxes`、`extensions/lua/syntaxes`）。写之前先把对应的 `.tmLanguage.json` 读一遍，按它实际产生的 scope 写，**不要发明自己的类别名**，也不要在本 crate 里维护映射表（D1 的做法，与 R09（tree-sitter 集成） 完全一致）。

Protobuf 在 VS Code 里没有内置语法，按 §2 的约定采用通用 scope：`keyword`、`storage.type`、`entity.name.type`、`string`、`comment`、`constant.numeric` 等。

### 4. 语言配置

接入 VS Code 内置扩展中 `hlsl`、`shaderlab`、`lua` 三份 `language-configuration.json`（MIT），复用 R12（语言配置） 已有的加载器与 JavaScript 正则兼容方案（`fancy-regex`），**不自己重写配置**。每份配置补一个测试，确保其中所有正则都能编译、行为与 JavaScript 一致（`SYNTAX_AND_LANGUAGES.md` §6 的实现注意）。Protobuf 没有内置配置，按 VS Code 默认值处理。

### 5. 文件关联

按扩展名识别并遵循 `files.associations`（机制在 R09（tree-sitter 集成），本 issue 只加表项）：

| 扩展名 | 语言 |
|---|---|
| `.hlsl`、`.cginc`、`.compute` | HLSL |
| `.shader` | ShaderLab |
| `.lua` | Lua |
| `.proto` | Protobuf |

### 6. 黄金测试与真实项目验证

- 用 R11（着色黄金测试） 的框架加三行配置，跑 `test_hlsl`、`test_shader`、`test_lua`，在 Dark Modern、Light Modern、Dark+、Light+ 四个主题下达到 **≥ 90%** 字符颜色一致（`SYNTAX_AND_LANGUAGES.md` §1.2 的阈值表）。不一致的位置按 R11（着色黄金测试） 的报告格式逐条列出并说明原因。
- Protobuf **不设阈值，人工检查**：挑一份真实 `.proto` 截图，与 VS Code（装了 Protobuf 扩展或纯文本）对照，在 PR 中说明判断依据。
- ShaderLab 的额外验收（§3）：解析两个 Unity 测试项目（`ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE`）中的**全部 `.shader` 文件**，允许个别错误节点，但必须**统计并列出**（文件数、含错误节点的文件数、错误节点总数、按语法构造分类的前若干例）。
- 阈值写进 R11（着色黄金测试） 的阈值数据文件，调低需要在 PR 中说明理由。

### 7. 按级别降级与登记

- `UX_SPEC.md` §3.1 的分级对这四种语言与 C# 一致：大文件的语法高亮在后台完成、超大文件不解析；遵循 `editor.maxTokenizationLineLength`。这些行为由 R09（tree-sitter 集成） / R20（代码视图） 统一处理，本 issue 只需确认新语言走的是同一条路径。
- 新增的设置项、界面字符串（语言显示名）通过 R01（【接口先行 A1】架构与接口契约） 的登记机制加入，不得写死；语法与许可写进 `THIRD_PARTY_NOTICES.md`。不得在本 PR 中修改 R01（【接口先行 A1】架构与接口契约） 的接口。`rove-syntax` 的依赖树中不得出现 `gpui`。

## 不包含

- 跳转、引用、悬停、语义高亮（D2：其他语言只有高亮和折叠）。
- 结构树、语法校验、格式化（R60（JSON/YAML/XML） 只做 JSON/YAML/XML）。
- 折叠区间的**界面**与缩进参考线（R20（代码视图））；折叠标记来自各语言的 `folding.markers`（R12（语言配置））。
- C#、JSON、YAML、XML、Markdown 的语法与查询（R09（tree-sitter 集成））；黄金测试框架本身与 ≥ 98% 阈值（R11（着色黄金测试））。
- 注释切换、自动闭合、自动缩进等用到这三份语言配置的**编辑功能**（R52（注释切换、自动缩进、自动闭合、包围、回车规则））——本 issue 只负责把配置接进来并保证它能被解析。
- Unity Shader 的语义能力（变量来源、include 跳转）：不在 `FEATURES.md` 中，不做。

## 验收标准

- [ ] HLSL、ShaderLab、Lua、Protobuf 四种语言各有一份「按行 token（字节区间 + scope）」的快照测试
- [ ] 四种语法全部编译进二进制，`THIRD_PARTY_NOTICES.md` 中注明来源、版本与 MIT 许可；运行时无网络访问
- [ ] `grammars/tree-sitter-shaderlab/` 存在，带 MIT 许可文件与 `test/corpus/` 用例，`tree-sitter test` 通过
- [ ] ShaderLab 的注入生效：`.shader` 文件中 `CGPROGRAM`、`HLSLPROGRAM`、`CGINCLUDE`、`HLSLINCLUDE` 四种块内部产生 HLSL 的 scope，各一例
- [ ] 文件关联：`.hlsl`、`.cginc`、`.compute`、`.shader`、`.lua`、`.proto` 各有测试；`files.associations` 覆盖后生效
- [ ] `hlsl`、`shaderlab`、`lua` 三份 `language-configuration.json` 已接入，每份都有「全部正则可编译且与 JavaScript 行为一致」的测试
- [ ] R11（着色黄金测试） 中 `test_hlsl`、`test_shader`、`test_lua` 在四个主题下全部 **≥ 90%**，PR 中附一致率表与逐条差异说明
- [ ] Protobuf 的人工检查有截图与判断依据
- [ ] 两个 Unity 测试项目中的全部 `.shader` 文件解析完毕，错误节点统计表已附（含按语法构造分类的前若干例）
- [ ] 四种语言都能折叠（语法区间；有 `folding.markers` 的按标记折叠），各一例
- [ ] 新增的设置项与界面字符串都经由登记机制，无硬编码
- [ ] R02（【接口先行 A2】行走骨架） 的端到端测试仍然通过；新增集成测试（打开 `.shader` → 断言外层 scope 与注入块内的 HLSL scope）
- [ ] `cargo tree -p rove-syntax -i gpui` 为空

## 风险与提示

- **ShaderLab 的注入是整个 issue 的技术核心**，也是最容易做成「能跑但不对」的地方。`CGPROGRAM`/`ENDCG` 这类定界符是**关键字而不是括号**，tree-sitter 里要写成明确的 `program_block` 节点并在注入查询里指定语言；先把四种块各写一个 corpus 用例，再去追黄金测试的百分比。
- 90% 的阈值比 M1 五种语言的 98% 低，原因是 VS Code 的 HLSL / ShaderLab TextMate 语法本身就比较粗（大量 `support.function` 之类的白名单）。追百分比时要看差异的**分布**：如果某一类 token 全错，即使总体 92% 也要修；反之个别关键字颜色不同可以记在报告里。
- 不要为了凑一致率把 scope 改成「只为这个 fixture 正确」的样子（R11（着色黄金测试） 的同一条提醒）。tree-sitter 更准确的地方记下来并解释，不要退化。
- `.cginc` 与 `.compute` 的内容都是 HLSL，但 Unity 的宏（`UNITY_DECLARE_TEX2D` 等）在上游语法里可能被解析成普通函数调用。这不算 bug，但要在报告里说明，免得后续被当成缺陷重复排查。
- 自写语法会进入发布包并影响体积守护（`PROJECT_CHARTER.md` §6：发布包 < 30 MB）。四种语法加起来的体积增量要在 PR 中给出数字，并知会 R75（Windows 发布流水线、安装包、体积守护、第三方许可声明）。
