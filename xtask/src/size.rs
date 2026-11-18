//! `cargo xtask check-size`：报告主程序、发布包和嵌入资源的体积。
//!
//! 每个 PR 报告两个数：
//! - `rove.exe` 的体积与相对基线（M0 实测 6.2 MB）的变化；
//! - **发布包**（`packaging/package-contents.toml` 里登记的全部文件）的体积，
//!   以及嵌入 `rove.exe` 的资产分项（主题、语言配置、codicons、Markdown 预览资源……）。
//!
//! C# 索引组件是独立的可选包，使用 `--component` 单独报告。

use std::fs;
use std::path::PathBuf;

use crate::budgets::Budgets;
use crate::pack::Contents;
use crate::release;
use crate::util;

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let budgets = Budgets::load_default()?;
    let size_baseline = budgets
        .size
        .ok_or("perf/budgets.toml 缺少 [size] 段（体积报告需要 baseline_bytes）")?;

    let exe = util::flag(args, "--binary")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            util::target_dir()
                .join("release")
                .join(if cfg!(windows) { "rove.exe" } else { "rove" })
        });
    let metadata = fs::metadata(&exe).map_err(|err| {
        format!(
            "找不到 {}：{err}\n先构建：cargo build --release -p rove-app",
            exe.display()
        )
    })?;
    let size = metadata.len();

    let baseline = match util::flag(args, "--baseline-bytes") {
        Some(raw) => raw.parse::<u64>()?,
        None => size_baseline.baseline_bytes,
    };

    println!("体积报告");
    println!("  文件：{}", exe.display());
    println!(
        "  体积：{}（{} B）",
        util::human_bytes(size),
        util::thousands(size)
    );
    println!(
        "  基线：{}（M0 实测，{} B）→ 变化 {:+.2}%（{:+.0} B）",
        util::human_bytes(baseline),
        util::thousands(baseline),
        percent_delta(size, baseline),
        size as i64 - baseline as i64
    );
    // ---------------------------------------------------------------- 发布包
    println!("\n发布包（packaging/package-contents.toml，不含 C# 索引组件）");
    let contents = Contents::load_default()?;
    let files = contents.resolve(Some(&exe))?;
    let package = release::print_package_report(&files, &contents)?;

    // 单独报告可选的 C# 索引组件包。
    if let Some(component) = util::flag(args, "--component") {
        let component = PathBuf::from(component);
        match fs::metadata(&component) {
            Ok(meta) => println!(
                "\nC# 索引组件包（可选）：{} = {}",
                component.display(),
                util::human_bytes(meta.len())
            ),
            Err(err) => println!(
                "\n::warning::读不到索引组件包 {}：{err}",
                component.display()
            ),
        }
    }

    // ::notice:: 会出现在 PR 的检查注解里，让体积变化在 PR 页面就能看到。
    println!(
        "\n::notice::rove.exe {}（{} B，相对 M0 基线 {:+.2}%）；发布包 {}（{} B）",
        util::human_bytes(size),
        util::thousands(size),
        percent_delta(size, baseline),
        util::human_bytes(package),
        util::thousands(package)
    );

    Ok(0)
}

fn percent_delta(actual: u64, baseline: u64) -> f64 {
    if baseline == 0 {
        return 0.0;
    }
    (actual as f64 / baseline as f64 - 1.0) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_larger_than_the_former_limit_is_reported_successfully() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("rove.exe");
        fs::File::create(&exe).unwrap().set_len(40_000_000).unwrap();

        let args = ["--binary".into(), exe.display().to_string()];
        assert_eq!(run(&args).unwrap(), 0);

        fs::remove_file(exe).unwrap();
        assert!(run(&args).is_err());
    }
}
