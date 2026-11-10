# Rove GPUI patches

## Upstream provenance

Vendored crates.io `gpui` **0.2.2**, copied from the local Cargo registry package, not from Zed git. Archive SHA-256: `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`. Original `LICENSE-APACHE`, `Cargo.toml.orig`, assets and examples are retained. License: Apache-2.0.

## R05 — Windows IMM32 candidate positioning

- `src/platform/windows/window.rs`: original `update_ime_position` was empty with the comment “There is no such thing on Windows.” It now forwards logical bounds to the Windows backend.

- `src/platform/windows/events.rs`: original `handle_ime_position` queried the whole selected range, placed candidates at mid-line using `CFS_CANDIDATEPOS`, and acquired HIMC before an early return that leaked the context. It now queries the active UTF-16 selection endpoint as an empty range, then shares positioning with `update_ime_position`.

- The shared helper reads the current window scale factor, rounds logical bounds into physical client coordinates, calls `ImmSetCompositionWindow(CFS_POINT)` and `ImmSetCandidateWindow(CFS_EXCLUDE)` with the caret rectangle, and releases every non-null HIMC. No screen-coordinate conversion is performed: IMM32 expects client coordinates. Candidate placement at screen edges remains the IME's responsibility and requires manual testing.

- Original `handle_ime_composition` only applied composition text. It now refreshes geometry after applying the event and releasing its context, preserving the original event return value only for non-null contexts. **Behavior change:** with a NULL HIMC the message is forwarded to the Windows default procedure (`None`) unless `lparam == 0`; see the NULL-HIMC entry below. No release is needed for NULL.

## Reapplying on upgrade

1. Obtain the published upstream archive; preserve its license and compare upstream IMM32 handling before carrying changes forward.

2. Check whether the empty platform hook and start-only positioning have already been fixed. If so, drop the corresponding patch after regression validation.

3. Reapply the forwarding hook, shared geometry helper, and post-composition update together. Preserve UTF-16 selection semantics and balanced acquire/release.

4. Build `spikes/ime-gpui`, run fmt/clippy/test, then follow its README on Microsoft Pinyin at 100% and a non-100% scale. Repeat after moving between monitors.

5. Recheck original and patched dependency counts, clean build duration and executable sizes under identical profiles. Do not count the independent spike as a workspace E2E pass.

## Validation status

Release build and prototype cargo check/clippy succeeded on 2026-11-09. The fresh release target took 9m51s with dependencies cached, while other work was running; this is not a controlled comparison against the historical 469s baseline. The prototype executable is 6,481,408 bytes. cargo tree resolves gpui to this vendor directory. Root Cargo.lock correctly records patch.unused because the current main UI crate has no GPUI dependency yet. Workspace fmt passed after rebase onto baseline fix 8e2a91b; workspace strict clippy (`--all-targets -- -D warnings`) and test passed after rebase. Prototype strict clippy and test also passed; that 2026-11-09 run contained zero automated tests, so these commands are compile/integration checks, not IME behavior coverage. Dependency counts (Windows target, package identities deduplicated) are 411 including the prototype root before and after patching. Interactive acceptance is not yet confirmed. See `spikes/ime-gpui/README.md`; no manual acceptance item is claimed as passed.

## Integration requirements and outstanding review items (2026-11-10)

- WM_IME_COMPOSITION updates text before querying geometry, but `bounds_for_range` may still read the previous frame's layout. The synchronous update alone can lag one frame. **R46 must call `Window::invalidate_character_coordinates()` when caret/layout changes**, so positioning is refreshed after the new layout. The IME prototype does this in TextInput::render; its correctness must not be attributed to the vendor event hook alone. Repeat after scroll, resize, caret movement and DPI changes.
- Empty UTF-16 selections can produce a zero-width `CFS_EXCLUDE.rcArea` (observed 0px × 20px). This patch deliberately retains the input handler geometry. Right/bottom-edge flip and line exclusion require real IME acceptance; do not mark them passed from code inspection.
- Build prerequisite: MSVC plus Windows SDK `fxc.exe` on PATH (GPUI build.rs invokes it). CI owners must initialize a VS developer environment and assert `where.exe fxc`; `.github/` remains outside this PR's ownership.
- Track upstream float-literal compatibility warnings in `src/taffy.rs:312` and proc-macro-error2 future incompatibility separately. Do not silently broaden this Windows patch.
- `patch.unused` remains until a workspace GPUI consumer exists; do not add a dummy dependency just to silence it. Upstream examples/assets remain intact for provenance, including approximately 4.6 MB of images.
- The prototype intentionally keeps the M0 viewer for before/after comparison; it is not a new minimal editor. Its composition-relative UTF-16 selection bug is fixed with four regression tests in `spikes/ime-gpui/src/ime_selection.rs`.

### R46 UTF-16 integration requirement (second review)

EntityInputHandler must interpret new_selected_range (UTF-16) relative to the replacement composition text, convert UTF-16 offsets inside that text to UTF-8 byte offsets, then add the replacement start. Do not convert against the full document: non-ASCII prefixes can produce out-of-bounds selections. See spikes/ime-gpui/src/ime_selection.rs and its four regression tests.

The spike is excluded from the workspace: cargo test --workspace does not execute these tests. Run cargo test --manifest-path spikes/ime-gpui/Cargo.toml separately.

The historical spikes/ui-gpui prototype still contains the old conversion and may panic when copying a composition after a non-ASCII prefix. It is outside this chain's permitted edit scope; use spikes/ime-gpui for IME acceptance and do not copy the old implementation into R46.

### Manual feedback 2026-11-10 (reported by reviewer)

Windows 11 23H2 22631.6199, Microsoft Pinyin 10.0.22621.5840, 1920x1080, 100%/125%: edge flipping, composition interruption/window switching, Chinese/English switching and emoji-prefix selection passed. Long sentence committed correctly; candidate anchor moved between committed segments. At 125%, three screen-edge cases passed. Within unchanged composition text, moving the caret left changed reported bounds but the candidate popup stayed still. Compare the same action in Notepad/VS Code before classifying this as system behavior. First coordinate query can still reflect the previous frame; the next is current.

Third-party IME untested. The manual run used an existing release executable without an embedded build identity; its precise commit cannot be proven retrospectively. Do not infer provenance from the current PR head or file timestamp. A new recorded run with an executable hash is required if identity cannot be established from the tester's build record. This remains a Draft gate, not a fabricated pass.

After rebasing onto main d645fac (#2 skeleton), cargo build, workspace fmt, strict clippy and 56 workspace tests passed (two environment-dependent doc tests ignored). cargo tree confirms rove-app and rove-ui resolve GPUI to vendor/gpui. The release IME prototype was rebuilt and all four selection tests passed; MANUAL_BUILD.json records its identity for the next manual comparison, not the earlier session. These checks do not substitute for GUI end-to-end acceptance.

### Follow-up: non-empty IMM exclusion and correct point (2026-11-10)

Manual evidence now has verified executable identity (f9b5ee2..., source a443093). It confirms edge/mixed-text behavior but composition-internal arrows do not move the popup despite changing reported coordinates. This is unresolved; no Notepad/VS Code control has been measured.

Corrected events.rs update_ime_position: CFS_EXCLUDE ptCurrentPos is the caret position (previous code used the bottom edge); rcArea now has at least one physical pixel of width even for a collapsed selection. This repairs a definite empty exclusion rectangle/API semantics defect, but does not prove that Microsoft Pinyin will reposition on unchanged composition text. Retest both arrow movement and edge flipping. Source: https://learn.microsoft.com/en-us/windows/win32/api/imm/ns-imm-candidateform .

The input spike has no mouse-down handler on TextInput, so its composition-click observation cannot be attributed to this platform patch. User confirmed this round only fixes platform positioning; composition-click behavior is deferred to the product editor. No silent commit/cancel behavior is introduced.

The latest manual feedback resolves the earlier provenance uncertainty: the tested release hash matches MANUAL_BUILD.json, source a443093, Microsoft Pinyin ChsPinyinDS.dll 10.0.22621.5983, 100% and 125%. Preserve that evidence as pre-exclusion-fix behavior. Screenshots/logs: https://github.com/Zhao-wl/Rove/tree/3e46ab2aadd555ee4d3cef39e09c8dcec6a5a06e/pr-80 . It confirms complete sentence submission, edge flipping, mode switching and mixed emoji text; IME-3 is only partially passed and unchanged-composition arrows remain failed. Do not reclassify these as passed without another run.

### Latest identified retest and Notepad control (2026-11-10)

The rebuilt exclusion patch was manually tested with hash b038fd9e11039dfe782c455705dfe2614c10fc8133d009d6fb80c4e9fab05c46, source 6eda4de. Same-machine Notepad 11.2607.14.0 and Rove both keep the candidate popup stationary during left/left/right inside unchanged composition, at both 100% and 125%. This establishes behavior parity, not the underlying cause, and does not satisfy a literal requirement that the popup must move on every arrow. No further speculative platform change is made for that parity result.

Full sentence commit, screen-edge flipping, input-mode switching, emoji/UTF-16 and mixed-DPI 120→96→120 movement passed the identified retest. Composition-click remains explicitly deferred to the product editor per user scope. Third-party IME, VS Code control and product GUI E2E remain untested. Evidence: https://github.com/Zhao-wl/Rove/tree/441136a0763b3866eb000ca2411ce126f9561337/retest-2026-11-10/pr-80 . Keep Draft until the owner accepts the system-parity interpretation and remaining product acceptance scope; do not fabricate literal arrow-follow success.

Latest documentation-only parity update rebased on main e3a93c9: workspace fmt/strict clippy/56 tests and IME release 4 tests passed; two environment-dependent doc tests ignored. No IME source change was made in this retest round.

### Product `rove.exe` before/after vendoring (2026-11-10)

Both sides measured on the same machine with `cargo build --release -p rove-app` into an empty `CARGO_TARGET_DIR`, dependencies already in the local registry (no download time). "Before" is `main` e3a93c9 (crates.io gpui 0.2.2); "after" is this branch rebased on e3a93c9 with `[patch.crates-io] gpui = { path = "vendor/gpui" }`. Package count: `cargo tree -p rove-app -e normal,build --prefix none --target all`, deduplicated by name+version.

| | before (crates.io) | after (vendor) |
|---|---:|---:|
| packages | 584 | 584 |
| clean release build | 521 s (8m41s) | 623 s (10m23s) |
| `rove.exe` bytes | 13,664,768 | 13,661,696 (−3,072) |

The build-time difference is **not** attributable to the patch: other sessions were compiling concurrently on the same 16-core machine during both runs (cargo reported waiting on the package-cache lock; a first "after" attempt hit a rustc `STATUS_STACK_BUFFER_OVERRUN` in the unmodified `windows` crate and was rerun). The patch changes only `platform/windows/{events,window}.rs`, so the dependency graph is identical and size differs by 3 KB. The historical 411 packages / 469 s / 6.2 MB baseline was the M0 standalone prototype, not today's `rove-app`, so it is not directly comparable; CI's size-guard job reports the PR's `rove.exe` against the 6.2 MB budget baseline.

### Follow-up: NULL HIMC must still clear an emptied composition (2026-11-10)

`handle_ime_composition` acquired the context first and returned `None` as soon as `ImmGetContext` yielded a NULL `HIMC`. Upstream 0.2.2 (`src/platform/windows/events.rs:657`) passes the NULL context straight into `handle_ime_composition_inner`, whose `lparam == 0` branch (Japanese IME signalling an empty composition string) calls `replace_text_in_range(None, "")` and never reads the context. The early return therefore dropped the only clear-composition path: if the IME was disabled or switched away mid-composition while the context was already gone, a stale preedit stayed in the buffer.

`handle_ime_composition` now returns `None` only for a NULL context with `lparam != 0`; a NULL context with `lparam == 0` still runs the inner handler so the composition is cleared. Nothing was acquired in that branch, so `ImmReleaseContext` is not called and no geometry update is attempted — both require a live context. Non-NULL contexts are unchanged: apply, release, then reposition.

## Known gaps (已知缺口)

Recorded for follow-up; **no code is changed for these in this PR**. The first three are expected to be covered either by the product editor (R46) calling `Window::invalidate_character_coordinates()` as a backstop, or by a dedicated follow-up issue.

- **Repositioning only happens on composition messages.** Candidate/composition geometry is pushed from `WM_IME_STARTCOMPOSITION` and `WM_IME_COMPOSITION` only. `WM_DPICHANGED`, `WM_SIZE` and scrolling do not refresh it, so an open candidate window keeps stale coordinates until the next composition message.
- **`CFS_EXCLUDE.rcArea` is one physical pixel wide for an empty selection.** `update_ime_position` widens a collapsed caret rect by exactly 1px (`caret.x.saturating_add(1)`) to keep the rectangle non-empty. It does not cover the composition string or the whole line, so the IME may still overlap the preedit. Needs real-device acceptance, not code inspection.
- **Coordinates are not clipped to the client area.** Logical bounds are rounded into client coordinates without clamping, so a caret scrolled out of the viewport yields negative (or out-of-bounds) coordinates handed to `ImmSetCompositionWindow` / `ImmSetCandidateWindow`.
- **Manual evidence lives on a side branch.** The runs are on `codex/manual-evidence-2026-11-10` (commits `3e46ab2` and `441136a`). Permalinks, so the evidence survives deletion of that branch:
  - https://github.com/Zhao-wl/Rove/tree/3e46ab2aadd555ee4d3cef39e09c8dcec6a5a06e/pr-80
  - https://github.com/Zhao-wl/Rove/tree/441136a0763b3866eb000ca2411ce126f9561337/retest-2026-11-10/pr-80
