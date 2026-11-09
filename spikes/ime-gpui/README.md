# R05 IME regression prototype

A separate copy of the M0 GPUI viewer/input field, with a local GPUI patch and a
coordinate invalidation on input rendering. The original M0 prototype is unchanged.
Build using the MSVC toolchain and Developer environment:

```powershell
cargo build --manifest-path spikes/ime-gpui/Cargo.toml --release
spikes/ime-gpui/target/release/ime-gpui.exe docs/UX_SPEC.md
```

## Manual acceptance (all pending)

1. Focus the bottom input field; select Microsoft Pinyin. Enter a sentence of at
   least 20 Chinese characters without ending the composition. Record a video
   showing the caret and candidate popup as the composition grows and its cursor
   moves. Include mixed emoji/CJK to check UTF-16 offsets.
2. Repeat with the window near the right and bottom screen edges. Candidates must
   remain on-screen and not cover the caret line; retain screenshots.
3. During composition click another location, then repeat while switching input
   methods. Check for crashes, orphaned composition text and incorrect commits.
4. Repeat at 125% or 150% Windows display scale (record the actual scale); if
   available, drag between monitors with different scales during composition.
5. Record Microsoft Pinyin version and optionally a third-party IME. Unavailable
   input methods remain untested, not passed.

The existing `spikes/ime_test.ps1` demonstrates foreground verification, but its
hard-coded output path and short sentence are not this acceptance suite. Any
future key-injection harness must check the foreground HWND before every action
and abort when it differs. Manual testing requires no injection.

Workspace currently has no GPUI consumer and #2 remains open. This spike proves
only its own integration; workspace patch resolution and #2 E2E must be checked
again after the skeleton is available. No runtime or build result is yet claimed.
