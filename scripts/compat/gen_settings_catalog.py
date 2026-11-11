"""生成 crates/rove-config/data/settings-catalog.json：Rove 登记的全部设置项（#13）。

与 docs/compat/SETTINGS.md 同源：VS Code 导出的默认设置 + rules.py 的判定 + gen_compat_docs.py 里的
rove.* / 隐藏设置 / 默认值差异表。判定有异议时改 rules.py，然后依次运行：
    python scripts/compat/gen_compat_docs.py
    python scripts/compat/gen_settings_catalog.py
rove-config 的测试会对照 SETTINGS.md 检查这份数据，防止漏登记或偷偷支持“不支持”的键。
"""
from __future__ import annotations
import json, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import classify as c  # noqa: E402
import gen_compat_docs as g  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "crates" / "rove-config" / "data" / "settings-catalog.json"

# VS Code 中可按语言覆盖（scope = language-overridable）的 files.* 设置项。
# 导出的默认设置不带 scope 信息，editor.* / diffEditor.* 统一视为可覆盖，其余按此表与
# 默认语言覆盖里出现过的键判定。
LANGUAGE_OVERRIDABLE_FILES = {
    "files.encoding",
    "files.autoGuessEncoding",
    "files.candidateGuessEncodings",
    "files.eol",
    "files.trimTrailingWhitespace",
    "files.trimTrailingWhitespaceInRegexAndStrings",
    "files.insertFinalNewline",
    "files.trimFinalNewlines",
}


def main() -> None:
    rows = c.classify_settings()
    lang_blocks = {k: v for k, v, _, cls in rows if k.startswith("[") and cls and cls[0] == "impl"}
    in_lang_defaults = {key for block in lang_blocks.values() for key in block}

    entries: dict[str, dict] = {}

    def overridable(key: str) -> bool:
        return (
            key.startswith(("editor.", "diffEditor."))
            or key in LANGUAGE_OVERRIDABLE_FILES
            or key in in_lang_defaults
        )

    for key, value, _, cls in rows:
        if key.startswith("[") or not cls or cls[0] != "impl":
            continue
        entries[key] = {"key": key, "priority": cls[1], "source": "vscode", "default": value,
                        "languageOverridable": overridable(key)}
    for key, value, pr, _ in g.HIDDEN_SETTINGS:
        entries[key] = {"key": key, "priority": pr, "source": "vscode-hidden", "default": json.loads(value),
                        "languageOverridable": overridable(key)}
    for key, value, pr, _ in g.ROVE_SETTINGS:
        entries[key] = {"key": key, "priority": pr, "source": "rove", "default": json.loads(value),
                        "languageOverridable": False}
    for key, vscode, rove, _ in g.DIFFERENT_DEFAULTS:
        entry = entries[key]  # 差异项必须是已支持的 VS Code 设置
        assert entry["default"] == json.loads(vscode), key
        entry["roveDefault"] = json.loads(rove)

    supported = set(entries)
    language_defaults = {
        k: {key: val for key, val in block.items() if key in supported and entries[key]["languageOverridable"]}
        for k, block in sorted(lang_blocks.items())
    }

    lines = ["{",
             '  "generatedBy": "scripts/compat/gen_settings_catalog.py",',
             f'  "vscodeVersion": {json.dumps(g.VSCODE_VERSION, ensure_ascii=False)},',
             '  "settings": [']
    items = [json.dumps(entries[k], ensure_ascii=False) for k in sorted(entries)]
    lines += [f"    {item}," for item in items[:-1]] + [f"    {items[-1]}", "  ],", '  "languageDefaults": {']
    langs = [f"    {json.dumps(k)}: {json.dumps(v, ensure_ascii=False)}" for k, v in language_defaults.items()]
    lines += [line + "," for line in langs[:-1]] + [langs[-1], "  }", "}", ""]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(f"{OUT.relative_to(ROOT)}: {len(entries)} settings, {len(language_defaults)} language blocks")


if __name__ == "__main__":
    main()
