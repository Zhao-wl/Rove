"""从 codicon.ttf 的 cmap 生成 `$(图标名)` 用的名称映射（#16）。

用法：python -I scripts/codicons/gen_mapping.py crates/rove-ui/assets/codicons/codicon.ttf \
          crates/rove-ui/assets/codicons/mapping.json

@vscode/codicons 的字体用图标名作为字形名（fantasticon 生成），所以 cmap 里的字形名就是图标名。
同一码位的别名（例如 gear / settings-gear）只保留字体里的那一个。需要 fontTools。
"""
import json
import sys

from fontTools.ttLib import TTFont

font = TTFont(sys.argv[1])
mapping = {}
for codepoint, name in font.getBestCmap().items():
    mapping.setdefault(name, codepoint)
with open(sys.argv[2], "w", encoding="utf-8", newline="\n") as out:
    json.dump(dict(sorted(mapping.items())), out, indent=0, separators=(",", ":"))
    out.write("\n")
print(f"{len(mapping)} icons", file=sys.stderr)
