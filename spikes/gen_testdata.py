#!/usr/bin/env python3
# Generates test files for the M0 UI spike into spikes/testdata/ (gitignored).
#   python -I gen_testdata.py
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "testdata")
os.makedirs(OUT, exist_ok=True)

MIXED = [
    "// ASCII only: fn main() { let x = 42; }",
    "// 中文注释：这是一个用于测试 CJK 字形渲染与字体回退的示例行。",
    "// Mixed 混合 text with emoji 🚀🔥✅ and symbols ±≈→∞ ok",
    "// 日本語テキスト・한국어 텍스트 mixed with English",
    "let emoji_table = [\"😀\", \"🇨🇳\", \"👨‍👩‍👧‍👦\", \"🧑🏽‍💻\"]; // ZWJ + skin tone",
    "\tpublic void Foo(int bar) { /* tab indented, 宽窄混排 */ }",
    "const LONG: &str = \"" + ("x" * 300) + "\"; // very long line",
]


def gen_small():
    p = os.path.join(OUT, "small.txt")
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        f.write("// Rove M0 UI spike - small mixed-script sample\n")
        for i in range(400):
            f.write("%4d %s\n" % (i + 1, MIXED[i % len(MIXED)]))
    return p


def gen_big(target_bytes, name):
    p = os.path.join(OUT, name)
    # code-like ASCII payload with a CJK line every 50 lines
    block = []
    for i in range(2000):
        if i % 50 == 7:
            block.append(
                "    // 第 %d 行：超大文件中的中文内容，用于验证回退字体在虚拟滚动下的表现。\n" % i
            )
        else:
            block.append(
                "    public static int Method_%06d(int a, int b) { return a * %d + b; }\n"
                % (i, i)
            )
    chunk = ("".join(block)).encode("utf-8")
    written = 0
    with open(p, "wb") as f:
        while written < target_bytes:
            f.write(chunk)
            written += len(chunk)
    return p, written


def gen_unique(lines, name):
    """Every line is textually unique, so a shaped-line cache never hits.
    Used to measure the real text-shaping cost of each UI stack."""
    p = os.path.join(OUT, name)
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        for i in range(lines):
            if i % 50 == 7:
                f.write("    // 第 %d 行：唯一内容，用于测量真实的文本整形开销 %d\n" % (i, i * 7919))
            else:
                f.write(
                    "    public static int Method_%08d(int a%d, int b%d) { return a * %d + b; }\n"
                    % (i, i % 97, i % 89, i)
                )
    return p


if __name__ == "__main__":
    s = gen_small()
    print("small:", s, os.path.getsize(s), "bytes")
    u = gen_unique(400000, "uniq.txt")
    print("uniq:", u, os.path.getsize(u), "bytes")
    gb = 1024 * 1024 * 1024
    p, n = gen_big(gb, "big1g.txt")
    print("big:", p, n, "bytes")
    with open(p, "rb") as f:
        lines = 0
        while True:
            b = f.read(1 << 24)
            if not b:
                break
            lines += b.count(b"\n")
    print("big lines:", lines)
