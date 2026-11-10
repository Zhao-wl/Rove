# GPUI / WebView2 spike

中文操作入口：[C 链手测清单](MANUAL_ACCEPTANCE.md)（含 #5/#6 的本机启动命令、步骤、通过标准及反馈模板）。

预算已确认：60 MB 只约束未开启预览的基础阅读器；WebView2 预览单列预算，具体上限尚未设定。

Build with the MSVC developer environment:

```powershell
cargo build --manifest-path spikes/webview-gpui/Cargo.toml --release
spikes/webview-gpui/target/release/webview-gpui.exe
```

O opens the preview; U toggles the raw overlap fixture; H toggles the hide-WebView2
mitigation; X closes the controller; T sends a light theme. Click the preview to
test the keyboard matrix in `docs/M0_WEBVIEW_SPIKE_REPORT.md`. Record both the raw
and mitigated overlap. Test Microsoft Pinyin/DPI separately in the #5 prototype.

`--auto` runs creation, raw overlap, mitigated overlap, disposal and recreation,
then exits. JSON lines on stdout record controller/page timing, blocked resources,
document-script probe, key routing and message round trips.

`measure.ps1` runs three fresh-profile and three reused-profile trials, preserving
logs and process-tree memory samples under ignored `runs/`. These are not reboot
cold-start measurements. No data is uploaded. Do not delete shared WebView2 data.

For failure injection, set `ROVE_WEBVIEW_BROWSER_FOLDER` to a nonexistent directory
for one process only, then restore it; never uninstall the system runtime. This
checks a missing binary folder, not a truly uninstalled machine.

The spike is outside the workspace and uses crates.io GPUI, independently of #5.
Current acceptance and unverified items are listed in the report.

## Script-policy comparison

Default: document CSP `script-src 'none'` rejects inline/document scripts while the
host injects its own packaged bridge through ExecuteScript. Set
`ROVE_WEBVIEW_DISABLE_SCRIPT=1` only to reproduce the IsScriptEnabled(false)
comparison: direct host injection runs, but event-based bridge replies were absent
in this environment. Restore the variable after the comparison.

Desktop capture may be unavailable (black screen/foreground denial). Never count
a PrintWindow capture without child HWND contents as overlap proof. Raw JSON
evidence is under `evidence/2026-11-09/`; visual and physical-key acceptance remains
manual. Source and report explicitly distinguish these limits.

After an auto run, execute `python spikes/webview-gpui/verify_run.py <events.jsonl>`
to validate the runtime probes. This verifier deliberately does not mark manual
overlap, keyboard or DPI requirements as passed.

## Build and evidence prerequisites (review follow-up)

Use MSVC Developer PowerShell with the Windows SDK. Both `where.exe fxc` and
`git --version` must succeed before Cargo: GPUI build.rs needs fxc.exe; this spike
uses Git to embed provenance. CI owners must initialize that environment; this PR
does not modify .github/. No wry prototype was tested: direct COM was selected to
control the existing HWND without introducing another event-loop integration.

Every new run starts with schema 2 `run_start`: Git commit, source Git-blob
fingerprint, dirty flag and build Unix timestamp. Commit source before producing
release acceptance evidence; the run manifest must retain executable SHA-256.
The fingerprint covers build.rs, manifest/lock, main.rs and packaged assets.
Historical 2026-11-09 logs are unversioned intermediate-build measurements, not
proof of the current binary. Do not run the current strict verifier on them as if
they shared its schema. summarize.py labels missing response kinds as unknown,
and reports total/per-kind p99, maximum and >16ms count/percentage.

The image CSP intentionally allows HTTPS in this spike so that the remote-image
fixture reaches WebResourceRequested and exercises the 403/counting path. It is
not a production defense-in-depth CSP: the interceptor is the remote-image gate.
Product integration must combine sanitized local resources, a narrower CSP and
explicit per-document allow policy; do not copy the fixture policy unchanged.

Bounds and visibility are applied only when changed. Continuous frame requests
and parent-position notifications remain intentional benchmark instrumentation;
measurements are not idle CPU/memory evidence. Event tokens are retained by kind
and unsubscribed at Close. Async initialization/script strings are retained by
completion closures. Physical-key and visible-overlap acceptance is still pending.
