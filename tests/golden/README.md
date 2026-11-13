# 着色黄金测试数据（VS Code `vscode-colorize-tests`，原样取用）

本目录是 Microsoft VS Code `extensions/vscode-colorize-tests/test/` 下的着色测试数据，**逐字节原样复制**：

- `colorize-fixtures/`：测试源文件。
- `colorize-results/`：每个源文件的期望结果。一个 token 数组，`c` 是 token 文本、`t` 是 TextMate
  scope 路径、`r` 是十个内置主题下「命中的规则: #颜色」。token 按文档顺序平铺整份文件，
  只跳过行终止符。
- `thresholds.toml`：Rove 这一侧的阈值（本仓库自己的文件，不是上游数据）。

用法、口径与差异成因见 [docs/GOLDEN_COLORS.md](../../docs/GOLDEN_COLORS.md)。

## 版本

- 上游：<https://github.com/microsoft/vscode>
- 标签：`1.145.2`
- 许可：MIT（版权与许可声明见仓库根目录 `THIRD_PARTY_NOTICES.md`）

## 取用范围

上游的 `colorize-results/` 有 109 个文件、约 10 MB，覆盖几十种 Rove 不会支持的语言。这里**按文件名
挑选**出与 Rove 语言清单相关的那些，**文件内容不做任何加工**（不裁剪 token、不改格式），升级时
整体替换即可：

- M1 的五种语言：C#、JSON、YAML、XML、Markdown，以及 issue #11 点名的问题用例；
- `test.lua` / `test_lua.json`：R64 的占位语言，用来演示「加一种语言只需加一行配置」。

R64 接入 HLSL / ShaderLab / Lua 时，按同样的方式补 `test.hlsl`、`test.shader` 与对应的结果文件。

## 文件与哈希

源文件（`colorize-fixtures/`）：

| 文件 | SHA-256 |
|---|---|
| `issue-1550.yaml` | `519b593d7adfd7eef50e10d2e57f5ef6cb62baa875b1d0f490f289161f7cc9b6` |
| `issue-224862.yaml` | `8ee709359308a319a2033b3d56872405980cce93a21ed9688df2a5f3c573220a` |
| `issue-279576.md` | `8413cfcc79b3b0b517add81c3602e24af4e5e791b5a241e20a8190f0ab55a265` |
| `issue-4008.yaml` | `6837cb4eec0eb451daae9b486127c7c2f67c104567684e99dec4607cfaad86f3` |
| `issue-6303.yaml` | `4d736a63de165dc0586b9de74cfaaa5479f6a4db6da0102513f97424ce07c55c` |
| `md-math.md` | `7273ccbad2825fef088cc639492c85c0baeae138d7aa2c13af705f6d6f2c2585` |
| `test-33886.md` | `26defce7cb29ec6dfe3735fdb3ce8de886e7784532bd14b550b1f58dfee9f209` |
| `test-7115.xml` | `3dc8c0e0f3d83146bf135ea938ae4ca88264a472a9613b1f56a1e66f8087b97f` |
| `test.cs` | `8a212bd53825066db646eb50f58ec3f0856ee758dcbdeb94ded65f46d70a43b6` |
| `test.json` | `b2d531295f4856719d5eb1bf2f5fa42bc0ea8436b5687cde0ed19e504866c360` |
| `test.lua` | `b185a4bbde3dc97e26208afc6cfdbe48494674eedc5b034d666a19e4c53f579d` |
| `test.md` | `ebe849c7b7147933eb203b4828ecef50d6e1780d343801949acff5900fc569e8` |
| `test.xml` | `8f9f0fa2bf19a850f479df0402982096884fccd9e161f7bfb5b4823116ef3113` |
| `test.yaml` | `1d02d6f3f20d03bc05e5382c67aad087319a6cf0846422c99a987f8f3e083fcc` |
| `tsconfig_off.json` | `8de2606d58ba6b5e408878317e191f2e2ddbaa23ca572b241d03b1351967788b` |

期望结果（`colorize-results/`）：

| 文件 | SHA-256 |
|---|---|
| `issue-1550_yaml.json` | `4868dfe94890c3a2087a9401eaf8ad3fbe8437543dc5e5a67069d4a2b721b622` |
| `issue-224862_yaml.json` | `2f3a8b91fb14aaa9b1d45ca46b31f85266245a98ff203e663a0b475bf34d3d55` |
| `issue-279576_md.json` | `591c1b8eed4c353bd196bc121bfcf383b112c94dc0b193a8fceaccbdb5f8273b` |
| `issue-4008_yaml.json` | `f61640abd75a2ed3e9163bd84f120da889421413592bed5deadb2d7e99191265` |
| `issue-6303_yaml.json` | `cdc5b6613c9ccc28a0aa66f3a80578673eb6e4fafac084daecd4eebe16869843` |
| `md-math_md.json` | `1f66c4010bb0c68d2424576b85d343299ebb702d7ab49b4209865d4efa0099a4` |
| `test-33886_md.json` | `418305e7bade43ef682b5bcb6cd8e3d00ddb38bef8b6e6c277de0a09cb61d9dd` |
| `test-7115_xml.json` | `29e15a5c372f273199a9b8d360f445df3d7844a1f0148bc88edab278e5b4aea1` |
| `test_cs.json` | `36ce4b7aade5329e7a99fe4f0431c0e4c82e6f6c7605e5db62d823af2fa6f27c` |
| `test_json.json` | `b5b7401ddbb3d72346defbbc46f518bb9f5ae9e3a623dcb0c49c06445117fc8d` |
| `test_lua.json` | `4a030787c512db1d0c812a13c6e220f9fd5cd2ffdcdcf2b65e771923a5bc58a8` |
| `test_md.json` | `c1ff84b39b6eb4c1c9b1e5f0ca6bf5a411f044b15fb14a5ef9328d125d323009` |
| `test_xml.json` | `7560dbbe6955ad50a24ce2ba621d5717cf9e3c7e0297b87cfeb43b0a3a85b726` |
| `test_yaml.json` | `beb0e5608959ff8bf7bd0a70ec1a0eb482256ea02e2c5c32acf2b6c10cbd1237` |
| `tsconfig_off_json.json` | `8bd582d96ba30731f5cca2a343fcbb368f1b88c6d6851f5c07b4d72d87992dc2` |

## 升级步骤

1. 用新版本 VS Code 的同名文件整体替换（不要手工改内容），更新本文件的标签与哈希。
2. 主题文件也要同步升级（`crates/rove-theme/data/themes/`，见那里的 `README.md`）：
   期望颜色是用那一版主题算出来的，两边版本必须一致。
3. `cargo run -p rove-syntax --example golden_colors`，对照 `out/golden/report.md` 看一致率有没有掉。
4. 一致率变化要在 PR 里说明；**不允许为了让测试通过而调低 `thresholds.toml` 里的阈值**
   （低于 0.98 的阈值必须带 `reason`，否则测试直接失败）。
