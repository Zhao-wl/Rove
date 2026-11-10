"""Summarize each run separately, including tail latency and missing provenance."""
import json
import sys
from pathlib import Path
from evidence_stats import summarize_events

results=[]
for path in sorted(Path(sys.argv[1]).glob("*.jsonl")):
    events=[json.loads(s) for s in path.read_text(encoding="utf-8-sig").splitlines() if s.strip()]
    results.append({"trial":path.stem,**summarize_events(events)})
print(json.dumps(results,ensure_ascii=False,indent=2))
