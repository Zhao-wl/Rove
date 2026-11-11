"""从 VS Code 源码提取 `registerColor` 的默认值，生成 rove-theme 的颜色默认值表（#10）。

一次性脚本，开发时手动运行，运行期不联网、不读 VS Code 源码：

    python scripts/theme/gen_color_defaults.py <vscode 源码根目录> <版本号> <提交>

例：python scripts/theme/gen_color_defaults.py D:/Temp/vscode-1.145.2 1.145.2 c4453f8e...

输出 `crates/rove-theme/src/defaults/generated.rs`。只认 VS Code 源码里出现过的写法：
字符串（`#hex` 或另一个颜色键）、导出常量引用、`null`、`Color.white` 等常量、`Color.fromHex`、
`new Color(new RGBA(...))`、`darken/lighten/transparent/withOpacity/opaque/oneOf/
ifDefinedThenElse/lessProminent`，以及 `{ dark, light, hcDark, hcLight }` 对象。
遇到不认识的写法直接报错退出，不会静默生成错误的默认值。
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "crates" / "rove-theme" / "src" / "defaults" / "generated.rs"
KINDS = ("dark", "light", "hcDark", "hcLight")
# 与 base/common/color.ts 中的静态常量一致。
COLOR_CONSTS = {
    "white": "#ffffff",
    "black": "#000000",
    "red": "#ff0000",
    "blue": "#0000ff",
    "green": "#00ff00",
    "cyan": "#00ffff",
    "lightgrey": "#d3d3d3",
    "transparent": "#00000000",
}
TRANSFORMS = {
    "darken": 2,
    "lighten": 2,
    "transparent": 2,
    "withOpacity": 2,
    "opaque": 2,
    "ifDefinedThenElse": 3,
    "lessProminent": 4,
}

TOKEN = re.compile(
    r"""
    (?P<ws>\s+)
  | (?P<lc>//[^\n]*)
  | (?P<bc>/\*.*?\*/)
  | (?P<str>'(?:\\.|[^'\\])*'|"(?:\\.|[^"\\])*"|`(?:\\.|[^`\\])*`)
  | (?P<num>\d+(?:\.\d+)?(?:e-?\d+)?|\.\d+)
  | (?P<id>[A-Za-z_$][\w$]*)
  | (?P<p>=>|\.\.\.|[{}()\[\],.:;=+\-*/!?<>&|%])
    """,
    re.S | re.X,
)


def tokenize(text: str):
    out, i = [], 0
    while i < len(text):
        m = TOKEN.match(text, i)
        if not m:
            out.append(("p", text[i]))
            i += 1
            continue
        i = m.end()
        kind = m.lastgroup
        if kind in ("ws", "lc", "bc"):
            continue
        out.append((kind, m.group(kind)))
    return out


class Parser:
    """够用的 JS 表达式解析器：生成元组形式的语法树，不认识的结构保留为 ('?', 文本)。"""

    def __init__(self, toks, pos):
        self.t, self.i = toks, pos

    def peek(self, k=0):
        j = self.i + k
        return self.t[j] if j < len(self.t) else ("eof", "")

    def take(self, value=None):
        tok = self.peek()
        if tok[0] == "eof":
            raise SyntaxError("unexpected end of file")
        if value is not None and tok[1] != value:
            raise SyntaxError(f"expected {value!r}, got {tok!r}")
        self.i += 1
        return tok

    def expr(self):
        node = self.unary()
        # 二元运算只会出现在描述文字里（字符串拼接）；保留为未知节点。
        while self.peek()[1] in ("+", "-", "*", "/", "||", "&&", "?", ":") and self.peek()[0] == "p":
            if self.peek()[1] in ("?", ":"):
                break
            op = self.take()[1]
            node = ("bin", op, node, self.unary())
        return node

    def unary(self):
        if self.peek() == ("p", "-"):
            self.take()
            inner = self.unary()
            if inner[0] == "num":
                return ("num", -inner[1])
            return ("neg", inner)
        if self.peek() == ("p", "!"):
            self.take()
            return ("not", self.unary())
        return self.postfix(self.primary())

    def args(self):
        self.take("(")
        out = []
        while self.peek()[1] != ")":
            out.append(self.expr())
            if self.peek()[1] == ",":
                self.take()
        self.take(")")
        return out

    def postfix(self, node):
        while True:
            tok = self.peek()
            if tok == ("p", "."):
                self.take()
                node = ("member", node, self.take()[1])
            elif tok == ("p", "("):
                node = ("call", node, self.args())
            elif tok == ("p", "["):
                self.take()
                idx = self.expr()
                self.take("]")
                node = ("index", node, idx)
            elif tok == ("p", "<") and node[0] in ("id", "member"):
                # 泛型参数，如 localize2<...>；本表用不到，跳过尖括号。
                depth, start = 0, self.i
                while True:
                    if self.i - start > 30:
                        raise SyntaxError("not a generic argument list")
                    t = self.take()[1]
                    depth += t == "<"
                    depth -= t == ">"
                    if depth == 0:
                        break
            elif tok == ("id", "as"):
                self.take()
                self.primary()
            else:
                return node

    def primary(self):
        kind, value = self.take()
        if kind == "str":
            return ("str", bytes(value[1:-1], "utf-8").decode("unicode_escape"))
        if kind == "num":
            return ("num", float(value))
        if kind == "id":
            if value == "null" or value == "undefined":
                return ("null",)
            if value == "new":
                callee = self.primary()
                while self.peek() == ("p", "."):
                    self.take()
                    callee = ("member", callee, self.take()[1])
                return ("new", callee, self.args() if self.peek()[1] == "(" else [])
            return ("id", value)
        if value == "(":
            node = self.expr()
            self.take(")")
            return node
        if value == "{":
            props = {}
            while self.peek()[1] != "}":
                key = self.take()[1].strip("'\"")
                if self.peek()[1] == ":":
                    self.take()
                    props[key] = self.expr()
                else:
                    props[key] = ("id", key)
                if self.peek()[1] == ",":
                    self.take()
            self.take("}")
            return ("obj", props)
        if value == "[":
            items = []
            while self.peek()[1] != "]":
                items.append(self.expr())
                if self.peek()[1] == ",":
                    self.take()
            self.take("]")
            return ("arr", items)
        return ("?", value)


def scan_files(src: Path):
    files = []
    for path in sorted(src.rglob("*.ts")):
        rel = path.relative_to(src).as_posix()
        if "/test/" in rel or rel.startswith("vs/sessions/") or rel.endswith(".d.ts"):
            continue
        text = path.read_text(encoding="utf-8")
        if "registerColor(" in text:
            files.append((rel, text))
    return files


def collect(files):
    """返回 (颜色键 → (来源文件, 默认值表达式)) 与 (常量名 → 表达式)。"""
    colors, consts, order, local = {}, {}, [], {}
    for rel, text in files:
        toks = tokenize(text)
        own = local.setdefault(rel, {})
        for i, (kind, value) in enumerate(toks):
            # `const NAME = <expr>` 收集常量（包括 registerColor 的返回值即颜色键）。
            if kind == "id" and value == "const" and toks[i + 1][0] == "id" and toks[i + 2] == ("p", "="):
                name = toks[i + 1][1]
                try:
                    node = Parser(toks, i + 3).expr()
                except (SyntaxError, IndexError):
                    continue
                if node[0] == "call" and node[1] == ("id", "registerColor"):
                    node = ("str", node[2][0][1])
                    consts[name] = node  # 导出的颜色常量，跨文件引用
                own.setdefault(name, node)
            if kind == "id" and value == "registerColor" and toks[i + 1] == ("p", "("):
                prev = toks[i - 1]
                if prev[1] in ("function", "."):
                    continue  # 定义处或 colorRegistry.registerColor 内部实现
                try:
                    args = Parser(toks, i + 1).args()
                except (SyntaxError, IndexError) as e:
                    raise SystemExit(f"{rel}: 无法解析 registerColor 调用：{e}")
                if not args or args[0][0] != "str":
                    continue  # registerColor(id, ...) 之类的转发
                cid = args[0][1]
                if cid in colors:
                    continue
                colors[cid] = (rel, args[1] if len(args) > 1 else ("null",))
                order.append(cid)
    return colors, consts, order, local


class Scope(dict):
    """导出常量表，外加当前文件的本地常量（同名时本地优先）。"""

    local: dict = {}


class Unknown(Exception):
    pass


def lower(node, consts, depth=0):
    """把语法树转换成规范形式：('hex',s) | ('ref',id) | ('none',) | ('op', name, args...)。"""
    if depth > 40:
        raise Unknown("引用层数过深")
    kind = node[0]
    if kind == "null":
        return ("none",)
    if kind == "str":
        s = node[1]
        return ("hex", s.lower()) if s.startswith("#") else ("ref", s)
    if kind == "num":
        return ("num", node[1])
    if kind == "id":
        if node[1] in consts.local:
            return lower(consts.local[node[1]], consts, depth + 1)
        if node[1] in consts:
            return lower(consts[node[1]], consts, depth + 1)
        raise Unknown(f"未知常量 {node[1]}")
    if kind == "member" and node[1] == ("id", "Color") and node[2] in COLOR_CONSTS:
        return ("hex", COLOR_CONSTS[node[2]])
    if kind == "call":
        callee = node[1]
        if callee == ("member", ("id", "Color"), "fromHex"):
            return lower(node[2][0], consts, depth + 1)
        if callee[0] == "id" and callee[1] in TRANSFORMS:
            args = [lower(a, consts, depth + 1) for a in node[2]]
            return ("op", callee[1], *args)
        if callee[0] == "member" and callee[2] in ("darken", "lighten", "transparent"):
            # Color 对象上的即时方法（如 Color.white.transparent(0.1)），与同名变换语义相同。
            return ("op", callee[2], lower(callee[1], consts, depth + 1), lower(node[2][0], consts, depth + 1))
        if callee == ("id", "oneOf"):
            return ("op", "oneOf", *[lower(a, consts, depth + 1) for a in node[2]])
    if kind == "new" and node[1] == ("id", "Color") and node[2] and node[2][0][0] == "new":
        inner = node[2][0]
        if inner[1] == ("id", "RGBA"):
            nums = [lower(a, consts, depth + 1)[1] for a in inner[2]]
            r, g, b = (int(x) for x in nums[:3])
            a = nums[3] if len(nums) > 3 else 1.0
            return ("rgba", r, g, b, a)
    if kind == "obj" and node[1].get("op") == ("member", ("id", "ColorTransformType"), "Mix"):
        p = node[1]
        ratio = lower(p["ratio"], consts, depth + 1) if "ratio" in p else ("num", None)
        return ("op", "mix", lower(p["color"], consts, depth + 1), lower(p["with"], consts, depth + 1), ratio)
    if kind == "obj" and set(node[1]) >= {"dark", "light"}:
        return ("kinds", {k: lower(node[1].get(k, ("null",)), consts, depth + 1) for k in KINDS})
    raise Unknown(f"不认识的写法 {node!r:.200}")


def rust(v) -> str:
    kind = v[0]
    if kind == "none":
        return "V::None"
    if kind == "hex":
        return f'V::Hex("{v[1]}")'
    if kind == "rgba":
        return f"V::Rgba({v[1]}, {v[2]}, {v[3]}, {fnum(v[4])})"
    if kind == "ref":
        return f'V::Ref("{v[1]}")'
    if kind == "num":
        raise Unknown("数字不能作为颜色")
    op, args = v[1], v[2:]
    if op in ("darken", "lighten", "transparent", "withOpacity"):
        name = {"darken": "Darken", "lighten": "Lighten", "transparent": "Transparent", "withOpacity": "Opacity"}[op]
        return f"V::{name}(&{rust(args[0])}, {fnum(args[1][1])})"
    if op == "opaque":
        return f"V::Opaque(&{rust(args[0])}, &{rust(args[1])})"
    if op == "mix":
        ratio = "None" if args[2][1] is None else f"Some({fnum(args[2][1])})"
        return f"V::Mix(&{rust(args[0])}, &{rust(args[1])}, {ratio})"
    if op == "oneOf":
        return "V::OneOf(&[" + ", ".join(rust(a) for a in args) + "])"
    if op == "ifDefinedThenElse":
        if args[0][0] != "ref":
            raise Unknown("ifDefinedThenElse 的条件必须是颜色键")
        return f'V::IfDefined("{args[0][1]}", &{rust(args[1])}, &{rust(args[2])})'
    if op == "lessProminent":
        return f"V::LessProminent(&{rust(args[0])}, &{rust(args[1])}, {fnum(args[2][1])}, {fnum(args[3][1])})"
    raise Unknown(f"未知变换 {op}")


def fnum(x: float) -> str:
    s = repr(float(x))
    return s if "." in s or "e" in s else s + ".0"


def p0_groups():
    doc = (ROOT / "docs" / "compat" / "THEME_COLORS.md").read_text(encoding="utf-8")
    section = doc.split("## 3. 支持（P0）", 1)[1].split("\n## ", 1)[0]
    for m in re.finditer(r"^\*\*(.+?)\*\*（.*?）：(.+)$", section, re.M):
        yield m.group(1), re.findall(r"`([^`]+)`", m.group(2))


def main():
    if len(sys.argv) != 4:
        print(__doc__)
        sys.exit(2)
    vscode, version, commit = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
    files = scan_files(vscode / "src")
    colors, exported, order, local = collect(files)
    rows, failures = [], []
    for cid in sorted(order):
        rel, node = colors[cid]
        try:
            consts = Scope(exported)
            consts.local = local[rel]
            v = lower(node, consts)
            per_kind = v[1] if v[0] == "kinds" else {k: v for k in KINDS}
            rows.append((cid, rel, [rust(per_kind[k]) for k in KINDS]))
        except Unknown as e:
            failures.append(f"{cid} ({rel}): {e}")
    if failures:
        print("无法解析以下颜色的默认值：", *failures, sep="\n  ")
        sys.exit(1)
    lines = [
        "// @generated by scripts/theme/gen_color_defaults.py — 不要手改，改脚本后重新生成。",
        f"// 来源：Microsoft VS Code {version}（提交 {commit}，MIT），src/vs/**/*.ts 中的 registerColor。",
        "// 每行：颜色键、[dark, light, hcDark, hcLight] 四套默认值。",
        "#![allow(clippy::all)]",
        "use super::V;",
        "",
        f'pub const VSCODE_VERSION: &str = "{version}";',
        f'pub const VSCODE_COMMIT: &str = "{commit}";',
        "",
        "pub static COLOR_DEFAULTS: &[(&str, [V; 4])] = &[",
    ]
    for cid, rel, vals in rows:
        lines.append(f"    // {rel}")
        lines.append(f'    ("{cid}", [{", ".join(vals)}]),')
    lines.append("];")
    lines += ["", "/// docs/compat/THEME_COLORS.md §3「支持（P0）」的键，按文档分组；注释是使用这些键的界面区域。",
              "pub static P0_KEYS: &[&str] = &["]
    known = {cid for cid, _, _ in rows}
    for group, keys in p0_groups():
        lines.append(f"    // {group}")
        for k in keys:
            if k not in known:
                print(f"P0 键 {k} 在 VS Code 源码中没有 registerColor")
                sys.exit(1)
            lines.append(f'    "{k}",')
    lines.append("];")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"写入 {OUT.relative_to(ROOT)}：{len(rows)} 个颜色键")


if __name__ == "__main__":
    main()
