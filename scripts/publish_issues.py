"""把 docs/issues/ 中渲染好的 issue 发布到 GitHub。

数据来源：scripts/issues/issues_data.py（骨架）+ docs/issues/Rxx-<键>.md（由 scripts/issues/plan.py 渲染的正文）
+ docs/issues/README.md（路线图总览）。发布前请先运行 `python scripts/issues/plan.py`。

用法:
    python scripts/publish_issues.py --milestones M1,M2            # 预演：只打印将要创建的内容
    python scripts/publish_issues.py --milestones M1,M2 --apply    # 实际创建

--milestones 必须显式指定，只发布已经审阅确认的里程碑（审阅状态见 docs/PLAN.md 阶段 0.4）。
依赖未发布里程碑中的 issue 时，正文中保留 Rxx 编号，待该里程碑发布后重新运行即可补上链接与依赖关系。

流程:
    1. 创建或更新标签（难度、领域、状态）与里程碑
    2. 按 R01..Rnn 顺序创建 issue（标题带 Rxx 前缀，正文先用占位）
    3. 第二遍：正文中的 Rxx 替换为 #编号，写入正文；尝试设置 GitHub 原生的 blocked-by 依赖
    4. 创建或更新路线图 issue（README.md，Rxx 同样替换为 #编号）

幂等：已创建的编号保存在 docs/issues/.published.json；重复运行会跳过已创建的 issue，只刷新正文与依赖。
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = "Zhao-wl/Rove"
ROOT = Path(__file__).resolve().parent.parent
ISSUE_DIR = ROOT / "docs" / "issues"
STATE = ISSUE_DIR / ".published.json"
sys.path.insert(0, str(ROOT / "scripts" / "issues"))
from issues_data import ISSUES  # noqa: E402
from plan import MILESTONES  # noqa: E402

DIFFICULTY = {
    1: ("difficulty/1-easy", "0e8a16", "简单：需求明确、改动集中、几乎无需设计"),
    2: ("difficulty/2-medium", "fbca04", "中等：需要一定的模块内设计"),
    3: ("difficulty/3-hard", "d93f0b", "困难：跨模块接口、性能敏感、需要权衡"),
    4: ("difficulty/4-expert", "b60205", "专家：深入框架或编译器内部，存在未知风险"),
}
AREAS = {
    "infra": "工程基础设施与集成验收", "core": "核心层（文本缓冲、设置、键位、编辑模型）", "syntax": "语法分析与高亮",
    "workspace": "工作区与搜索", "ui": "界面（GPUI）", "lang-cs": "C# 语言智能（Rove 侧）",
    "indexer": "C# 索引进程（.NET）", "docview": "文档类型视图", "release": "打包发布",
}
STATUS = {"status/ready": ("0e8a16", "依赖已满足，可以认领"), "status/blocked": ("cccccc", "存在未完成的前置依赖")}
RID = re.compile(r"\bR(\d{2})\b")


NETWORK_ERRORS = ("TLS handshake timeout", "timeout", "connection reset", "EOF", "502", "503", "504",
                  "error connecting", "check your internet connection", "connection refused", "no such host")
PROGRESS = ISSUE_DIR / ".publish_progress.json"


def gh(*args: str, input_text: str | None = None, retries: int = 8) -> str:
    """调用 gh；遇到网络类错误时指数退避重试。"""
    import time
    for attempt in range(retries):
        r = subprocess.run(["gh", *args], input=input_text, capture_output=True, text=True, encoding="utf-8")
        if r.returncode == 0:
            return r.stdout.strip()
        err = r.stderr.strip()
        if attempt < retries - 1 and any(e.lower() in err.lower() for e in NETWORK_ERRORS):
            time.sleep(min(2 ** attempt, 30))
            continue
        raise RuntimeError(f"gh {' '.join(args[:3])}… 失败：{err}")
    raise AssertionError("unreachable")


def load():
    issues = []
    for n, (key, ms, diff, area, title, deps) in enumerate(ISSUES, start=1):
        rid = f"R{n:02d}"
        body_file = ISSUE_DIR / f"{rid}-{key}.md"
        if not body_file.exists():
            raise SystemExit(f"缺少正文 {body_file.name}，请先运行 python scripts/issues/plan.py")
        body = body_file.read_text(encoding="utf-8")
        body = body.split("\n", 1)[1].lstrip()  # 去掉第一行标题（GitHub issue 标题单独设置）
        issues.append(dict(rid=rid, key=key, ms=ms, diff=diff, area=area, title=title, deps=deps, body=body))
    by_key = {i["key"]: i for i in issues}
    for i in issues:
        i["dep_rids"] = [by_key[d]["rid"] for d in i["deps"]]
        i["labels"] = [DIFFICULTY[i["diff"]][0], f"area/{i['area']}",
                       "status/blocked" if i["deps"] else "status/ready"]
    return issues


def replace_ids(text: str, mapping: dict[str, int]) -> str:
    return RID.sub(lambda m: f"#{mapping[m.group(0)]}" if m.group(0) in mapping else m.group(0), text)


def main() -> None:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    apply = "--apply" in sys.argv
    if "--milestones" not in sys.argv:
        raise SystemExit("必须用 --milestones 指定要发布的里程碑，例如 --milestones M1,M2（只发布已审阅的里程碑）")
    wanted = set(sys.argv[sys.argv.index("--milestones") + 1].split(","))
    unknown = wanted - set(MILESTONES)
    if unknown:
        raise SystemExit(f"未知的里程碑：{sorted(unknown)}")
    issues = [i for i in load() if i["ms"] in wanted]
    state = json.loads(STATE.read_text(encoding="utf-8")) if STATE.exists() else {}
    print(f"{len(issues)} 个 issue（{','.join(sorted(wanted))}）→ {REPO}（{'实际执行' if apply else '预演'}）")
    for i in issues:
        mark = "已创建" if i["rid"] in state else "新建"
        print(f"  [{mark}] {i['rid']} [{MILESTONES[i['ms']]}] {i['title'][:50]}  标签={i['labels']}  依赖={i['dep_rids']}")
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

    # 2. 创建
    for i in issues:
        if i["rid"] in state:
            continue
        args = ["issue", "create", "-R", REPO, "--title", f"[{i['rid']}] {i['title']}", "--body", "（正文生成中）",
                "--milestone", MILESTONES[i["ms"]]]
        for label in i["labels"]:
            args += ["--label", label]
        url = gh(*args)
        state[i["rid"]] = int(url.rstrip("/").rsplit("/", 1)[1])
        STATE.write_text(json.dumps(state, indent=2), encoding="utf-8")
        print(f"  已创建 {i['rid']} → #{state[i['rid']]}")

    # 3. 正文与依赖（记录进度，中断后重跑只处理未完成的部分）
    import hashlib
    prog = json.loads(PROGRESS.read_text(encoding="utf-8")) if PROGRESS.exists() else {"body": {}, "deps": []}
    def save_prog():
        PROGRESS.write_text(json.dumps(prog, indent=2, ensure_ascii=False), encoding="utf-8")
    failed = []
    for i in issues:
        num = state[i["rid"]]
        text = replace_ids(i["body"], state)
        digest = hashlib.sha1(text.encode("utf-8")).hexdigest()
        if prog["body"].get(i["rid"]) != digest:
            gh("issue", "edit", str(num), "-R", REPO, "--body-file", "-", input_text=text)
            prog["body"][i["rid"]] = digest
            save_prog()
        for d in i["dep_rids"]:
            edge = f"{i['rid']}<-{d}"
            if d not in state or edge in prog["deps"]:
                continue  # 依赖所在的里程碑尚未发布，或已经建立
            try:
                dep_id = json.loads(gh("api", f"repos/{REPO}/issues/{state[d]}"))["id"]
                gh("api", "-X", "POST", f"repos/{REPO}/issues/{num}/dependencies/blocked_by", "-F", f"issue_id={dep_id}")
            except RuntimeError as e:
                msg = str(e).lower()
                if "already" not in msg and "exists" not in msg:
                    failed.append(edge)
                    print(f"  （原生依赖未建立 {edge}：{str(e)[:80]}）")
                    continue
            prog["deps"].append(edge)
            save_prog()
        print(f"  已更新 #{num}")
    if failed:
        print(f"  有 {len(failed)} 条原生依赖未建立，重跑本脚本即可补上：{failed}")

    # 4. 路线图
    roadmap = replace_ids((ISSUE_DIR / "README.md").read_text(encoding="utf-8"), state)
    if "roadmap" not in state:
        url = gh("issue", "create", "-R", REPO, "--title", "[Roadmap] Rove issue 总览：拆分原则、依赖与并行批次",
                 "--body-file", "-", input_text=roadmap)
        state["roadmap"] = int(url.rstrip("/").rsplit("/", 1)[1])
    else:
        gh("issue", "edit", str(state["roadmap"]), "-R", REPO, "--body-file", "-", input_text=roadmap)
    STATE.write_text(json.dumps(state, indent=2), encoding="utf-8")
    print(f"路线图 → #{state['roadmap']}")


if __name__ == "__main__":
    main()
