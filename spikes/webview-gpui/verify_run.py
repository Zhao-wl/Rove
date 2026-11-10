"""Verify only the automated WebView2 runtime evidence, not visual/key/DPI acceptance."""
import json
import sys
from pathlib import Path

def require(condition, message):
    if not condition:
        raise SystemExit('INVALID EVIDENCE: ' + message)

events = [json.loads(line) for line in Path(sys.argv[1]).read_text(encoding='utf-8-sig').splitlines()]
starts = [e['data'] for e in events if e['event'] == 'run_start']
if len(starts) != 1 or starts[0].get('schema') != 2:
    raise SystemExit('LEGACY/UNVERSIONED: not current acceptance evidence; use summarize.py for historical metrics only')
import re
for field in ('commit', 'source_git_blob'):
    require(re.fullmatch('[0-9a-f]{40}', starts[0].get(field, '')), f'Missing build {field}')
require(int(starts[0]['built_unix_seconds']) > 0, 'Invalid build timestamp')
# Match exact build inputs, not just a plausible commit-shaped string.
import hashlib
root = Path(__file__).resolve().parent
payload = bytearray()
for name in ('build.rs', 'Cargo.toml', 'Cargo.lock', 'src/main.rs', 'assets/bridge.js', 'assets/index.html', 'assets/local.svg'):
    payload.extend(name.encode()); payload.append(0)
    payload.extend((root / name).read_bytes()); payload.append(0)
expected = hashlib.sha1(b'blob ' + str(len(payload)).encode() + b'\0' + payload).hexdigest()
if starts[0]['source_git_blob'] != expected:
    raise SystemExit('SOURCE MISMATCH: rebuild and record new evidence after changing build inputs; use summarize.py for historical logs')

ready = [e['data'] for e in events if e['event'] == 'ready']
require(len(ready) == 2, 'Expected creation and recreation to become ready')
require(all((e['documentScriptBlocked'] is True for e in ready)), 'Document script executed')
require(all((e.get('localImageLoaded') is True for e in ready)), 'Local mapped image did not decode')
require(sum((e['event'] == 'resource_blocked' for e in events)) == 2, 'Missing blocked-resource evidence')
require(sum((e['event'] == 'closed' for e in events)) == 2, 'Missing lifecycle evidence')
require(not any((e['event'] in ('error', 'layout_error') for e in events)), 'Runtime error')
for kind in ('pong', 'themeApplied', 'scrollApplied'):
    require(any((e['event'] == 'roundtrip' and e['data'].get('kind') == kind for e in events)), f'Missing {kind}')
require(any((e['event'] == 'message' and e['data'].get('type') == 'scroll' for e in events)), 'Missing page-to-host scroll')
print('PASS: lifecycle, blocked document script/resource, local image, theme and bidirectional scroll/message probes')
print('NOT COVERED: visible overlap, physical keys, DPI, true absent runtime, OS network access, product budgets')
