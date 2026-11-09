## 背景

"直接加载 VS Code 主题"这件事只有被逐字符验证过，才算真的成立。`SYNTAX_AND_LANGUAGES.md` §1.2 给出了现成的验证手段：VS Code 仓库里的 `extensions/vscode-colorize-tests` 为每种语言准备了测试源文件（`colorize-fixtures/`），以及每个 token 在 10 个内置主题下的期望颜色（`colorize-results/*.json`，记录了文本、scope、命中的规则和最终颜色）。

本 issue 把这些数据接成 Rove 的黄金测试：**比的是最终颜色，不是 scope**——只要颜色一致，scope 有出入也可以接受。

## 目标

### 1. 引入测试数据

把 `colorize-fixtures/` 与 `colorize-results/` 放进 `tests/golden/`（MIT 许可，在 `THIRD_PARTY_NOTICES.md` 注明来源、VS Code 版本与许可）。数据不加工、不裁剪，便于将来跟随 VS Code 升级整体替换。

### 2. 比对流程

1. 用 [[syntax]] 对同一个 fixture 产生高亮 token；
2. 用 [[theme]] 在指定主题下计算**每个字符**的前景色与字体样式；
3. 与 `colorize-results` 中的期望颜色逐字符比较，统计一致率。

主题取六个：Dark 2026、Light 2026（VS Code 当前默认主题）、Dark Modern、Light Modern、Dark+、Light+。`colorize-results` 覆盖的另外 4 个主题（Dark VS、Light VS、两个高对比度主题）**只统计、不设阈值**，数字写进报告。

### 3. 阈值

| 语言 | 测试数据 | 阈值 |
|---|---|---|
| C#、JSON、YAML、XML、Markdown | `test_cs`、`test_json`、`test_yaml` 及 3 个 YAML 问题用例、`test_xml`、`test-7115_xml`、`test_md` 及 3 个 Markdown 问题用例 | **≥ 98%** 字符颜色一致 |

HLSL、ShaderLab、Lua 的 ≥ 90% 阈值由 [[langs-extra]] 负责；Protobuf 没有 VS Code 数据，不设阈值。本 issue 把框架做成"加一种语言只需加一行配置"，让 [[langs-extra]] 直接复用。

### 4. 差异报告

不一致的位置逐条列出：文件、行列、文本片段、期望颜色、实际颜色、VS Code 命中的规则、Rove 命中的规则与 scope。输出到 `out/golden/`（gitignore），并在 PR 中附一份摘要（按语言与主题分组的一致率表 + 前 20 条差异）。

**每一类差异都要给出原因**，典型的几类：tree-sitter 能区分而 TextMate 正则区分不了（或相反）、注入边界不同、`colorize-results` 里 VS Code 自身的已知问题用例。

### 5. 接入 CI

并入 `cargo test --workspace`：四个主题 × 五种语言的一致率低于阈值即失败。阈值写在一个数据文件里，和 [[perf]] 的预算文件一样，**调低阈值需要在 PR 中说明理由**。

## 不包含

- HLSL、ShaderLab、Lua、Protobuf 的语法与阈值（[[langs-extra]]）。
- 语义高亮的颜色比对（[[semantic]]）。
- 界面层的渲染正确性（本 issue 只比对"字符 → 颜色"的计算结果，不截图比对）。

## 验收标准

- [ ] 测试数据已引入，`THIRD_PARTY_NOTICES.md` 中注明来源、VS Code 版本与 MIT 许可
- [ ] 比对器可用：输入 fixture + 主题，输出每个字符的颜色并与期望值比较
- [ ] C#、JSON、YAML、XML、Markdown 在四个主题下全部 ≥ 98%，PR 中附一致率表
- [ ] 另外 4 个主题的一致率已统计并写入报告（不设阈值）
- [ ] 差异报告按上述字段逐条列出，并且每一类差异都有原因说明
- [ ] 框架支持"加一种语言只需加一行配置"，并以一个占位语言演示
- [ ] 阈值数据文件存在，低于阈值时测试失败
- [ ] 新增集成测试走完整条链路（fixture → [[syntax]] → [[theme]] → 比对）；[[skeleton]] 的端到端测试继续通过
- [ ] 核心层 crate 的依赖树中仍无 `gpui`

## 风险与提示

- 为了凑一致率去改 `highlights.scm` 是**允许的**（查询文件不是契约），但不要把 scope 改成"只为这个 fixture 正确"的样子。遇到 tree-sitter 更准确的情况，宁可记下差异并在报告里解释，也不要退化。
- 不要为了达标去动 [[contracts]] 的接口或 [[theme]] 的匹配算法语义。匹配算法应当与 `vscode-textmate` 一致；如果为了达标必须改它，说明 [[theme]] 的移植有缺陷，应该作为缺陷修复而不是调参。
- 98% 这个阈值是按"大部分字符是普通标识符和空白"算出来的，剩下 2% 往往集中在少数几个构造上。看一致率的同时一定要看差异的**分布**，否则会漏掉"某一类 token 全错"的情况。
