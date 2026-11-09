"""生成 docs/issues/COVERAGE.md：FEATURES.md 中每一个 P0/P1 功能 → 负责的 issue。

规则：每一行都必须至少有一个负责 issue；FEATURES.md 新增或改名的 P0/P1 行如果没有出现在下面的映射里，脚本直接失败。
    python scripts/issues/coverage.py
"""
from __future__ import annotations
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from issues_data import ISSUES  # noqa: E402

# (FEATURES 行首单元格中的唯一片段, [负责的 issue 键（第一个为主负责）])
MAP = [
    ("标题栏（自定义标题栏", ["shell"]),
    ("活动栏", ["shell"]),
    ("主侧边栏", ["shell"]),
    ("编辑区（编辑器组网格）", ["groups"]),
    ("面板（底部）", ["shell"]),
    ("状态栏", ["shell", "statusbar"]),
    ("通知", ["shell"]),
    ("全屏 / Zen 模式", ["zen"]),
    ("语法高亮（tokenization）", ["syntax", "codeview-hl"]),
    ("语义高亮（semanticTokens）", ["semantic"]),
    ("行号、当前行高亮", ["codeview", "codeview-hl"]),
    ("括号匹配（bracketMatching）", ["codeview-hl"]),
    ("括号对着色", ["brackets"]),
    ("代码折叠（folding）", ["codeview-hl"]),
    ("粘性滚动", ["sticky"]),
    ("面包屑", ["outline"]),
    ("缩略图（minimap）", ["minimap"]),
    ("滚动条上的标记", ["minimap"]),
    ("同名高亮（wordHighlighter）", ["codeview-hl", "nav-ui"]),
    ("悬停（hover）", ["nav-ui"]),
    ("链接检测（links）", ["links"]),
    ("软换行", ["wrap"]),
    ("缩放", ["shell", "codeview"]),
    ("长行处理", ["largefile-ui"]),
    ("只读提示", ["codeview-hl"]),
    ("基本输入、IME、剪贴板", ["edit-ui", "codeview"]),
    ("撤销 / 重做", ["edit-core", "edit-ui"]),
    ("多光标（multicursor）", ["multicursor"]),
    ("行操作（linesOperations）", ["multicursor"]),
    ("单词操作", ["edit-core", "edit-ui"]),
    ("智能扩选", ["smartselect"]),
    ("注释切换", ["autoedit"]),
    ("自动缩进、缩进转换", ["autoedit", "edit-ui"]),
    ("自动闭合括号与引号", ["autoedit"]),
    ("末尾自动加换行、去除行尾空格", ["save"]),
    ("查找 / 替换（find）", ["findwidget", "replace"]),
    ("格式化（format）", ["structure"]),
    ("转到定义 / 声明 / 类型定义", ["nav-ui", "nav-extra"]),
    ("查找所有引用", ["peek", "nav-extra"]),
    ("速览（peekView", ["peek"]),
    ("转到实现", ["nav-ui"]),
    ("转到文件中的符号", ["outline"]),
    ("转到工作区中的符号", ["nav-extra"]),
    ("转到行", ["quickopen"]),
    ("导航历史", ["groups", "nav-ui"]),
    ("大纲视图（outline）", ["outline"]),
    ("问题面板（markers）", ["structure"]),
    ("不生效的预处理代码置灰", ["semantic", "idx-format"]),
    ("语言检测（languageDetection）", ["langdetect"]),
    ("语言状态（languageStatus）", ["statusbar", "idx-life"]),
    ("资源管理器（files）", ["explorer"]),
    ("资源管理器中的文件操作", ["fileops"]),
    ("文件排除", ["workspace"]),
    ("文件监听与自动重新加载", ["workspace", "save"]),
    ("快速打开（quickaccess", ["quickopen"]),
    ("命令面板", ["quickopen"]),
    ("全局搜索（search）", ["search"]),
    ("全局替换", ["replace-all"]),
    ("图片预览", ["image"]),
    ("跟随文件末尾", ["tail"]),
    ("欢迎页 / 最近打开", ["welcome"]),
    ("热退出", ["hotexit"]),
    ("颜色主题（themes）", ["theme", "ext-themes"]),
    ("文件图标主题", ["ext-themes"]),
    ("产品图标（codicons）", ["shell"]),
    ("设置（`settings.json`）", ["settings"]),
    ("设置界面（preferences）", ["settings-ui"]),
    ("键位（keybindings）", ["keys"]),
    ("界面语言（localization）", ["i18n", "contracts"]),
    ("Markdown 预览", ["markdown"]),
    ("Markdown 大纲、链接跳转", ["outline", "links", "markdown"]),
    ("JSON/YAML/XML 结构树视图", ["structure"]),
    ("上下文篮子", ["basket"]),
    ("JSON：高亮、折叠、校验、格式化", ["syntax", "structure"]),
    ("YAML、XML：高亮、折叠、校验", ["syntax", "structure"]),
    ("首批编程语言：C#", ["syntax", "l1", "router", "nav-ui", "peek"]),
    ("其他语言只做高亮和折叠", ["langs-extra"]),
]


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    ids = {k: f"R{i + 1:02d}" for i, (k, *_) in enumerate(ISSUES)}
    ms = {k: m for k, m, *_ in ISSUES}
    titles = {k: t.split("：")[0].split("（")[0] for k, _, _, _, t, _ in ISSUES}
    text = (ROOT / "docs" / "FEATURES.md").read_text(encoding="utf-8")
    rows, sec = [], None
    for line in text.splitlines():
        m = re.match(r"^## (\d+)\. (.+)", line)
        if m:
            sec = m.group(1)
        if line.startswith("|") and not line.startswith("|---") and sec in ("2", "3", "4", "5", "6", "7", "9"):
            cells = [c.strip() for c in line.strip("|").split("|")]
            pr = next((c for c in cells if re.match(r"^\**P[01]", c)), None)
            if pr:
                rows.append((sec, cells[0], pr.strip("*")))
    errors, out = [], []
    used = set()
    for sec, name, pr in rows:
        hits = [(f, ks) for f, ks in MAP if f in name]
        if len(hits) != 1:
            errors.append(f"§{sec} 「{name}」匹配到 {len(hits)} 条映射")
            continue
        f, ks = hits[0]
        used.add(f)
        for k in ks:
            if k not in ids:
                errors.append(f"「{name}」引用了不存在的 issue 键 {k}")
        owners = "、".join(f"{ids[k]} {titles[k]}（{ms[k]}）" for k in ks if k in ids)
        out.append(f"| §{sec} | {name} | {pr} | {owners} |")
    for f, _ in MAP:
        if f not in used:
            errors.append(f"映射「{f}」在 FEATURES.md 中找不到对应的行")
    L = ["# 功能覆盖表：FEATURES.md 的 P0/P1 → 负责 issue\n",
         "> 由 `scripts/issues/coverage.py` 生成，请勿直接修改。FEATURES.md 的 P0/P1 行每一行都必须有负责 issue；"
         "第一个为主负责，其余为参与方。M4 集成验收（R74）按本表逐行核对，未完成的 P1 一律是里程碑阻塞，除非 FEATURES.md 已正式改判。\n",
         "| 章节 | 功能 | 优先级 | 负责 issue |\n|---|---|---|---|", *out, ""]
    if errors:
        L.append("> ⚠ 校验错误：\n" + "\n".join(f"> - {e}" for e in errors))
    (ROOT / "docs" / "issues" / "COVERAGE.md").write_text("\n".join(L), encoding="utf-8")
    print(f"{len(out)} 行；错误：{errors or '无'}")
    if errors:
        sys.exit(1)


if __name__ == "__main__":
    main()
