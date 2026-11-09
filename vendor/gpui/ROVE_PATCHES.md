# Rove GPUI patches



## Upstream provenance



Vendored crates.io `gpui` **0.2.2**, copied from the local Cargo registry package, not from Zed git. Archive SHA-256: `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`. Original `LICENSE-APACHE`, `Cargo.toml.orig`, assets and examples are retained. License: Apache-2.0.



## R05 — Windows IMM32 candidate positioning



- `src/platform/windows/window.rs`: original `update_ime_position` was empty with the comment “There is no such thing on Windows.” It now forwards logical bounds to the Windows backend.

- `src/platform/windows/events.rs`: original `handle_ime_position` queried the whole selected range, placed candidates at mid-line using `CFS_CANDIDATEPOS`, and acquired HIMC before an early return that leaked the context. It now queries the active UTF-16 selection endpoint as an empty range, then shares positioning with `update_ime_position`.

- The shared helper reads the current window scale factor, rounds logical bounds into physical client coordinates, calls `ImmSetCompositionWindow(CFS_POINT)` and `ImmSetCandidateWindow(CFS_EXCLUDE)` with the caret rectangle, and releases every non-null HIMC. No screen-coordinate conversion is performed: IMM32 expects client coordinates. Candidate placement at screen edges remains the IME's responsibility and requires manual testing.

- Original `handle_ime_composition` only applied composition text. It now refreshes geometry after applying the event and releasing its context, preserving the original event return value. Null contexts are safely ignored.



## Reapplying on upgrade



1. Obtain the published upstream archive; preserve its license and compare upstream IMM32 handling before carrying changes forward.

2. Check whether the empty platform hook and start-only positioning have already been fixed. If so, drop the corresponding patch after regression validation.

3. Reapply the forwarding hook, shared geometry helper, and post-composition update together. Preserve UTF-16 selection semantics and balanced acquire/release.

4. Build `spikes/ime-gpui`, run fmt/clippy/test, then follow its README on Microsoft Pinyin at 100% and a non-100% scale. Repeat after moving between monitors.

5. Recheck original and patched dependency counts, clean build duration and executable sizes under identical profiles. Do not count the independent spike as a workspace E2E pass.



## Validation status



Implementation in progress. Build and interactive acceptance are not yet confirmed. See `spikes/ime-gpui/README.md`; no manual acceptance item is claimed as passed.

