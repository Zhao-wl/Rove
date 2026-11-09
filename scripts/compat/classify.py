"""按 rules.py 对 VS Code 默认键位与默认设置分类，打印统计与未分类项。"""
from __future__ import annotations
import re, sys
from collections import Counter
sys.path.insert(0, __file__.rsplit("\\", 1)[0].rsplit("/", 1)[0])
import vscode_defaults as vd
from rules import COMMAND_RULES, SETTING_RULES, EXT_NAMESPACES

_cmd = [(re.compile(p), d, pr, a, n) for p, d, pr, a, n in COMMAND_RULES]
_set = [(re.compile(p), d, pr, a, n) for p, d, pr, a, n in SETTING_RULES]


def is_ext(name: str) -> bool:
    head = name.lstrip("-").split(".")[0]
    return head in EXT_NAMESPACES


def classify(name: str, rules):
    if is_ext(name):
        return ("ext", "", "扩展", "用户安装的扩展，不属于 VS Code 默认")
    for rx, d, pr, a, n in rules:
        if rx.search(name):
            return (d, pr, a, n)
    return None


def classify_commands():
    bindings, unbound = vd.load_keybindings()
    rows = []
    for b in bindings:
        cmd = b["command"]
        rows.append((b, classify(cmd.lstrip("-"), _cmd)))
    return rows, unbound


def classify_settings():
    values, descs = vd.load_settings()
    return [(k, values[k], descs.get(k, ""), classify(k, _set)) for k in values]


if __name__ == "__main__":
    rows, _ = classify_commands()
    miss = sorted({b["command"] for b, c in rows if c is None})
    print("commands unclassified:", len(miss))
    print("\n".join(miss[:300]))
    print(Counter((c[0], c[1]) for _, c in rows if c))
    srows = classify_settings()
    smiss = [k for k, _, _, c in srows if c is None]
    print("settings unclassified:", len(smiss), smiss[:100])
    print(Counter((c[0], c[1]) for *_, c in srows if c))
