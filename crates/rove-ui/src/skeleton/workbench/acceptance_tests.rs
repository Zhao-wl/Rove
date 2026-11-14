//! #31 M1 集成验收：把场景清单里**可自动化**的部分放到两个真实 Unity 项目上跑一遍。
//!
//! 与各 issue 自己的入口测试的区别：那些用临时目录里的合成文件，覆盖的是单个模块；这里用
//! `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 指向的真实项目，覆盖的是**接缝**——
//! 排除规则与 `.gitignore` 的关系、真实文件的编码与换行符识别、`.md` 的来源语义（文件树
//! 进预览、搜索结果进源码）、包缓存只读、共用设置热加载。
//!
//! 环境变量未设置时整组跳过（AGENTS.md §8），所以 CI 上 `cargo test --workspace` 仍然全绿。
//! 路径不写进仓库：只从环境变量读，断言里不出现具体目录名以外的项目路径。
//!
//! 运行：`cargo test -p rove-ui acceptance_ -j 1 -- --nocapture`。
use super::*;
use crate::{
    editors::settings::is_markdown_preview, search::view::State as SearchState,
    status_items::registry as status_ids,
};
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{ByteOffset, ByteRange, Position, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};
use rove_test_support::UnityProject;
use rove_theme::ThemeKind;
use rove_workspace::{Folder, WorkspaceConfig};
use std::path::Path;

/// 真实项目上的窗口夹具：根目录是项目本身（**只读使用，不写入项目**），共用设置文件放在临时
/// 用户目录里（场景 13 在那里改设置，模拟 VS Code 改共用 `settings.json`）。
struct RealProject {
    user: tempfile::TempDir,
    root: PathBuf,
    service: Arc<SettingsService>,
}

impl RealProject {
    /// 环境变量缺失时返回 `None`（调用方 `return`，测试算通过并在日志里写明跳过原因）。
    fn open(slot: UnityProject, settings_json: &str) -> Option<Self> {
        let root = slot.path()?;
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), settings_json).unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        Some(Self {
            user,
            root,
            service,
        })
    }

    fn window<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root = self.root.clone();
        let settings: Arc<dyn Settings> = self.service.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            Workbench::with_settings(root, Instant::now(), LayoutStore::disabled(), settings, cx)
                .unwrap()
        });
        // 真实项目的扫描是流式的：根目录先出来，其余在后台补。
        settle(cx);
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
        (view, cx)
    }

    /// 改共用 `settings.json`（场景 13：VS Code 侧改、Rove 实时生效）。
    fn change(&self, settings_json: &str, cx: &mut VisualTestContext) {
        std::fs::write(self.user.path().join("settings.json"), settings_json).unwrap();
        self.service.reload();
        settle(cx);
    }
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
    }
}

fn open_file(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    path: &str,
    options: OpenOptions,
) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                options,
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
}

fn status_text(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) -> String {
    view.read_with(cx, |v, _| {
        v.shell
            .status_bar
            .get(id)
            .map(|e| e.text.clone())
            .unwrap_or_default()
    })
}

fn status_tooltip(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) -> String {
    view.read_with(cx, |v, _| {
        v.shell
            .status_bar
            .get(id)
            .and_then(|e| e.tooltip.clone())
            .unwrap_or_default()
    })
}

/// 整篇正文（真实文件都不大：查找时已经限制在 512 KB 以内）。
fn document_text(text: &dyn rove_buffer::TextSource) -> String {
    let mut out = String::new();
    let all = ByteRange::new(ByteOffset(0), ByteOffset(text.byte_len())).unwrap();
    text.visit_chunks(all, &mut |chunk| out.push_str(chunk))
        .unwrap();
    out
}

/// 工作区相对路径（正斜杠）。项目根以外的路径返回 `None`。
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// 在项目里按条件找文件：深度优先、跳过 Rove 默认排除的目录、最多看 `budget` 个文件。
///
/// 真实项目有十几万个文件，所有查找都带预算，找不到就跳过对应断言并在日志里说明。
fn find_file(
    root: &Path,
    budget: usize,
    skip_excluded: bool,
    mut matches: impl FnMut(&Path, &std::fs::Metadata) -> bool,
) -> Option<PathBuf> {
    let excluded = ["Library", "Temp", "obj", "Logs", ".git"];
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let path = entry.path();
            if meta.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if skip_excluded && excluded.iter().any(|e| *e == name) {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file() {
                seen += 1;
                if seen > budget {
                    return None;
                }
                if matches(&path, &meta) {
                    return Some(path);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------- 场景 2 / 11

/// 场景 2（排除规则与 `.gitignore`），两个项目各跑一遍。
///
/// - `Library/`、`Temp/`、`obj/`、`Logs/` 等 Unity 生成目录不出现在工作区里；
/// - 项目自己的 `.gitignore` 忽略了 `*.csproj` 之类的东西，**Rove 不读 `.gitignore`**，
///   所以这些文件仍然要出现（AGENTS.md §1 的第二条硬约束）；
/// - 普通源码文件不排除、不只读；被排除的路径报告 `excluded`（只读规则对排除路径不求值）。
#[test]
fn acceptance_scenario_2_workspace_excludes_ignore_gitignore() {
    for slot in UnityProject::ALL {
        let Some(root) = slot.path() else { continue };
        let started = Instant::now();
        let folder =
            Folder::scan_root(&root, WorkspaceConfig::default().without_watching()).unwrap();
        let scan_ms = started.elapsed().as_secs_f64() * 1000.0;

        // 全量文件列表（分页读完）。
        let mut paths: Vec<String> = Vec::new();
        let mut from = 0usize;
        loop {
            let page = folder.files(from..from + 20_000).unwrap();
            if page.items.is_empty() {
                assert!(page.complete, "流式扫描没有读完就报告了空页");
                break;
            }
            from += page.items.len();
            paths.extend(page.items.iter().map(|e| e.path.as_str().to_owned()));
        }
        eprintln!(
            "[#31 场景 2] {} Unity 项目：{} 个文件，全量扫描 {scan_ms:.0} ms（机器负载较高）",
            slot.label(),
            paths.len()
        );
        assert!(
            paths.len() > 100,
            "{} 项目扫描结果过少：{}",
            slot.label(),
            paths.len()
        );

        // Unity 生成目录按 rove.files.extraExclude 排除。
        for prefix in ["Library/", "Temp/", "obj/", "Logs/"] {
            let leaked: Vec<&String> = paths
                .iter()
                .filter(|p| p.starts_with(prefix))
                .take(3)
                .collect();
            assert!(
                leaked.is_empty(),
                "{} 项目里 {prefix} 没有被排除：{leaked:?}",
                slot.label()
            );
        }

        // 不读 .gitignore：被 .gitignore 忽略、但不在排除规则里的文件仍然可见。
        let gitignore = root.join(".gitignore");
        if gitignore.is_file() {
            let csproj = paths.iter().filter(|p| p.ends_with(".csproj")).count();
            // 两个测试项目的 .gitignore 都忽略 *.csproj（Unity 工程的惯例），磁盘上却有这些文件。
            let on_disk = std::fs::read_dir(&root)
                .unwrap()
                .flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("csproj"))
                })
                .count();
            if on_disk > 0 {
                assert!(
                    csproj >= on_disk,
                    "{} 项目：磁盘上有 {on_disk} 个 .csproj，工作区只看到 {csproj} 个——\
                     看起来读了 .gitignore",
                    slot.label()
                );
                eprintln!(
                    "[#31 场景 2] {}：.gitignore 存在，{csproj} 个 .csproj 仍然可见（没有读 .gitignore）",
                    slot.label()
                );
            } else {
                eprintln!(
                    "[#31 场景 2] 已跳过 .gitignore 对照：{} 项目根目录里没有 .csproj",
                    slot.label()
                );
            }
        }

        // 被排除的路径报告 excluded（只读规则对排除路径不求值，见 `Folder::rules`：树里看不到
        // 的文件没有装饰可言。真正打开包缓存文件时的只读由场景 11 的窗口测试验证）。
        let pkg = WorkspacePath::new("Library/PackageCache/com.unity.x/Runtime/A.cs").unwrap();
        assert!(
            folder.rules(&pkg).unwrap().excluded,
            "Library/ 下的路径应当是被排除的"
        );
        // 工作区里的普通源码文件：不排除、不只读。
        let sample = paths
            .iter()
            .find(|p| p.starts_with("Assets/") && p.ends_with(".cs"));
        if let Some(sample) = sample {
            let rules = folder
                .rules(&WorkspacePath::new(sample).unwrap())
                .unwrap_or_else(|e| panic!("{sample} 的规则查询失败：{e:?}"));
            assert!(!rules.excluded, "{sample} 不应被排除");
            assert!(
                rules.read_only.is_empty(),
                "{sample} 不应只读：{:?}",
                rules.read_only
            );
        }
    }
}

// ---------------------------------------------------------------- 场景 6 / 7

/// 场景 6（GBK 中文注释）：两个测试项目里**没有**非 UTF-8 的源码文件（实测扫过各自前 2 万个
/// 文件都是 UTF-8），所以这里取**真实项目里带中文注释的 `.cs`**，把它的内容转成 GBK 放进临时
/// 工作区再打开——内容是真实的，编码是构造的，不往测试项目里写任何东西。
#[gpui::test]
fn acceptance_scenario_6_real_chinese_source_as_gbk(cx: &mut TestAppContext) {
    let Some(root) = UnityProject::Medium.path() else {
        return;
    };
    // 带中文的真实源码文件。
    let chinese = find_file(&root, 20_000, true, |path, meta| {
        if meta.len() == 0 || meta.len() > 256 * 1024 {
            return false;
        }
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cs"))
        {
            return false;
        }
        std::fs::read_to_string(path)
            .is_ok_and(|text| text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)))
    });
    let Some(chinese) = chinese else {
        eprintln!("已跳过场景 6：中型 Unity 项目的前 2 万个文件里没有带中文的 .cs");
        return;
    };
    let original = std::fs::read_to_string(&chinese).unwrap();
    let (gbk, _, unmappable) = encoding_rs::GBK.encode(&original);
    if unmappable {
        eprintln!(
            "已跳过场景 6：{} 里有 GBK 表示不了的字符",
            chinese.display()
        );
        return;
    }
    assert!(
        String::from_utf8(gbk.to_vec()).is_err(),
        "转码后应当不是合法 UTF-8"
    );

    let user = tempfile::tempdir().unwrap();
    std::fs::write(user.path().join("settings.json"), "{}").unwrap();
    let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
    options.watch = None;
    let service = Arc::new(SettingsService::open(
        options,
        crate::app_settings::registry().unwrap(),
    ));
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("Assets")).unwrap();
    let file = folder.path().join("Assets/中文注释.cs");
    std::fs::write(&file, gbk.as_ref()).unwrap();
    let before = std::fs::read(&file).unwrap();

    let settings: Arc<dyn Settings> = service.clone();
    let root_path: PathBuf = folder.path().into();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_settings(
            root_path,
            Instant::now(),
            LayoutStore::disabled(),
            settings,
            cx,
        )
        .unwrap()
    });
    settle(cx);
    open_file(&view, cx, "Assets/中文注释.cs", OpenOptions::default());

    let encoding = status_text(&view, cx, status_ids::ENCODING);
    let tooltip = status_tooltip(&view, cx, status_ids::ENCODING);
    eprintln!(
        "[#31 场景 6] 真实中文源码（{}，{} 字节）转成 GBK 后识别为「{encoding}」，提示「{tooltip}」",
        relative(&root, &chinese).unwrap_or_default(),
        gbk.len()
    );
    assert_eq!(encoding, "GBK", "带中文注释的 GBK 文件应当自动识别为 GBK");
    assert!(tooltip.contains("自动识别"), "编码提示：{tooltip}");

    // 解码结果与原文一致：没有替换字符，也没有乱码。
    let text = view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .and_then(Tab::opened)
            .map(|o| document_text(&*o.document.text))
            .unwrap_or_default()
    });
    assert!(!text.contains('\u{fffd}'), "解码结果里出现了替换字符");
    assert_eq!(
        text.replace("\r\n", "\n"),
        original.replace("\r\n", "\n"),
        "GBK 解码后的正文与原文不一致"
    );
    // 只读浏览不改磁盘字节。
    assert_eq!(std::fs::read(&file).unwrap(), before, "文件字节被改动了");
}

/// 场景 7（混合换行符）：打开**真实项目里换行符混用**的文件，状态栏显示「混合」，
/// 浏览后磁盘字节一个也没变。
#[gpui::test]
fn acceptance_scenario_7_real_mixed_line_endings(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(UnityProject::Medium, "{}") else {
        return;
    };
    let root = fx.root.clone();
    let mixed = find_file(&root, 20_000, true, |path, meta| {
        if meta.len() == 0 || meta.len() > 512 * 1024 {
            return false;
        }
        let ext = path.extension().unwrap_or_default().to_string_lossy();
        if !["cs", "txt", "md", "json", "xml", "shader"].contains(&&*ext.to_ascii_lowercase()) {
            return false;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let crlf = text.matches("\r\n").count();
        let lf = text.matches('\n').count() - crlf;
        crlf > 0 && lf > 0
    });
    let Some(path) = mixed else {
        eprintln!("已跳过场景 7：中型 Unity 项目的前 2 万个文件里没有混合换行符的文件");
        return;
    };
    let before = std::fs::read(&path).unwrap();
    let rel = relative(&root, &path).unwrap();

    let (view, cx) = fx.window(cx);
    // `.md` 默认进预览（场景 10），预览不是文本编辑器、没有换行符条目；这里要看的是源码一侧，
    // 所以显式按源码打开。
    open_file(
        &view,
        cx,
        &rel,
        OpenOptions {
            presentation: crate::documents::Presentation::Source,
            preview_tab: false,
            ..OpenOptions::default()
        },
    );
    let eol = status_text(&view, cx, status_ids::EOL);
    eprintln!("[#31 场景 7] 真实混合换行符文件（{rel}）状态栏显示「{eol}」");
    assert_eq!(eol, "混合", "混合换行符的文件状态栏要显示「混合」");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "{rel} 的字节被改动了"
    );
}

// ---------------------------------------------------------------- 场景 10 / 11

/// 场景 10 的来源语义与场景 11 的界面一侧，都在真实项目上：
///
/// - 真实 `.md` 从文件树打开是**预览**，`Ctrl+Shift+V` 切源码；
/// - 同一个 `.md` 从**搜索结果**打开是**源码**（来源语义，最容易在接缝处丢）；
/// - `Library/PackageCache/**` 下的真实文件打开后带锁，状态栏「只读」且有原因。
#[gpui::test]
fn acceptance_scenario_10_11_markdown_source_semantics_and_package_cache(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(UnityProject::Medium, "{}") else {
        return;
    };
    let root = fx.root.clone();
    let markdown = find_file(&root, 20_000, true, |path, meta| {
        meta.len() > 0
            && meta.len() < 256 * 1024
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
    });
    let package = find_file(
        &root.join("Library/PackageCache"),
        4_000,
        false,
        |path, meta| {
            meta.len() > 0
                && meta.len() < 256 * 1024
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("cs"))
        },
    );

    let (view, cx) = fx.window(cx);

    if let Some(path) = &markdown {
        let rel = relative(&root, path).unwrap();
        // 文件树（Explorer）来源：默认进预览（UX_SPEC D6）。
        open_file(
            &view,
            cx,
            &rel,
            OpenOptions {
                source: OpenSource::Explorer,
                ..OpenOptions::default()
            },
        );
        let preview = view.read_with(cx, |v, _| {
            v.editor
                .active()
                .current()
                .map(|t| is_markdown_preview(&t.kind))
        });
        assert_eq!(preview, Some(true), "{rel} 从文件树打开应当进预览");

        // 搜索结果来源：同一个文件要打开**源码**并定位。
        open_file(
            &view,
            cx,
            &rel,
            OpenOptions {
                source: OpenSource::Search,
                location: Some(Position {
                    line: Line(0),
                    column: Utf8Column(0),
                }),
                ..OpenOptions::default()
            },
        );
        let source = view.read_with(cx, |v, _| {
            v.editor
                .groups
                .values()
                .any(|g| g.current().is_some_and(|t| !is_markdown_preview(&t.kind)))
        });
        assert!(source, "{rel} 从搜索结果打开应当是源码而不是预览");
        eprintln!("[#31 场景 10] 真实 .md（{rel}）：文件树 → 预览，搜索结果 → 源码");
    } else {
        eprintln!("已跳过场景 10：中型 Unity 项目里没有找到 .md");
    }

    if let Some(path) = &package {
        let rel = relative(&root, path).unwrap();
        open_file(
            &view,
            cx,
            &rel,
            OpenOptions {
                preview_tab: false,
                ..OpenOptions::default()
            },
        );
        let reasons = view.read_with(cx, |v, _| {
            v.editor
                .active()
                .current()
                .and_then(Tab::opened)
                .map(|o| o.document.metadata.read_only.to_vec())
                .unwrap_or_default()
        });
        assert!(
            reasons
                .iter()
                .any(|r| matches!(r, rove_buffer::ReadOnlyReason::Rule(_))),
            "{rel}（包缓存）应当是只读的，实际 {reasons:?}"
        );
        assert_eq!(status_text(&view, cx, status_ids::READ_ONLY), "只读");
        let tooltip = status_tooltip(&view, cx, status_ids::READ_ONLY);
        assert!(tooltip.contains("包缓存"), "只读原因提示：{tooltip}");
        eprintln!("[#31 场景 11] 真实包缓存文件（{rel}）：锁形标记 + 只读原因「{tooltip}」");
    } else {
        eprintln!("已跳过场景 11：中型 Unity 项目的 Library/PackageCache 下没有找到 .cs");
    }
}

// ---------------------------------------------------------------- 场景 3

/// 场景 3：真实项目上的快速打开（`Ctrl+P`）——逐键有结果、选中后文档打开并定位，
/// 每次按键的耗时打到日志（预算判定在 `cargo xtask bench` 的 `quickopen/first_key_large`）。
#[gpui::test]
fn acceptance_scenario_3_quick_open_on_real_project(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(UnityProject::Medium, "{}") else {
        return;
    };
    let (view, cx) = fx.window(cx);

    cx.simulate_keystrokes("ctrl-p");
    settle(cx);
    let mut per_key = Vec::new();
    for ch in "Player".chars() {
        let started = Instant::now();
        cx.simulate_input(&ch.to_string());
        settle(cx);
        per_key.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let rows = view.read_with(cx, |v, cx| {
        v.quick
            .view
            .as_ref()
            .and_then(|q| q.read(cx).service.active().map(|a| a.list.len()))
            .unwrap_or(0)
    });
    eprintln!(
        "[#31 场景 3] 中型项目快速打开「Player」：{rows} 行，逐键（含一次完整重绘）{:?} ms（机器负载较高）",
        per_key
            .iter()
            .map(|ms| format!("{ms:.1}"))
            .collect::<Vec<_>>()
    );
    assert!(rows > 0, "真实项目里 Player 应当有匹配的文件");

    // 回车打开第一条：标签页上的资源与列表首行一致。
    cx.simulate_keystrokes("enter");
    settle(cx);
    let opened = view.read_with(cx, |v, _| {
        v.editor.active().current().map(|t| match &t.resource {
            Resource::Workspace(p) => p.as_str().to_owned(),
            other => format!("{other:?}"),
        })
    });
    assert!(opened.is_some(), "快速打开没有打开任何文件");
    let opened = opened.unwrap();
    eprintln!("[#31 场景 3] 打开了 {opened}");

    // 真实项目里快速打开会列出 `.png` 之类的二进制文件（VS Code 也列）。打开它要走
    // 「不受支持」的错误页而不是一屏乱码；M1 没有图片预览（#63 是 M4）。
    if opened.ends_with(".png") {
        let failed = view.read_with(cx, |v, _| {
            v.editor
                .active()
                .current()
                .is_some_and(|t| matches!(&t.content, Content::Failed(Error::Unsupported)))
        });
        assert!(
            failed,
            "二进制文件应当以「不受支持」结束，而不是当成文本显示"
        );
        eprintln!(
            "[#31 场景 3] 二进制文件（{opened}）显示「无法打开编辑器：文件格式或编码不受支持。」\
             错误页上只有「重试」与「在资源管理器中显示」，没有 UX_SPEC §3.1 要求的「仍然打开」"
        );
    }
}

// ---------------------------------------------------------------- 场景 12

/// 场景 12：真实项目里的五种文件类型（C# / JSON / YAML / XML / Markdown）都要有大纲与面包屑，
/// 不支持的类型显示标准空状态（`Status::Unsupported`）。
#[gpui::test]
fn acceptance_scenario_12_outline_for_five_real_file_types(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(UnityProject::Medium, "{}") else {
        return;
    };
    let root = fx.root.clone();
    let pick = |exts: &[&str]| -> Option<PathBuf> {
        let exts: Vec<String> = exts.iter().map(|e| e.to_ascii_lowercase()).collect();
        find_file(&root, 20_000, true, |path, meta| {
            meta.len() > 64
                && meta.len() < 256 * 1024
                && path
                    .extension()
                    .is_some_and(|e| exts.contains(&e.to_string_lossy().to_ascii_lowercase()))
                && std::fs::read_to_string(path).is_ok()
        })
    };
    let samples = [
        ("C#", pick(&["cs"])),
        ("JSON", pick(&["json"])),
        ("YAML", pick(&["yaml", "yml"])),
        ("XML", pick(&["xml"])),
        ("Markdown", pick(&["md"])),
    ];

    let (view, cx) = fx.window(cx);
    for (label, path) in &samples {
        let Some(path) = path else {
            eprintln!("已跳过场景 12 的 {label}：中型 Unity 项目里没有找到这种文件");
            continue;
        };
        let rel = relative(&root, path).unwrap();
        open_file(
            &view,
            cx,
            &rel,
            OpenOptions {
                presentation: crate::documents::Presentation::Source,
                preview_tab: false,
                ..OpenOptions::default()
            },
        );
        let (status, rows, crumbs) = view.read_with(cx, |v, cx| {
            let w = &v.outline.list.read(cx).widget;
            (
                v.outline.status,
                w.len(),
                v.group_segments(v.editor.focused)
                    .map(|(_, _, segments)| segments.len())
                    .unwrap_or(0),
            )
        });
        eprintln!(
            "[#31 场景 12] {label}（{rel}）：大纲 {status:?}，{rows} 个节点，面包屑 {crumbs} 段"
        );
        assert!(
            matches!(status, crate::outline::view::Status::Ready),
            "{label}（{rel}）的大纲状态：{status:?}"
        );
        assert!(rows > 0, "{label}（{rel}）的大纲是空的");
        assert!(crumbs > 0, "{label}（{rel}）没有面包屑");
    }
}

// ---------------------------------------------------------------- 场景 4

/// 场景 4：真实项目上的全局搜索——常见标识符有结果，没有结果时空状态带
/// 「已排除 Unity 生成的目录。」；`.gitignore` 不影响结果（见场景 2）。
#[gpui::test]
fn acceptance_scenario_4_search_on_real_project(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(UnityProject::Medium, "{}") else {
        return;
    };
    let (view, cx) = fx.window(cx);

    let search = |query: &str, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("ctrl-shift-f");
        settle(cx);
        cx.update(|window, cx| {
            let q = view.read(cx).search.read(cx).query.focus_handle(cx);
            window.focus(&q);
        });
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input(query);
        cx.simulate_keystrokes("enter");
        let started = Instant::now();
        for _ in 0..600 {
            cx.run_until_parked();
            let done = view.read_with(cx, |v, cx| {
                *v.search.read(cx).state() != SearchState::Searching
            });
            if done {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        settle(cx);
        started.elapsed().as_secs_f64() * 1000.0
    };

    let ms = search("GetComponent", cx);
    let rows = view.read_with(cx, |v, cx| {
        let rows = v.search.read(cx).rows();
        crate::lists::ListModel::rows(&*rows, 0..rows.len())
            .unwrap()
            .len()
    });
    eprintln!(
        "[#31 场景 4] 中型项目搜索 GetComponent：{rows} 行结果，界面侧端到端 {ms:.0} ms \
         （引擎侧预算判定见 search/first_batch_large 与 search/full_large；机器负载较高）"
    );
    assert!(rows > 0, "真实项目里 GetComponent 应当有结果");

    // 没有结果时的空状态（UX_SPEC §6.3）：Rove 的默认排除影响了结果 → 额外提示
    //「已排除 Unity 生成的目录。」与「包含排除的文件再搜索一次」。
    search("ZzQqX不存在的标识符ZzQqX", cx);
    let state = view.read_with(cx, |v, cx| v.search.read(cx).state().clone());
    let SearchState::Done(summary) = state else {
        panic!("搜索没有正常结束：{state:?}");
    };
    assert!(
        summary.excluded_by_rove_defaults,
        "真实 Unity 项目里 Rove 的默认排除一定影响了结果，空状态要给出排除提示"
    );
    // 文案本身走 #1 的字符串登记表（不写死在视图里）。
    let excluded_hint = crate::search::registry::STRINGS
        .iter()
        .find(|(key, _)| *key == "search.roveExcluded")
        .map(|(_, text)| *text)
        .expect("空状态的排除提示没有登记");
    assert!(
        excluded_hint.contains("已排除 Unity 生成的目录"),
        "空状态的排除提示文案：{excluded_hint}"
    );
    eprintln!("[#31 场景 4] 空状态提示：{excluded_hint}");
}

// ---------------------------------------------------------------- 场景 13

/// 场景 13：共用 `settings.json` 被外部（VS Code）改掉后 Rove 实时生效——主题、字号与
/// `files.exclude` 三件事都在真实项目上验证。
#[gpui::test]
fn acceptance_scenario_13_shared_settings_hot_reload(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(
        UnityProject::Medium,
        r#"{ "workbench.colorTheme": "Dark 2026" }"#,
    ) else {
        return;
    };
    let (view, cx) = fx.window(cx);

    let kind = view.read_with(cx, |v, _| v.theme_service.current().kind());
    assert!(
        matches!(kind, ThemeKind::Dark),
        "初始应当是深色主题：{kind:?}"
    );

    // ① 外部切浅色主题。
    fx.change(r#"{ "workbench.colorTheme": "Light 2026" }"#, cx);
    let kind = view.read_with(cx, |v, _| v.theme_service.current().kind());
    assert!(
        matches!(kind, ThemeKind::Light),
        "外部改 workbench.colorTheme 后应当实时切成浅色：{kind:?}"
    );

    // ② 外部改字号。
    fx.change(
        r#"{ "workbench.colorTheme": "Light 2026", "editor.fontSize": 22 }"#,
        cx,
    );
    let size = view.read_with(cx, |v, cx| v.code_view().read(cx).font_size());
    assert_eq!(size, 22.0, "外部改 editor.fontSize 后应当实时生效");

    // ③ 外部改 files.exclude：把 Assets 排除掉，文件树里就不该再有它。
    let labels = |cx: &mut VisualTestContext| -> Vec<String> {
        view.read_with(cx, |v, cx| {
            v.explorer
                .read(cx)
                .rows(cx)
                .into_iter()
                .map(|(_, label, _)| label.to_string())
                .collect()
        })
    };
    let before = labels(cx);
    assert!(
        before.iter().any(|r| r == "Assets"),
        "真实 Unity 项目的文件树里应当有 Assets：{before:?}"
    );
    fx.change(
        r#"{ "workbench.colorTheme": "Light 2026", "editor.fontSize": 22,
             "files.exclude": { "**/Assets": true } }"#,
        cx,
    );
    let after = labels(cx);
    assert!(
        !after.iter().any(|r| r == "Assets"),
        "外部改 files.exclude 后 Assets 仍然在树里：{after:?}"
    );
    eprintln!(
        "[#31 场景 13] 共用 settings.json 外部改动实时生效：主题、字号、files.exclude \
         （文件树 {} 行 → {} 行）",
        before.len(),
        after.len()
    );
}

// ---------------------------------------------------------------- 场景 16

/// 场景 16：命令面板只列**已实现**的命令（UX_SPEC §1.7：未实现的不显示、不置灰）。
///
/// 各 issue 的测试用的是自己装配的小命令表；这里用真实项目上**完整装配过的窗口**，
/// 把「已登记但未实现」的命令（例如 M4 才做的上下文篮子）逐条拿去比对面板的行。
#[gpui::test]
fn acceptance_scenario_16_command_palette_lists_only_implemented(cx: &mut TestAppContext) {
    let Some(fx) = RealProject::open(UnityProject::Medium, "{}") else {
        return;
    };
    let (view, cx) = fx.window(cx);

    // 已登记但未实现的命令（标题经字符串表解析，与面板显示的是同一份文字）。
    let pending: Vec<(String, String)> = view.update(cx, |v, cx| {
        v.commands
            .registry()
            .entries()
            .filter(|(id, _)| !crate::keys::is_implemented(id, cx))
            .map(|(id, command)| (id.to_owned(), v.label(&command.title_key, &[])))
            .collect()
    });
    assert!(
        !pending.is_empty(),
        "M1 阶段应当确实有「已登记但未实现」的命令（M2–M4 的功能），否则这条断言没有意义"
    );

    cx.simulate_keystrokes("ctrl-shift-p");
    settle(cx);
    let rows: Vec<String> = view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref().expect("命令面板已创建").read(cx);
        let active = q.service.active().expect("命令面板已打开");
        active
            .list
            .display_rows(0..active.list.len())
            .iter()
            .map(|r| r.label.to_string())
            .collect()
    });
    assert!(
        rows.len() > 20,
        "命令面板应当列出已实现的命令：{}",
        rows.len()
    );
    for (id, title) in &pending {
        assert!(
            !rows.contains(title),
            "未实现的命令 {id}（「{title}」）出现在了命令面板里"
        );
    }
    eprintln!(
        "[#31 场景 16] 命令面板列出 {} 行；{} 条「已登记但未实现」的命令一条都没出现",
        rows.len(),
        pending.len()
    );
}
