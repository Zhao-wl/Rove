//! 着色黄金测试的运行器（#11）。
//!
//! 用 VS Code `vscode-colorize-tests` 的测试数据，逐字符比对 Rove 算出的前景色：
//! fixture → #9 的 tree-sitter 高亮 → #10 的主题引擎 → 与 `colorize-results` 的期望颜色比较。
//!
//! ```powershell
//! cargo run -p rove-syntax --example golden_colors                       # 跑整个矩阵并写报告
//! cargo run -p rove-syntax --example golden_colors -- --language csharp  # 只跑一种语言
//! cargo run -p rove-syntax --example golden_colors -- --fixture test.cs --theme dark_modern --chars
//! ```
//!
//! 退出码：有（语言 × 受阈值约束的主题）低于阈值时为 1，其余为 0。判定由
//! `cargo test -p rove-syntax --test golden_colors` 在 CI 上执行，这里只是方便本地排查。

// 这几个模块同时被 `tests/golden_colors.rs` 用 `#[path]` 引入，两边用到的接口不完全相同。
#![allow(dead_code)]

mod check;
mod config;
mod data;
mod report;

use std::path::PathBuf;
use std::process::ExitCode;

struct Args {
    filter: check::Filter,
    out: PathBuf,
    top: usize,
    chars: bool,
    write: bool,
}

fn usage() -> &'static str {
    "用法：cargo run -p rove-syntax --example golden_colors -- [选项]\n\
     \n\
     --language <ID>   只跑这一种语言（阈值文件里的键）\n\
     --fixture <文件>  只跑这一个 fixture（colorize-fixtures 下的文件名）\n\
     --theme <键>      只跑这一个主题（colorize-results 里的主题键）\n\
     --chars           逐字符输出期望颜色与实际颜色（建议配合 --fixture 与 --theme）\n\
     --out <目录>      报告输出目录，默认 out/golden\n\
     --top <N>         摘要里列出的差异条数，默认 20\n\
     --no-write        不写报告文件，只打印摘要\n"
}

fn parse() -> Result<Args, String> {
    let mut args = Args {
        filter: check::Filter::default(),
        out: PathBuf::from("out/golden"),
        top: 20,
        chars: false,
        write: true,
    };
    let mut rest = std::env::args().skip(1);
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().ok_or(format!("{arg} 后面要跟一个值"));
        match arg.as_str() {
            "--language" => args.filter.language = Some(value()?),
            "--fixture" => args.filter.fixture = Some(value()?),
            "--theme" => args.filter.theme = Some(value()?),
            "--out" => args.out = PathBuf::from(value()?),
            "--top" => {
                args.top = value()?
                    .parse()
                    .map_err(|e| format!("--top 要是个整数：{e}"))?
            }
            "--chars" => args.chars = true,
            "--no-write" => args.write = false,
            "-h" | "--help" => return Err(usage().into()),
            other => return Err(format!("不认识的参数 {other}\n\n{}", usage())),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args = parse()?;
    let root = config::repo_root();
    let config = config::Config::load(&root)?;

    if args.chars {
        return dump_chars(&root, &config, &args.filter).map(|_| ExitCode::SUCCESS);
    }

    let runs = check::run_matrix(&root, &config, &args.filter)?;
    println!("{}", report::rate_table(&config, &runs));

    if args.write {
        let dir = if args.out.is_absolute() {
            args.out.clone()
        } else {
            root.join(&args.out)
        };
        let written = report::write(&dir, &config, &runs, args.top)?;
        println!("报告：{}", written.markdown.display());
        println!("明细：{}", written.json.display());
    }

    let failures = check::failures(&config, &runs);
    if failures.is_empty() {
        println!("受阈值约束的（语言 × 主题）全部达标。");
        return Ok(ExitCode::SUCCESS);
    }
    for (language, theme, rate, threshold) in &failures {
        println!(
            "未达标：{language} × {theme} = {:.2}% < {:.2}%",
            rate * 100.0,
            threshold * 100.0
        );
    }
    Ok(ExitCode::FAILURE)
}

/// `--chars`：对一个 fixture + 一个主题，逐字符打印期望颜色与实际颜色。
fn dump_chars(
    root: &std::path::Path,
    config: &config::Config,
    filter: &check::Filter,
) -> Result<(), String> {
    let themes = match &filter.theme {
        Some(theme) => vec![theme.clone()],
        None => config.enforced_themes.clone(),
    };
    for language in &config.languages {
        if filter
            .language
            .as_ref()
            .is_some_and(|want| want != &language.id)
        {
            continue;
        }
        for name in &language.fixtures {
            if filter.fixture.as_ref().is_some_and(|want| want != name) {
                continue;
            }
            let fixture = data::load(root, &language.id, name)?;
            let scopes = check::highlight(&fixture.text, &language.id)?;
            for key in &themes {
                let theme = check::load(key)?;
                println!("# {} / {} / {}", language.id, name, key);
                println!("行:列\t字符\t期望\t实际\t结果\tVS Code scope\tRove scope");
                for c in check::characters(&fixture, &scopes, key, &theme) {
                    println!(
                        "{}:{}\t{:?}\t{}\t{}\t{}\t{}\t{}",
                        c.line,
                        c.column,
                        c.ch,
                        rove_theme::format_hex(c.expected),
                        rove_theme::format_hex(c.actual),
                        if c.matches() { "一致" } else { "差异" },
                        c.vscode_scopes,
                        c.rove_scopes,
                    );
                }
            }
        }
    }
    Ok(())
}
