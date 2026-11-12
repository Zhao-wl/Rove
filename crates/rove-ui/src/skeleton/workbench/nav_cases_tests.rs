//! #40 验收：用 #39 的导航用例（`tests/nav-cases/medium-*.toml`）在**界面**里抽查 `F12`：真实 GPUI 工作台打开
//! 真实 Unity 项目的文件，插入符放到用例的查询点，经键位服务按 `f12`，读回落点（单个候选：活动标签与插入符；
//! 多个候选：交给速览的请求；外部符号：状态栏提示），与用例的期望定义（文件 + 行）比较。
//!
//! 需要本地环境，CI 上不跑：
//! - `ROVE_TEST_UNITY_MEDIUM`（项目根目录，路径不写进仓库）；
//! - 中型项目的 L2 索引：`ROVE_NAV_INDEX_MEDIUM`，或 `target/nav-check/medium.idx`
//!   （`cargo run --release -p rove-lang-cs --example nav_check -- --build --project medium` 生成）。
//!
//! ```text
//! cargo test -p rove-ui ui_nav_cases -- --ignored --nocapture
//! ```
//!
//! 输出逐例结果表（stdout），并写到 `target/nav-check/ui-nav-cases-medium.md`。
use super::*;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_lang_cs::router::{ManualSemantic, SemanticSource};
use std::path::Path;

/// 抽查的分类（issue #40 验收：var、重载、扩展方法、泛型、partial、接口实现、Unity API）。
const CATEGORIES: &[&str] = &[
    "var",
    "overload",
    "extension",
    "generic",
    "partial",
    "interface",
    "external",
];

#[derive(Clone, Debug, Default)]
struct Case {
    id: String,
    category: String,
    file: String,
    line: u64,
    col: u64,
    token: String,
    defs: Vec<(String, u64)>,
    external: Option<String>,
}

/// 解析用例文件里本测试用到的字段（格式见 `tests/nav-cases/README.md`；只读这几个键，不依赖 toml 库）。
fn parse_cases(text: &str) -> Vec<Case> {
    fn value(line: &str, key: &str) -> Option<String> {
        let rest = line
            .trim()
            .strip_prefix(key)?
            .trim_start()
            .strip_prefix('=')?
            .trim();
        Some(rest.trim_matches(|c| c == '"' || c == '\'').to_string())
    }
    fn field<'a>(s: &'a str, key: &str) -> Option<&'a str> {
        let at = s.find(&format!("{key} = "))? + key.len() + 3;
        let rest = &s[at..];
        if let Some(q) = rest.strip_prefix('\'').or_else(|| rest.strip_prefix('"')) {
            let end = q.find(['\'', '"'])?;
            Some(&q[..end])
        } else {
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            Some(rest[..end].trim())
        }
    }
    let mut cases = Vec::new();
    let mut current: Option<Case> = None;
    let mut in_expect = false;
    // `defs = [` 跨多行时，逐行累积到 `]`。
    let mut defs_open = false;
    for line in text.lines() {
        let t = line.trim();
        if defs_open {
            if let Some(c) = current.as_mut() {
                if let (Some(f), Some(l)) = (field(t, "file"), field(t, "line")) {
                    c.defs.push((f.to_string(), l.parse().unwrap_or(0)));
                }
            }
            defs_open = !t.starts_with(']');
            continue;
        }
        if t == "[[case]]" {
            cases.extend(current.take());
            current = Some(Case::default());
            in_expect = false;
            continue;
        }
        if t == "[case.expect]" {
            in_expect = true;
            continue;
        }
        let Some(c) = current.as_mut() else { continue };
        if !in_expect {
            if let Some(v) = value(t, "id") {
                c.id = v;
            } else if let Some(v) = value(t, "category") {
                c.category = v;
            } else if let Some(v) = value(t, "file") {
                c.file = v;
            } else if let Some(v) = value(t, "line") {
                c.line = v.parse().unwrap_or(0);
            } else if let Some(v) = value(t, "col") {
                c.col = v.parse().unwrap_or(0);
            } else if let Some(v) = value(t, "token") {
                c.token = v;
            }
        } else if t.starts_with("defs") {
            defs_open = t.ends_with('[');
            for item in t.split('{').skip(1) {
                if let (Some(f), Some(l)) = (field(item, "file"), field(item, "line")) {
                    c.defs.push((f.to_string(), l.parse().unwrap_or(0)));
                }
            }
        } else if let Some(v) = value(t, "external") {
            c.external = Some(v);
        }
    }
    cases.extend(current);
    cases
}

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

/// 用例的列是 1 起算的字符列（制表符算 1 列）→ UTF-8 字节列。
fn byte_column(line: &str, col: u64) -> u64 {
    line.char_indices()
        .nth(col.saturating_sub(1) as usize)
        .map_or(line.len(), |(i, _)| i) as u64
}

#[gpui::test]
#[ignore = "需要 ROVE_TEST_UNITY_MEDIUM 与中型项目的 L2 索引；见模块文档"]
fn ui_nav_cases_medium(cx: &mut TestAppContext) {
    let Some(root) = std::env::var_os("ROVE_TEST_UNITY_MEDIUM").map(PathBuf::from) else {
        eprintln!("已跳过：未设置 ROVE_TEST_UNITY_MEDIUM");
        return;
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index_path = std::env::var_os("ROVE_NAV_INDEX_MEDIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/nav-check/medium.idx"));
    if !root.is_dir() || !index_path.is_file() {
        eprintln!(
            "已跳过：项目或索引不存在（{}，{}）",
            root.display(),
            index_path.display()
        );
        return;
    }
    let index = Arc::new(rove_lang_cs::CsIndex::open(&index_path).expect("open index"));
    let semantic: Arc<dyn SemanticSource> = Arc::new(ManualSemantic::ready(index));
    let mut cases = Vec::new();
    for file in ["medium-a.toml", "medium-b.toml"] {
        let text = std::fs::read_to_string(repo.join("tests/nav-cases").join(file)).unwrap();
        cases.extend(parse_cases(&text));
    }
    cases.retain(|c| CATEGORIES.contains(&c.category.as_str()));
    // 每个分类最多 5 个（外部符号只取 Unity API）。
    let mut picked: Vec<Case> = Vec::new();
    for cat in CATEGORIES {
        picked.extend(
            cases
                .iter()
                .filter(|c| c.category == *cat)
                .filter(|c| {
                    c.category != "external"
                        || c.external
                            .as_deref()
                            .is_some_and(|e| e.contains("UnityEngine"))
                })
                .take(5)
                .cloned(),
        );
    }
    let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root.clone(), Instant::now(), cx));
    cx.run_until_parked();
    view.update(cx, |v, cx| v.attach_semantic(Some(semantic), cx));
    let mut rows = Vec::new();
    let mut passed = 0;
    for case in &picked {
        let resource = Resource::Workspace(WorkspacePath::new(&case.file).unwrap());
        view.update(cx, |v, cx| {
            v.nav.last_peek = None;
            v.nav.symbols = None;
            for key in ["nav.status", "nav.symbols"] {
                let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, key);
            }
            v.shell.feedback.clear_cursor_hint();
            let _ = v.open_documents().open(
                resource.clone(),
                OpenOptions::default(),
                Cancellation::default(),
            );
            cx.notify();
        });
        cx.run_until_parked();
        refresh(cx);
        refresh(cx);
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
        let line_text = code.read_with(cx, |c, _| {
            c.text()
                .and_then(|t| crate::code_view::columns::read_line(&**t, case.line - 1))
                .unwrap_or_default()
        });
        let p = Position {
            line: Line(case.line - 1),
            column: Utf8Column(byte_column(&line_text, case.col)),
        };
        code.update(cx, |c, cx| {
            c.set_selection(
                code_view::Selection::caret(p),
                Some(Reveal::CenterIfOutside),
                cx,
            )
        });
        refresh(cx);
        cx.simulate_keystrokes("f12");
        cx.run_until_parked();
        refresh(cx);
        refresh(cx);
        let (landed, peek, status, hint) = view.read_with(cx, |v, cx| {
            let path = match &v.editor.active().current().unwrap().resource {
                Resource::Workspace(p) => p.as_str().to_string(),
                other => format!("{other:?}"),
            };
            let line = v.code_view().read(cx).selection().active.line.0 + 1;
            let peek = v.nav.last_peek.as_ref().map(|r| {
                r.targets
                    .iter()
                    .map(|t| match &t.resource {
                        Resource::Workspace(p) => {
                            (p.as_str().to_string(), t.selection.start().line.0 + 1)
                        }
                        other => (format!("{other:?}"), 0),
                    })
                    .collect::<Vec<_>>()
            });
            let status = shell::view::status_message_text(v);
            let hint = v
                .shell
                .feedback
                .cursor_hint()
                .map(|h| h.message.key.to_string());
            ((path, line), peek, status, hint)
        });
        let (ok, got) = if let Some(ext) = &case.external {
            // #42 合并前：外部符号在状态栏提示“未找到 `X` 的元数据”，不打开空标签。
            let ok =
                status.as_deref().is_some_and(|s| s.contains("元数据")) && landed.0 == case.file;
            (
                ok,
                format!("外部 {ext} → {}", status.clone().unwrap_or_default()),
            )
        } else if let Some(targets) = peek {
            let mut want = case.defs.clone();
            let mut got = targets.clone();
            want.sort();
            got.sort();
            (want == got, format!("速览 {} 个候选：{got:?}", got.len()))
        } else {
            let ok = case.defs.len() == 1 && case.defs[0] == landed;
            (ok, format!("{}:{}", landed.0, landed.1))
        };
        let got = match (&hint, ok) {
            (Some(h), false) => format!("{got}（光标提示 {h}）"),
            _ => got,
        };
        passed += usize::from(ok);
        rows.push(format!(
            "| {} | {} | `{}` | {} | {} |",
            case.id,
            case.category,
            case.token,
            if ok { "通过" } else { "失败" },
            got
        ));
        // 回到干净状态：关闭符号导航模式与速览请求。
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
    }
    let report = format!(
        "# 界面 F12 抽查（中型项目，#40）\n\n通过 {passed} / {}\n\n| 用例 | 分类 | 标识符 | 结果 | 落点 |\n|---|---|---|---|---|\n{}\n",
        picked.len(),
        rows.join("\n")
    );
    println!("{report}");
    let out = repo.join("target/nav-check");
    let _ = std::fs::create_dir_all(&out);
    let _ = std::fs::write(out.join("ui-nav-cases-medium.md"), &report);
    assert!(picked.len() >= 20, "抽查不足 20 个：{}", picked.len());
}

/// 用例解析器能读出仓库里的全部中型用例（不需要 Unity 项目）。
#[test]
fn nav_case_files_parse() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut all = Vec::new();
    for file in ["medium-a.toml", "medium-b.toml"] {
        let text = std::fs::read_to_string(repo.join("tests/nav-cases").join(file)).unwrap();
        all.extend(parse_cases(&text));
    }
    assert_eq!(all.len(), 100);
    for c in &all {
        assert!(
            !c.id.is_empty() && !c.file.is_empty() && c.line > 0 && c.col > 0,
            "{c:?}"
        );
        assert!(!c.defs.is_empty() || c.external.is_some(), "{c:?}");
    }
    let picked = CATEGORIES
        .iter()
        .map(|cat| all.iter().filter(|c| c.category == *cat).count().min(5))
        .sum::<usize>();
    assert!(picked >= 20, "{picked}");
    assert_eq!(
        byte_column("\tvar x = 中文.Run();", 13),
        "\tvar x = 中文.".len() as u64
    );
}
