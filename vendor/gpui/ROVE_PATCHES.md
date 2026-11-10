# Rove GPUI patches

## Upstream provenance

Vendored crates.io `gpui` **0.2.2**, copied from the local Cargo registry package, not from Zed git. Archive SHA-256: `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`. Original `LICENSE-APACHE`, `Cargo.toml.orig`, assets and examples are retained. License: Apache-2.0.

## R05 — Windows IMM32 candidate positioning

- `src/platform/windows/window.rs`: original `update_ime_position` was empty with the comment “There is no such thing on Windows.” It now forwards logical bounds to the Windows backend.

- `src/platform/windows/events.rs`: original `handle_ime_position` queried the whole selected range, placed candidates at mid-line using `CFS_CANDIDATEPOS`, and acquired HIMC before an early return that leaked the context. It now queries the active UTF-16 selection endpoint as an empty range, then shares positioning with `update_ime_position`.

- The shared helper reads the current window scale factor, rounds logical bounds into physical client coordinates, calls `ImmSetCompositionWindow(CFS_POINT)` and `ImmSetCandidateWindow(CFS_EXCLUDE)` with the caret rectangle, and releases every non-null HIMC. No screen-coordinate conversion is performed: IMM32 expects client coordinates. Candidate placement at screen edges remains the IME's responsibility and requires manual testing.

- Original `handle_ime_composition` only applied composition text. It now refreshes geometry after applying the event and releasing its context, preserving the original event return value only for non-null contexts. **Behavior change:** a NULL HIMC now returns `None` immediately, so the Windows default procedure receives the message; the upstream implementation attempted composition handling with the null context. No release is needed for NULL.

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
