#[path = "../build.rs"]
mod build_script;

use std::{env, fs, process::Command};

const CHILD: &str = "ROVE_BUILD_METADATA_TEST_CHILD";
const TEST: &str = "build_metadata_uses_runtime_manifest_dir";
const GPUI_VERSION: &str = "0.99.363";

#[test]
fn build_metadata_uses_runtime_manifest_dir() {
    if env::var_os(CHILD).is_some() {
        build_script::main();
        return;
    }

    let workspace = tempfile::tempdir().unwrap();
    let manifest = workspace.path().join("Cargo.toml");
    let crate_dir = workspace.path().join("crates/rove-ui");
    let out_dir = workspace.path().join("out");
    fs::create_dir_all(crate_dir.join("assets/markdown")).unwrap();
    fs::create_dir(&out_dir).unwrap();
    fs::write(
        &manifest,
        format!("[workspace.dependencies]\ngpui = {{ version = \"={GPUI_VERSION}\" }}\n"),
    )
    .unwrap();

    let release = [
        ("ROVE_BUILD_COMMIT", "0123456789abcdef"),
        ("ROVE_BUILD_DATE", "2000-01-01"),
        ("ROVE_BUILD_CHANNEL", "test-release"),
    ];
    let output = Command::new(env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .current_dir(&crate_dir)
        .env(CHILD, "1")
        .env("CARGO_MANIFEST_DIR", &crate_dir)
        .env("OUT_DIR", out_dir)
        .envs(release)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "build script child failed ({})\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let assert_directive = |expected: String| {
        assert!(
            stdout.lines().any(|line| line == expected),
            "missing directive {expected:?} in:\n{stdout}"
        );
    };
    assert_directive(format!("cargo:rerun-if-changed={}", manifest.display()));
    assert_directive(format!("cargo:rustc-env=ROVE_GPUI_VERSION={GPUI_VERSION}"));
    for (key, value) in release {
        assert_directive(format!("cargo:rerun-if-env-changed={key}"));
        assert_directive(format!("cargo:rustc-env={key}={value}"));
    }
}
