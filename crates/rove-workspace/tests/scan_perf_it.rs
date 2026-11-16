//! Entry-point tests for the scan fast paths (#210).
//!
//! #210 made the full scan cheaper in four places: the workspace-relative path of a **file** is
//! built once (in the walker's visitor) instead of twice (visitor plus `filter_entry`); the
//! read-only and reparse-point attributes come from the metadata the directory enumeration
//! already returned instead of an extra `stat`; the publisher folds each path into one identity
//! key instead of three; and the final sort compares prepared keys instead of re-folding inside
//! the comparator. None of that is allowed to change **what** a scan produces, so these tests
//! pin the observable result rather than the speed:
//!
//! - directories are still *pruned*, not merely filtered (an excluded subtree is never read);
//! - file-level exclusions still apply even though files are now decided in the visitor;
//! - `files.readonlyFromPermissions` still sees the on-disk attribute;
//! - the order of `files()` / `children()` is still the case-folded identity order, including
//!   names where the folded form is not the ASCII lowercase one;
//! - on a real Unity project the whole entry set matches an independent walk written here.
//!
//! The speed itself is judged by `cargo xtask bench --check-budgets`
//! (`workspace/full_scan_large` against `perf/budgets.toml`), not by these tests: AGENTS §10
//! keeps absolute performance judgement out of the test suite.

use rove_base::WorkspacePath;
use rove_workspace::{default_settings, Folder, RuleSet, Workspace, WorkspaceConfig};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

fn config() -> WorkspaceConfig {
    WorkspaceConfig::from_settings(&default_settings())
        .expect("registered defaults")
        .without_watching()
}

fn exclude_rules(patterns: &[(&str, bool)]) -> RuleSet {
    let map: std::collections::BTreeMap<String, rove_base::Value> = patterns
        .iter()
        .map(|(pattern, value)| ((*pattern).to_string(), rove_base::Value::Bool(*value)))
        .collect();
    RuleSet::from_value(&rove_base::Value::Object(std::sync::Arc::new(map)))
        .expect("valid glob patterns")
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write file");
}

fn all_files(folder: &Folder) -> Vec<String> {
    folder
        // The large project has more than 150k visible files; take the whole list, not a page.
        .files(0..usize::MAX)
        .expect("files page")
        .items
        .iter()
        .map(|entry| entry.path.as_str().to_string())
        .collect()
}

/// Files are classified in the walker's visitor now, not in `filter_entry`. A file-level glob
/// must still exclude the file, and an excluded file must not be counted as "skipped" either.
#[test]
fn file_level_exclusions_still_apply() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("Assets/Keep.cs"), "class Keep {}\n");
    write(&dir.path().join("Assets/Drop.tmp"), "junk\n");
    write(&dir.path().join("Assets/Deep/Drop.bak"), "junk\n");
    write(&dir.path().join("Assets/Deep/Keep.cs"), "class Deep {}\n");

    let mut config = config();
    config.exclude = exclude_rules(&[("**/*.{tmp,bak}", true)]);
    let folder = Folder::scan_root(dir.path(), config).expect("scan");

    assert_eq!(
        all_files(&folder),
        ["Assets/Deep/Keep.cs", "Assets/Keep.cs"]
    );
    assert_eq!(folder.skipped(), 0, "an excluded file is not a skipped one");
}

/// Directories must be *pruned*: an excluded directory's contents are never read, so nothing
/// below it can appear no matter how deep it is, and the directory itself stays invisible.
#[test]
fn an_excluded_directory_subtree_is_never_visible() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("Assets/Keep.cs"), "class Keep {}\n");
    for depth in 0..6 {
        let mut nested = dir.path().join("Library");
        for level in 0..depth {
            nested = nested.join(format!("Level{level}"));
        }
        write(&nested.join("Buried.cs"), "class Buried {}\n");
    }

    let folder = Folder::scan_root(dir.path(), config()).expect("scan");
    assert_eq!(all_files(&folder), ["Assets/Keep.cs"]);
    assert!(
        folder
            .children(Some(&WorkspacePath::new("Library").expect("path")), 0..16)
            .is_err(),
        "an excluded directory must not even have a bucket"
    );
}

/// The read-only attribute now comes from the metadata the walk already had. It must still be
/// reported, and `files.readonlyExclude` must still cancel it.
#[test]
fn the_read_only_attribute_is_read_from_the_walk() {
    let dir = tempfile::tempdir().expect("temp dir");
    let locked = dir.path().join("Assets/Locked.cs");
    let open = dir.path().join("Assets/Open.cs");
    write(&locked, "class Locked {}\n");
    write(&open, "class Open {}\n");
    let mut permissions = std::fs::metadata(&locked).expect("metadata").permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&locked, permissions).expect("set read-only");

    let mut config = config();
    config.readonly.from_permissions = true;
    let folder = Folder::scan_root(dir.path(), config).expect("scan");

    let locked_rules = folder
        .rules(&WorkspacePath::new("Assets/Locked.cs").expect("path"))
        .expect("known path");
    let open_rules = folder
        .rules(&WorkspacePath::new("Assets/Open.cs").expect("path"))
        .expect("known path");
    assert!(
        !locked_rules.read_only.is_empty(),
        "the read-only attribute must survive the walk: {locked_rules:?}"
    );
    assert!(open_rules.read_only.is_empty(), "{open_rules:?}");

    // Leave the file writable so the temp directory can be removed on Windows.
    let mut permissions = std::fs::metadata(&locked).expect("metadata").permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&locked, permissions).expect("clear read-only");
}

/// The final sort compares prepared identity keys instead of folding inside the comparator.
/// Names whose folded form is not simply the ASCII lowercase one must still land in the same
/// place: `KELVIN SIGN` folds to the ASCII `k`, `İ` folds to `i` + U+0307.
#[test]
fn the_sort_order_is_the_case_folded_identity_order() {
    let dir = tempfile::tempdir().expect("temp dir");
    let names = [
        "Zebra.cs",
        "apple.cs",
        "Apple2.cs",
        "\u{212A}elvin.cs",
        "kelvin2.cs",
        "\u{130}stanbul.cs",
        "资源.cs",
    ];
    let mut created = Vec::new();
    for name in names {
        let path = dir.path().join("Assets").join(name);
        if std::fs::create_dir_all(path.parent().expect("parent")).is_err() {
            continue;
        }
        match std::fs::write(&path, "class X {}\n") {
            Ok(()) => created.push(format!("Assets/{name}")),
            Err(error) => eprintln!("已跳过：无法创建 {name}（{error}）"),
        }
    }

    let folder = Folder::scan_root(dir.path(), config()).expect("scan");
    let scanned = all_files(&folder);

    let mut expected = created;
    expected.sort_by_key(|path| fold(path));
    assert_eq!(scanned, expected);
}

/// Reference folding: the Unicode default lowercase mapping applied character by character,
/// written out independently of the crate's own helper.
fn fold(path: &str) -> String {
    #[cfg(windows)]
    {
        let mut folded = String::with_capacity(path.len());
        for character in path.chars() {
            folded.extend(character.to_lowercase());
        }
        folded
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

/// Whole-project equivalence on a real Unity project: the scan reports exactly what an
/// independent recursive `read_dir` walk written here finds under the same documented rules
/// (default exclusions, `.gitignore` never consulted, junctions not followed).
///
/// A real project is a *live* directory — an open Unity, a running import or a build writes to
/// it while the test runs, and two walks seconds apart genuinely differ. So the reference walk
/// runs **before and after** the scan and the assertion is made only about what cannot be
/// explained by that churn: every path both reference walks agree on must have been scanned, and
/// the scan must not report a path neither walk ever saw. A real defect in the walker (a missed
/// subtree, an invented path, a wrongly applied rule) breaks both halves; concurrent writes
/// break neither.
///
/// Skipped without `ROVE_TEST_UNITY_LARGE` (AGENTS §8); the elapsed time is printed for the
/// record but never asserted — the budget lives in `perf/budgets.toml`.
#[test]
fn a_real_project_scan_matches_an_independent_walk() {
    let Some(root) = rove_test_support::UnityProject::Large.path() else {
        eprintln!(
            "已跳过：a_real_project_scan_matches_an_independent_walk 需要环境变量 {}",
            rove_test_support::UnityProject::Large.env_var()
        );
        return;
    };

    let config = config();
    let exclude = config.exclude.clone();
    let canonical = std::fs::canonicalize(&root).expect("canonical root");

    let mut before = BTreeSet::new();
    reference_walk(&canonical, &canonical, &exclude, &mut before);

    let start = Instant::now();
    let folder = Folder::scan_root(&root, config).expect("scan the large project");
    let elapsed = start.elapsed();
    let scanned: BTreeSet<String> = all_files(&folder).into_iter().collect();

    let mut after = BTreeSet::new();
    reference_walk(&canonical, &canonical, &exclude, &mut after);

    assert!(!scanned.is_empty(), "the project must contain files");

    let stable: BTreeSet<&String> = before.intersection(&after).collect();
    let missing: Vec<&&String> = stable
        .iter()
        .filter(|path| !scanned.contains(**path))
        .take(10)
        .collect();
    assert!(
        missing.is_empty(),
        "独立遍历前后都存在、扫描却没报告的路径：{missing:?}"
    );

    let invented: Vec<&String> = scanned
        .iter()
        .filter(|path| !before.contains(*path) && !after.contains(*path))
        .take(10)
        .collect();
    assert!(
        invented.is_empty(),
        "扫描报告了两次独立遍历都没见过的路径：{invented:?}"
    );

    let churn = before.symmetric_difference(&after).count();
    eprintln!(
        "[#210] 大型项目全量扫描：{} 个文件（稳定集 {}，两次遍历间变动 {churn}），{elapsed:?}；\
         机器负载影响绝对值，判定见 perf/budgets.toml",
        scanned.len(),
        stable.len()
    );
}

/// Independent recursive walk: default exclusions by workspace-relative path, no ignore files,
/// no symlink/junction traversal, non-UTF-8 names dropped. Deliberately serial and naive.
fn reference_walk(root: &Path, directory: &Path, exclude: &RuleSet, into: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let Some(relative) = relative(root, &path) else {
            continue;
        };
        if exclude.matches_self(&relative) == Some(true) {
            continue;
        }
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if is_reparse_point(&path) {
                continue;
            }
            reference_walk(root, &path, exclude, into);
        } else if file_type.is_file() {
            into.insert(relative);
        }
    }
}

fn relative(root: &Path, absolute: &Path) -> Option<String> {
    let relative = absolute.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(name) => parts.push(name.to_str()?.to_string()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(windows)]
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_reparse_point(_: &Path) -> bool {
    false
}
