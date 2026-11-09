"""生成 docs/compat/KEYBINDINGS.md 与 docs/compat/SETTINGS.md。

输入：docs/compat/vscode/ 下从 VS Code 导出的默认键位与默认设置；规则：rules.py。
VS Code 升级后重新导出这两个文件并运行本脚本即可更新清单：
    python scripts/compat/gen_compat_docs.py
"""
from __future__ import annotations
import json, re, sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import classify as c  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "compat"
VSCODE_VERSION = "1.145.2（Windows，中文界面导出）"
PRI_ORDER = {"P0": 0, "P1": 1, "P2": 2, "": 3}

ROVE_COMMANDS = [
    ("rove.contextBasket.add", "ctrl+k b", "editorTextFocus", "P1", "把选区（没有选区时为光标所在符号）加入上下文篮子（UX_SPEC §5.1）"),
    ("rove.contextBasket.focus", "ctrl+shift+alt+b", "", "P1", "打开上下文篮子视图"),
    ("rove.contextBasket.copyAsMarkdown", "", "", "P1", "复制为 Markdown（UX_SPEC §5.3）"),
    ("rove.contextBasket.copyAsReferences", "", "", "P1", "复制为引用（UX_SPEC §5.3）"),
    ("rove.contextBasket.clear", "", "", "P1", "清空上下文篮子"),
    ("rove.editor.followTail", "", "", "P1", "跟随文件末尾（UX_SPEC §3.2）"),
    ("rove.editor.viewFormatted", "", "", "P1", "格式化后查看（只读副本，UX_SPEC §3.3）"),
    ("rove.editor.openLongLine", "", "", "P0", "在临时标签中查看被截断的超长行（UX_SPEC §3.3）"),
    ("rove.json.minify", "", "", "P1", "压缩 JSON（UX_SPEC §4.2）"),
    ("rove.structure.copyPath", "", "", "P1", "复制结构路径（JSON/YAML 为 `$.a[3]`，XML 为 XPath，UX_SPEC §4.2）"),
    ("rove.structure.copyValue", "", "", "P1", "复制节点的值（UX_SPEC §4.2）"),
    ("rove.index.rebuild", "", "", "P0", "重建 C# 语义索引（UX_SPEC §2.1）"),
    ("rove.index.showDetails", "", "", "P0", "显示 C# 索引详情（UX_SPEC §2.1）"),
    ("rove.index.openLog", "", "", "P0", "打开索引日志（UX_SPEC §2.1）"),
]

ROVE_SETTINGS = [
    ("rove.vscodeUserDir", "\"\"", "P0", "共用的 VS Code 用户配置目录；为空时使用 `%APPDATA%\\Code\\User`（FEATURES §13 规则 1）"),
    ("rove.largeFile.threshold", "67108864", "P0", "超大文件（只读、内存映射）的阈值，单位字节，默认 64 MB（UX_SPEC §3.1）"),
    ("rove.files.extraExclude", "{\"**/Library\": true, \"**/Temp\": true, \"**/obj\": true, \"**/Logs\": true}", "P0", "附加的排除规则，与 `files.exclude` 合并生效（FEATURES §13 规则 5）"),
    ("rove.files.extraReadonlyInclude", "{\"**/Library/PackageCache/**\": true}", "P0", "附加的只读规则，与 `files.readonlyInclude` 合并生效（UX_SPEC §3.4）"),
    ("rove.index.cacheDir", "\"\"", "P0", "C# 索引缓存目录；为空时使用 `%LOCALAPPDATA%\\Rove\\index`"),
    ("rove.index.enabled", "true", "P0", "是否启用 C# 语义索引"),
    ("rove.metadataView.showDocs", "true", "P1", "元数据视图中是否显示文档注释（UX_SPEC §4.1）"),
    ("rove.markdown.webviewIdleTimeout", "300", "P0", "所有 Markdown 预览关闭后，多少秒销毁 WebView2 实例（UX_SPEC §4.3）"),
    ("rove.markdown.allowRemoteResources", "false", "P0", "预览中是否加载远程图片等资源（UX_SPEC §4.3）"),
    ("rove.contextBasket.referenceTemplate", "\"@{path} (lines {start}-{end})\"", "P1", "“复制为引用”的格式（UX_SPEC §5.3）"),
    ("rove.contextBasket.contextLines", "3", "P1", "从速览或搜索结果加入时，额外带上的前后行数（UX_SPEC §5.1）"),
    ("rove.contextBasket.clearAfterCopy", "false", "P1", "复制后是否清空篮子（UX_SPEC §5.3）"),
]

# Rove 的默认值与 VS Code 不同（只在用户未设置时生效，不写入共用文件；FEATURES §13 规则 5）
DIFFERENT_DEFAULTS = [
    ("files.autoGuessEncoding", "false", "true", "Unity 项目中常见 GBK 文件（UX_SPEC §3.5）"),
    ("files.candidateGuessEncodings", "[]", "[\"gbk\"]", "同上"),
    ("workbench.editorAssociations", "{}", "{\"*.md\": \"vscode.markdown.preview.editor\"}", "`.md` 默认以预览打开（D6）"),
    ("search.useIgnoreFiles", "true", "（忽略）", "Rove 不读取 `.gitignore`（FEATURES §6）"),
]


def esc(s: str) -> str:
    return s.replace("|", "\\|").replace("\n", " ")


def short(v, n=60) -> str:
    s = json.dumps(v, ensure_ascii=False)
    return s if len(s) <= n else s[: n - 1] + "…"


def gen_keybindings():
    rows, unbound = c.classify_commands()
    stats = Counter()
    impl = defaultdict(list)
    no = defaultdict(set)
    ctx = Counter()
    for b, (d, pr, area, note) in rows:
        stats[(d, pr)] += 1
        if d == "impl":
            impl[area].append((pr, b["key"], b["command"], b.get("when", ""), note))
            for k in re.findall(r"[A-Za-z_][\w.\-]*", b.get("when", "")):
                if k not in ("true", "false", "config", "in", "not") and not k.startswith("config."):
                    ctx[k] += 1
        elif d == "no":
            no[area].add(b["command"].lstrip("-"))
    ext = stats[("ext", "")]
    total = len(rows)

    vs_keys = defaultdict(list)
    for b, _ in rows:
        vs_keys[b["key"]].append(b["command"])

    L = []
    L.append("# 键位兼容清单（阶段 0.3a）\n")
    L.append(f"> 版本：v1.0（自动生成，2026-11-09 已确认判定规则） · 来源：VS Code {VSCODE_VERSION}的默认键位 · 生成脚本：`scripts/compat/gen_compat_docs.py` · 判定规则：`scripts/compat/rules.py`\n")
    L.append("## 1. 说明\n")
    L.append("- Rove 与 VS Code 共用 `keybindings.json`（D4），命令 ID 一律沿用 VS Code。下表中“实现”的条目，Rove 必须以**相同的命令 ID、相同的默认键位、相同的 `when` 条件**提供。")
    L.append("- 判定依据是 `FEATURES.md` 与 `UX_SPEC.md`。对判定有异议时，修改 `rules.py` 后重新生成，不要直接改本文件。")
    L.append("- 导出文件中出现的 `oem_2`、`oem_5` 等是 Windows 虚拟键码名（与键盘布局有关）。Rove 按 VS Code 的 `keyboard.dispatch` 规则解析，保持一致。")
    L.append("- 用户安装的扩展（GitLens、C# Dev Kit 等）贡献的键位不属于 VS Code 默认，已排除。\n")
    L.append("## 2. 统计\n")
    L.append("| 判定 | 条数 |\n|---|---|")
    for key, label in [(("impl", "P0"), "实现 P0"), (("impl", "P1"), "实现 P1"), (("impl", "P2"), "实现 P2"), (("no", ""), "不做"), (("ext", ""), "扩展贡献（排除）")]:
        L.append(f"| {label} | {stats[key]} |")
    L.append(f"| **合计** | {total} |\n")
    L.append(f"另外，导出文件末尾还列出了 {len(unbound)} 个没有默认键位的命令。它们不影响键位兼容；实现某个功能时，相应命令的 ID 同样沿用 VS Code。\n")

    L.append("## 3. 需要实现的键位（按功能区）\n")
    for area in sorted(impl, key=lambda a: (min(PRI_ORDER[x[0]] for x in impl[a]), a)):
        items = sorted(impl[area], key=lambda x: (PRI_ORDER[x[0]], x[2], x[1]))
        L.append(f"### {area}（{len(items)}）\n")
        L.append("| 优先级 | 键 | 命令 | when |\n|---|---|---|---|")
        for pr, key, cmd, when, _ in items:
            L.append(f"| {pr} | `{esc(key)}` | `{esc(cmd)}` | {('`' + esc(when) + '`') if when else ''} |")
        L.append("")

    L.append("## 4. 不做的键位（按原因）\n")
    for area in sorted(no):
        cmds = sorted(no[area])
        L.append(f"<details><summary>{area}：{len(cmds)} 个命令</summary>\n")
        L.append(", ".join(f"`{x}`" for x in cmds))
        L.append("\n</details>\n")

    L.append("## 5. Rove 专有命令\n")
    L.append("命令 ID 以 `rove.` 开头。默认键位已与 VS Code 的全部默认键位逐一核对，没有冲突；还特意避开了 `ctrl+alt+a`（国内常见的截图全局热键）。\n")
    L.append("| 优先级 | 命令 | 默认键 | when | 说明 | 冲突检查 |\n|---|---|---|---|---|---|")
    chord_first = {k.split(" ")[0] for k in vs_keys if " " in k}
    for cmd, key, when, pr, desc in ROVE_COMMANDS:
        chk = ""
        if key:
            hit = vs_keys.get(key)
            chk = "无冲突" if not hit and key not in chord_first else f"冲突：{hit}"
        L.append(f"| {pr} | `{cmd}` | {('`' + key + '`') if key else '—'} | {('`' + when + '`') if when else ''} | {desc} | {chk} |")
    L.append("")

    L.append("## 6. 需要支持的 `when` 上下文键\n")
    L.append("以下是“需要实现的键位”中引用到的上下文键（按引用次数排序）。Rove 必须实现这些上下文键；不认识的上下文键一律视为 false（FEATURES §13 规则 7）。\n")
    L.append(", ".join(f"`{k}`（{n}）" for k, n in ctx.most_common()))
    L.append("")
    (OUT / "KEYBINDINGS.md").write_text("\n".join(L), encoding="utf-8")
    return stats


def gen_settings():
    rows = c.classify_settings()
    stats = Counter()
    impl = defaultdict(list)
    no = defaultdict(list)
    for key, val, desc, (d, pr, area, note) in rows:
        stats[(d, pr)] += 1
        if d == "impl":
            impl[area].append((pr, key, val, desc, note))
        elif d == "no":
            no[area].append(key)

    L = []
    L.append("# 设置兼容清单（阶段 0.3b）\n")
    L.append(f"> 版本：v1.0（自动生成，2026-11-09 已确认判定规则） · 来源：VS Code {VSCODE_VERSION}的默认设置 · 生成脚本：`scripts/compat/gen_compat_docs.py` · 判定规则：`scripts/compat/rules.py`\n")
    L.append("## 1. 说明\n")
    L.append("- Rove 与 VS Code 共用 `settings.json`（D4）。下表中“支持”的设置项，Rove 必须遵循 VS Code 的语义和默认值；“不支持”的设置项会被静默忽略（FEATURES §13 规则 2）。")
    L.append("- “支持”指 Rove 会读取并遵循该设置项。功能本身的优先级决定它何时生效：例如 `editor.minimap.*` 标为 P1，是因为缩略图是 P1 功能。")
    L.append("- 用户安装的扩展贡献的设置项已排除。对判定有异议时，修改 `rules.py` 后重新生成。\n")
    L.append("## 2. 统计\n")
    L.append("| 判定 | 条数 |\n|---|---|")
    for k, label in [(("impl", "P0"), "支持 P0"), (("impl", "P1"), "支持 P1"), (("impl", "P2"), "支持 P2"), (("no", ""), "不支持（忽略）"), (("ext", ""), "扩展贡献（排除）")]:
        L.append(f"| {label} | {stats[k]} |")
    L.append(f"| **合计** | {sum(stats.values())} |\n")

    L.append("## 3. Rove 默认值与 VS Code 不同的设置项\n")
    L.append("这些默认值**只在用户没有设置该项时生效**，并且不会写入共用的设置文件（FEATURES §13 规则 5）。\n")
    L.append("| 设置项 | VS Code 默认值 | Rove 默认值 | 原因 |\n|---|---|---|---|")
    for k, a, b, why in DIFFERENT_DEFAULTS:
        L.append(f"| `{k}` | `{esc(a)}` | `{esc(b)}` | {why} |")
    L.append("")

    L.append("## 4. Rove 专有设置项（`rove.*`）\n")
    L.append("| 优先级 | 设置项 | 默认值 | 说明 |\n|---|---|---|---|")
    for k, v, pr, desc in ROVE_SETTINGS:
        L.append(f"| {pr} | `{k}` | `{esc(v)}` | {desc} |")
    L.append("")

    L.append("## 5. 支持的 VS Code 设置项（按功能区）\n")
    for area in sorted(impl, key=lambda a: (min(PRI_ORDER[x[0]] for x in impl[a]), a)):
        items = sorted(impl[area], key=lambda x: (PRI_ORDER[x[0]], x[1]))
        L.append(f"### {area}（{len(items)}）\n")
        L.append("| 优先级 | 设置项 | VS Code 默认值 | 说明 |\n|---|---|---|---|")
        for pr, key, val, desc, _ in items:
            first = desc.split("\n")[0] if desc else ""
            first = first if len(first) <= 70 else first[:69] + "…"
            L.append(f"| {pr} | `{esc(key)}` | `{esc(short(val))}` | {esc(first)} |")
        L.append("")

    L.append("## 6. 不支持的设置项（按原因）\n")
    for area in sorted(no):
        keys = sorted(no[area])
        heads = Counter(k.split(".")[0] for k in keys)
        L.append(f"<details><summary>{area}：{len(keys)} 项（{', '.join(f'{h} {n}' for h, n in heads.most_common(12))}）</summary>\n")
        L.append(", ".join(f"`{k}`" for k in keys))
        L.append("\n</details>\n")
    (OUT / "SETTINGS.md").write_text("\n".join(L), encoding="utf-8")
    return stats


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    print("keybindings", dict(gen_keybindings()))
    print("settings", dict(gen_settings()))
