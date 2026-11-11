//! C# 导航准确率回归运行器（issue #39）。
//!
//! 读取 `tests/nav-cases/*.toml`，在 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 指向的
//! 真实 Unity 项目上逐例查询，输出分类准确率、失败详情，并写出机器可读的结果文件。
//! 环境变量未设置时跳过并说明原因，退出码为 0。
//!
//! 用法见 [`USAGE`]（`--help` 打印同一份）。
//!
//! 结构：`cases.rs`（用例格式）→ `locate.rs`（锚点定位）→ `adapter.rs`（**唯一接触查询接口的
//! 地方**）→ `check.rs`（判定）→ `report.rs`（统计与输出）。

mod adapter;
mod cases;
mod check;
mod locate;
mod report;

use adapter::{L1Decls, L2Index, NavSource};
use cases::{CaseFile, Project};
use check::{run_case, Status, Workspace};
use rove_test_support::UnityProject;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
cargo run -p rove-lang-cs --example nav_check -- [选项]
  --project medium|large|all   只跑某个项目（默认 all）
  --source l2|l1               查询来源：完整语义索引 / L1 声明索引（默认 l2）
  --index-medium <idx>         中型项目的索引（默认 $ROVE_NAV_INDEX_MEDIUM，再默认 target/nav-check/medium.idx）
  --index-large <idx>          大型项目的索引（默认 $ROVE_NAV_INDEX_LARGE，再默认 target/nav-check/large.idx）
  --build                      先用 tools/rove-indexer-cs 重建索引（需先 dotnet build -c Release）
  --cases <dir>                用例目录（默认 tests/nav-cases）
  --out <dir>                  结果目录（默认 target/nav-check）
  --baseline <json>            与上一次的结果文件对比，列出状态变化
  --fail-on-regression         与 --baseline 合用：有“通过 → 失败”时退出码为 1
  --window <n>                 锚点重定位的搜索范围（上下各 n 行，默认 40）
  --lint                       只做静态检查与锚点检查，不查询";

struct Opts {
    projects: Vec<Project>,
    source: String,
    index: [Option<PathBuf>; 2],
    build: bool,
    cases: PathBuf,
    out: PathBuf,
    baseline: Option<PathBuf>,
    fail_on_regression: bool,
    window: u32,
    lint: bool,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse_args() -> Result<Opts, String> {
    let root = repo_root();
    let mut o = Opts {
        projects: Project::ALL.to_vec(),
        source: "l2".to_owned(),
        index: [None, None],
        build: false,
        cases: root.join("tests/nav-cases"),
        out: root.join("target/nav-check"),
        baseline: None,
        fail_on_regression: false,
        window: locate::DEFAULT_WINDOW,
        lint: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("{a} 需要一个参数"));
        match a.as_str() {
            "--project" => {
                let v = val()?;
                o.projects = match v.as_str() {
                    "all" => Project::ALL.to_vec(),
                    p => vec![Project::parse(p).ok_or_else(|| format!("未知项目 {p}"))?],
                };
            }
            "--source" => {
                o.source = val()?;
                if !matches!(o.source.as_str(), "l2" | "l1") {
                    return Err(format!("未知查询来源 {}（l2 / l1）", o.source));
                }
            }
            "--index-medium" => o.index[0] = Some(val()?.into()),
            "--index-large" => o.index[1] = Some(val()?.into()),
            "--build" => o.build = true,
            "--cases" => o.cases = val()?.into(),
            "--out" => o.out = val()?.into(),
            "--baseline" => o.baseline = Some(val()?.into()),
            "--fail-on-regression" => o.fail_on_regression = true,
            "--window" => o.window = val()?.parse().map_err(|_| "--window 需要整数")?,
            "--lint" => o.lint = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("未知参数 {a}（--help 查看用法）")),
        }
    }
    Ok(o)
}

fn slot(p: Project) -> (UnityProject, usize, &'static str) {
    match p {
        Project::Medium => (UnityProject::Medium, 0, "ROVE_NAV_INDEX_MEDIUM"),
        Project::Large => (UnityProject::Large, 1, "ROVE_NAV_INDEX_LARGE"),
    }
}

fn index_path(o: &Opts, p: Project) -> PathBuf {
    let (_, i, env) = slot(p);
    o.index[i]
        .clone()
        .or_else(|| std::env::var_os(env).map(PathBuf::from))
        .unwrap_or_else(|| repo_root().join(format!("target/nav-check/{}.idx", p.key())))
}

fn build_index(project_root: &Path, idx: &Path) -> Result<(), String> {
    let exe = repo_root().join("tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.exe");
    if !exe.is_file() {
        return Err(format!(
            "找不到索引进程 {}，先运行 dotnet build tools/rove-indexer-cs -c Release",
            exe.display()
        ));
    }
    if let Some(dir) = idx.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    eprintln!("正在建索引 → {} …", idx.display());
    let status = std::process::Command::new(&exe)
        .arg(project_root)
        .arg(idx)
        .status()
        .map_err(|e| format!("启动索引进程失败：{e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("索引进程退出码 {status}"))
}

fn open_source(o: &Opts, p: Project, root: &Path) -> Result<Box<dyn NavSource>, String> {
    match o.source.as_str() {
        "l1" => Ok(Box::new(L1Decls::open(root)?)),
        _ => {
            let idx = index_path(o, p);
            if o.build {
                build_index(root, &idx)?;
            } else if !idx.is_file() {
                return Err(format!(
                    "找不到索引 {}（加 --build 自动生成，或用 --index-{} 指定）",
                    idx.display(),
                    p.key()
                ));
            }
            Ok(Box::new(L2Index::open(&idx)?))
        }
    }
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("nav_check: {e}");
            return ExitCode::from(2);
        }
    };
    let files = match cases::load_dir(&o.cases) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("nav_check: {e}");
            return ExitCode::from(2);
        }
    };
    let errs = cases::lint_all(&files, false);
    if !errs.is_empty() {
        for e in &errs {
            eprintln!("用例文件错误：{e}");
        }
        return ExitCode::from(2);
    }

    let mut results = Vec::new();
    let mut skipped = Vec::new();
    let mut sources = Vec::new();
    for &p in &o.projects {
        let (unity, _, _) = slot(p);
        let Some(root) = unity.path() else {
            skipped.push(format!(
                "{}：未设置环境变量 {} 或目录不存在",
                p.key(),
                unity.env_var()
            ));
            continue;
        };
        let mine: Vec<&CaseFile> = files.iter().filter(|f| f.project == p).collect();
        let mut ws = Workspace::new(&root);
        if o.lint {
            for f in &mine {
                for c in &f.cases {
                    let r = run_case(p.key(), c, &mut ws, &NoQuery, o.window);
                    if r.status == Status::Stale || r.relocated {
                        println!(
                            "{} {}：{}",
                            c.id,
                            if r.relocated {
                                "已重定位"
                            } else {
                                "过期"
                            },
                            r.reasons.join("；")
                        );
                    }
                }
            }
            continue;
        }
        let source = match open_source(&o, p, &root) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("已跳过：{} 项目 —— {e}", p.key());
                skipped.push(format!("{}：{e}", p.key()));
                continue;
            }
        };
        let desc = format!("{}：{}", p.key(), source.describe());
        println!("{desc}");
        sources.push(desc);
        let mut newer = 0;
        for f in &mine {
            for c in &f.cases {
                let r = run_case(p.key(), c, &mut ws, source.as_ref(), o.window);
                newer += usize::from(r.source_newer_than_index);
                results.push(r);
            }
        }
        if newer > 0 {
            eprintln!(
                "警告：{} 项目有 {newer} 个用例所在文件比索引新，结果可能受影响（加 --build 重建索引）",
                p.key()
            );
        }
    }
    if o.lint {
        println!("静态检查与锚点检查完成。");
        return ExitCode::SUCCESS;
    }
    if results.is_empty() {
        println!(
            "nav_check：没有运行任何用例（{}），退出码 0。",
            skipped.join("；")
        );
        return ExitCode::SUCCESS;
    }

    let report = report::build(&o.source, sources, skipped, results);
    println!("{}", report::summary_table(&report));
    let failed = report::details(&report, &[Status::Fail, Status::Stale]);
    if !failed.is_empty() {
        println!("\n## 失败与过期用例\n\n{failed}");
    }

    let mut code = ExitCode::SUCCESS;
    if let Some(b) = &o.baseline {
        match std::fs::read_to_string(b)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<report::RunReport>(&t).map_err(|e| e.to_string()))
        {
            Ok(prev) => {
                let (diff, regressions) = report::compare(&prev, &report);
                println!("\n## 与 {} 对比\n\n{diff}", b.display());
                if regressions > 0 && o.fail_on_regression {
                    eprintln!("有 {regressions} 个用例由通过变为失败");
                    code = ExitCode::from(1);
                }
            }
            Err(e) => eprintln!("读取基线 {} 失败：{e}", b.display()),
        }
    }

    let _ = std::fs::create_dir_all(&o.out);
    let json = o.out.join(format!("nav-check-{}.json", o.source));
    let md = o.out.join(format!("nav-check-{}.md", o.source));
    let title = format!("C# 导航准确率报告（{}）", o.source);
    let written = std::fs::write(
        &json,
        serde_json::to_string_pretty(&report).expect("序列化"),
    )
    .and_then(|_| std::fs::write(&md, report::markdown(&report, &title)));
    match written {
        Ok(()) => println!("\n结果文件：{}\n报告：{}", json.display(), md.display()),
        Err(e) => eprintln!("写结果文件失败：{e}"),
    }
    code
}

/// `--lint` 用：不查询，只走定位流程。
struct NoQuery;

impl NavSource for NoQuery {
    fn name(&self) -> &'static str {
        "none"
    }
    fn definition(
        &self,
        _: &adapter::QueryPos<'_>,
    ) -> Result<adapter::DefAnswer, adapter::SourceError> {
        Err(adapter::SourceError::Unsupported(String::new()))
    }
    fn references(
        &self,
        _: &adapter::QueryPos<'_>,
    ) -> Result<adapter::RefsAnswer, adapter::SourceError> {
        Err(adapter::SourceError::Unsupported(String::new()))
    }
}
