"""Verify only the automated WebView2 runtime evidence, not visual/key/DPI acceptance."""
import json
import sys
from pathlib import Path

events = [json.loads(line) for line in Path(sys.argv[1]).read_text(encoding='utf-8-sig').splitlines()]
starts = [e['data'] for e in events if e['event'] == 'run_start']
if len(starts) != 1 or starts[0].get('schema') != 2:
    raise SystemExit('LEGACY/UNVERSIONED: not current acceptance evidence; use summarize.py for historical metrics only')
import re
for field in ('commit', 'source_git_blob'):
    assert re.fullmatch(r'[0-9a-f]{40}', starts[0].get(field, '')), f'Missing build {field}'
assert int(starts[0]['built_unix_seconds']) > 0
ready = [e['data'] for e in events if e['event'] == 'ready']
assert len(ready) == 2, 'Expected creation and recreation to become ready'
assert all(e['documentScriptBlocked'] is True for e in ready), 'Document script executed'
assert all(e.get('localImageLoaded') is True for e in ready), 'Local mapped image did not decode'
assert sum(e['event'] == 'resource_blocked' for e in events) == 2, 'Missing blocked-resource evidence'
assert sum(e['event'] == 'closed' for e in events) == 2, 'Missing lifecycle evidence'
assert not any(e['event'] == 'error' for e in events), 'Runtime error'
for kind in ('pong', 'themeApplied', 'scrollApplied'):
    assert any(e['event'] == 'roundtrip' and e['data'].get('kind') == kind for e in events), f'Missing {kind}'
assert any(e['event'] == 'message' and e['data'].get('type') == 'scroll' for e in events), 'Missing page-to-host scroll'
print('PASS: lifecycle, blocked document script/resource, local image, theme and bidirectional scroll/message probes')
print('NOT COVERED: visible overlap, physical keys, DPI, true absent runtime, OS network access, product budgets')
