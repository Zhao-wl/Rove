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

## R74 — 把 `gpui_http_client` 换成 Rove 自己的替身（`vendor/gpui` 源码零改动）

### 改了什么

**`vendor/gpui/` 下的文件一个字都没改。** 本条补丁完全在仓库根目录的 workspace 清单里：

- 根 `Cargo.toml` 的 `[patch.crates-io]` 新增一行
  `gpui_http_client = { path = "crates/rove-gpui-http-stub" }`；
- 新增 crate `crates/rove-gpui-http-stub`。包名必须是 `gpui_http_client`、版本 `0.2.2`，才能满足
  `vendor/gpui/Cargo.toml` 里的
  `[dependencies.http_client] version = "0.2.2", package = "gpui_http_client"`。

上游 `gpui_http_client` 0.2.2（Zed Industries，Apache-2.0）的源码**没有**被复制进仓库，它整个被换掉。
替身是 Rove 自己写的，只保留 gpui 编译所需的公开 API 形状，内部没有任何网络实现；
许可声明见 `THIRD_PARTY_NOTICES.md`。

### 为什么选路线 B（patch 依赖），不选路线 A（给 gpui 加 optional feature）

实际看过代码后选 B：

- **A 的改动面大得多。** `vendor/gpui` 里有 7 处引用 `http_client`：`src/gpui.rs`（`pub use http_client;`）、
  `src/app.rs`（`NullHttpClient`、`App::http_client` / `set_http_client`、`Application::with_http_client`）、
  `src/elements/img.rs`（远程图片资源，且 `ImageCacheError::BadStatus` 的字段类型就是
  `http_client::StatusCode`）、`src/app/test_context.rs`（`FakeHttpClient`）、
  `src/platform/linux/wayland/client.rs` 与 `src/platform/linux/x11/client.rs`（`http_client::Url`）。
  做成 optional feature 要在这些地方加 `#[cfg]` 并提供两套类型，`Cargo.toml` 与多个源文件都会成为补丁，
  每次升级 gpui 都要重打，而且源码变动时会**静默冲突**。
- **B 对 gpui 源码的改动是 0 行**，升级时 `vendor/gpui` 整包替换即可。
- **B 的代价**是必须跟住 `gpui_http_client` 的 API：它升级时（例如 `HttpClient` 加方法）替身要跟着改。
  这个代价可控——编译器会直接指出缺了什么，而且它的 API 面很小（一个 trait + 两个包装 + 一个 body 类型）。
- 两条路线的运行期语义相同（调用即失败）；B 还顺带把日志补上了（见下）。

上游化：`optional = true` 这种改动上游（zed-industries/zed）接受的可能性不低，合并后本条补丁连同
`crates/rove-gpui-http-stub` 都能删掉。本机访问 GitHub 受限，本轮**没有**提上游 PR，记为人工事项。

### 替身与上游 0.2.2 的差异（全部是删减，没有新增语义）

| 删掉的东西 | 为什么 |
|---|---|
| `github` / `github_download` 模块 | 只服务 Zed 的「下载 GitHub Release」，会拖进 `zed-async-tar` / `async-compression` / `sha2` / `tempfile`；gpui 自身不用 |
| `HttpClient::send_multipart_form` | 签名里直接出现 `reqwest::multipart::Form`，保留它就等于保留 reqwest；gpui 自身不调用 |
| `impl http_body::Body for AsyncBody`、`TryFrom<reqwest::Body>` | 前者只为 hyper 服务，后者需要 reqwest。gpui 读响应体走 `futures::AsyncRead`，这条仍然保留 |
| `HttpClientWithUrl::build_zed_api_url` / `build_zed_cloud_url` / `build_zed_llm_url` | 只是把 `zed.dev` 系列域名拼进 URL，对不联网的阅读器没有意义，也不该把这些域名留在二进制里 |

保留且与上游签名一致的部分：`HttpClient` trait（含 `get` / `post_json` 默认实现）、`AsyncBody` / `Inner`、
`HttpClientWithProxy` / `HttpClientWithUrl`、`RedirectPolicy` / `FollowRedirects` / `HttpRequestExt`、
`BlockedHttpClient`、`read_proxy_from_env` / `read_no_proxy_from_env`，以及 `test-support` feature 下的
`FakeHttpClient`（gpui 的 `TestAppContext` 需要它）。`gpui` 与 `gpui` 的 `test-support` 都能原样编译。

### 语义：调用即失败，而且看得见

- 替身里唯一的非测试实现 `BlockedHttpClient` 对每个请求返回 `Err`，并在**构造 future 的那一刻**
  （不是 await 时）写一条 `log::error!` 和一行 stderr，内容带固定前缀 `ROVE_NO_NETWORK` 以及请求方法与 URI。
  调用方即使把 future 丢掉，这次尝试也已经留在日志里。
- `rove-ui` 启动时用 `Application::with_http_client` 把它装进 gpui（`crates/rove-ui/src/net.rs`，
  在 `skeleton::launch` 里调用）。gpui 自带的 `NullHttpClient` 只返回 `No HttpClient available`、
  **不写日志**，所以不能只靠它。
- 测试：`cargo test -p gpui_http_client`（5 个单元测试）与 `cargo test -p rove-ui --test no_network`
  （2 个入口测试，其中一个走 gpui 自己的 `cx.http_client()`）。

### 前后对照（2026-11-14；**本机负载较高**，同时开着 Unity 等程序）

口径：同一台机器、同一个 worktree、依赖已在本地 registry 缓存（无下载时间）、空的 `CARGO_TARGET_DIR`，
`cargo build --release -p rove-app -j 4`，两侧都加 `CARGO_PROFILE_RELEASE_LTO=off`
（仓库默认 `lto = "thin"` + `codegen-units = 1`，本机内存不够跑；**所以体积的绝对值不是发布口径**，
发布口径由 CI 的「体积守护」任务报告）。包数取
`cargo tree -p rove-app -e normal,build --target all --prefix none`，按「名字 + 版本」去重。

| 指标 | 改动前 | 改动后 | 变化 |
|---|---:|---:|---|
| 去重后的依赖包数（`rove-app`，normal+build） | 629 | 578 | **−51** |
| clean build（release，LTO=off，`-j 4`） | 599 s | 440 s | **−159 s（−26.6%）** |
| 只改 `main.rs` 的 release 增量构建 | 5.4 s | 3.4 s | −2.0 s（小量级，机器负载噪声同数量级） |
| `rove.exe` 体积（LTO=off，非发布配置） | 29,396,480 B | 29,405,184 B | +8,704 B（+0.03%，实质不变） |

消失的包是整条 HTTP + TLS 栈：`zed-reqwest`、`hyper`、`hyper-rustls`、`hyper-util`、`h2`、`rustls`、
`rustls-native-certs`、`rustls-pemfile`、`rustls-pki-types`、`rustls-webpki`、`tokio-rustls`、`tokio`、
`tokio-util`、`tokio-socks`、`ring`、`zed-async-tar` 等。

**与 M0 报告的基线不可直接比较**：报告里的 411 个包 / 469.0 s / 6.2 MB 是 M0 的独立原型（只有 gpui
加少量依赖、默认 release 配置），今天的 `rove-app` 已经带上了全部产品 crate 与 WebView2 等依赖。
体积几乎不变是**预期结果**（链接器本来就剔除了未用代码），本条补丁的收益是**审计面与编译时间**。

`async-compression` 仍在依赖图里，但它**不是** HTTP 栈：路径是
`gpui → gpui_util → async_zip → async-compression`（只用 `futures-io`，不带 tokio、不发请求）。
它写在 `xtask/src/deps.rs` 的 `ALLOWED` 白名单里并注明了来源；整理 gpui 其余依赖不属于本 issue。

### 防回归

`cargo xtask check-deps`（`xtask/src/deps.rs`）解析 `cargo tree`，在 workspace 全体成员的
`normal,build` 与 `normal,build,dev` 两个范围上判定，出现 `reqwest` / `zed-reqwest` / `hyper` /
`rustls` / `ring` / `tokio` 等包即以非零码退出；已加进 `.github/workflows/ci.yml` 的 `rust` 任务，
放在 clippy 之前（和核心层检查一样只解析依赖图、不编译）。确实去不掉的项写进 `ALLOWED` 并注明来源与理由，
**不允许**直接从 `DENIED` 里删包名。已实测：把 `zed-reqwest` 加回任意一个 workspace 成员的依赖后，
检查以 1 退出并逐条列出被禁的包（`zed-reqwest` / `hyper` / `hyper-util` / `h2` / `rustls-*` /
`native-tls` / `openssl` / `tokio` / `tokio-util`）。

### 升级 GPUI 时如何重新应用

1. 整包替换 `vendor/gpui`。**本条补丁不碰它**，所以这里没有要重打的 hunk；R05 的 IMM32 补丁仍按前面那一节的步骤重打。
2. 看新版 `vendor/gpui/Cargo.toml` 里 `[dependencies.http_client]` 的 `version`，把
   `crates/rove-gpui-http-stub/Cargo.toml` 的 `version` 改成能满足它的值（包名保持 `gpui_http_client` 不变）。
3. `cargo build -p rove-app`。编译器会直接指出替身缺了哪些 API；对照新版上游 `gpui_http_client` 的
   `src/http_client.rs` 补上签名，**实现仍然返回错误并写日志**，不要引入任何真实网络实现。
   上游新加的、只服务 Zed 的模块按上表的原则继续删掉。
4. `cargo xtask check-deps` 必须通过。如果新版 gpui 从别的路径又拉进了 HTTP 栈，先查清来源
   （`cargo tree -i <包> --target all -e normal,build`），再决定扩大 patch 还是写白名单——不要直接改 `DENIED`。
5. 重跑 `cargo test -p gpui_http_client` 与 `cargo test -p rove-ui --test no_network`，
   并更新上面的前后对照表（注明机器负载）。
