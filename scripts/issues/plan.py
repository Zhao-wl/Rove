"""Issue 拆分骨架（阶段 0.4）：唯一数据源。

每项：(键, 里程碑, 难度, 领域, 标题, [前置依赖的键])
编号 Rxx 按本列表顺序自动分配。运行本文件生成 docs/issues/README.md（总览、批次、校验）。
    python scripts/issues/plan.py
"""
from __future__ import annotations
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from issues_data import ISSUES  # noqa: E402

MILESTONES = {"M1": "M1 只读阅读器", "M2": "M2 C# 导航", "M3": "M3 轻编辑", "M4": "M4 体验补全", "M5": "M5 发布"}
DIFF = {1: "1 简单", 2: "2 中等", 3: "3 困难", 4: "4 专家"}


def build():
    ids = {k: f"R{i + 1:02d}" for i, (k, *_) in enumerate(ISSUES)}
    by = {k: (m, d, a, t, deps) for k, m, d, a, t, deps in ISSUES}
    errors = []
    for k, (m, d, a, t, deps) in by.items():
        for dep in deps:
            if dep not in by:
                errors.append(f"{k}: 未知依赖 {dep}")
    batch = {}

    def depth(k, stack=()):
        if k in stack:
            errors.append(f"循环依赖：{' -> '.join(stack + (k,))}")
            return 0
        if k not in batch:
            batch[k] = 1 + max((depth(x, stack + (k,)) for x in by[k][4] if x in by), default=0)
        return batch[k]

    for k in by:
        depth(k)
    order = {m: i for i, m in enumerate(MILESTONES)}
    for k, (m, d, a, t, deps) in by.items():
        for dep in deps:
            if dep in by and order[by[dep][0]] > order[m]:
                errors.append(f"{ids[k]}（{m}）依赖了更晚里程碑的 {ids[dep]}（{by[dep][0]}）")
    return ids, by, batch, errors


def render(ids, by, batch, errors):
    dependents = {k: [x for x in by if k in by[x][4]] for k in by}
    L = ["# Rove Issue 拆分骨架（阶段 0.4）\n",
         "> 版本：v0.2（骨架，待审阅；接口先行、纵向合并、集成验收） · 由 `scripts/issues/plan.py` 生成，请勿直接修改本文件",
         "> 依据：`FEATURES.md`、`UX_SPEC.md`、`docs/compat/*`、`PLAN.md` 的里程碑划分。骨架确认后，再按里程碑逐个编写 issue 正文。\n",
         "## 1. 难度分级（用于选择执行模型）\n",
         "| 难度 | 含义 | 参考模型 |\n|---|---|---|",
         "| 1 简单 | 需求完全明确，改动集中，几乎不需要设计决策 | Haiku / Sonnet |",
         "| 2 中等 | 需求明确，需要一定的模块内设计 | Sonnet |",
         "| 3 困难 | 跨模块接口设计、性能敏感、需要权衡；产出被多个 issue 依赖 | Opus |",
         "| 4 专家 | 深入第三方框架或编译器内部、逼近性能极限、有未知风险 | Opus / Fable（高 effort） |\n",
         "难度评的是“完成质量对模型能力的敏感度”，不是工作量。\n",
         "## 2. 统计\n"]
    c_m = Counter(v[0] for v in by.values())
    c_d = Counter(v[1] for v in by.values())
    L.append("| 里程碑 | 数量 |\n|---|---|")
    for m, name in MILESTONES.items():
        L.append(f"| {name} | {c_m[m]} |")
    L.append(f"| **合计** | {len(by)} |\n")
    L.append("难度分布：" + "、".join(f"{DIFF[d]} {c_d[d]} 个" for d in sorted(c_d)) + "\n")
    if errors:
        L.append("> ⚠ 校验错误：\n" + "\n".join(f"> - {e}" for e in errors) + "\n")
    else:
        L.append("校验：依赖都存在、没有循环依赖、没有依赖更晚的里程碑。\n")

    L.append("## 3. 并行批次\n")
    L.append("同一批次中的 issue 互相没有依赖，可以同时开工。某个 issue 的前置依赖全部完成后就可以开工，不必等整批结束。\n")
    L.append("| 批次 | 数量 | Issue |\n|---|---|---|")
    for b in sorted(set(batch.values())):
        ks = [k for k in by if batch[k] == b]
        L.append(f"| 第 {b} 批 | {len(ks)} | " + "、".join(ids[k] for k in ks) + " |")
    L.append("")
    # 关键路径：最长依赖链
    def chain(k):
        deps = by[k][4]
        if not deps:
            return [k]
        best = max((chain(d) for d in deps), key=len)
        return best + [k]
    longest = max((chain(k) for k in by), key=len)
    L.append("**最长依赖链**：" + " → ".join(f"{ids[k]}（{by[k][3].split('：')[0].split('（')[0]}）" for k in longest) + "\n")
    L.append("**认领建议**：\n")
    early = [k for k in by if batch[k] == 1 and k != "contracts"]
    L.append(f"- **接口先行**：{ids['contracts']}（接口契约）与 {ids['skeleton']}（行走骨架）必须串行完成并合并，建议交给最强的模型。"
             f"在此期间可以并行的是第 1 批中的其他条目（{'、'.join(ids[k] for k in early)}），以及只依赖它们的条目，"
             f"例如 C# 索引进程（{ids['idx-perf']}–{ids['idx-incr']}）。")
    L.append("- 批次只反映依赖关系。M4 中有些条目依赖很少、批次靠前，但建议优先推进 M1–M3，有空闲会话时再认领 M4。")
    L.append("- 每个里程碑最后的【集成验收】由一个会话负责，它的前置依赖就是该里程碑的全部条目。\n")

    for m, name in MILESTONES.items():
        L.append(f"## {name}\n")
        L.append("| ID | 标题 | 难度 | 领域 | 批次 | 前置依赖 | 被依赖 |\n|---|---|---|---|---|---|---|")
        for k, (mm, d, a, t, deps) in by.items():
            if mm != m:
                continue
            L.append(f"| {ids[k]} | {t} | {d} | {a} | {batch[k]} | {'、'.join(ids[x] for x in deps) or '—'} | {'、'.join(ids[x] for x in dependents[k]) or '—'} |")
        L.append("")
    return "\n".join(L)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    ids, by, batch, errors = build()
    out = Path(__file__).resolve().parents[2] / "docs" / "issues" / "README.md"
    out.parent.mkdir(exist_ok=True)
    out.write_text(render(ids, by, batch, errors), encoding="utf-8")
    print(len(by), "issues; errors:", errors or "none")
    print(Counter(batch.values()))
