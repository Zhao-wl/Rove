# Rust 依赖的许可汇总

由 `cargo xtask licenses --write` 从 `cargo metadata --all-features` 生成，**不要手改**。
CI 用 `cargo xtask licenses --check` 校验：许可不在 `packaging/licenses/allowed.toml` 的
允许列表里，或这份清单与依赖图不一致，都会失败。

清单覆盖全部 feature 的依赖图，因此包含只在构建、基准与测试时使用、**不进 `rove.exe`** 的包。
移植代码与随包资产的逐项声明在 [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md)。

依赖总数 798，许可种类 36。

## 按许可汇总

| 许可（SPDX） | 依赖数 |
|---|---|
| `(Apache-2.0 OR MIT) AND BSD-3-Clause` | 1 |
| `(MIT OR Apache-2.0) AND NCSA` | 1 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 2 |
| `0BSD OR MIT OR Apache-2.0` | 1 |
| `Apache-2.0` | 25 |
| `Apache-2.0 / MIT` | 1 |
| `Apache-2.0 AND ISC` | 1 |
| `Apache-2.0 OR BSL-1.0` | 1 |
| `Apache-2.0 OR GPL-2.0-only` | 1 |
| `Apache-2.0 OR ISC OR MIT` | 4 |
| `Apache-2.0 OR MIT` | 80 |
| `Apache-2.0 WITH LLVM-exception` | 1 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 7 |
| `Apache-2.0/MIT` | 6 |
| `BSD-2-Clause` | 4 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 |
| `BSD-3-Clause` | 12 |
| `BSD-3-Clause OR Apache-2.0` | 2 |
| `CC0-1.0` | 3 |
| `CC0-1.0 OR Apache-2.0` | 1 |
| `CC0-1.0 OR MIT-0 OR Apache-2.0` | 1 |
| `ISC` | 5 |
| `MIT` | 175 |
| `MIT / Apache-2.0` | 4 |
| `MIT OR Apache-2.0` | 348 |
| `MIT OR Apache-2.0 OR BSD-1-Clause` | 1 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 |
| `MIT OR Apache-2.0 OR Zlib` | 7 |
| `MIT OR Zlib OR Apache-2.0` | 2 |
| `MIT/Apache-2.0` | 43 |
| `MPL-2.0` | 5 |
| `Unicode-3.0` | 22 |
| `Unlicense OR MIT` | 11 |
| `Unlicense/MIT` | 2 |
| `Zlib` | 5 |
| `Zlib OR Apache-2.0 OR MIT` | 9 |

## 逐个依赖

| crate | 版本 | 许可（SPDX） | 来源 |
|---|---|---|---|
| addr2line | 0.27.1 | `Apache-2.0 OR MIT` | crates.io |
| adler2 | 2.0.1 | `0BSD OR MIT OR Apache-2.0` | crates.io |
| aes | 0.8.4 | `MIT OR Apache-2.0` | crates.io |
| ahash | 0.8.12 | `MIT OR Apache-2.0` | crates.io |
| aho-corasick | 1.1.5 | `Unlicense OR MIT` | crates.io |
| aligned | 0.4.3 | `MIT OR Apache-2.0` | crates.io |
| aligned-vec | 0.6.4 | `MIT` | crates.io |
| ammonia | 4.2.4 | `MIT OR Apache-2.0` | crates.io |
| anes | 0.1.6 | `MIT OR Apache-2.0` | crates.io |
| anstyle | 1.0.14 | `MIT OR Apache-2.0` | crates.io |
| anyhow | 1.0.104 | `MIT OR Apache-2.0` | crates.io |
| ar_archive_writer | 0.5.3 | `Apache-2.0 WITH LLVM-exception` | crates.io |
| arbitrary | 1.5.0 | `MIT OR Apache-2.0` | crates.io |
| arg_enum_proc_macro | 0.3.4 | `MIT` | crates.io |
| arrayref | 0.3.9 | `BSD-2-Clause` | crates.io |
| arrayvec | 0.7.8 | `MIT OR Apache-2.0` | crates.io |
| as-raw-xcb-connection | 1.0.1 | `MIT OR Apache-2.0` | crates.io |
| as-slice | 0.2.1 | `MIT OR Apache-2.0` | crates.io |
| ash | 0.38.0+1.3.281 | `MIT OR Apache-2.0` | crates.io |
| ash-window | 0.13.0 | `MIT OR Apache-2.0` | crates.io |
| ashpd | 0.11.1 | `MIT` | crates.io |
| ashpd | 0.12.3 | `MIT` | crates.io |
| async-broadcast | 0.7.2 | `MIT OR Apache-2.0` | crates.io |
| async-channel | 1.9.0 | `Apache-2.0 OR MIT` | crates.io |
| async-channel | 2.5.0 | `Apache-2.0 OR MIT` | crates.io |
| async-compression | 0.4.52 | `MIT OR Apache-2.0` | crates.io |
| async-executor | 1.14.1 | `Apache-2.0 OR MIT` | crates.io |
| async-fs | 2.2.0 | `Apache-2.0 OR MIT` | crates.io |
| async-global-executor | 2.4.1 | `Apache-2.0 OR MIT` | crates.io |
| async-io | 2.6.0 | `Apache-2.0 OR MIT` | crates.io |
| async-lock | 3.5.0 | `Apache-2.0 OR MIT` | crates.io |
| async-net | 2.0.0 | `Apache-2.0 OR MIT` | crates.io |
| async-process | 2.5.0 | `Apache-2.0 OR MIT` | crates.io |
| async-recursion | 1.2.0 | `MIT OR Apache-2.0` | crates.io |
| async-signal | 0.2.14 | `Apache-2.0 OR MIT` | crates.io |
| async-std | 1.13.2 | `Apache-2.0 OR MIT` | crates.io |
| async-task | 4.7.1 | `Apache-2.0 OR MIT` | crates.io |
| async-trait | 0.1.92 | `MIT OR Apache-2.0` | crates.io |
| async_zip | 0.0.17 | `MIT` | crates.io |
| atomic | 0.5.3 | `Apache-2.0/MIT` | crates.io |
| atomic-waker | 1.1.2 | `Apache-2.0 OR MIT` | crates.io |
| autocfg | 1.5.1 | `Apache-2.0 OR MIT` | crates.io |
| av-scenechange | 0.14.1 | `MIT` | crates.io |
| av1-grain | 0.2.5 | `BSD-2-Clause` | crates.io |
| avif-serialize | 0.8.9 | `BSD-3-Clause` | crates.io |
| backtrace | 0.3.77 | `MIT OR Apache-2.0` | crates.io |
| base64 | 0.22.1 | `MIT OR Apache-2.0` | crates.io |
| bindgen | 0.71.1 | `BSD-3-Clause` | crates.io |
| bit-set | 0.11.1 | `Apache-2.0 OR MIT` | crates.io |
| bit-set | 0.8.0 | `Apache-2.0 OR MIT` | crates.io |
| bit-vec | 0.10.2 | `Apache-2.0 OR MIT` | crates.io |
| bit-vec | 0.8.0 | `Apache-2.0 OR MIT` | crates.io |
| bit_field | 0.10.3 | `Apache-2.0/MIT` | crates.io |
| bitflags | 1.3.2 | `MIT/Apache-2.0` | crates.io |
| bitflags | 2.13.2 | `MIT OR Apache-2.0` | crates.io |
| bitstream-io | 4.10.0 | `MIT/Apache-2.0` | crates.io |
| blade-graphics | 0.7.1 | `MIT` | crates.io |
| blade-macros | 0.3.0 | `MIT` | crates.io |
| blade-util | 0.3.0 | `MIT` | crates.io |
| block | 0.1.6 | `MIT` | crates.io |
| block-buffer | 0.10.4 | `MIT OR Apache-2.0` | crates.io |
| block-buffer | 0.12.1 | `MIT OR Apache-2.0` | crates.io |
| block-padding | 0.3.3 | `MIT OR Apache-2.0` | crates.io |
| block2 | 0.6.2 | `MIT` | crates.io |
| blocking | 1.7.0 | `Apache-2.0 OR MIT` | crates.io |
| borsh | 1.8.1 | `MIT OR Apache-2.0` | crates.io |
| borsh-derive | 1.8.1 | `Apache-2.0` | crates.io |
| bstr | 1.13.1 | `MIT OR Apache-2.0` | crates.io |
| built | 0.8.1 | `MIT` | crates.io |
| bumpalo | 3.20.3 | `MIT OR Apache-2.0` | crates.io |
| bytemuck | 1.25.2 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| bytemuck_derive | 1.12.1 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| byteorder | 1.5.0 | `Unlicense OR MIT` | crates.io |
| byteorder-lite | 0.1.0 | `Unlicense OR MIT` | crates.io |
| bytes | 1.12.1 | `MIT` | crates.io |
| calloop | 0.13.0 | `MIT` | crates.io |
| calloop-wayland-source | 0.3.0 | `MIT` | crates.io |
| cast | 0.3.0 | `MIT OR Apache-2.0` | crates.io |
| cbc | 0.1.2 | `MIT OR Apache-2.0` | crates.io |
| cbindgen | 0.28.0 | `MPL-2.0` | crates.io |
| cc | 1.11.1 | `MIT OR Apache-2.0` | crates.io |
| cexpr | 0.6.0 | `Apache-2.0/MIT` | crates.io |
| cfg-if | 1.0.5 | `MIT OR Apache-2.0` | crates.io |
| cfg_aliases | 0.2.2 | `MIT` | crates.io |
| cgl | 0.3.2 | `MIT / Apache-2.0` | crates.io |
| chacha20 | 0.10.2 | `MIT OR Apache-2.0` | crates.io |
| chardetng | 0.1.17 | `Apache-2.0 OR MIT` | crates.io |
| ciborium | 0.2.2 | `Apache-2.0` | crates.io |
| ciborium-io | 0.2.2 | `Apache-2.0` | crates.io |
| ciborium-ll | 0.2.2 | `Apache-2.0` | crates.io |
| cipher | 0.4.4 | `MIT OR Apache-2.0` | crates.io |
| clang-sys | 1.9.1 | `Apache-2.0` | crates.io |
| clap | 4.6.7 | `MIT OR Apache-2.0` | crates.io |
| clap_builder | 4.6.7 | `MIT OR Apache-2.0` | crates.io |
| clap_lex | 1.1.1 | `MIT OR Apache-2.0` | crates.io |
| cocoa | 0.25.0 | `MIT OR Apache-2.0` | crates.io |
| cocoa | 0.26.0 | `MIT OR Apache-2.0` | crates.io |
| cocoa-foundation | 0.1.2 | `MIT OR Apache-2.0` | crates.io |
| cocoa-foundation | 0.2.0 | `MIT OR Apache-2.0` | crates.io |
| codespan-reporting | 0.12.0 | `Apache-2.0` | crates.io |
| color_quant | 1.1.0 | `MIT` | crates.io |
| command-fds | 0.3.3 | `Apache-2.0` | crates.io |
| compression-codecs | 0.4.47 | `MIT OR Apache-2.0` | crates.io |
| compression-core | 0.4.34 | `MIT OR Apache-2.0` | crates.io |
| concurrent-queue | 2.5.0 | `Apache-2.0 OR MIT` | crates.io |
| const-oid | 0.10.2 | `Apache-2.0 OR MIT` | crates.io |
| const-random | 0.1.18 | `MIT OR Apache-2.0` | crates.io |
| const-random-macro | 0.1.16 | `MIT OR Apache-2.0` | crates.io |
| convert_case | 0.4.0 | `MIT` | crates.io |
| core-foundation | 0.10.0 | `MIT OR Apache-2.0` | crates.io |
| core-foundation | 0.9.4 | `MIT OR Apache-2.0` | crates.io |
| core-foundation-sys | 0.8.7 | `MIT OR Apache-2.0` | crates.io |
| core-graphics | 0.23.2 | `MIT OR Apache-2.0` | crates.io |
| core-graphics | 0.24.0 | `MIT OR Apache-2.0` | crates.io |
| core-graphics-helmer-fork | 0.24.0 | `MIT OR Apache-2.0` | crates.io |
| core-graphics-types | 0.1.3 | `MIT OR Apache-2.0` | crates.io |
| core-graphics-types | 0.2.0 | `MIT OR Apache-2.0` | crates.io |
| core-graphics2 | 0.4.1 | `MIT OR Apache-2.0` | crates.io |
| core-text | 21.0.0 | `MIT OR Apache-2.0` | crates.io |
| core-video | 0.4.3 | `MIT OR Apache-2.0` | crates.io |
| core_detect | 1.0.0 | `MIT/Apache-2.0` | crates.io |
| core_maths | 0.1.1 | `MIT` | crates.io |
| cosmic-text | 0.14.2 | `MIT OR Apache-2.0` | crates.io |
| cpufeatures | 0.2.17 | `MIT OR Apache-2.0` | crates.io |
| cpufeatures | 0.3.1 | `MIT OR Apache-2.0` | crates.io |
| crc32fast | 1.5.4 | `MIT OR Apache-2.0` | crates.io |
| criterion | 0.5.1 | `Apache-2.0 OR MIT` | crates.io |
| criterion-plot | 0.5.0 | `MIT/Apache-2.0` | crates.io |
| crossbeam-deque | 0.8.8 | `MIT OR Apache-2.0` | crates.io |
| crossbeam-epoch | 0.9.21 | `MIT OR Apache-2.0` | crates.io |
| crossbeam-queue | 0.3.14 | `MIT OR Apache-2.0` | crates.io |
| crossbeam-utils | 0.8.23 | `MIT OR Apache-2.0` | crates.io |
| crunchy | 0.2.4 | `MIT` | crates.io |
| crypto-common | 0.1.7 | `MIT OR Apache-2.0` | crates.io |
| crypto-common | 0.2.2 | `MIT OR Apache-2.0` | crates.io |
| cssparser | 0.38.0 | `MPL-2.0` | crates.io |
| ctor | 0.4.3 | `Apache-2.0 OR MIT` | crates.io |
| ctor-proc-macro | 0.0.6 | `Apache-2.0 OR MIT` | crates.io |
| curve25519-dalek | 5.0.0 | `BSD-3-Clause` | crates.io |
| curve25519-dalek-derive | 0.1.1 | `MIT/Apache-2.0` | crates.io |
| data-url | 0.3.2 | `MIT OR Apache-2.0` | crates.io |
| deflate64 | 0.1.12 | `MIT` | crates.io |
| derive_more | 0.99.20 | `MIT` | crates.io |
| digest | 0.10.7 | `MIT OR Apache-2.0` | crates.io |
| digest | 0.11.3 | `MIT OR Apache-2.0` | crates.io |
| dirs | 4.0.0 | `MIT OR Apache-2.0` | crates.io |
| dirs | 5.0.1 | `MIT OR Apache-2.0` | crates.io |
| dirs-sys | 0.3.7 | `MIT OR Apache-2.0` | crates.io |
| dirs-sys | 0.4.1 | `MIT OR Apache-2.0` | crates.io |
| dispatch | 0.2.0 | `MIT` | crates.io |
| dispatch2 | 0.3.1 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| displaydoc | 0.2.7 | `MIT OR Apache-2.0` | crates.io |
| dlib | 0.5.3 | `MIT` | crates.io |
| downcast-rs | 1.2.1 | `MIT/Apache-2.0` | crates.io |
| dtoa | 1.0.11 | `MIT OR Apache-2.0` | crates.io |
| dtoa-short | 0.3.5 | `MPL-2.0` | crates.io |
| dtor | 0.0.6 | `Apache-2.0 OR MIT` | crates.io |
| dtor-proc-macro | 0.0.5 | `Apache-2.0 OR MIT` | crates.io |
| dunce | 1.0.5 | `CC0-1.0 OR MIT-0 OR Apache-2.0` | crates.io |
| dwrote | 0.11.5 | `MPL-2.0` | crates.io |
| dyn-clone | 1.0.20 | `MIT OR Apache-2.0` | crates.io |
| ed25519 | 3.0.0 | `Apache-2.0 OR MIT` | crates.io |
| ed25519-dalek | 3.0.0 | `BSD-3-Clause` | crates.io |
| either | 1.19.0 | `MIT OR Apache-2.0` | crates.io |
| embed-resource | 3.0.12 | `MIT` | crates.io |
| encoding_rs | 0.8.44 | `(Apache-2.0 OR MIT) AND BSD-3-Clause` | crates.io |
| encoding_rs_io | 0.1.8 | `MIT OR Apache-2.0` | crates.io |
| endi | 1.1.1 | `MIT` | crates.io |
| enumflags2 | 0.7.12 | `MIT OR Apache-2.0` | crates.io |
| enumflags2_derive | 0.7.12 | `MIT OR Apache-2.0` | crates.io |
| equator | 0.4.2 | `MIT` | crates.io |
| equator-macro | 0.4.2 | `MIT` | crates.io |
| equivalent | 1.0.3 | `Apache-2.0 OR MIT` | crates.io |
| erased-serde | 0.4.11 | `MIT OR Apache-2.0` | crates.io |
| errno | 0.3.14 | `MIT OR Apache-2.0` | crates.io |
| etagere | 0.2.15 | `MIT/Apache-2.0` | crates.io |
| euclid | 0.22.14 | `MIT OR Apache-2.0` | crates.io |
| event-listener | 2.5.3 | `Apache-2.0 OR MIT` | crates.io |
| event-listener | 5.4.2 | `Apache-2.0 OR MIT` | crates.io |
| event-listener-strategy | 0.5.4 | `Apache-2.0 OR MIT` | crates.io |
| exr | 1.74.3 | `BSD-3-Clause` | crates.io |
| fancy-regex | 0.20.1 | `MIT` | crates.io |
| fastrand | 1.9.0 | `Apache-2.0 OR MIT` | crates.io |
| fastrand | 2.5.0 | `Apache-2.0 OR MIT` | crates.io |
| fax | 0.2.7 | `MIT` | crates.io |
| fdeflate | 0.3.7 | `MIT OR Apache-2.0` | crates.io |
| fiat-crypto | 0.3.0 | `MIT OR Apache-2.0 OR BSD-1-Clause` | crates.io |
| filedescriptor | 0.8.3 | `MIT` | crates.io |
| filetime | 0.2.29 | `MIT/Apache-2.0` | crates.io |
| find-msvc-tools | 0.1.15 | `MIT OR Apache-2.0` | crates.io |
| flate2 | 1.1.11 | `MIT OR Apache-2.0` | crates.io |
| float-cmp | 0.9.0 | `MIT` | crates.io |
| float-ord | 0.3.2 | `MIT / Apache-2.0` | crates.io |
| float_next_after | 1.0.0 | `MIT` | crates.io |
| flume | 0.11.1 | `Apache-2.0/MIT` | crates.io |
| fnv | 1.0.7 | `Apache-2.0 / MIT` | crates.io |
| foldhash | 0.1.5 | `Zlib` | crates.io |
| foldhash | 0.2.0 | `Zlib` | crates.io |
| font-types | 0.12.7 | `MIT OR Apache-2.0` | crates.io |
| fontconfig-parser | 0.5.8 | `MIT` | crates.io |
| fontdb | 0.16.2 | `MIT` | crates.io |
| fontdb | 0.23.0 | `MIT` | crates.io |
| foreign-types | 0.5.0 | `MIT/Apache-2.0` | crates.io |
| foreign-types-macros | 0.2.4 | `MIT/Apache-2.0` | crates.io |
| foreign-types-shared | 0.3.1 | `MIT/Apache-2.0` | crates.io |
| form_urlencoded | 1.2.2 | `MIT OR Apache-2.0` | crates.io |
| freetype-sys | 0.20.1 | `MIT` | crates.io |
| fsevent-sys | 4.1.0 | `MIT` | crates.io |
| futf | 0.1.5 | `MIT / Apache-2.0` | crates.io |
| futures | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-channel | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-core | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-executor | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-io | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-lite | 1.13.0 | `Apache-2.0 OR MIT` | crates.io |
| futures-lite | 2.6.1 | `Apache-2.0 OR MIT` | crates.io |
| futures-macro | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-sink | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-task | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| futures-util | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| generic-array | 0.14.7 | `MIT` | crates.io |
| gethostname | 1.1.0 | `Apache-2.0` | crates.io |
| getrandom | 0.2.17 | `MIT OR Apache-2.0` | crates.io |
| getrandom | 0.3.4 | `MIT OR Apache-2.0` | crates.io |
| getrandom | 0.4.3 | `MIT OR Apache-2.0` | crates.io |
| gif | 0.14.2 | `MIT OR Apache-2.0` | crates.io |
| gimli | 0.34.0 | `MIT OR Apache-2.0` | crates.io |
| git2 | 0.20.4 | `MIT OR Apache-2.0` | crates.io |
| glob | 0.3.4 | `MIT OR Apache-2.0` | crates.io |
| globset | 0.4.21 | `Unlicense OR MIT` | crates.io |
| gloo-timers | 0.3.0 | `MIT OR Apache-2.0` | crates.io |
| glow | 0.16.0 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| gpu-alloc | 0.6.2 | `MIT OR Apache-2.0` | crates.io |
| gpu-alloc-ash | 0.7.1 | `MIT OR Apache-2.0` | crates.io |
| gpu-alloc-types | 0.3.1 | `MIT OR Apache-2.0` | crates.io |
| gpui | 0.2.2 | `Apache-2.0` | vendor/gpui/Cargo.toml |
| gpui-macros | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_collections | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_derive_refineable | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_http_client | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_media | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_perf | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_refineable | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_semantic_version | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_sum_tree | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_util | 0.2.2 | `Apache-2.0` | crates.io |
| gpui_util_macros | 0.2.2 | `Apache-2.0` | crates.io |
| grep-matcher | 0.1.9 | `Unlicense OR MIT` | crates.io |
| grep-regex | 0.1.14 | `Unlicense OR MIT` | crates.io |
| grep-searcher | 0.1.17 | `Unlicense OR MIT` | crates.io |
| grid | 0.18.0 | `MIT` | crates.io |
| h2 | 0.4.20 | `MIT` | crates.io |
| half | 2.7.1 | `MIT OR Apache-2.0` | crates.io |
| hashbrown | 0.14.5 | `MIT OR Apache-2.0` | crates.io |
| hashbrown | 0.15.5 | `MIT OR Apache-2.0` | crates.io |
| hashbrown | 0.17.1 | `MIT OR Apache-2.0` | crates.io |
| heck | 0.4.1 | `MIT OR Apache-2.0` | crates.io |
| heck | 0.5.0 | `MIT OR Apache-2.0` | crates.io |
| hermit-abi | 0.5.3 | `MIT OR Apache-2.0` | crates.io |
| hex | 0.4.3 | `MIT OR Apache-2.0` | crates.io |
| hexf-parse | 0.2.1 | `CC0-1.0` | crates.io |
| hidden-trait | 0.1.2 | `MIT` | crates.io |
| hkdf | 0.12.4 | `MIT OR Apache-2.0` | crates.io |
| hmac | 0.12.1 | `MIT OR Apache-2.0` | crates.io |
| home | 0.5.12 | `MIT OR Apache-2.0` | crates.io |
| html5ever | 0.40.1 | `MIT OR Apache-2.0` | crates.io |
| http | 1.5.0 | `MIT OR Apache-2.0` | crates.io |
| http-body | 1.1.0 | `MIT` | crates.io |
| http-body-util | 0.1.5 | `MIT` | crates.io |
| httparse | 1.10.1 | `MIT OR Apache-2.0` | crates.io |
| hybrid-array | 0.4.15 | `MIT OR Apache-2.0` | crates.io |
| hyper | 1.12.0 | `MIT` | crates.io |
| hyper-rustls | 0.27.10 | `Apache-2.0 OR ISC OR MIT` | crates.io |
| hyper-util | 0.1.21 | `MIT` | crates.io |
| icu_collections | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_locale_core | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_locale_fallback | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_locale_fallback_data | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_normalizer | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_normalizer_data | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_properties | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_properties_data | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_provider | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_segmenter | 2.4.0 | `Unicode-3.0` | crates.io |
| icu_segmenter_data | 2.4.0 | `Unicode-3.0` | crates.io |
| idna | 1.1.0 | `MIT OR Apache-2.0` | crates.io |
| idna_adapter | 1.2.2 | `Apache-2.0 OR MIT` | crates.io |
| ignore | 0.4.33 | `Unlicense OR MIT` | crates.io |
| image | 0.25.10 | `MIT OR Apache-2.0` | crates.io |
| image-webp | 0.2.4 | `MIT OR Apache-2.0` | crates.io |
| imagesize | 0.13.0 | `MIT` | crates.io |
| imgref | 1.12.3 | `CC0-1.0 OR Apache-2.0` | crates.io |
| indexmap | 2.14.2 | `Apache-2.0 OR MIT` | crates.io |
| inotify | 0.11.5 | `ISC` | crates.io |
| inotify-sys | 0.1.8 | `ISC` | crates.io |
| inout | 0.1.4 | `MIT OR Apache-2.0` | crates.io |
| instant | 0.1.13 | `BSD-3-Clause` | crates.io |
| interpolate_name | 0.2.4 | `MIT` | crates.io |
| inventory | 0.3.25 | `MIT OR Apache-2.0` | crates.io |
| io-surface | 0.16.1 | `MIT OR Apache-2.0` | crates.io |
| ipnet | 2.12.2 | `MIT OR Apache-2.0` | crates.io |
| is-docker | 0.2.0 | `MIT` | crates.io |
| is-terminal | 0.4.17 | `MIT` | crates.io |
| is-wsl | 0.4.0 | `MIT` | crates.io |
| itertools | 0.10.5 | `MIT/Apache-2.0` | crates.io |
| itertools | 0.13.0 | `MIT OR Apache-2.0` | crates.io |
| itertools | 0.14.0 | `MIT OR Apache-2.0` | crates.io |
| itoa | 1.0.19 | `MIT OR Apache-2.0` | crates.io |
| jobserver | 0.1.35 | `MIT OR Apache-2.0` | crates.io |
| js-sys | 0.3.107 | `MIT OR Apache-2.0` | crates.io |
| jsonc-parser | 0.34.0 | `MIT` | crates.io |
| khronos-egl | 6.0.0 | `MIT/Apache-2.0` | crates.io |
| kqueue | 1.2.1 | `MIT` | crates.io |
| kqueue-sys | 1.1.2 | `MIT` | crates.io |
| kurbo | 0.11.3 | `Apache-2.0 OR MIT` | crates.io |
| kv-log-macro | 1.0.7 | `MIT OR Apache-2.0` | crates.io |
| lazy_static | 1.5.1 | `MIT OR Apache-2.0` | crates.io |
| leak | 0.1.2 | `Apache-2.0 OR MIT` | crates.io |
| leaky-cow | 0.1.1 | `MIT / Apache-2.0` | crates.io |
| lebe | 0.5.3 | `BSD-3-Clause` | crates.io |
| libc | 0.2.191 | `MIT OR Apache-2.0` | crates.io |
| libfuzzer-sys | 0.4.13 | `(MIT OR Apache-2.0) AND NCSA` | crates.io |
| libgit2-sys | 0.18.8+1.9.7 | `MIT OR Apache-2.0` | crates.io |
| libloading | 0.8.9 | `ISC` | crates.io |
| libm | 0.2.16 | `MIT` | crates.io |
| libredox | 0.1.25 | `MIT` | crates.io |
| libz-sys | 1.1.32 | `MIT OR Apache-2.0` | crates.io |
| linux-raw-sys | 0.12.1 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| linux-raw-sys | 0.4.15 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| litemap | 0.8.3 | `Unicode-3.0` | crates.io |
| lock_api | 0.4.14 | `MIT OR Apache-2.0` | crates.io |
| log | 0.4.35 | `MIT OR Apache-2.0` | crates.io |
| loop9 | 0.1.5 | `MIT` | crates.io |
| lru-slab | 0.1.3 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| lyon | 1.0.19 | `MIT OR Apache-2.0` | crates.io |
| lyon_algorithms | 1.0.22 | `MIT OR Apache-2.0` | crates.io |
| lyon_geom | 1.0.20 | `MIT OR Apache-2.0` | crates.io |
| lyon_path | 1.0.19 | `MIT OR Apache-2.0` | crates.io |
| lyon_tessellation | 1.0.23 | `MIT OR Apache-2.0` | crates.io |
| mac | 0.1.1 | `MIT/Apache-2.0` | crates.io |
| malloc_buf | 0.0.6 | `MIT` | crates.io |
| maplit | 1.0.2 | `MIT/Apache-2.0` | crates.io |
| markup5ever | 0.40.0 | `MIT OR Apache-2.0` | crates.io |
| maybe-rayon | 0.1.1 | `MIT` | crates.io |
| md-5 | 0.10.6 | `MIT OR Apache-2.0` | crates.io |
| memchr | 2.8.3 | `Unlicense OR MIT` | crates.io |
| memmap2 | 0.9.12 | `MIT OR Apache-2.0` | crates.io |
| memoffset | 0.9.1 | `MIT` | crates.io |
| memory-stats | 1.2.0 | `MIT OR Apache-2.0` | crates.io |
| metal | 0.29.0 | `MIT OR Apache-2.0` | crates.io |
| mime | 0.3.18 | `MIT OR Apache-2.0` | crates.io |
| mime_guess | 2.0.5 | `MIT` | crates.io |
| minimal-lexical | 0.2.1 | `MIT/Apache-2.0` | crates.io |
| miniz_oxide | 0.8.9 | `MIT OR Zlib OR Apache-2.0` | crates.io |
| miniz_oxide | 0.9.1 | `MIT OR Zlib OR Apache-2.0` | crates.io |
| mint | 0.5.9 | `MIT` | crates.io |
| mio | 1.2.4 | `MIT` | crates.io |
| moxcms | 0.8.1 | `BSD-3-Clause OR Apache-2.0` | crates.io |
| multiversion | 0.8.1 | `MIT OR Apache-2.0` | crates.io |
| multiversion-macros | 0.8.1 | `MIT OR Apache-2.0` | crates.io |
| multiversion_no_op | 1.0.0 | `Apache-2.0 OR MIT` | crates.io |
| naga | 25.0.1 | `MIT OR Apache-2.0` | crates.io |
| nanorand | 0.7.0 | `Zlib` | crates.io |
| new_debug_unreachable | 1.0.6 | `MIT` | crates.io |
| nix | 0.29.0 | `MIT` | crates.io |
| nix | 0.31.3 | `MIT` | crates.io |
| no_std_io2 | 0.9.4 | `Apache-2.0 OR MIT` | crates.io |
| nom | 7.1.3 | `MIT` | crates.io |
| nom | 8.0.0 | `MIT` | crates.io |
| noop_proc_macro | 0.3.0 | `MIT` | crates.io |
| notify | 8.2.0 | `CC0-1.0` | crates.io |
| notify-types | 2.1.0 | `MIT OR Apache-2.0` | crates.io |
| ntapi | 0.4.3 | `Apache-2.0 OR MIT` | crates.io |
| num | 0.4.3 | `MIT OR Apache-2.0` | crates.io |
| num-bigint | 0.4.8 | `MIT OR Apache-2.0` | crates.io |
| num-bigint-dig | 0.8.6 | `MIT/Apache-2.0` | crates.io |
| num-complex | 0.4.6 | `MIT OR Apache-2.0` | crates.io |
| num-derive | 0.4.2 | `MIT OR Apache-2.0` | crates.io |
| num-integer | 0.1.47 | `MIT OR Apache-2.0` | crates.io |
| num-iter | 0.1.46 | `MIT OR Apache-2.0` | crates.io |
| num-rational | 0.4.2 | `MIT OR Apache-2.0` | crates.io |
| num-traits | 0.2.19 | `MIT OR Apache-2.0` | crates.io |
| num_cpus | 1.17.0 | `MIT OR Apache-2.0` | crates.io |
| objc | 0.2.7 | `MIT` | crates.io |
| objc-foundation | 0.1.1 | `MIT` | crates.io |
| objc2 | 0.6.5 | `MIT` | crates.io |
| objc2-app-kit | 0.3.2 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| objc2-core-foundation | 0.3.2 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| objc2-encode | 4.1.0 | `MIT` | crates.io |
| objc2-foundation | 0.3.2 | `MIT` | crates.io |
| objc2-metal | 0.3.2 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| objc2-quartz-core | 0.3.2 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| objc2-ui-kit | 0.3.2 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| objc_exception | 0.1.2 | `MIT` | crates.io |
| objc_id | 0.1.1 | `MIT` | crates.io |
| object | 0.39.1 | `Apache-2.0 OR MIT` | crates.io |
| object | 0.40.0 | `Apache-2.0 OR MIT` | crates.io |
| once_cell | 1.21.4 | `MIT OR Apache-2.0` | crates.io |
| oo7 | 0.5.0 | `MIT` | crates.io |
| oorandom | 11.1.5 | `MIT` | crates.io |
| open | 5.4.5 | `MIT` | crates.io |
| openssl-probe | 0.2.2 | `MIT OR Apache-2.0` | crates.io |
| option-ext | 0.2.0 | `MPL-2.0` | crates.io |
| ordered-stream | 0.2.0 | `MIT OR Apache-2.0` | crates.io |
| parking | 2.2.1 | `Apache-2.0 OR MIT` | crates.io |
| parking_lot | 0.12.5 | `MIT OR Apache-2.0` | crates.io |
| parking_lot_core | 0.9.12 | `MIT OR Apache-2.0` | crates.io |
| paste | 1.0.15 | `MIT OR Apache-2.0` | crates.io |
| pastey | 0.1.1 | `MIT OR Apache-2.0` | crates.io |
| pathfinder_geometry | 0.5.1 | `MIT/Apache-2.0` | crates.io |
| pathfinder_simd | 0.5.6 | `MIT OR Apache-2.0` | crates.io |
| pbkdf2 | 0.12.2 | `MIT OR Apache-2.0` | crates.io |
| percent-encoding | 2.3.2 | `MIT OR Apache-2.0` | crates.io |
| phf | 0.14.0 | `MIT` | crates.io |
| phf_codegen | 0.14.0 | `MIT` | crates.io |
| phf_generator | 0.14.0 | `MIT` | crates.io |
| phf_shared | 0.14.0 | `MIT` | crates.io |
| pico-args | 0.5.0 | `MIT` | crates.io |
| pin-project | 1.1.13 | `Apache-2.0 OR MIT` | crates.io |
| pin-project-internal | 1.1.13 | `Apache-2.0 OR MIT` | crates.io |
| pin-project-lite | 0.2.17 | `Apache-2.0 OR MIT` | crates.io |
| pin-utils | 0.1.1 | `MIT OR Apache-2.0` | crates.io |
| piper | 0.2.5 | `MIT OR Apache-2.0` | crates.io |
| pkg-config | 0.3.34 | `MIT OR Apache-2.0` | crates.io |
| png | 0.17.16 | `MIT OR Apache-2.0` | crates.io |
| png | 0.18.1 | `MIT OR Apache-2.0` | crates.io |
| polling | 3.11.0 | `Apache-2.0 OR MIT` | crates.io |
| pollster | 0.2.5 | `Apache-2.0/MIT` | crates.io |
| postage | 0.5.0 | `MIT` | crates.io |
| potential_utf | 0.1.6 | `Unicode-3.0` | crates.io |
| ppv-lite86 | 0.2.21 | `MIT OR Apache-2.0` | crates.io |
| precomputed-hash | 0.1.1 | `MIT` | crates.io |
| prettyplease | 0.2.37 | `MIT OR Apache-2.0` | crates.io |
| proc-macro-crate | 3.5.0 | `MIT OR Apache-2.0` | crates.io |
| proc-macro-error-attr2 | 2.0.0 | `MIT OR Apache-2.0` | crates.io |
| proc-macro-error2 | 2.0.1 | `MIT OR Apache-2.0` | crates.io |
| proc-macro2 | 1.0.107 | `MIT OR Apache-2.0` | crates.io |
| profiling | 1.0.18 | `MIT OR Apache-2.0` | crates.io |
| profiling-procmacros | 1.0.18 | `MIT OR Apache-2.0` | crates.io |
| proptest | 1.12.0 | `MIT OR Apache-2.0` | crates.io |
| psm | 0.1.32 | `MIT OR Apache-2.0` | crates.io |
| pulldown-cmark | 0.13.6 | `MIT` | crates.io |
| pxfm | 0.1.30 | `BSD-3-Clause OR Apache-2.0` | crates.io |
| qoi | 0.4.1 | `MIT/Apache-2.0` | crates.io |
| quick-error | 1.2.3 | `MIT/Apache-2.0` | crates.io |
| quick-error | 2.0.1 | `MIT/Apache-2.0` | crates.io |
| quick-xml | 0.41.0 | `MIT` | crates.io |
| quinn | 0.11.12 | `MIT OR Apache-2.0` | crates.io |
| quinn-proto | 0.11.19 | `MIT OR Apache-2.0` | crates.io |
| quinn-udp | 0.5.16 | `MIT OR Apache-2.0` | crates.io |
| quote | 1.0.47 | `MIT OR Apache-2.0` | crates.io |
| r-efi | 5.3.0 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | crates.io |
| r-efi | 6.0.0 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | crates.io |
| rand | 0.10.3 | `MIT OR Apache-2.0` | crates.io |
| rand | 0.8.8 | `MIT OR Apache-2.0` | crates.io |
| rand | 0.9.5 | `MIT OR Apache-2.0` | crates.io |
| rand_chacha | 0.3.1 | `MIT OR Apache-2.0` | crates.io |
| rand_chacha | 0.9.0 | `MIT OR Apache-2.0` | crates.io |
| rand_core | 0.10.1 | `MIT OR Apache-2.0` | crates.io |
| rand_core | 0.6.4 | `MIT OR Apache-2.0` | crates.io |
| rand_core | 0.9.5 | `MIT OR Apache-2.0` | crates.io |
| rand_pcg | 0.10.2 | `MIT OR Apache-2.0` | crates.io |
| rand_xorshift | 0.5.0 | `MIT OR Apache-2.0` | crates.io |
| rangemap | 1.8.0 | `MIT/Apache-2.0` | crates.io |
| rav1e | 0.8.1 | `BSD-2-Clause` | crates.io |
| ravif | 0.13.0 | `BSD-3-Clause` | crates.io |
| raw-window-handle | 0.6.2 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| raw-window-metal | 0.4.0 | `MIT OR Apache-2.0` | crates.io |
| rayon | 1.12.0 | `MIT OR Apache-2.0` | crates.io |
| rayon-core | 1.13.0 | `MIT OR Apache-2.0` | crates.io |
| read-fonts | 0.41.0 | `MIT OR Apache-2.0` | crates.io |
| redox_syscall | 0.2.16 | `MIT` | crates.io |
| redox_syscall | 0.5.18 | `MIT` | crates.io |
| redox_users | 0.4.6 | `MIT` | crates.io |
| ref-cast | 1.0.27 | `MIT OR Apache-2.0` | crates.io |
| ref-cast-impl | 1.0.27 | `MIT OR Apache-2.0` | crates.io |
| regex | 1.13.1 | `MIT OR Apache-2.0` | crates.io |
| regex-automata | 0.4.18 | `MIT OR Apache-2.0` | crates.io |
| regex-syntax | 0.8.11 | `MIT OR Apache-2.0` | crates.io |
| resvg | 0.45.1 | `Apache-2.0 OR MIT` | crates.io |
| rgb | 0.8.53 | `MIT` | crates.io |
| ring | 0.17.14 | `Apache-2.0 AND ISC` | crates.io |
| ropey | 1.6.1 | `MIT` | crates.io |
| roxmltree | 0.20.0 | `MIT OR Apache-2.0` | crates.io |
| rust-embed | 8.13.0 | `MIT` | crates.io |
| rust-embed-impl | 8.13.0 | `MIT` | crates.io |
| rust-embed-utils | 8.13.0 | `MIT` | crates.io |
| rustc-demangle | 0.1.28 | `MIT/Apache-2.0` | crates.io |
| rustc-hash | 1.1.0 | `Apache-2.0/MIT` | crates.io |
| rustc-hash | 2.1.3 | `Apache-2.0 OR MIT` | crates.io |
| rustc_version | 0.4.1 | `MIT OR Apache-2.0` | crates.io |
| rustix | 0.38.44 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| rustix | 1.1.6 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| rustls | 0.23.46 | `Apache-2.0 OR ISC OR MIT` | crates.io |
| rustls-native-certs | 0.8.5 | `Apache-2.0 OR ISC OR MIT` | crates.io |
| rustls-pemfile | 2.2.0 | `Apache-2.0 OR ISC OR MIT` | crates.io |
| rustls-pki-types | 1.15.1 | `MIT OR Apache-2.0` | crates.io |
| rustls-webpki | 0.103.16 | `ISC` | crates.io |
| rustversion | 1.0.23 | `MIT OR Apache-2.0` | crates.io |
| rusty-fork | 0.3.1 | `MIT/Apache-2.0` | crates.io |
| rustybuzz | 0.14.1 | `MIT` | crates.io |
| rustybuzz | 0.20.1 | `MIT` | crates.io |
| ryu | 1.0.23 | `Apache-2.0 OR BSL-1.0` | crates.io |
| same-file | 1.0.6 | `Unlicense/MIT` | crates.io |
| schannel | 0.1.30 | `MIT` | crates.io |
| schemars | 1.2.3 | `MIT` | crates.io |
| schemars_derive | 1.2.3 | `MIT` | crates.io |
| scoped-tls | 1.0.1 | `MIT/Apache-2.0` | crates.io |
| scopeguard | 1.2.0 | `MIT OR Apache-2.0` | crates.io |
| screencapturekit | 0.2.8 | `MIT OR Apache-2.0` | crates.io |
| screencapturekit-sys | 0.2.8 | `MIT OR Apache-2.0` | crates.io |
| seahash | 4.1.0 | `MIT` | crates.io |
| security-framework | 3.7.0 | `MIT OR Apache-2.0` | crates.io |
| security-framework-sys | 2.17.0 | `MIT OR Apache-2.0` | crates.io |
| self_cell | 1.3.0 | `Apache-2.0 OR GPL-2.0-only` | crates.io |
| semver | 1.0.28 | `MIT OR Apache-2.0` | crates.io |
| serde | 1.0.229 | `MIT OR Apache-2.0` | crates.io |
| serde_core | 1.0.229 | `MIT OR Apache-2.0` | crates.io |
| serde_derive | 1.0.229 | `MIT OR Apache-2.0` | crates.io |
| serde_derive_internals | 0.30.0 | `MIT OR Apache-2.0` | crates.io |
| serde_fmt | 1.1.0 | `Apache-2.0 OR MIT` | crates.io |
| serde_json | 1.0.154 | `MIT OR Apache-2.0` | crates.io |
| serde_json_lenient | 0.2.4 | `MIT/Apache-2.0` | crates.io |
| serde_repr | 0.1.21 | `MIT OR Apache-2.0` | crates.io |
| serde_spanned | 0.6.9 | `MIT OR Apache-2.0` | crates.io |
| serde_spanned | 1.1.3 | `MIT OR Apache-2.0` | crates.io |
| serde_urlencoded | 0.7.1 | `MIT/Apache-2.0` | crates.io |
| sha1_smol | 1.0.1 | `BSD-3-Clause` | crates.io |
| sha2 | 0.10.9 | `MIT OR Apache-2.0` | crates.io |
| sha2 | 0.11.1 | `MIT OR Apache-2.0` | crates.io |
| shlex | 1.3.0 | `MIT OR Apache-2.0` | crates.io |
| shlex | 2.0.1 | `MIT OR Apache-2.0` | crates.io |
| signal-hook-registry | 1.4.8 | `MIT OR Apache-2.0` | crates.io |
| signature | 3.0.0 | `Apache-2.0 OR MIT` | crates.io |
| simd-adler32 | 0.3.10 | `MIT` | crates.io |
| simd_helpers | 0.1.0 | `MIT` | crates.io |
| simdutf8 | 0.1.5 | `MIT OR Apache-2.0` | crates.io |
| simplecss | 0.2.2 | `Apache-2.0 OR MIT` | crates.io |
| siphasher | 1.0.4 | `MIT OR Apache-2.0` | crates.io |
| skrifa | 0.44.0 | `MIT OR Apache-2.0` | crates.io |
| slab | 0.4.12 | `MIT` | crates.io |
| slotmap | 1.1.1 | `Zlib` | crates.io |
| smallvec | 1.16.5 | `MIT OR Apache-2.0` | crates.io |
| smol | 2.0.2 | `Apache-2.0 OR MIT` | crates.io |
| smol_str | 0.2.2 | `MIT OR Apache-2.0` | crates.io |
| socket2 | 0.6.5 | `MIT OR Apache-2.0` | crates.io |
| spin | 0.9.9 | `MIT` | crates.io |
| spirv | 0.3.0+sdk-1.3.268.0 | `Apache-2.0` | crates.io |
| stable_deref_trait | 1.2.1 | `MIT OR Apache-2.0` | crates.io |
| stacker | 0.1.25 | `MIT OR Apache-2.0` | crates.io |
| stacksafe | 0.1.4 | `Apache-2.0` | crates.io |
| stacksafe-macro | 0.1.4 | `Apache-2.0` | crates.io |
| static_assertions | 1.1.0 | `MIT OR Apache-2.0` | crates.io |
| str_indices | 0.4.4 | `MIT OR Apache-2.0` | crates.io |
| streaming-iterator | 0.1.9 | `MIT OR Apache-2.0` | crates.io |
| strict-num | 0.1.1 | `MIT` | crates.io |
| string_cache | 0.11.0 | `MIT OR Apache-2.0` | crates.io |
| string_cache_codegen | 0.11.2 | `MIT OR Apache-2.0` | crates.io |
| strum | 0.26.3 | `MIT` | crates.io |
| strum | 0.27.2 | `MIT` | crates.io |
| strum_macros | 0.26.4 | `MIT` | crates.io |
| strum_macros | 0.27.2 | `MIT` | crates.io |
| subtle | 2.6.1 | `BSD-3-Clause` | crates.io |
| sval | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_buffer | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_dynamic | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_fmt | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_json | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_nested | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_ref | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| sval_serde | 2.22.0 | `Apache-2.0 OR MIT` | crates.io |
| svg_fmt | 0.4.5 | `MIT/Apache-2.0` | crates.io |
| svgtypes | 0.15.3 | `Apache-2.0 OR MIT` | crates.io |
| swash | 0.2.10 | `Apache-2.0 OR MIT` | crates.io |
| syn | 1.0.109 | `MIT OR Apache-2.0` | crates.io |
| syn | 2.0.119 | `MIT OR Apache-2.0` | crates.io |
| syn | 3.0.10 | `MIT OR Apache-2.0` | crates.io |
| sync_wrapper | 1.0.2 | `Apache-2.0` | crates.io |
| synstructure | 0.14.0 | `MIT` | crates.io |
| sys-locale | 0.3.2 | `MIT OR Apache-2.0` | crates.io |
| sysinfo | 0.31.4 | `MIT` | crates.io |
| system-configuration | 0.6.1 | `MIT OR Apache-2.0` | crates.io |
| system-configuration-sys | 0.6.0 | `MIT OR Apache-2.0` | crates.io |
| taffy | 0.9.0 | `MIT` | crates.io |
| take-until | 0.2.0 | `MIT` | crates.io |
| tao-core-video-sys | 0.2.0 | `MIT` | crates.io |
| target-features | 0.1.6 | `MIT OR Apache-2.0` | crates.io |
| tempfile | 3.27.0 | `MIT OR Apache-2.0` | crates.io |
| tendril | 0.4.3 | `MIT/Apache-2.0` | crates.io |
| tendril | 0.5.1 | `MIT OR Apache-2.0` | crates.io |
| termcolor | 1.4.1 | `Unlicense OR MIT` | crates.io |
| thiserror | 1.0.69 | `MIT OR Apache-2.0` | crates.io |
| thiserror | 2.0.21 | `MIT OR Apache-2.0` | crates.io |
| thiserror-impl | 1.0.69 | `MIT OR Apache-2.0` | crates.io |
| thiserror-impl | 2.0.21 | `MIT OR Apache-2.0` | crates.io |
| tiff | 0.11.3 | `MIT` | crates.io |
| tiny-keccak | 2.0.2 | `CC0-1.0` | crates.io |
| tiny-skia | 0.11.4 | `BSD-3-Clause` | crates.io |
| tiny-skia-path | 0.11.4 | `BSD-3-Clause` | crates.io |
| tinystr | 0.8.5 | `Unicode-3.0` | crates.io |
| tinytemplate | 1.2.1 | `Apache-2.0 OR MIT` | crates.io |
| tinyvec | 1.13.3 | `Zlib OR Apache-2.0 OR MIT` | crates.io |
| tokio | 1.54.1 | `MIT` | crates.io |
| tokio-rustls | 0.26.6 | `MIT OR Apache-2.0` | crates.io |
| tokio-socks | 0.5.3 | `MIT` | crates.io |
| tokio-util | 0.7.20 | `MIT` | crates.io |
| toml | 0.8.23 | `MIT OR Apache-2.0` | crates.io |
| toml | 1.2.1+spec-1.1.0 | `MIT OR Apache-2.0` | crates.io |
| toml_datetime | 0.6.11 | `MIT OR Apache-2.0` | crates.io |
| toml_datetime | 1.1.3+spec-1.1.0 | `MIT OR Apache-2.0` | crates.io |
| toml_edit | 0.22.27 | `MIT OR Apache-2.0` | crates.io |
| toml_edit | 0.25.19+spec-1.1.0 | `MIT OR Apache-2.0` | crates.io |
| toml_parser | 1.1.5+spec-1.1.0 | `MIT OR Apache-2.0` | crates.io |
| toml_write | 0.1.2 | `MIT OR Apache-2.0` | crates.io |
| toml_writer | 1.2.0+spec-1.1.0 | `MIT OR Apache-2.0` | crates.io |
| tower | 0.5.3 | `MIT` | crates.io |
| tower-layer | 0.3.3 | `MIT` | crates.io |
| tower-service | 0.3.3 | `MIT` | crates.io |
| tracing | 0.1.44 | `MIT` | crates.io |
| tracing-attributes | 0.1.31 | `MIT` | crates.io |
| tracing-core | 0.1.36 | `MIT` | crates.io |
| tree-sitter | 0.25.10 | `MIT` | crates.io |
| tree-sitter-c-sharp | 0.23.5 | `MIT` | crates.io |
| tree-sitter-json | 0.24.8 | `MIT` | crates.io |
| tree-sitter-language | 0.1.9 | `MIT` | crates.io |
| tree-sitter-md | 0.5.4 | `MIT` | crates.io |
| tree-sitter-xml | 0.7.0 | `MIT` | crates.io |
| tree-sitter-yaml | 0.7.2 | `MIT` | crates.io |
| try-lock | 0.2.5 | `MIT` | crates.io |
| ttf-parser | 0.20.0 | `MIT OR Apache-2.0` | crates.io |
| ttf-parser | 0.21.1 | `MIT OR Apache-2.0` | crates.io |
| ttf-parser | 0.25.1 | `MIT OR Apache-2.0` | crates.io |
| typeid | 1.0.3 | `MIT OR Apache-2.0` | crates.io |
| typenum | 1.20.1 | `MIT OR Apache-2.0` | crates.io |
| uds_windows | 1.2.1 | `MIT` | crates.io |
| unarray | 0.1.4 | `MIT OR Apache-2.0` | crates.io |
| unicase | 2.10.0 | `MIT OR Apache-2.0` | crates.io |
| unicode-bidi | 0.3.18 | `MIT OR Apache-2.0` | crates.io |
| unicode-bidi-mirroring | 0.2.0 | `MIT/Apache-2.0` | crates.io |
| unicode-bidi-mirroring | 0.4.0 | `MIT/Apache-2.0` | crates.io |
| unicode-ccc | 0.2.0 | `MIT/Apache-2.0` | crates.io |
| unicode-ccc | 0.4.0 | `MIT/Apache-2.0` | crates.io |
| unicode-ident | 1.0.27 | `(MIT OR Apache-2.0) AND Unicode-3.0` | crates.io |
| unicode-linebreak | 0.1.5 | `Apache-2.0` | crates.io |
| unicode-properties | 0.1.5 | `(MIT OR Apache-2.0) AND Unicode-3.0` | crates.io |
| unicode-script | 0.5.9 | `MIT OR Apache-2.0` | crates.io |
| unicode-segmentation | 1.13.3 | `MIT OR Apache-2.0` | crates.io |
| unicode-vo | 0.1.0 | `MIT/Apache-2.0` | crates.io |
| unicode-width | 0.2.3 | `MIT OR Apache-2.0` | crates.io |
| untrusted | 0.9.0 | `ISC` | crates.io |
| url | 2.5.8 | `MIT OR Apache-2.0` | crates.io |
| usvg | 0.45.1 | `Apache-2.0 OR MIT` | crates.io |
| utf-8 | 0.7.6 | `MIT OR Apache-2.0` | crates.io |
| utf8_iter | 1.1.0 | `Apache-2.0 OR MIT` | crates.io |
| utf_types | 1.0.0 | `Apache-2.0 OR MIT` | crates.io |
| uuid | 1.29.2 | `Apache-2.0 OR MIT` | crates.io |
| v_frame | 0.3.9 | `BSD-2-Clause` | crates.io |
| value-bag | 1.14.1 | `Apache-2.0 OR MIT` | crates.io |
| value-bag-serde1 | 1.14.1 | `Apache-2.0 OR MIT` | crates.io |
| value-bag-sval2 | 1.14.1 | `Apache-2.0 OR MIT` | crates.io |
| vcpkg | 0.2.15 | `MIT/Apache-2.0` | crates.io |
| version_check | 0.9.5 | `MIT/Apache-2.0` | crates.io |
| vswhom | 0.1.0 | `MIT` | crates.io |
| vswhom-sys | 0.1.3 | `MIT` | crates.io |
| wait-timeout | 0.2.1 | `MIT/Apache-2.0` | crates.io |
| waker-fn | 1.2.0 | `Apache-2.0 OR MIT` | crates.io |
| walkdir | 2.5.0 | `Unlicense/MIT` | crates.io |
| want | 0.3.2 | `MIT` | crates.io |
| wasi | 0.11.1+wasi-snapshot-preview1 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| wasip2 | 1.0.4+wasi-0.2.12 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| wasm-bindgen | 0.2.130 | `MIT OR Apache-2.0` | crates.io |
| wasm-bindgen-futures | 0.4.80 | `MIT OR Apache-2.0` | crates.io |
| wasm-bindgen-macro | 0.2.130 | `MIT OR Apache-2.0` | crates.io |
| wasm-bindgen-macro-support | 0.2.130 | `MIT OR Apache-2.0` | crates.io |
| wasm-bindgen-shared | 0.2.130 | `MIT OR Apache-2.0` | crates.io |
| wasm-streams | 0.4.2 | `MIT OR Apache-2.0` | crates.io |
| wayland-backend | 0.3.17 | `MIT` | crates.io |
| wayland-client | 0.31.15 | `MIT` | crates.io |
| wayland-cursor | 0.31.14 | `MIT` | crates.io |
| wayland-protocols | 0.31.2 | `MIT` | crates.io |
| wayland-protocols | 0.32.13 | `MIT` | crates.io |
| wayland-protocols-plasma | 0.2.0 | `MIT` | crates.io |
| wayland-scanner | 0.31.11 | `MIT` | crates.io |
| wayland-sys | 0.31.11 | `MIT` | crates.io |
| web-sys | 0.3.107 | `MIT OR Apache-2.0` | crates.io |
| web-time | 1.1.0 | `MIT OR Apache-2.0` | crates.io |
| web_atoms | 0.3.0 | `MIT OR Apache-2.0` | crates.io |
| webview2-com | 0.38.2 | `MIT` | crates.io |
| webview2-com-macros | 0.8.1 | `MIT` | crates.io |
| webview2-com-sys | 0.38.2 | `MIT` | crates.io |
| weezl | 0.1.12 | `MIT OR Apache-2.0` | crates.io |
| which | 6.0.3 | `MIT` | crates.io |
| winapi | 0.3.9 | `MIT/Apache-2.0` | crates.io |
| winapi-i686-pc-windows-gnu | 0.4.0 | `MIT/Apache-2.0` | crates.io |
| winapi-util | 0.1.11 | `Unlicense OR MIT` | crates.io |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | `MIT/Apache-2.0` | crates.io |
| windows | 0.57.0 | `MIT OR Apache-2.0` | crates.io |
| windows | 0.61.3 | `MIT OR Apache-2.0` | crates.io |
| windows-capture | 1.5.0 | `MIT` | crates.io |
| windows-collections | 0.2.0 | `MIT OR Apache-2.0` | crates.io |
| windows-core | 0.57.0 | `MIT OR Apache-2.0` | crates.io |
| windows-core | 0.61.2 | `MIT OR Apache-2.0` | crates.io |
| windows-future | 0.2.1 | `MIT OR Apache-2.0` | crates.io |
| windows-implement | 0.57.0 | `MIT OR Apache-2.0` | crates.io |
| windows-implement | 0.60.2 | `MIT OR Apache-2.0` | crates.io |
| windows-interface | 0.57.0 | `MIT OR Apache-2.0` | crates.io |
| windows-interface | 0.59.3 | `MIT OR Apache-2.0` | crates.io |
| windows-link | 0.1.3 | `MIT OR Apache-2.0` | crates.io |
| windows-link | 0.2.1 | `MIT OR Apache-2.0` | crates.io |
| windows-numerics | 0.2.0 | `MIT OR Apache-2.0` | crates.io |
| windows-registry | 0.4.0 | `MIT OR Apache-2.0` | crates.io |
| windows-registry | 0.5.3 | `MIT OR Apache-2.0` | crates.io |
| windows-result | 0.1.2 | `MIT OR Apache-2.0` | crates.io |
| windows-result | 0.3.4 | `MIT OR Apache-2.0` | crates.io |
| windows-strings | 0.3.1 | `MIT OR Apache-2.0` | crates.io |
| windows-strings | 0.4.2 | `MIT OR Apache-2.0` | crates.io |
| windows-sys | 0.48.0 | `MIT OR Apache-2.0` | crates.io |
| windows-sys | 0.52.0 | `MIT OR Apache-2.0` | crates.io |
| windows-sys | 0.60.2 | `MIT OR Apache-2.0` | crates.io |
| windows-sys | 0.61.2 | `MIT OR Apache-2.0` | crates.io |
| windows-targets | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows-targets | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows-targets | 0.53.5 | `MIT OR Apache-2.0` | crates.io |
| windows-threading | 0.1.0 | `MIT OR Apache-2.0` | crates.io |
| windows_aarch64_gnullvm | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_aarch64_gnullvm | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_aarch64_gnullvm | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_aarch64_msvc | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_aarch64_msvc | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_aarch64_msvc | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_gnu | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_gnu | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_gnu | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_gnullvm | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_gnullvm | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_msvc | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_msvc | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_i686_msvc | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_gnu | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_gnu | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_gnu | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_gnullvm | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_gnullvm | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_gnullvm | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_msvc | 0.48.5 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_msvc | 0.52.6 | `MIT OR Apache-2.0` | crates.io |
| windows_x86_64_msvc | 0.53.1 | `MIT OR Apache-2.0` | crates.io |
| winnow | 0.7.15 | `MIT` | crates.io |
| winnow | 1.0.5 | `MIT` | crates.io |
| winreg | 0.56.0 | `MIT` | crates.io |
| winsafe | 0.0.19 | `MIT` | crates.io |
| wio | 0.2.2 | `MIT/Apache-2.0` | crates.io |
| wit-bindgen | 0.57.1 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | crates.io |
| writeable | 0.6.4 | `Unicode-3.0` | crates.io |
| x11 | 2.21.0 | `MIT` | crates.io |
| x11-clipboard | 0.9.3 | `MIT` | crates.io |
| x11rb | 0.13.2 | `MIT OR Apache-2.0` | crates.io |
| x11rb-protocol | 0.13.2 | `MIT OR Apache-2.0` | crates.io |
| xattr | 0.2.3 | `MIT/Apache-2.0` | crates.io |
| xcb | 1.7.1 | `MIT` | crates.io |
| xcursor | 0.3.11 | `MIT` | crates.io |
| xim-ctext | 0.3.0 | `MIT` | crates.io |
| xim-parser | 0.2.2 | `MIT` | crates.io |
| xkbcommon | 0.8.0 | `MIT` | crates.io |
| xkeysym | 0.2.1 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| xmlwriter | 0.1.0 | `MIT` | crates.io |
| y4m | 0.8.0 | `MIT` | crates.io |
| yazi | 0.2.2 | `Apache-2.0 OR MIT` | crates.io |
| yeslogic-fontconfig-sys | 6.0.1 | `MIT` | crates.io |
| yoke | 0.8.4 | `Unicode-3.0` | crates.io |
| yoke-derive | 0.8.4 | `Unicode-3.0` | crates.io |
| zbus | 5.19.0 | `MIT` | crates.io |
| zbus_macros | 5.19.0 | `MIT` | crates.io |
| zbus_names | 4.3.5 | `MIT` | crates.io |
| zcheapstr | 1.2.0 | `MIT` | crates.io |
| zed-async-tar | 0.5.0-zed | `MIT/Apache-2.0` | crates.io |
| zed-font-kit | 0.14.1-zed | `MIT OR Apache-2.0` | crates.io |
| zed-reqwest | 0.12.15-zed | `MIT OR Apache-2.0` | crates.io |
| zed-scap | 0.0.8-zed | `MIT` | crates.io |
| zed-xim | 0.4.0-zed | `MIT` | crates.io |
| zeno | 0.3.3 | `Apache-2.0 OR MIT` | crates.io |
| zerocopy | 0.8.63 | `BSD-2-Clause OR Apache-2.0 OR MIT` | crates.io |
| zerocopy-derive | 0.8.63 | `BSD-2-Clause OR Apache-2.0 OR MIT` | crates.io |
| zerofrom | 0.1.8 | `Unicode-3.0` | crates.io |
| zerofrom-derive | 0.1.8 | `Unicode-3.0` | crates.io |
| zeroize | 1.9.1 | `Apache-2.0 OR MIT` | crates.io |
| zeroize_derive | 1.5.0 | `Apache-2.0 OR MIT` | crates.io |
| zerotrie | 0.2.5 | `Unicode-3.0` | crates.io |
| zerovec | 0.11.9 | `Unicode-3.0` | crates.io |
| zerovec-derive | 0.11.7 | `Unicode-3.0` | crates.io |
| zlib-rs | 0.6.8 | `Zlib` | crates.io |
| zmij | 1.0.23 | `MIT` | crates.io |
| zune-core | 0.5.3 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| zune-inflate | 0.2.54 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| zune-jpeg | 0.5.15 | `MIT OR Apache-2.0 OR Zlib` | crates.io |
| zvariant | 5.15.1 | `MIT` | crates.io |
| zvariant_derive | 5.15.1 | `MIT` | crates.io |
| zvariant_utils | 4.2.0 | `MIT` | crates.io |
