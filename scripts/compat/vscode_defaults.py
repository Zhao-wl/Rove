"""解析从 VS Code 导出的默认键位 / 默认设置（JSONC），供兼容清单生成脚本使用。"""
from __future__ import annotations
import json, re
from pathlib import Path

VSCODE_DIR = Path(__file__).resolve().parents[2] / "docs" / "compat" / "vscode"


def strip_jsonc(text: str) -> str:
    """去掉 // 与 /* */ 注释（字符串内的不动）和尾逗号。"""
    out, i, n = [], 0, len(text)
    in_str = False
    while i < n:
        c = text[i]
        if in_str:
            out.append(c)
            if c == "\\":
                out.append(text[i + 1]); i += 2; continue
            if c == '"':
                in_str = False
            i += 1
        elif c == '"':
            in_str = True; out.append(c); i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i); i = n if j < 0 else j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2); i = n if j < 0 else j + 2
        else:
            out.append(c); i += 1
    s = "".join(out)
    return re.sub(r",(\s*[\]}])", r"\1", s)


def load_keybindings():
    text = (VSCODE_DIR / "default-keybindings.json").read_text(encoding="utf-8-sig")
    bindings = json.loads(strip_jsonc(text))
    unbound = re.findall(r"^// - (\S+)", text, re.M)
    return bindings, unbound


def load_settings():
    text = (VSCODE_DIR / "default-settings.jsonc").read_text(encoding="utf-8-sig")
    values = json.loads(strip_jsonc(text))
    descs, section, pending = {}, {}, []
    current_section = ""
    for line in text.splitlines():
        m = re.match(r"^\t// ?(.*)$", line)
        if m:
            pending.append(m.group(1)); continue
        m = re.match(r'^\t"([^"]+)":', line)
        if m:
            key = m.group(1)
            # 段落标题：注释块中第一行（在空行后单独出现的标题会被合并进第一条设置）
            descs[key] = "\n".join(pending).strip()
            pending = []
            continue
        if not line.strip():
            if len(pending) == 1:  # 单独一行的注释视为段落标题
                current_section = pending[0]
            pending = [] if len(pending) == 1 else pending
        section_line = line
    return values, descs
