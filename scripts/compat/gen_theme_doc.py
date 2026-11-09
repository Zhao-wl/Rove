"""生成 docs/compat/THEME_COLORS.md：VS Code 主题颜色键的兼容判定。

输入：VS Code 官方文档仓库中的主题颜色参考（microsoft/vscode-docs: api/references/theme-color.md）。
本脚本通过 gh 下载到 out/theme-color.md（不提交），只在生成的清单中引用颜色键名，不复制原文说明。
    python scripts/compat/gen_theme_doc.py
"""
from __future__ import annotations
import base64, json, re, subprocess, sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "out" / "theme-color.md"
OUT = ROOT / "docs" / "compat" / "THEME_COLORS.md"

SECTION_RULES = {
    # P0：首版界面上都有对应元素
    "P0": ["Contrast colors", "Base colors", "Window border", "Text colors", "Action colors", "Button control",
           "Dropdown control", "Input control", "Scrollbar control", "Badge", "Progress bar", "Lists and trees",
           "Activity Bar", "Side Bar", "Editor Groups & Tabs", "Editor colors", "Editor widget colors",
           "Peek view colors", "Panel colors", "Status Bar colors", "Title Bar colors", "Menu Bar colors",
           "Command Center colors", "Notification colors", "Quick picker colors", "Keybinding label colors",
           "Breadcrumbs colors", "Symbol Icons colors", "Action Bar colors", "Welcome page colors", "Markdown"],
    "P1": ["Minimap", "Settings Editor colors", "Simple Find Widget colors"],
    "P2": ["Diff editor colors", "Keyboard shortcut table colors"],
    "no": ["Modern UI colors", "Profiles", "Chat colors", "Agent sessions colors", "Inline Chat colors",
           "Panel Chat colors", "Merge conflicts colors", "Banner colors", "Extensions colors",
           "Integrated Terminal colors", "Debug colors", "Testing colors", "Git colors",
           "Source Control Graph colors", "Snippets colors", "Debug Icons colors", "Notebook colors",
           "Chart colors", "Ports colors", "Comments View colors", "Gauge colors", "Agent Session colors",
           "Extension colors"],
}
SECTION_NOTE = {
    "Markdown": "Markdown 预览中的提示块（D5）",
    "Simple Find Widget colors": "Markdown 预览（WebView2）中的查找条",
    "Welcome page colors": "只用到“开始”和“最近”相关的颜色",
}

# 支持的分区中，逐个键排除（对应功能不做）
KEY_NO = re.compile(
    r"(CodeLens|codeLens|LightBulb|lightBulb|InlayHint|inlayHint|GhostText|ghostText|inlineEdit|InlineEdit|"
    r"[cC]hat|commentsWidget|CommentsWidget|editorCommentsWidget|SuggestWidget|suggestWidget|editorUnnecessaryCode|"
    r"editorGutter\.(added|modified|deleted|commentRange|commentGlyph|commentUnresolvedGlyph|itemGlyph|itemBackground|addedSecondary|modifiedSecondary|deletedSecondary)|"
    r"editorOverviewRuler\.(added|modified|deleted|comment|incoming|current|common|inlineChat)|"
    r"[dD]ebug|[tT]esting|[sS]cm|[tT]erminal|remote|Remote|offline|Offline|profile|Profile|extensionButton|extensionBadge|extensionIcon|"
    r"walkThrough|walkthrough|welcomePage\.(progress|tileBorder|tileShadow)|editorActionList|actionList|"
    r"editorHoverWidget\.highlightForeground|ports|notebook|inlineCompletions|editorParameterHint)")
KEY_P1 = re.compile(r"(editorStickyScroll|editorBracketHighlight|editorBracketPairGuide|editorMultiCursor|"
                    r"editorLink|editorError|editorWarning|editorInfo|editorHint|editorMarkerNavigation|minimap|"
                    r"editorOverviewRuler|editorRuler|editorWordHighlight|wordHighlight|editorSelectionHighlight)")
KEY_P2 = re.compile(r"(searchEditor|editorUnicodeHighlight|linkedEditing|diffEditor|editor\.placeholder|editorWatermark\b)")


def fetch():
    if SRC.exists():
        return SRC.read_text(encoding="utf-8")
    data = subprocess.run(["gh", "api", "repos/microsoft/vscode-docs/contents/api/references/theme-color.md"],
                          capture_output=True, text=True, check=True).stdout
    text = base64.b64decode(json.loads(data)["content"]).decode("utf-8")
    SRC.parent.mkdir(exist_ok=True)
    SRC.write_text(text, encoding="utf-8")
    return text


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    text = fetch()
    approved = re.search(r"DateApproved: (\S+)", text)
    sec_of = {s: p for p, ss in SECTION_RULES.items() for s in ss}
    rows = []
    section = None
    for line in text.splitlines():
        m = re.match(r"^## (.+)$", line)
        if m:
            section = m.group(1).strip(); continue
        m = re.match(r"^- `([^`]+)`", line)
        if m and section:
            rows.append((section, m.group(1)))
    unknown = sorted({s for s, _ in rows if s not in sec_of})
    out = defaultdict(lambda: defaultdict(list))
    stats = defaultdict(int)
    for sec, key in rows:
        p = sec_of.get(sec, "no")
        if p != "no":
            if KEY_NO.search(key):
                p = "no"
            elif p == "P0" and KEY_P2.search(key):
                p = "P2"
            elif p == "P0" and KEY_P1.search(key):
                p = "P1"
        out[p][sec].append(key)
        stats[p] += 1

    L = ["# 主题颜色键兼容清单（阶段 0.3c-1）\n",
         f"> 版本：v1.0（自动生成，2026-11-09 已确认判定规则） · 来源：VS Code 官方文档的主题颜色参考（vscode-docs，批准日期 {approved.group(1) if approved else '未知'}） · 生成脚本：`scripts/compat/gen_theme_doc.py`\n",
         "## 1. 说明\n",
         "- Rove 直接加载 VS Code 的颜色主题 JSON（FEATURES §7）。主题中 `colors` 部分的颜色键，按下表决定 Rove 是否使用；不使用的键静默忽略。",
         "- 主题没有提供某个键时，与 VS Code 一样回退到该键的默认值（深色、浅色、高对比度各一套）。默认值取自 VS Code 源码中 `registerColor` 的注册，需要在实现时整理成数据表（不在本清单中）。",
         "- 优先级跟随对应功能的优先级，例如缩略图是 P1，`minimap.*` 就是 P1。",
         "- 对判定有异议时，修改生成脚本中的规则后重新生成。\n",
         "## 2. 统计\n", "| 判定 | 颜色键数 |\n|---|---|"]
    for p, label in [("P0", "支持 P0"), ("P1", "支持 P1"), ("P2", "支持 P2"), ("no", "不支持（忽略）")]:
        L.append(f"| {label} | {stats[p]} |")
    L.append(f"| **合计** | {len(rows)} |\n")
    if unknown:
        L.append(f"> 注意：以下分区没有判定规则，已按“不支持”处理：{', '.join(unknown)}\n")
    for p, title in [("P0", "3. 支持（P0）"), ("P1", "4. 支持（P1）"), ("P2", "5. 支持（P2）")]:
        L.append(f"## {title}\n")
        for sec in sorted(out[p]):
            keys = out[p][sec]
            note = f" · {SECTION_NOTE[sec]}" if sec in SECTION_NOTE else ""
            L.append(f"**{sec}**（{len(keys)}{note}）：" + ", ".join(f"`{k}`" for k in keys) + "\n")
    L.append("## 6. 不支持（按分区）\n")
    for sec in sorted(out["no"]):
        keys = out["no"][sec]
        L.append(f"<details><summary>{sec}：{len(keys)}</summary>\n\n" + ", ".join(f"`{k}`" for k in keys) + "\n\n</details>\n")
    OUT.write_text("\n".join(L), encoding="utf-8")
    print(dict(stats), "unknown sections:", unknown)


if __name__ == "__main__":
    main()
