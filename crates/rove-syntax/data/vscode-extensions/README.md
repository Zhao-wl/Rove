# VS Code 语言配置（原样取用）

本目录是 Microsoft VS Code 内置扩展中的 `language-configuration.json`，**逐字节原样复制**，不做任何修改。
Rove 直接使用这些文件实现括号、注释、自动闭合、缩进、单词边界与折叠标记，行为因此与 VS Code 一致
（见 `docs/compat/SYNTAX_AND_LANGUAGES.md` §6 与 `docs/FEATURES.md` §11）。

## 版本

- 上游：<https://github.com/microsoft/vscode>
- 标签：`1.145.2`
- 提交：`c4453f8ea430795f941c1b2ff6c9321135a74a02`
- 许可：MIT（版权与许可声明见仓库根目录 `THIRD_PARTY_NOTICES.md`）

## 文件

| 本地路径 | 上游路径 | SHA-256 |
|---|---|---|
| `csharp/language-configuration.json` | `extensions/csharp/language-configuration.json` | `ee6c48fcecc08c89189bde625039aeb75fb7f9d3f03ecfd4af89309c15ba1124` |
| `json/language-configuration.json` | `extensions/json/language-configuration.json` | `0963af900927e3398cc4e40a853b4091d23361ef2d3f89f8f663c8219a2fc8ca` |
| `markdown-basics/language-configuration.json` | `extensions/markdown-basics/language-configuration.json` | `4b7aa0ec72cd9507a5ae947c92e0eeba861205dfb6d9cc6788cd800dd2c80db1` |
| `xml/xml.language-configuration.json` | `extensions/xml/xml.language-configuration.json` | `385a24a9da4ed89366d691881e1dfcdac984de6ac7e805776add43ed1731fa7c` |
| `yaml/language-configuration.json` | `extensions/yaml/language-configuration.json` | `306c8f6951dddc6214ff80766123a0315fc63773a0bd2909423968fa712b4a26` |

语言 ID 与文件的对应关系（六个语言 ID 共用五份文件，与 VS Code 相同）：

| 语言 ID | 文件 |
|---|---|
| `csharp` | `csharp/language-configuration.json` |
| `json`、`jsonc` | `json/language-configuration.json` |
| `markdown` | `markdown-basics/language-configuration.json` |
| `xml` | `xml/xml.language-configuration.json` |
| `yaml` | `yaml/language-configuration.json` |

`kebab-case`、`jsonl`、`snippets`、`xsl`、`dockercompose` 等其余语言 ID 不在 M1 范围内；
`hlsl`、`shaderlab`、`lua` 随其他语言高亮一起接入（见 issue #12 的说明）。

## 升级步骤

1. 从新的 VS Code 标签按上表路径整体替换文件（不要手工改内容）。
2. 更新 `crates/rove-syntax/src/language_config/mod.rs` 的 `VSCODE_VERSION` 与 `VSCODE_COMMIT`，
   以及本文件的版本、提交和哈希。
3. 运行 `cargo test -p rove-syntax`：
   - `every_regex_in_every_configuration_compiles` 会遍历本目录，新增文件必须先在
     `SOURCE_FILES`/`LANGUAGE_IDS` 登记；
   - `field_level_assertions_match_the_vscode_files` 会在字段变化时失败，逐项核对后更新断言。
4. 正则若用到尚不支持的 JavaScript 特性，加载会报出配置路径与字段名（不会静默失效）。
