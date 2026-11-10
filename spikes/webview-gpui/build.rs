use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
fn git(args: &[&str]) -> String {
    let result = Command::new("git")
        .args(args)
        .output()
        .expect("git required for build provenance");
    assert!(result.status.success(), "git provenance command failed");
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}
fn main() {
    let files = [
        "build.rs",
        "Cargo.toml",
        "Cargo.lock",
        "src/main.rs",
        "assets/bridge.js",
        "assets/index.html",
        "assets/local.svg",
    ];
    let mut source = Vec::new();
    for file in files {
        println!("cargo:rerun-if-changed={file}");
        source.extend_from_slice(file.as_bytes());
        source.push(0);
        source.extend_from_slice(&fs::read(file).unwrap());
        source.push(0);
    }
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&source).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let fingerprint = String::from_utf8(result.stdout).unwrap();
    println!(
        "cargo:rustc-env=ROVE_SOURCE_GIT_BLOB={}",
        fingerprint.trim()
    );
    println!(
        "cargo:rustc-env=ROVE_BUILD_COMMIT={}",
        git(&["rev-parse", "HEAD"])
    );
    println!(
        "cargo:rustc-env=ROVE_BUILD_DIRTY={}",
        !git(&["status", "--porcelain", "--", "."]).is_empty()
    );
    println!(
        "cargo:rustc-env=ROVE_BUILD_UNIX_SECONDS={}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    println!(
        "cargo:rerun-if-changed={}",
        git(&["rev-parse", "--git-path", "HEAD"])
    );
    let branch = git(&["symbolic-ref", "HEAD"]);
    println!(
        "cargo:rerun-if-changed={}",
        git(&["rev-parse", "--git-path", &branch])
    );
}
