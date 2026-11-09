# GPUI / WebView2 spike

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
