"""Summarize measurements without confusing profile-cold with OS-cold."""
import json
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
results = []
for path in sorted(root.glob('*.jsonl')):
    events = [json.loads(line) for line in path.read_text(encoding='utf-8-sig').splitlines() if line.strip()]
    def values(kind, field):
        return [event['data'][field] for event in events if event['event'] == kind and field in event['data']]
    ready = values('ready', 'ms')
    rtt = sorted(values('roundtrip', 'ms'))
    memory_path = path.with_suffix('.memory.json')
    memory = json.loads(memory_path.read_text(encoding='utf-8-sig')) if memory_path.exists() else []
    active = [sample for sample in memory if 4 <= sample['seconds'] < 10]
    def quantile(fraction):
        return rtt[min(len(rtt)-1, int((len(rtt)-1)*fraction))] if rtt else None
    result = {
        'trial': path.stem,
        'first_render_ms': values('first_render','ms'),
        'controller_ms': values('controller','ms'),
        'ready_ms': ready,
        'script_blocked': values('ready','documentScriptBlocked'),
        'blocked_resource_count': sum(e['event']=='resource_blocked' for e in events),
        'rtt_samples':len(rtt), 'rtt_p50_ms':quantile(.5), 'rtt_p95_ms':quantile(.95), 'rtt_p99_ms':quantile(.99),
        'active_tree_working_set_mib_median':statistics.median(x['workingSetBytes'] for x in active)/1048576 if active else None,
        'active_tree_private_mib_median':statistics.median(x['privateBytes'] for x in active)/1048576 if active else None,
        'active_process_count_max':max((x['processCount'] for x in active),default=None),
        'errors':[e['data'] for e in events if e['event']=='error'],
    }
    results.append(result)
print(json.dumps(results,ensure_ascii=False,indent=2))
