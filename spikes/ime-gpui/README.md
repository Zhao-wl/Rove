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
again after the skeleton is available. Release build, cargo check, strict clippy and cargo test passed on 2026-11-09. The 2026-11-09 run had zero automated cases. Review follow-up now adds four composition-selection regression tests; interactive acceptance remains pending. The executable was 6,481,408 bytes; dependency count stayed at 411 packages including the prototype. See ROVE_PATCHES.md for measurement limits.

## Build prerequisites and manual evidence

Use a Visual Studio Developer PowerShell with the Windows SDK installed. Run
`where.exe fxc` before Cargo: GPUI build.rs requires fxc.exe on PATH; an ordinary
shell without it can exit 101. CI must use the same developer environment (owned
by the CI chain, not changed here).

PowerShell, from this worktree root, with stdout retained:

```powershell
New-Item -ItemType Directory spikes/ime-gpui/runs -Force | Out-Null
& ./spikes/ime-gpui/target/release/ime-gpui.exe ./docs/UX_SPEC.md 2>&1 | Tee-Object spikes/ime-gpui/runs/manual-ime.log
```

Follow the five manual steps above. Give screen-edge flipping special attention:
the collapsed caret exclusion rectangle can be zero-width. Record screenshots or
video plus `IME bounds_for_range` logs; logs alone do not prove candidate visibility.
The updated mixed CJK/emoji probe now uses composition-relative UTF-16 offsets.
This README is the self-contained #5 manual entrypoint; it does not require #81.

回归测试单独运行：`cargo test --manifest-path spikes/ime-gpui/Cargo.toml`（workspace 测试不包含该原型）。
`MANUAL_BUILD.json` 记录本次重新构建的 release exe 哈希与源码提交，仅用于下一轮手测。启动前用 `Get-FileHash spikes/ime-gpui/target/release/ime-gpui.exe -Algorithm SHA256` 对照；在记事本/VS Code 与本原型分别做组合内左移对照，记录行为，不能回溯认定旧手测版本。

最新手测已确认旧exe身份：f9b5ee2...，源码a443093，微软拼音10.0.22621.5983。长句提交/边缘/切换/emoji通过，组合内方向键候选跟随失败，点击换位部分通过。新版修改了CFS_EXCLUDE锚点与非零排除矩形，需对照新版MANUAL_BUILD.json重测方向键和边缘；不能将旧结果自动继承为新版通过。用户已确认本轮只修平台定位，组合内点击行为留给产品编辑器；原型不增加提交/保留或点击定位逻辑。
