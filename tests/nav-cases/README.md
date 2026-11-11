# C# 导航准确率测试集（#39）

200 个人工整理的导航用例：中型、大型两个本地 Unity 项目各 100 个，由
`cargo run -p rove-lang-cs --example nav_check` 运行。用法、环境变量和过期用例的重新整理步骤见
[AGENTS.md](../../AGENTS.md) §8；运行器结构见 `crates/rove-lang-cs/examples/nav_check/main.rs`。

| 文件 | 项目 | 分类 |
|---|---|---|
| `medium-a.toml` / `large-a.toml` | 中型 / 大型 | var、chain、overload、extension、generic、inheritance、interface、partial |
| `medium-b.toml` / `large-b.toml` | 中型 / 大型 | lambda、conditional、external、local、event、operator、indexer、type_ref |
| `baseline-l2.md` | 两者 | 用当前完整语义索引跑出的基线报告（含失败原因） |

## 用例格式

```toml
project = "medium"          # medium → ROVE_TEST_UNITY_MEDIUM，large → ROVE_TEST_UNITY_LARGE

[[case]]
id = "m-var-01"             # 全局唯一；m- / l- 前缀 + 分类 + 序号
category = "var"            # 分类键，见下表
file = "Assets/Scripts/Game.cs"   # 相对项目根目录，斜杠分隔；不允许绝对路径
line = 15                   # 1 起算
col = 15                    # 1 起算，按 Unicode 字符计（制表符算 1 列），指向 token 的第一个字符
token = "Move"              # 光标处的标识符（运算符用例写运算符本身，索引器写 "["）
anchor = "p.Move();"        # 该行去掉首尾空白后的完整文本
why = "var p = GetPlayer() 推断出 Player；项目中另有 4 个同名 Move"   # 为什么选这个点
source = "人工阅读：GetPlayer() 返回 Player（Game.cs:10）；rg -w Move 确认同名方法"  # 期望值怎么确认的
[case.expect]
defs = [{ file = "Assets/Scripts/Player.cs", line = 5, anchor = "public void Move() { }" }]
refs = 1                    # 精确引用数（不含定义处），只用于 < 20 的符号
refs_include = [{ file = "Assets/Scripts/Game.cs", line = 15, anchor = "p.Move();" }]
```

- `expect.defs` 与 `expect.external` 二选一。partial 类型等有多处定义时，`defs` 列出全部。
- `expect.external` 是外部符号（DLL 中的符号）的文档注释 ID，例如 `M:UnityEngine.GameObject.GetComponent``1`。
- `expect.refs` 与 `expect.refs_include` 至少填一个；引用 ≥ 20 的符号只能用 `refs_include`。
- `branch = "active" | "inactive"` 只用于 `conditional`，标明查询点在 `#if` 的生效一侧还是不生效一侧。
- `expect.precision`（可选，`exact` / `approximate` / `syntax`）：查询来源报告精确度时一起断言（#38 之后）。

- 源码不是 UTF-8（例如 GBK 中文注释）的文件，锚点行与期望位置都选纯 ASCII 的行：运行器按 UTF-8 宽松解码，非 ASCII 行的文本与列号对不上。

判定：定义按“文件 + 行”的集合比较；引用数量比较的是不含定义处的引用；`refs_include` 的每一项必须出现在引用列表中（同一行即可）。

## 分类

| 键 | 分类 | 选点要点 |
|---|---|---|
| `var` | var 推断 | 经 `var` 变量访问成员，且成员名在项目中不唯一 |
| `chain` | 链式调用 | `a.GetB().GetC().Run()` 中段或末段，需要逐步知道返回类型 |
| `overload` | 方法重载 | 同一类型内同名不同参数的调用点，期望落在正确的那个重载 |
| `extension` | 扩展方法 | LINQ（外部）与项目自定义扩展方法都要有 |
| `generic` | 泛型 | 含 `GetComponent<T>()`、泛型类型参数替换后的成员访问 |
| `inheritance` | 继承与重写 | `base.X()`、`override` 成员、经基类引用调用 |
| `interface` | 接口实现 | 经接口调用、显式接口实现 |
| `partial` | partial | partial 类型（多处定义）与跨文件的成员 |
| `lambda` | Lambda / LINQ | lambda 参数的成员访问（类型来自委托推断） |
| `conditional` | 条件编译 | `#if UNITY_EDITOR` 等，生效一侧与不生效一侧都要有 |
| `external` | 外部符号 | Unity API、BCL 等，#34 之前必然失败 |
| `local` | 局部变量与参数 | 同名局部变量/参数遮蔽、模式变量、foreach 变量 |
| `event` | 事件 / 委托 | `+=` / `-=` 的事件、委托类型、事件调用 |
| `operator` | 运算符重载 | 光标在运算符上，期望跳到 `operator` 声明 |
| `indexer` | 索引器 | 光标在 `[` 上，期望跳到 `this[...]` 声明 |
| `type_ref` | 类型引用 | #37（L1）的验收口径：类型名在声明、`new`、泛型实参、`typeof`、继承列表等处的引用 |

## 期望值来源

每个用例的 `source` 写明期望值是怎么确认的：人工阅读源码（写出关键的推导，例如返回类型在哪里声明），
引用数量用全文搜索交叉核对（写出命中数与排除了哪些同名符号）。期望值**不能**从被测索引里抄。
