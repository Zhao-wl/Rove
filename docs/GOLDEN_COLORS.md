# 着色黄金测试：逐字符比对 VS Code 的颜色（#11）

> 数据：VS Code 1.145.2 `extensions/vscode-colorize-tests`（MIT，见 [tests/golden/README.md](../tests/golden/README.md)）
> 相关：[compat/SYNTAX_AND_LANGUAGES.md](compat/SYNTAX_AND_LANGUAGES.md) §1.2、`crates/rove-syntax`（#9）、`crates/rove-theme`（#10）

「直接加载 VS Code 主题」这件事只有被逐字符验证过才算成立。本文件记录验证方式、当前数字，
以及每一类差异的原因。

## 1. 怎么跑

```powershell
cargo test -p rove-syntax --test golden_colors          # CI 判定，顺带把报告写到 out/golden/
cargo run  -p rove-syntax --example golden_colors       # 本地排查：打印一致率表并写报告
cargo run  -p rove-syntax --example golden_colors -- --language csharp
cargo run  -p rove-syntax --example golden_colors -- --fixture test.cs --theme dark_modern --chars
```

- `--chars` 逐字符打印「期望颜色 / 实际颜色 / 是否一致 / 两边的 scope」，排查单个 token 用它。
- 报告写在 `out/golden/`（已 gitignore）：`report.md` 给人看，`report.json` 给机器看。
- 阈值在 [tests/golden/thresholds.toml](../tests/golden/thresholds.toml)。**加一种语言只需要在
  `[languages]` 下加一行**（`lua` 就是这样的占位语言：列了 fixture、不设阈值，只统计）。

## 2. 比对链路与口径

1. fixture → `rove_buffer::RopeText`；
2. → #9 的 `TreeSitterSyntax::analyze`，得到每个字节的 scope 路径；
3. → #10 的 `ResolvedTheme::token_style`，得到每个字符的前景色；
4. → 与 `colorize-results` 里该主题的期望颜色**逐字符**比较。

- **比的是最终前景色，不是 scope**：scope 有出入但颜色一致，算通过。
- 行终止符不参与比较（VS Code 的期望结果里本来就没有）。
- 受阈值约束的主题是 VS Code 当前的默认主题及其 include 链：Dark/Light 2026、Dark/Light Modern、
  Dark+/Light+。另外四个（Dark/Light VS、两个高对比度）只统计、不判定。
- 不支持的语言（占位语言）不会报错：所有字符退回主题默认前景色，照常统计。

## 3. 当前一致率

2026-11-13 实测（`out/golden/report.md`，标 `*` 的主题不设阈值）：

| 语言 | 阈值 | 2026-dark | 2026-light | dark_modern | light_modern | dark_plus | light_plus | dark_vs* | light_vs* | hc_black* | hc_light* |
|---|---|---|---|---|---|---|---|---|---|---|---|
| csharp | 98% | 100.00% | 100.00% | 100.00% | 100.00% | 100.00% | 100.00% | 100.00% | 100.00% | 100.00% | 100.00% |
| json | 98% | 98.41% | 98.41% | 98.41% | 98.41% | 98.41% | 98.41% | 100.00% | 100.00% | 98.41% | 98.41% |
| xml | 97% | 97.97% | 97.97% | 97.97% | 97.97% | 97.97% | 97.97% | 97.97% | 97.97% | 97.97% | 97.97% |
| yaml | 77% | 77.55% | 77.55% | 90.11% | 90.11% | 90.11% | 90.11% | 90.11% | 90.11% | 90.11% | 90.11% |
| markdown | 77% | 77.31% | 77.31% | 77.31% | 77.31% | 85.14% | 85.14% | 88.71% | 88.71% | 84.84% | 84.15% |
| lua（占位） | — | 36.93% | 36.93% | 36.93% | 36.93% | 39.00% | 39.00% | 53.53% | 53.53% | 36.93% | 36.93% |

逐个 fixture（dark_modern）：

| fixture | 一致率 | fixture | 一致率 |
|---|---|---|---|
| `test.cs` | 100.00% | `test.md` | 88.22% |
| `test.json` | 98.10% | `issue-279576.md` | 41.74% |
| `tsconfig_off.json` | 100.00% | `md-math.md` | 76.21% |
| `test.xml` | 100.00% | `test-33886.md` | 31.37% |
| `test-7115.xml` | 91.77% | `test.yaml` | 85.13%（2026 主题 56.06%） |
| `issue-1550.yaml` | 100.00% | `issue-224862.yaml` | 76.23% |
| `issue-4008.yaml` | 100.00% | `issue-6303.yaml` | 98.13% |

**没有达到 issue #11 的 98% 目标的是 XML、YAML、Markdown**，原因见下一节；阈值与理由写在
`thresholds.toml` 里（低于 98% 的阈值必须带 `reason`，否则测试失败）。

### 扣除成因之后

报告里的「差异成因」表按 VS Code 的 scope 路径自动归类，并给出扣掉「不在 Rove 控制范围内」的
三类之后的一致率（dark_modern）：

| 语言 | 差异字符 | 注入语言未内置 | VS Code 判为非法 | 占位语言 | Rove 侧 | 扣除后一致率 |
|---|---|---|---|---|---|---|
| csharp | 0 | 0 | 0 | 0 | 0 | 100.00% |
| json | 4 | 0 | 0 | 0 | 4 | 98.41% |
| xml | 19 | 0 | 0 | 0 | 19 | 97.97% |
| yaml | 100 | 0 | 41 | 0 | 59 | 93.92% |
| markdown | 1049 | 829 | 0 | 0 | 220 | 94.20% |

## 4. 差异分类与原因

### 4.1 VS Code 注入了 Rove 不内置的语言（Markdown 差异的 79%）

VS Code 的 Markdown 语法会把围栏代码块、内嵌 HTML、`$...$` 数学块交给对应语言的 TextMate 语法。
Rove 只内置章程语言清单里的语法（M1 五种 + M4 的 HLSL / ShaderLab / Lua / Protobuf），HTML、LaTeX、
TypeScript / JavaScript / CSS 都不在其中：

- `test-33886.md`（31%）整份文件都是 `<pre><code>` HTML 块；
- `issue-279576.md`（42%）三个围栏块里有两个是 ts / tsx；
- `md-math.md`（76%）大量 `$$\theta$$` 这类 TeX 命令。

Rove 的行为与「VS Code 没有装对应语法扩展」时一致：这段文本保持无 token，用编辑器前景色。
围栏块里语言**已内置**时，Rove 会像 VS Code 一样加上 `meta.embedded.block.<语言>` 外层 scope，
主题的 `meta.embedded` 规则因此能生效（这一条把 `issue-279576.md` 的 C# 块从 0 拉到基本全对）。

### 4.2 VS Code 自己把文本判为非法（YAML 差异的 41%）

三个 YAML 问题用例本身就是写坏的文档。VS Code 的 TextMate 语法逐行扫描，把对不上缩进的部分标成
`invalid.illegal.*`（红色）；tree-sitter 是整树解析，遇到同样的位置会产生 ERROR 节点，连块结构
（`meta.block.sequence`）都给不出来。两种纠错方式不可能给出相同结果，也不该为了对齐而把正常文本标红：

- `issue-224862.yaml`：第二个文档体被 VS Code 整行标红（29 个字符）。
- `test.yaml` 第 10 行起缩进是错的：VS Code 标红 `spotSize:`，后面的 `John Smith` 之类仍按字符串着色；
  Rove 这一段落在 ERROR 子树里，没有 token。Dark/Light 2026 主题还给 `meta.block` 定了前景色，
  结构给不出来就连带着差得更多（56.06% vs 其余主题的 85.13%）。

### 4.3 上游语法的节点划分与 TextMate 不同

- **XML 的非拉丁名字**（19 个字符）：`tree-sitter-xml` 的 `Name` 模式要求首字符是 ASCII 字母，
  `Поле="tagnotwork"` 整个解析成 ERROR。`test-7115.xml` 正是 VS Code issue 7115 的复现用例。
- **JSON 的 `V`**（4 个字符）：`tree-sitter-json` 的 `escape_sequence` 只到 `\u`，后面四位十六进制
  不在节点里，于是按字符串着色而不是转义着色。
- **Markdown 列表标记**（13 个字符）：`list_marker_dot` 把标记后面的空格一起吃掉，VS Code 的
  `punctuation.definition.list.begin.markdown` 只到 `1.`。

这几处都是上游语法的节点边界问题，查询层面改不了；改语法不属于本 issue。

### 4.4 Rove 这一侧已知还差的

- **数学块的 `$` 定界符**（Markdown 内 153 个字符）：tree-sitter-md 的 `latex_block` 里只有定界符是
  命名节点，中间的公式文本没有节点，没法单独捕获。Rove 只能把整个 `latex_block` 标成
  `meta.embedded.math.markdown`，于是定界符也跟着变色（VS Code 给的是默认前景色）。
  整体仍然划算：`md-math.md` 因此从 43% 升到 76%。
- **内嵌 HTML 标签**：`<div class="x">` 这种 VS Code 会拆成标签名、属性名、字符串分别着色，
  tree-sitter-md 只给出整块 `html_block` / `html_tag`。目前只特判了 HTML 注释
  （`^<!--` → `comment.block.html`，与 VS Code 一致），其余不拆。

### 4.5 为了对齐做过的修改（都是「向 VS Code 看齐」，不是为单个 fixture 调参）

记录在这里便于回看，细节见 `crates/rove-syntax/src/highlight/queries/` 的注释：

| 改动 | 原因 |
|---|---|
| Markdown 标题整体带 `markup.heading.markdown`，`#` 标记作为内层 | 主题的规则写在 `markup.heading` 上，只给标记打 `punctuation.definition.heading` 会丢色 |
| setext 标题只给下划线行，正文仍是段落 | VS Code 就是这样 |
| 围栏块内容加 `meta.embedded.block.<语言>`（只在语言已内置时） | 主题的 `meta.embedded` 规则 |
| 整个围栏块加 `markup.fenced_code.block.markdown` | 与 VS Code 的层级一致 |
| 块引用整体加 `markup.quote.markdown` | 同上 |
| YAML 锚点 / 别名整体 `keyword.control.flow.*` | `&` 与名字的颜色都来自这一层 |
| YAML 块序列加 `meta.block.sequence.yaml` | Dark/Light 2026 给 `meta.block` 定了前景色 |
| XML 声明里的 `xml` / `version` / `encoding` / `standalone` 与它们的值 | 原来整条 `<?xml ... ?>` 只有一个 `meta.tag.preprocessor.xml` |
| C# 的类型引用、表达式里的标识符、成员访问、调用名 | VS Code 的 C# 语法给这些 `entity.name.type` / `variable.other.*` / `entity.name.function` |
| C# 比较运算符限定在二元表达式里 | `<`/`>` 也是泛型实参列表的括号，那里不是运算符 |
| C# 独占一行的注释，捕获向行首空白扩展 | VS Code 的 `(^\s+)?(?=//)`；缩进与注释同色在小地图和带背景的主题下看得出来 |

## 5. 加一种语言

在 `tests/golden/thresholds.toml` 的 `[languages]` 下加一行：

```toml
hlsl = { fixtures = ["test.hlsl"], threshold = 0.90 }
```

再把对应的 `colorize-fixtures/test.hlsl` 与 `colorize-results/test_hlsl.json` 从 VS Code 仓库原样
复制进 `tests/golden/`（见那里的 `README.md`）。运行器不需要改动。省略 `threshold` 表示只统计、
不判定，适合语法还没接入的阶段。
