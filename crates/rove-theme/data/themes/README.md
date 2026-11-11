# VS Code 默认颜色主题（原样取用）

本目录是 Microsoft VS Code `extensions/theme-defaults/themes/` 下的颜色主题文件，**逐字节原样复制**，
由 `rove-theme` 以 `include_str!` 打包进二进制（运行期不读磁盘、不联网）。主题 ID 与界面类型对应
`extensions/theme-defaults/package.json` 的 `contributes.themes`，登记在 `src/service.rs` 的 `BUILTIN_THEMES`。

## 版本

- 上游：<https://github.com/microsoft/vscode>
- 标签：`1.145.2`
- 提交：`c4453f8ea430795f941c1b2ff6c9321135a74a02`
- 许可：MIT（版权与许可声明见仓库根目录 `THIRD_PARTY_NOTICES.md`）

## 文件

| 文件 | 主题 ID（`workbench.colorTheme`） | include | SHA-256 |
|---|---|---|---|
| `2026-dark.json` | Dark 2026 | dark_modern.json | `c784a81a9ad60c3e2aa905a562e9bc91a034dc3eb204c00ec4259279e7d37e4d` |
| `2026-light.json` | Light 2026 | light_modern.json | `7c8d9be680828dd769149c7c1c43cae4e06a8796186fa142f4fc6b94cc8658e6` |
| `dark_modern.json` | Dark Modern | dark_plus.json | `58d4ea55a2a36f65c0d5caf033dfddc4ec7d4b3a5d8b731a84bc36001d26a339` |
| `light_modern.json` | Light Modern | light_plus.json | `831b47d624e9acd6f09da58a699889415be7e45fccffb01177bc718b20e7e25c` |
| `dark_plus.json` | Dark+ | dark_vs.json | `409f33088a5cfd6589384900ef80627787c568d367c25b12f31bc7940936e573` |
| `light_plus.json` | Light+ | light_vs.json | `26fbb053bec8ce2b65706d5fcb993da8c19174cfdbf35a49b465236b5ed26203` |
| `dark_vs.json` | Visual Studio Dark | — | `9fc2c16761400632eac6166ff7bb0435c5190a94bffa51ec1ef19b2ce78576b2` |
| `light_vs.json` | Visual Studio Light | — | `ce562a2ebec8c9a91021afb9ba7c9e8bcf4176df634fec8b77c748b42e3ce748` |
| `hc_black.json` | Default High Contrast | — | `07f6250833d40ca9ed7399f328816d4e0c59ddc21c64c8af754a6b260725b0ad` |
| `hc_light.json` | Default High Contrast Light | — | `1cc2ccb7d2fb7990d22baa1dd2bb7174817ac45ddc8b46c4d29c3ddb864b4ce2` |

## 升级步骤

1. 用新版本 VS Code 的同名文件整体替换（不要手工改内容），更新本文件的版本、提交与哈希。
2. 重新生成颜色默认值表：`python scripts/theme/gen_color_defaults.py <vscode 源码> <版本> <提交>`。
3. `cargo test -p rove-theme`，并检查 `docs/compat/THEME_COLORS.md` 的 P0 清单是否需要随版本更新。
