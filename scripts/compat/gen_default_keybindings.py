"""生成 Rove 的默认键位表 crates/rove-commands/data/default-keybindings.json（#14）。

输入：docs/compat/vscode/default-keybindings.json（VS Code 导出），判定规则：rules.py，
另加 gen_compat_docs.py 中的人工补充键位与 Rove 专有命令的默认键位。

- 只收录判定为“实现”（P0/P1/P2）的条目，保持导出文件中的原有顺序（VS Code 的裁决规则是“后定义优先”）；
- 人工补充键位与 Rove 专有命令的键位追加在最后；
- when 中导出者本机的用户目录统一替换为 %APPDATA%（与 KEYBINDINGS.md 一致），不把个人路径写进仓库。

用法：
    python scripts/compat/gen_default_keybindings.py          # 重新生成
    python scripts/compat/gen_default_keybindings.py --check  # CI：生成物与源数据不一致时以非零码退出
"""
from __future__ import annotations
import json, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import classify as c  # noqa: E402
from gen_compat_docs import EXTRA_BINDINGS, ROVE_COMMANDS  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "crates" / "rove-commands" / "data" / "default-keybindings.json"


def sanitize(when: str) -> str:
    return re.sub(r"[cC](?::|%3A)/Users/[^/]+/AppData/Roaming", "%APPDATA%", when)


def entries():
    rows, _ = c.classify_commands()
    out = []
    for b, cls in rows:
        if not cls or cls[0] != "impl":
            continue
        e = {"key": b["key"], "command": b["command"]}
        if b.get("when"):
            e["when"] = sanitize(b["when"])
        if "args" in b:
            e["args"] = b["args"]
        e["priority"] = cls[1]
        e["source"] = "vscode"
        out.append(e)
    for key, cmd, when, pr, _ in EXTRA_BINDINGS:
        e = {"key": key, "command": cmd}
        if when:
            e["when"] = when
        e.update(priority=pr, source="supplement")
        out.append(e)
    for cmd, key, when, pr, _ in ROVE_COMMANDS:
        if not key:
            continue
        e = {"key": key, "command": cmd}
        if when:
            e["when"] = when
        e.update(priority=pr, source="rove")
        out.append(e)
    return out


def render() -> str:
    lines = [json.dumps(e, ensure_ascii=False, separators=(", ", ": ")) for e in entries()]
    return "[\n" + ",\n".join(lines) + "\n]\n"


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    text = render()
    if "--check" in sys.argv[1:]:
        current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
        if current.replace("\r\n", "\n") != text:
            print(f"::error::{OUT.relative_to(ROOT)} 与源数据不一致，请运行 python scripts/compat/gen_default_keybindings.py 重新生成")
            sys.exit(1)
        print(f"{OUT.relative_to(ROOT)} 与源数据一致（{text.count(chr(10)) - 2} 条）")
    else:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_text(text, encoding="utf-8", newline="\n")
        print(f"已生成 {OUT.relative_to(ROOT)}（{text.count(chr(10)) - 2} 条）")
