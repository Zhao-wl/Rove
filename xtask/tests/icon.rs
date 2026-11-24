use std::process::Command;

#[test]
fn icon_check_accepts_the_committed_icon() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["icon", "--check"])
        .output()
        .expect("启动 xtask");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
