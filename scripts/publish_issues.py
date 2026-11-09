"""把 docs/issues/ 中的 issue 草稿发布到 GitHub。

用法:
    python scripts/publish_issues.py            # 预演：只打印将要创建的内容
    python scripts/publish_issues.py --apply    # 实际创建标签、里程碑、issue 和路线图 issue

流程:
    1. 创建/更新标签（难度、领域、状态）与里程碑
    2. 按 R01..R42 顺序创建 issue（正文先保留 Rxx 文本）
    3. 第二遍编辑每个 issue：把正文中的 Rxx 替换为 #编号，加上元信息表头，
       并尝试设置 GitHub 原生的 "blocked by" 依赖关系
    4. 创建路线图 issue（docs/issues/README.md，Rxx 同样替换为 #编号）

幂等：已创建的映射保存在 docs/issues/.published.json，重复运行会跳过已创建的 issue，只刷新正文。
"""

from __future__ import annotations

import glob
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = "Zhao-wl/Rove"
ROOT = Path(__file__).resolve().parent.parent
ISSUE_DIR = ROOT / "docs" / "issues"
STATE = ISSUE_DIR / ".published.json"

MILESTONES = {
    "M1": "M1 基础与阅读器",
    "M2": "M2 C# 导航",
    "M3": "M3 编辑",
    "M4": "M4 文档渲染与发布",
}
DIFFICULTY = {
    "1": ("difficulty/1-easy", "0e8a16", "简单：需求明确、改动集中、几乎无需设计"),
    "2": ("difficulty/2-medium", "fbca04", "中等：需要一定的模块内设计"),
    "3": ("difficulty/3-hard", "d93f0b", "困难：跨模块接口、性能敏感、需要权衡"),
    "4": ("difficulty/4-expert", "b60205", "专家：深入框架或编译器内部，存在未知风险"),
}
AREAS = {
    "infra": "工程基础设施", "core": "核心层（文本缓冲、配置）", "syntax": "语法分析",
    "workspace": "工作区、搜索", "ui": "界面（GPUI）", "lang-cs": "C# 语言智能（Rove 侧）",
    "indexer": "C# 索引进程（.NET）", "docview": "文档类型渲染", "release": "打包发布",
}
STATUS = {
    "status/ready": ("0e8a16", "依赖已满足，可以认领"),
    "status/blocked": ("cccccc", "存在未完成的前置依赖"),
}


def parse_issues() -> dict[str, dict]:
    issues = {}
    for f in sorted(glob.glob(str(ISSUE_DIR / "0*.md"))):
        text = Path(f).read_text(encoding="utf-8")
        parts = re.split(r"^(?=## R\d\d )", text, flags=re.M)
        for part in parts:
            m = re.match(r"## (R\d\d) (.+)\n<!-- meta: (.*?) -->\n", part)
            if not m:
                continue
            meta = dict(kv.strip().split("=", 1) for kv in m.group(3).split(";"))
            body = part[m.end():].strip()
            body = re.sub(r"\n---\s*$", "", body).strip()
            issues[m.group(1)] = {
                "id": m.group(1),
                "title": m.group(2).strip(),
                "milestone": meta["milestone"],
                "difficulty": meta["difficulty"],
                "areas": [a for a in meta["area"].split(",") if a],
                "depends": [d for d in meta["depends"].split(",") if d],
                "body": body,
            }
    return issues


def batches(issues: dict[str, dict]) -> dict[str, int]:
    b: dict[str, int] = {}
    def depth(r: str) -> int:
        if r not in b:
            deps = issues[r]["depends"]
            b[r] = 1 + max((depth(d) for d in deps), default=0)
        return b[r]
    for r in issues:
        depth(r)
    return b


def gh(*args: str, input_text: str | None = None) -> str:
    r = subprocess.run(["gh", *args], input=input_text, capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:3])}… failed: {r.stderr.strip()}")
    return r.stdout.strip()


def replace_refs(text: str, mapping: dict[str, int]) -> str:
    return re.sub(r"\bR(\d\d)\b", lambda m: f"#{mapping[m.group(0)]}" if m.group(0) in mapping else m.group(0), text)


def header(issue: dict, mapping: dict[str, int], batch: int, dependents: list[str]) -> str:
    d = DIFFICULTY[issue["difficulty"]]
    dep = "、".join(f"#{mapping[x]}" for x in issue["depends"]) or "无"
    nxt = "、".join(f"#{mapping[x]}" for x in dependents) or "无"
    return (
        f"| 计划编号 | 难度 | 并行批次 | 前置依赖 | 被依赖 |\n|---|---|---|---|---|\n"
        f"| {issue['id']} | **{issue['difficulty']}** · {d[2]} | 第 {batch} 批 | {dep} | {nxt} |\n\n"
        f"> 认领前请阅读 `AGENTS.md` 与 `docs/issues/README.md`。前置依赖全部关闭后，本 issue 才可以开工。\n\n"
    )


def main() -> None:
    sys.stdout.reconfigure(encoding="utf-8")
    apply = "--apply" in sys.argv
    issues = parse_issues()
    order = sorted(issues)
    batch = batches(issues)
    dependents = {r: sorted(x for x in issues if r in issues[x]["depends"]) for r in issues}
    state = json.loads(STATE.read_text(encoding="utf-8")) if STATE.exists() else {}

    print(f"{len(issues)} issues → {REPO}  ({'APPLY' if apply else 'dry-run'})")
    for r in order:
        i = issues[r]
        labels = [DIFFICULTY[i["difficulty"]][0], *(f"area/{a}" for a in i["areas"]),
                  "status/ready" if not i["depends"] else "status/blocked"]
        print(f"  {r} [{MILESTONES[i['milestone']]}] 第{batch[r]}批 {i['title']}  labels={labels}  deps={i['depends']}")
    if not apply:
        print("\n预演结束。确认无误后加 --apply 执行。")
        return

    # 1. 标签与里程碑
    for name, color, desc in DIFFICULTY.values():
        gh("label", "create", name, "-R", REPO, "--color", color, "--description", desc, "--force")
    for a, desc in AREAS.items():
        gh("label", "create", f"area/{a}", "-R", REPO, "--color", "1d76db", "--description", desc, "--force")
    for name, (color, desc) in STATUS.items():
        gh("label", "create", name, "-R", REPO, "--color", color, "--description", desc, "--force")
    existing = {m["title"] for m in json.loads(gh("api", f"repos/{REPO}/milestones?state=all&per_page=100"))}
    for title in MILESTONES.values():
        if title not in existing:
            gh("api", f"repos/{REPO}/milestones", "-f", f"title={title}")

    # 2. 创建 issue
    for r in order:
        if r in state:
            continue
        i = issues[r]
        labels = [DIFFICULTY[i["difficulty"]][0], *(f"area/{a}" for a in i["areas"]),
                  "status/ready" if not i["depends"] else "status/blocked"]
        url = gh("issue", "create", "-R", REPO, "--title", f"[{r}] {i['title']}",
                 "--body", "（正文生成中）", "--milestone", MILESTONES[i["milestone"]],
                 *sum((["--label", l] for l in labels), []))
        state[r] = int(url.rstrip("/").rsplit("/", 1)[1])
        STATE.write_text(json.dumps(state, indent=2), encoding="utf-8")
        print(f"  created {r} → #{state[r]}")

    # 3. 第二遍：正文与依赖关系
    for r in order:
        i = issues[r]
        body = header(i, state, batch[r], dependents[r]) + replace_refs(i["body"], state)
        gh("issue", "edit", str(state[r]), "-R", REPO, "--body-file", "-", input_text=body)
        for d in i["depends"]:
            try:  # GitHub 原生 issue 依赖（若仓库/接口不支持则忽略）
                dep_id = json.loads(gh("api", f"repos/{REPO}/issues/{state[d]}"))["id"]
                gh("api", "-X", "POST", f"repos/{REPO}/issues/{state[r]}/dependencies/blocked_by",
                   "-F", f"issue_id={dep_id}")
            except RuntimeError as e:
                print(f"  (skip native dependency {r}←{d}: {str(e)[:80]})")
        print(f"  updated #{state[r]}")

    # 4. 路线图 issue
    roadmap = replace_refs((ISSUE_DIR / "README.md").read_text(encoding="utf-8"), state)
    if "roadmap" not in state:
        url = gh("issue", "create", "-R", REPO, "--title", "[Roadmap] Issue 拆分总览、依赖与并行批次",
                 "--body-file", "-", input_text=roadmap)
        state["roadmap"] = int(url.rstrip("/").rsplit("/", 1)[1])
    else:
        gh("issue", "edit", str(state["roadmap"]), "-R", REPO, "--body-file", "-", input_text=roadmap)
    STATE.write_text(json.dumps(state, indent=2), encoding="utf-8")
    print(f"roadmap → #{state['roadmap']}")


if __name__ == "__main__":
    main()
