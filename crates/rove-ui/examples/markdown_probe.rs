//! #29 真实 WebView2 验收探针（Windows，真实窗口，不进 CI）：
//!
//! 1. 冷启动后、打开预览前：进程树里没有 `msedgewebview2.exe`；
//! 2. 打开 `.md`（文件树来源）→ 预览：首开耗时（打开请求 → 页面回报“已渲染”）、controller 创建耗时、
//!    渲染统计（Mermaid / KaTeX / 代码高亮 / 本地图片 / 被阻止的远程图片 / 文档脚本是否执行）；
//! 3. 长文档（2400 段、4800 行）并排的双向滚动同步误差（源码行）；
//! 4. 主题切换：浏览器进程不变（不重建实例）；
//! 5. 关闭全部预览 → 挂起；`rove.markdown.webviewIdleTimeout` 到期 → 实例与浏览器进程全部退出；
//! 6. 全程每 500 ms 用 `netstat -ano` 采样 WebView2 进程树的网络连接（应为 0）。
//!
//! 每个阶段记录宿主进程与 WebView2 子进程树的工作集 / 私有提交（`Win32_Process`）。
//! 运行：`cargo run -p rove-ui --release --example markdown_probe`，报告写到
//! `target/markdown-probe/report.json`。**注明机器负载**：开着 Unity 等程序时数字只作参考。
use gpui::{prelude::*, px, size, App, Application, Bounds, WindowBounds, WindowOptions};
use rove_config::{SettingsOptions, SettingsService};
use rove_ui::{shell::layout::LayoutStore, skeleton::Workbench};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const SAMPLE: &str = r#"---
title: 渲染样例
---
# 渲染样例

| 左对齐 | 居中 | 右对齐 |
|:--|:-:|--:|
| a | b | c |

- [x] 已完成
- [ ] 未完成

<details><summary>内嵌 HTML</summary><b>粗体</b></details>

<script>window.__roveDocumentScript = 1;</script>
<img src="x" onerror="window.__roveDocumentScript = 2">

```mermaid
graph TD; A[开始] --> B{判断}; B -->|是| C[结束]
```

行内公式 $E = mc^2$，块公式：

$$\int_0^1 x^2\,dx = \frac{1}{3}$$

> [!NOTE]
> 提示块。

![本地图片](images/dot.png)

![远程图片](__ROVE_TEST_ORIGIN__/remote.png)

<video controls src="__ROVE_TEST_ORIGIN__/remote.mp4"></video>

```csharp
public class Player { void Update() { } }
```
"#;

#[derive(Default)]
struct Report {
    phases: Vec<Value>,
    network: BTreeSet<String>,
    pids: BTreeSet<u32>,
}

/// 宿主进程与它下面所有 msedgewebview2 进程的内存（MiB）。
fn memory(host: u32) -> (Value, BTreeSet<u32>) {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,WorkingSetSize,PrivatePageCount | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(output) = output else {
        return (Value::Null, BTreeSet::new());
    };
    let all: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap_or_default();
    let mut tree = BTreeSet::from([host]);
    loop {
        let before = tree.len();
        for p in &all {
            let (pid, parent) = (p["ProcessId"].as_u64(), p["ParentProcessId"].as_u64());
            if let (Some(pid), Some(parent)) = (pid, parent) {
                if tree.contains(&(parent as u32)) {
                    tree.insert(pid as u32);
                }
            }
        }
        if tree.len() == before {
            break;
        }
    }
    let mib = |v: &Value| v.as_f64().unwrap_or(0.0) / 1024.0 / 1024.0;
    let (mut host_ws, mut host_private, mut web_ws, mut web_private, mut web_count) =
        (0.0, 0.0, 0.0, 0.0, 0);
    let mut webview = BTreeSet::new();
    for p in &all {
        let pid = p["ProcessId"].as_u64().unwrap_or(0) as u32;
        if !tree.contains(&pid) {
            continue;
        }
        if pid == host {
            host_ws = mib(&p["WorkingSetSize"]);
            host_private = mib(&p["PrivatePageCount"]);
        } else if p["Name"]
            .as_str()
            .is_some_and(|n| n.eq_ignore_ascii_case("msedgewebview2.exe"))
        {
            web_ws += mib(&p["WorkingSetSize"]);
            web_private += mib(&p["PrivatePageCount"]);
            web_count += 1;
            webview.insert(pid);
        }
    }
    (
        json!({
            "host_working_set_mib": host_ws, "host_private_mib": host_private,
            "webview_processes": web_count, "webview_working_set_mib": web_ws,
            "webview_private_mib": web_private, "total_working_set_mib": host_ws + web_ws,
            "total_private_mib": host_private + web_private,
        }),
        webview,
    )
}

/// `netstat -ano` 中属于这些进程的连接（TCP 与 UDP）。
fn connections(pids: &BTreeSet<u32>) -> Vec<String> {
    if pids.is_empty() {
        return Vec::new();
    }
    let Ok(output) = std::process::Command::new("netstat").arg("-ano").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .last()
                .and_then(|pid| pid.parse::<u32>().ok())
                .is_some_and(|pid| pids.contains(&pid))
        })
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn main() {
    let root = std::env::temp_dir().join(format!("rove-markdown-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("images")).unwrap();
    std::fs::write(root.join("images/dot.png"), PNG).unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let contacts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener_contacts = contacts.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            listener_contacts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            drop(stream);
        }
    });
    let sample = SAMPLE.replace("__ROVE_TEST_ORIGIN__", &origin);
    std::fs::write(root.join("Sample.md"), &sample).unwrap();
    std::fs::write(
        root.join("local.css"),
        format!(r#"@import url("{origin}/import.css");"#),
    )
    .unwrap();
    let long: String = (0..2400)
        .map(|i| format!("## 第 {i} 节\n\n段落 {i}：{}\n\n", "文字".repeat(i % 7 + 1)))
        .collect();
    std::fs::write(root.join("Long.md"), long).unwrap();
    let user = root.join("user");
    std::fs::create_dir_all(&user).unwrap();
    let idle = 5;
    let settings_json = move |theme: &str| {
        format!(
            r#"{{ "rove.index.enabled": false, "rove.markdown.allowRemoteResources": true, "markdown.styles": ["{origin}/style.css", "local.css"], "rove.markdown.webviewIdleTimeout": {idle}, "workbench.colorTheme": "{theme}", "markdown.preview.breaks": {breaks} }}"#,
            breaks = theme == "Light 2026",
        )
    };
    std::fs::write(user.join("settings.json"), settings_json("Dark 2026")).unwrap();
    let mut options = SettingsOptions::new(&user, user.join("backup"));
    options.watch = None;
    let service = Arc::new(SettingsService::open(
        options,
        rove_ui::app_settings::registry().unwrap(),
    ));
    let host = std::process::id();
    let report = Arc::new(Mutex::new(Report::default()));
    let started = Instant::now();

    // 网络采样线程：持续记录 WebView2 进程树的连接。
    let sampler = report.clone();
    std::thread::spawn(move || loop {
        let pids = sampler.lock().unwrap().pids.clone();
        for line in connections(&pids) {
            sampler.lock().unwrap().network.insert(line);
        }
        std::thread::sleep(Duration::from_millis(500));
    });

    rove_ui::markdown::prepare_hwnd_hosting();
    Application::new().run(move |cx: &mut App| {
        let _ = cx
            .text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(rove_ui::shell::icons::FONT)]);
        rove_ui::keys::init(cx);
        let settings: Arc<dyn rove_config::Settings> = service.clone();
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1400.0), px(900.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let mut view = Workbench::with_settings(
                            root.clone(),
                            started,
                            LayoutStore::disabled(),
                            settings,
                            cx,
                        )
                        .unwrap();
                        rove_ui::shell::Shell::attach_window(&mut view, window, cx);
                        view
                    })
                },
            )
            .unwrap();
        window
            .update(cx, |_, window, cx| {
                window.activate_window();
                cx.activate(true);
            })
            .unwrap();
        let report = report.clone();
        let service = service.clone();
        let root = root.clone();
        let contacts = contacts.clone();
        let sample = sample.clone();
        cx.spawn(async move |cx| {
            let executor = cx.background_executor().clone();
            let sleep = move |ms: u64| executor.timer(Duration::from_millis(ms));
            let phase = |name: &str, extra: Value| {
                let (mem, webview) = memory(host);
                let mut r = report.lock().unwrap();
                r.pids.extend(webview.iter().copied());
                let entry = json!({"phase": name, "t_ms": started.elapsed().as_millis() as u64, "memory": mem, "extra": extra});
                println!("MARKDOWN_PROBE {entry}");
                r.phases.push(entry);
                webview
            };
            let snapshot = |cx: &mut gpui::AsyncApp| {
                window
                    .update(cx, |v: &mut Workbench, _, _| v.markdown_snapshot())
                    .unwrap()
            };
            let open = |cx: &mut gpui::AsyncApp, path: &str, source: bool| {
                let path = path.to_string();
                window
                    .update(cx, |v: &mut Workbench, _, cx| {
                        use rove_ui::documents::*;
                        let options = OpenOptions {
                            source: OpenSource::Explorer,
                            presentation: if source { Presentation::Source } else { Presentation::Default },
                            ..Default::default()
                        };
                        v.open_documents()
                            .open(
                                rove_base::Resource::Workspace(rove_base::WorkspacePath::new(&path).unwrap()),
                                options,
                                rove_base::Cancellation::default(),
                            )
                            .unwrap();
                        cx.notify();
                    })
                    .unwrap();
            };
            sleep(2000).await;
            let before = phase("before_preview", json!({}));
            assert!(before.is_empty(), "打开预览前不应有 WebView2 进程");

            // 首开。
            open(cx, "Sample.md", false);
            let opened = Instant::now();
            let mut snap = snapshot(cx);
            while !snap.rendered && opened.elapsed() < Duration::from_secs(30) {
                sleep(50).await;
                snap = snapshot(cx);
            }
            sleep(500).await;
            let snap = snapshot(cx);
            phase(
                "first_open",
                json!({
                    "rendered": snap.rendered, "first_open_ms": snap.first_open_ms,
                    "controller_ms": snap.created_ms, "stats": snap.stats, "remote_blocked": snap.remote,
                    "runtime_missing": snap.runtime_missing,
                }),
            );
            println!("MARKDOWN_PROBE_SHOT_SAMPLE");
            sleep(6000).await;
            phase("preview_steady_6s", json!({}));

            // 旧放行配置仍启用；改预览选项触发重新渲染，再切换至另一个预览。
            std::fs::write(user_settings(&root), settings_json("Light 2026")).unwrap();
            service.reload();
            sleep(1000).await;
            let snap = snapshot(cx);
            phase("offline_rerender", json!({ "rendered": snap.rendered, "stats": snap.stats, "remote_blocked": snap.remote }));
            std::fs::write(user_settings(&root), settings_json("Dark 2026")).unwrap();
            service.reload();
            std::fs::write(root.join("Other.md"), &sample).unwrap();
            open(cx, "Other.md", false);
            sleep(1000).await;
            let snap = snapshot(cx);
            phase("offline_switch", json!({ "rendered": snap.rendered, "stats": snap.stats, "remote_blocked": snap.remote }));

            // 长文档并排：源码 → 预览、预览 → 源码。
            open(cx, "Long.md", true);
            sleep(1500).await;
            window
                .update(cx, |v: &mut Workbench, window, cx| {
                    rove_ui::shell::ShellHost::run_external_command(v, "markdown.showPreviewToSide", window, cx)
                })
                .unwrap();
            let start = Instant::now();
            while !snapshot(cx).rendered && start.elapsed() < Duration::from_secs(20) {
                sleep(50).await;
            }
            sleep(1000).await;
            let mut errors = Vec::new();
            for target in [600.0, 1800.0, 3000.0, 4200.0] {
                // 编辑器 → 预览
                window
                    .update(cx, |v: &mut Workbench, _, cx| {
                        let code = v.code_view().clone();
                        code.update(cx, |c, cx| {
                            c.set_scroll_position(rove_ui::code_view::ScrollPosition { top: target, x: 0.0 }, false, cx)
                        });
                    })
                    .unwrap();
                sleep(600).await;
                window
                    .update(cx, |v: &mut Workbench, _, _| v.markdown_probe("where", 0.0))
                    .unwrap();
                sleep(300).await;
                let preview = snapshot(cx).position;
                // 预览 → 编辑器
                window
                    .update(cx, |v: &mut Workbench, _, _| v.markdown_probe("userScroll", target + 300.0))
                    .unwrap();
                sleep(600).await;
                let editor = window
                    .update(cx, |v: &mut Workbench, _, cx| v.code_view().read(cx).scroll_position().top)
                    .unwrap();
                let rows = window
                    .update(cx, |v: &mut Workbench, _, cx| {
                        v.code_view().read(cx).viewport().map(|vp| vp.last_line - vp.first_line)
                    })
                    .unwrap();
                errors.push(json!({
                    "editor_top": target, "preview_top_line": preview,
                    "editor_to_preview_error_lines": preview.map(|p| (p - target).abs()),
                    "preview_target": target + 300.0, "editor_after_preview_scroll": editor,
                    "preview_to_editor_error_lines": (editor - (target + 300.0)).abs(),
                    "editor_screen_lines": rows,
                }));
            }
            phase("scroll_sync_long_4800_lines", json!({ "samples": errors }));

            // 主题切换：浏览器进程不变。
            let pids_before = snapshot(cx).process_ids;
            std::fs::write(user_settings(&root), settings_json("Light 2026")).unwrap();
            service.reload();
            sleep(1500).await;
            let pids_after = snapshot(cx).process_ids;
            phase("theme_switched", json!({ "browser_pid_before": pids_before, "browser_pid_after": pids_after }));

            // 关闭全部预览 → 挂起 → 超时回收。
            window
                .update(cx, |v: &mut Workbench, window, cx| {
                    rove_ui::shell::ShellHost::run_external_command(v, "workbench.action.closeAllEditors", window, cx)
                })
                .unwrap();
            sleep(2000).await;
            let snap = snapshot(cx);
            phase("closed_suspended", json!({ "lifecycle": snap.lifecycle, "idle_pending": snap.idle_pending }));
            sleep(idle as u64 * 1000 + 1500).await;
            let snap = snapshot(cx);
            sleep(3000).await;
            let left = phase("idle_disposed_plus_3s", json!({ "lifecycle": snap.lifecycle }));
            sleep(1000).await;
            let r = report.lock().unwrap();
            let out = json!({
                "phases": r.phases,
                "remote_resource_connections": contacts.load(std::sync::atomic::Ordering::Relaxed),
                "network_connections_of_webview_processes": r.network.iter().collect::<Vec<_>>(),
                "webview_processes_left": left.len(),
            });
            let dir = PathBuf::from("target/markdown-probe");
            let _ = std::fs::create_dir_all(&dir);
            std::fs::write(dir.join("report.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
            println!("MARKDOWN_PROBE_NETWORK {}", r.network.len());
            println!("MARKDOWN_PROBE_DONE {}", dir.join("report.json").display());
            for name in ["first_open", "offline_rerender", "offline_switch"] {
                let phase = r.phases.iter().find(|p| p["phase"] == name).unwrap();
                let extra = &phase["extra"];
                let stats = &extra["stats"];
                assert_eq!(extra["rendered"], true, "{name}: {extra}");
                assert_eq!(extra["remote_blocked"], 2, "{name}: {extra}");
                assert_eq!(stats["imagesLoaded"], 1, "{name}: {stats}");
                assert_eq!(stats["mermaid"], 1, "{name}: {stats}");
                assert_eq!(stats["mermaidErrors"], 0, "{name}: {stats}");
                assert_eq!(stats["math"], 2, "{name}: {stats}");
                assert_eq!(stats["mathErrors"], 0, "{name}: {stats}");
            }
            assert_eq!(out["remote_resource_connections"], 0, "{out}");
            drop(r);
            let _ = std::fs::remove_dir_all(&root);
            cx.update(|cx| cx.quit()).ok();
        })
        .detach();
        let _keep = Rc::new(());
    });
}

fn user_settings(root: &std::path::Path) -> PathBuf {
    root.join("user").join("settings.json")
}
