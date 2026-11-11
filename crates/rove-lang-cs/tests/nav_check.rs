//! nav_check 运行器（#39）的集成测试。
//!
//! 运行器是 example，这里用 `#[path]` 直接引入它的模块：
//! - 锚点重定位：在用例所在文件上方插入几行后仍能定位到正确位置；
//! - 锚点失效标为“过期”，计入统计但不计为失败；
//! - `tests/nav-cases/*.toml` 的静态校验（CI 上也跑）；
//! - 设置了 `ROVE_TEST_UNITY_*` 时，检查用例锚点在真实项目上仍然有效。
#![allow(dead_code)]

#[path = "../examples/nav_check/adapter.rs"]
mod adapter;
#[path = "../examples/nav_check/cases.rs"]
mod cases;
#[path = "../examples/nav_check/check.rs"]
mod check;
#[path = "../examples/nav_check/locate.rs"]
mod locate;
#[path = "../examples/nav_check/report.rs"]
mod report;

use adapter::{DefAnswer, NavSource, QueryPos, RefsAnswer, SourceError, SrcLine};
use cases::{parse_case_file, Category, Project};
use check::{run_case, Status, Workspace};
use locate::{locate_line, locate_token, SourceText};
use std::path::{Path, PathBuf};

const PLAYER: &str = "\
namespace Demo
{
    public class Player
    {
        public void Move() { }
    }

    public class Game
    {
        Player GetPlayer() => new Player();

        void Tick()
        {
            var p = GetPlayer();
            p.Move();
        }
    }
}
";

const CASES: &str = r#"
project = "medium"

[[case]]
id = "t-var-01"
category = "var"
file = "Assets/Game.cs"
line = 15
col = 15
token = "Move"
anchor = "p.Move();"
why = "var 推断出 Player"
source = "人工阅读：GetPlayer() 返回 Player"
[case.expect]
defs = [{ file = "Assets/Game.cs", line = 5, anchor = "public void Move() { }" }]
refs = 1
refs_include = [{ file = "Assets/Game.cs", line = 15, anchor = "p.Move();" }]
precision = "exact"
"#;

/// 只认 `p.Move()` 这一个点的假查询来源：定义在 `public void Move()` 那一行。
/// 它不读用例，而是自己在源码里找行号，这样插行后仍然给出“真实”的答案。
struct FakeSource {
    root: PathBuf,
}

impl FakeSource {
    fn find(&self, needle: &str) -> u32 {
        let text = std::fs::read_to_string(self.root.join("Assets/Game.cs")).unwrap();
        text.lines().position(|l| l.trim() == needle).unwrap() as u32
    }
}

impl NavSource for FakeSource {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn definition(&self, at: &QueryPos<'_>) -> Result<DefAnswer, SourceError> {
        let call = self.find("p.Move();");
        let mut a = DefAnswer {
            precision: Some("exact"),
            ..DefAnswer::default()
        };
        if at.file == "Assets/Game.cs" && at.line0 == call && at.utf8_col0 == 14 {
            a.defs.push(SrcLine {
                file: at.file.to_owned(),
                line0: self.find("public void Move() { }"),
            });
        }
        Ok(a)
    }
    fn references(&self, _: &QueryPos<'_>) -> Result<RefsAnswer, SourceError> {
        Ok(RefsAnswer {
            refs: vec![SrcLine {
                file: "Assets/Game.cs".to_owned(),
                line0: self.find("p.Move();"),
            }],
            precision: Some("exact"),
        })
    }
}

fn temp_project(name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rove-nav-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    std::fs::write(dir.join("Assets/Game.cs"), source).unwrap();
    dir
}

fn run_one(root: &Path) -> check::CaseResult {
    let file = parse_case_file(Path::new("t.toml"), CASES).unwrap();
    assert!(cases::lint_case(&file.cases[0]).is_empty());
    let mut ws = Workspace::new(root);
    let src = FakeSource {
        root: root.to_owned(),
    };
    run_case(
        "medium",
        &file.cases[0],
        &mut ws,
        &src,
        locate::DEFAULT_WINDOW,
    )
}

#[test]
fn case_passes_at_original_position() {
    let root = temp_project("orig", PLAYER);
    let r = run_one(&root);
    assert_eq!(r.status, Status::Pass, "{:?}", r.reasons);
    assert!(!r.relocated);
    assert_eq!(r.query.as_deref(), Some("Assets/Game.cs:15:15"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn anchors_relocate_after_lines_are_inserted_above() {
    // 人为在文件上方插入几行：查询点与期望的定义位置都下移 4 行。
    let shifted = format!("// a\n// b\nusing System;\n\n{PLAYER}");
    let root = temp_project("shift", &shifted);
    let r = run_one(&root);
    assert_eq!(r.status, Status::Pass, "{:?}", r.reasons);
    assert!(r.relocated);
    assert_eq!(r.query.as_deref(), Some("Assets/Game.cs:19:15"));
    assert_eq!(r.expected_defs, vec!["Assets/Game.cs:9".to_owned()]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn relocation_keeps_column_when_indentation_changes() {
    let text = SourceText::from_text("x\n\n\t\t\tp.Move();\n");
    let hit = locate_token(&text, 1, 15, "Move", "p.Move();", 10).unwrap();
    assert_eq!((hit.line0, hit.char_col0, hit.utf8_col0), (2, 5, 5));
    assert!(hit.moved);
}

#[test]
fn ambiguous_or_missing_anchor_is_an_error() {
    let text = SourceText::from_text("a();\nb();\na();\n");
    assert!(
        locate_line(&text, 2, "a();", 5).is_err(),
        "上下等距各一处 → 有歧义"
    );
    assert!(locate_line(&text, 2, "c();", 5).is_err());
    assert_eq!(locate_line(&text, 2, "b();", 5).unwrap().line0, 1);
}

#[test]
fn wrong_column_on_unchanged_line_is_reported() {
    let text = SourceText::from_text("p.Move();\n");
    assert!(locate_token(&text, 1, 1, "Move", "p.Move();", 5).is_err());
    // UTF-8 列：中文字符占 3 字节
    let text = SourceText::from_text("var 名 = p.Move();\n");
    let hit = locate_token(&text, 1, 11, "Move", "var 名 = p.Move();", 5).unwrap();
    assert_eq!((hit.char_col0, hit.utf8_col0), (10, 12));
}

#[test]
fn broken_anchor_is_stale_not_failure() {
    let changed = PLAYER.replace("p.Move();", "p.Run();");
    let root = temp_project("stale", &changed);
    let r = run_one(&root);
    assert_eq!(r.status, Status::Stale);
    assert!(r.reasons[0].contains("锚点"), "{:?}", r.reasons);
    let report = report::build("fake", Vec::new(), Vec::new(), vec![r]);
    let row = &report.summary[0];
    assert_eq!((row.tally.stale, row.tally.fail), (1, 0));
    assert_eq!(row.tally.accuracy(), None, "过期不计入准确率分母");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn wrong_answer_is_failure_with_details() {
    // 期望的定义写成 Tick：查询结果不符 → 失败，原因里有期望与实际。
    let root = temp_project("fail", PLAYER);
    let text = CASES.replace(
        r#"line = 5, anchor = "public void Move() { }""#,
        r#"line = 12, anchor = "void Tick()""#,
    );
    let file = parse_case_file(Path::new("t.toml"), &text).unwrap();
    let mut ws = Workspace::new(&root);
    let src = FakeSource { root: root.clone() };
    let r = run_case("medium", &file.cases[0], &mut ws, &src, 40);
    assert_eq!(r.status, Status::Fail);
    assert!(
        r.reasons[0].contains("Assets/Game.cs:12") && r.reasons[0].contains("Assets/Game.cs:5")
    );
    let prev = report::build("fake", Vec::new(), Vec::new(), vec![run_one(&root)]);
    let now = report::build("fake", Vec::new(), Vec::new(), vec![r]);
    let (diff, regressions) = report::compare(&prev, &now);
    assert_eq!(regressions, 1, "{diff}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn lint_rejects_absolute_paths_and_large_exact_ref_counts() {
    let bad = CASES
        .replace(
            r#"file = "Assets/Game.cs""#,
            r#"file = "C:/proj/Assets/Game.cs""#,
        )
        .replace("refs = 1", "refs = 25");
    let file = parse_case_file(Path::new("t.toml"), &bad).unwrap();
    let errs = cases::lint_case(&file.cases[0]);
    assert!(errs.iter().any(|e| e.contains("绝对路径")), "{errs:?}");
    assert!(errs.iter().any(|e| e.contains("≥ 20")), "{errs:?}");
}

fn repo_cases() -> Vec<cases::CaseFile> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/nav-cases");
    cases::load_dir(&dir).expect("读取 tests/nav-cases")
}

/// CI 上也跑：200 个用例、两个项目各 100 个、每类至少 5 个、条件编译两侧都有、无绝对路径。
#[test]
fn repository_case_files_are_complete_and_valid() {
    let files = repo_cases();
    let errs = cases::lint_all(&files, true);
    assert!(errs.is_empty(), "用例文件不合格：\n{}", errs.join("\n"));
    let total: usize = files.iter().map(|f| f.cases.len()).sum();
    assert_eq!(total, 200);
    let type_refs = files
        .iter()
        .flat_map(|f| &f.cases)
        .filter(|c| c.category == Category::TypeRef)
        .count();
    assert!(type_refs >= 10);
}

/// 设置了 Unity 测试项目时：用例锚点在磁盘上仍可定位（过期会打印出来，供重新整理）。
fn anchors_resolve(slot: rove_test_support::UnityProject, project: Project) {
    let root = rove_test_support::require_unity_project!(slot);
    struct NoQuery;
    impl NavSource for NoQuery {
        fn name(&self) -> &'static str {
            "none"
        }
        fn definition(&self, _: &QueryPos<'_>) -> Result<DefAnswer, SourceError> {
            Err(SourceError::Unsupported(String::new()))
        }
        fn references(&self, _: &QueryPos<'_>) -> Result<RefsAnswer, SourceError> {
            Err(SourceError::Unsupported(String::new()))
        }
    }
    let mut ws = Workspace::new(&root);
    let mut stale = Vec::new();
    let mut n = 0;
    for f in repo_cases().iter().filter(|f| f.project == project) {
        for c in &f.cases {
            n += 1;
            let r = run_case(project.key(), c, &mut ws, &NoQuery, locate::DEFAULT_WINDOW);
            if r.status == Status::Stale {
                stale.push(format!("{}：{}", c.id, r.reasons.join("；")));
            }
        }
    }
    eprintln!(
        "{} 项目：{n} 个用例，过期 {} 个",
        project.key(),
        stale.len()
    );
    for s in &stale {
        eprintln!("  过期 {s}");
    }
    // 项目在持续变化，允许少量过期；大面积过期说明用例需要按 AGENTS.md §8 的步骤重新整理。
    assert!(stale.len() * 10 <= n, "过期用例超过 10%，需要重新整理");
}

#[test]
fn medium_project_anchors_resolve() {
    anchors_resolve(rove_test_support::UnityProject::Medium, Project::Medium);
}

#[test]
fn large_project_anchors_resolve() {
    anchors_resolve(rove_test_support::UnityProject::Large, Project::Large);
}

/// L1 适配（#37）：光标处写出的类型名带上左侧限定与泛型实参个数。
#[test]
fn l1_adapter_reads_written_type_name() {
    let t = adapter::type_text_at;
    assert_eq!(t("var p = new Player();", 12).as_deref(), Some("Player"));
    assert_eq!(
        t("x = new Game.Player();", 13).as_deref(),
        Some("Game.Player")
    );
    assert_eq!(t("global::A.B c;", 10).as_deref(), Some("global::A.B"));
    assert_eq!(
        t("Dictionary<int, List<string>> d;", 2).as_deref(),
        Some("Dictionary<,>")
    );
    assert_eq!(t("typeof(Foo<>)", 7).as_deref(), Some("Foo<>"));
    assert_eq!(t("if (a < b) {", 4).as_deref(), Some("a"));
    assert_eq!(t("a + b", 2), None);
}
